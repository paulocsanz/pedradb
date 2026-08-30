//! Adversarial campaigns on the rocksdb-compat layer (our existing suite style).
//!
//! - Model-checked random ops (put/delete/batch/delete_range/scan) vs BTreeMap.
//! - `FailingEnv` fault injection mid-stream (dead disk, sync-fail, short write):
//!   a failed op must be a visible error, never a silent divergence.
//! - Reopen on a healed Env: durable content must be a prefix-consistent subset
//!   of the model (Ok'd writes may be missing only if their fsync failed), and
//!   every durable value must equal the model value (no silent-wrong).

use pedradb_sim::{FailingEnv, FaultKind};
use rocksdb_compat::{Direction, IteratorMode, Options, WriteBatch, DB};
use std::collections::BTreeMap;

type Model = BTreeMap<Vec<u8>, Vec<u8>>;

/// Post-crash acceptable durable values per key: `None` = must be absent,
/// `Some(v)` = value is acceptable. An **Ok** (synced) write pins exactly one
/// outcome; an **Err** write unions its outcome with the previous ones, because
/// whether the bytes landed depends on which fallible op (write vs sync) the
/// fault hit — the harness cannot distinguish, so it admits both.
type Accept = BTreeMap<Vec<u8>, std::collections::BTreeSet<Option<Vec<u8>>>>;

fn admit(acc: &mut Accept, key: &[u8], outcome: Option<Vec<u8>>, certain: bool) {
    let e = acc.entry(key.to_vec()).or_insert_with(|| {
        // Basis: a key never certainly written may legitimately be absent.
        let mut s = std::collections::BTreeSet::new();
        s.insert(None);
        s
    });
    if certain {
        e.clear();
    }
    e.insert(outcome);
}

fn admit_range_delete(acc: &mut Accept, a: &[u8], b: &[u8], certain: bool) {
    let covered: Vec<Vec<u8>> = acc
        .range(a.to_vec()..b.to_vec())
        .map(|(k, _)| k.clone())
        .collect();
    for k in covered {
        admit(acc, &k, None, certain);
    }
}

/// Adversarial campaigns test **Ok = durable**. Drop-in `Options::sync`
/// defaults to Rocks-shaped async (RFC-0054); these tests opt into G1.
fn g1_opts() -> Options {
    let mut o = Options::new();
    o.set_sync(true);
    o
}

fn tmp(tag: &str, seed: u64) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("rdbcompat-adv-{tag}-{seed}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn xorshift(rng: &mut u64) -> u64 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    *rng
}

/// Full-scan compare (compat iterator Start..End) — silent-wrong detector.
fn scan_all(db: &DB<pedradb_sim::FailingEnv>) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut out = Vec::new();
    let mut it = db.iterator(IteratorMode::Start).expect("iterator");
    while it.valid() {
        out.push((it.key().to_vec(), it.value().to_vec()));
        it.next();
        if out.len() > 4096 {
            break;
        }
    }
    out
}

fn run_campaign(seed: u64, kind: Option<FaultKind>, ops: usize) -> (Model, Model, u64, u64) {
    let tag = match kind {
        None => "camp-io",
        Some(FaultKind::SyncFail) => "camp-sync",
        Some(FaultKind::ShortWrite) => "camp-short",
        Some(_) => "camp-other",
    };
    let dir = tmp(tag, seed);
    // Open healthy (from_seed budget would fire during open's own I/O), then arm
    // the fault schedule on the shared Rc state via the env clone.
    let env = FailingEnv::passing();
    let db = DB::open_cf_with_env(&g1_opts(), &dir, &["raft"], env.clone()).expect("open");
    let kind = kind.unwrap_or(FaultKind::IoError);
    env.arm_with_kind(FailingEnv::seed_to_fail_after(seed), false, kind);

    let mut model: Model = BTreeMap::new();
    let mut accept: Accept = BTreeMap::new();
    let mut raft_accept: Accept = BTreeMap::new();
    let mut rng = 0xC0DE_0000_u64 ^ seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut errs = 0u64;
    let mut oks = 0u64;

    for i in 0..ops {
        let op = xorshift(&mut rng) % 10;
        let k = format!("k{:03}", xorshift(&mut rng) % 40).into_bytes();
        match op {
            0..=3 => {
                let v = format!("v{}", xorshift(&mut rng) % 1000).into_bytes();
                match db.put(&k, &v) {
                    Ok(()) => {
                        model.insert(k.clone(), v.clone());
                        admit(&mut accept, &k, Some(v), true);
                        oks += 1;
                    }
                    Err(_) => {
                        admit(&mut accept, &k, Some(v), false);
                        errs += 1;
                    }
                }
            }
            4 => match db.delete(&k) {
                Ok(()) => {
                    model.remove(&k);
                    admit(&mut accept, &k, None, true);
                    oks += 1;
                }
                Err(_) => {
                    admit(&mut accept, &k, None, false);
                    errs += 1;
                }
            },
            5..=6 => {
                // Atomic batch: on Ok the effects are certain; on Err the batch
                // landed wholly or not at all — per-key both outcomes stay valid.
                let k2 = format!("k{:03}", xorshift(&mut rng) % 40).into_bytes();
                let v1 = format!("b{}a", i).into_bytes();
                let v2 = format!("b{}b", i).into_bytes();
                let dk = format!("k{:03}", i % 7).into_bytes();
                let mut wb = WriteBatch::new();
                wb.put(&k, &v1);
                wb.put(&k2, &v2);
                wb.delete(&dk);
                match db.write(&wb) {
                    Ok(()) => {
                        model.insert(k.clone(), v1.clone());
                        model.insert(k2.clone(), v2.clone());
                        model.remove(&dk);
                        admit(&mut accept, &k, Some(v1), true);
                        admit(&mut accept, &k2, Some(v2), true);
                        admit(&mut accept, &dk, None, true);
                        oks += 3;
                    }
                    Err(_) => {
                        admit(&mut accept, &k, Some(v1), false);
                        admit(&mut accept, &k2, Some(v2), false);
                        admit(&mut accept, &dk, None, false);
                        errs += 1;
                    }
                }
            }
            7 => {
                // Range delete [a, b) — model filter on Ok only.
                let a = format!("k{:03}", xorshift(&mut rng) % 20).into_bytes();
                let b = format!("k{:03}", 20 + xorshift(&mut rng) % 20).into_bytes();
                match db.delete_range_cf(&db.cf_handle("default").unwrap(), &a, &b) {
                    Ok(()) => {
                        let ks: Vec<Vec<u8>> = model
                            .range(a.clone()..b.clone())
                            .map(|(k, _)| k.clone())
                            .collect();
                        for k in &ks {
                            model.remove(k);
                        }
                        admit_range_delete(&mut accept, &a, &b, true);
                        oks += 1;
                    }
                    Err(_) => {
                        admit_range_delete(&mut accept, &a, &b, false);
                        errs += 1;
                    }
                }
            }
            8 => {
                // Read compare under no fault pressure: value must equal model.
                let got = db.get(&k).expect("get");
                let exp = model.get(&k).cloned();
                assert_eq!(got, exp, "silent-wrong at seed={seed} op={i} key={k:?}");
            }
            _ => {
                let raft = db.cf_handle("raft").unwrap();
                let rk = format!("r{:03}", xorshift(&mut rng) % 16).into_bytes();
                let rv = format!("rv{}", i).into_bytes();
                match db.put_cf(&raft, &rk, &rv) {
                    Ok(()) => {
                        admit(&mut raft_accept, &rk, Some(rv), true);
                        oks += 1;
                    }
                    Err(_) => {
                        admit(&mut raft_accept, &rk, Some(rv), false);
                        errs += 1;
                    }
                }
            }
        }
        if i % 16 == 15 {
            // Periodic scan-compare: engine scan must never contain a wrong value.
            let scan = scan_all(&db);
            for (sk, sv) in &scan {
                if let Some(mv) = model.get(sk) {
                    assert_eq!(mv, sv, "scan silent-wrong seed={seed} key={sk:?}");
                }
            }
        }
    }

    // Live-vs-model before crash: every model key present unless an op failed
    // (only Op failures allowed divergence, and only toward missing).
    for (k, v) in &model {
        if let Some(got) = db.get(k).expect("get") {
            assert_eq!(&got, v, "pre-crash silent-wrong seed={seed} key={k:?}");
        }
    }

    drop(db);

    // Reopen on a healed env. ShortWrite may leave a torn WAL record; Pedra is
    // fail-closed there (CRC stops open — operator repairs), which is the
    // intended integrity contract, not a regression.
    let reopened = DB::open_cf_with_env(&g1_opts(), &dir, &["raft"], FailingEnv::passing());
    let db2 = match reopened {
        Ok(db) => db,
        Err(e) => {
            assert!(
                kind == FaultKind::ShortWrite && e.to_string().contains("crc"),
                "seed={seed}: unexpected reopen error (kind={kind:?}): {e}"
            );
            let _ = std::fs::remove_dir_all(&dir);
            return (model, BTreeMap::new(), oks, errs);
        }
    };
    let mut durable: Model = BTreeMap::new();
    for (k, v) in scan_all(&db2) {
        durable.insert(k, v);
    }
    // Raft CF durable under the same accept semantics.
    let raft = db2.cf_handle("raft").unwrap();
    let mut raft_durable: Model = BTreeMap::new();
    let mut rit = db2.iterator_cf(&raft, IteratorMode::Start).unwrap();
    while rit.valid() {
        raft_durable.insert(rit.key().to_vec(), rit.value().to_vec());
        rit.next();
    }
    let mut wrong: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();
    for (k, allowed) in accept.iter().chain(raft_accept.iter()) {
        let universe: &Model = if k.starts_with(b"r") && k.len() == 4 {
            // keys are "kNNN" (default) vs "rNNN" (raft) in this harness
            if durable.contains_key(k) {
                &durable
            } else {
                &raft_durable
            }
        } else if durable.contains_key(k) {
            &durable
        } else {
            &raft_durable
        };
        let got = universe.get(k).cloned();
        if !allowed.contains(&got) {
            wrong.push((k.clone(), got));
        }
    }
    assert!(
        wrong.is_empty(),
        "reopen silent-wrong seed={seed} kind={kind:?} durable={durable:?} raft={raft_durable:?} wrong={wrong:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
    (model, durable, oks, errs)
}

/// Dead-disk campaigns: deterministic seeds, model-checked, reopen-consistent.
#[test]
fn adversarial_failing_env_campaigns() {
    let mut total_oks = 0u64;
    let mut total_errs = 0u64;
    for seed in 0..=31u64 {
        let (_, _, oks, errs) = run_campaign(seed, None, 96);
        total_oks += oks;
        total_errs += errs;
    }
    // The sweep must exercise both success and fault paths (individual seeds
    // with a tiny fail-after budget may refuse every op).
    assert!(total_oks > 0, "no successful ops across sweep");
    assert!(total_errs > 0, "no injected-fault errors across sweep");
}

/// Sync-fail edge (write lands, fsync errors): reopen must never be wrong.
#[test]
fn adversarial_sync_fail_campaigns() {
    for seed in 0..=7u64 {
        let _ = run_campaign(seed, Some(FaultKind::SyncFail), 96);
    }
}

/// Short-write (torn record): reopen must never be wrong.
#[test]
fn adversarial_short_write_campaigns() {
    for seed in 0..=7u64 {
        let _ = run_campaign(seed, Some(FaultKind::ShortWrite), 96);
    }
}

/// Batch atomicity under fault: a failed batch leaves nothing behind.
#[test]
fn adversarial_batch_all_or_nothing() {
    for seed in 0..=15u64 {
        let dir = tmp("atomic", seed);
        let env = FailingEnv::passing();
        let db = DB::open_cf_with_env(&g1_opts(), &dir, &[], env.clone()).expect("open");
        // Seed base while healthy; fault schedule starts after.
        db.put(b"base", b"0").unwrap();
        // Budget ≥4 so some batches land before the disk dies (tiny budgets
        // refuse every batch and prove nothing about atomicity).
        env.arm_with_kind(4 + seed % 20, false, FaultKind::IoError);
        let mut attempted = 0u64;
        let mut applied = 0u64;
        for i in 0..64u64 {
            let mut wb = WriteBatch::new();
            wb.put(format!("x{i}").as_bytes(), b"v");
            wb.put(format!("y{i}").as_bytes(), b"v");
            wb.delete(b"never-existed");
            attempted += 1;
            match db.write(&wb) {
                Ok(()) => applied += 1,
                Err(_) => break,
            }
        }
        assert!(applied > 0, "seed {seed}: no batch applied");
        let mut it = db.iterator(IteratorMode::Start).unwrap();
        let mut count = 0u64;
        while it.valid() {
            count += 1;
            it.next();
        }
        // base + 2 keys per applied batch, all-or-nothing.
        assert_eq!(
            count,
            1 + applied * 2,
            "seed {seed}: batch was not atomic (applied={applied}, attempted={attempted})"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Iterator positioning sanity under churn (seek-from correctness).
#[test]
fn adversarial_iterator_positioning() {
    for seed in 0..=7u64 {
        let dir = tmp("iterpos", seed);
        let env = FailingEnv::passing();
        let db = DB::open_cf_with_env(&g1_opts(), &dir, &[], env.clone()).expect("open");
        env.arm_with_kind(
            FailingEnv::seed_to_fail_after(seed),
            false,
            FaultKind::IoError,
        );
        let mut model: Model = BTreeMap::new();
        let mut rng = 0xBEEF ^ seed;
        for _ in 0..64 {
            let k = format!("k{:02}", xorshift(&mut rng) % 30).into_bytes();
            let v = format!("v{}", xorshift(&mut rng) % 99).into_bytes();
            if db.put(&k, &v).is_ok() {
                model.insert(k, v);
            }
        }
        for probe in ["k05", "k15", "k25"] {
            let mut it = db
                .iterator(IteratorMode::From(probe.as_bytes(), Direction::Forward))
                .unwrap();
            let got: Vec<(Vec<u8>, Vec<u8>)> = it.collect_rest();
            let exp: Vec<(Vec<u8>, Vec<u8>)> = model
                .range(probe.as_bytes().to_vec()..)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            assert_eq!(got, exp, "seed {seed} forward-from {probe}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// RFC-0047 P1.1: `DB::resume()` after a durability fence (WAL write fails
/// → fence) reopens and reports the typed uncertain range — the lost write
/// stays lost, the DB is writable again, and nothing is silent.
#[test]
fn compat_resume_reports_uncertain_range() {
    use pedradb_sim::OpClass;

    let dir = tmp("resume", 0x4747);
    let env = FailingEnv::passing();
    let db = DB::open_cf_with_env(&g1_opts(), &dir, &[], env.clone()).expect("open");
    db.put(b"a", b"1").expect("put");
    // One-shot failure on the next file write = the WAL frame write.
    env.arm_op_class(OpClass::Write, 0, true, FaultKind::IoError);
    assert!(
        db.put(b"b", b"2").is_err(),
        "injected WAL write failure fences"
    );
    db.resume().expect("resume after fence");
    let rec = db.last_fence_recovery().expect("typed fence report");
    assert_eq!(rec.fence.uncertain_from, 2);
    assert_eq!(rec.fence.uncertain_through, 2);
    assert_eq!(rec.replayed_through, 1);
    assert!(rec.lost_writes, "the write never reached the WAL");
    assert_eq!(db.get(b"a").unwrap().as_deref(), Some(&b"1"[..]));
    assert_eq!(db.get(b"b").unwrap(), None);
    // Resumed DB is writable again (fence cleared by the reopen).
    db.put(b"c", b"3").expect("put after resume");
    assert_eq!(db.get(b"c").unwrap().as_deref(), Some(&b"3"[..]));
    // Defensive resume on a healthy DB is a no-op Ok.
    db.resume().expect("resume on healthy db");
    let _ = std::fs::remove_dir_all(&dir);
}

/// RFC-0047 P1.2: auto-resume only for the Transient class (ENOSPC-like);
/// Persistent stays manual. `try_auto_resume` is the exact tick the host
/// compact worker runs when `auto_resume_transient` is on.
#[test]
fn compat_auto_resume_transient_only() {
    use pedradb_sim::OpClass;

    assert!(
        Options::default().auto_resume_transient,
        "drop-in default: transient fences auto-resume (Rocks-shaped)"
    );

    // (a) ENOSPC write failure → Transient → auto tick resumes.
    let dir = tmp("resume-eno", 0x4748);
    let env = FailingEnv::passing();
    let db = DB::open_cf_with_env(&g1_opts(), &dir, &[], env.clone()).expect("open");
    db.put(b"a", b"1").expect("put");
    env.arm_op_class(OpClass::Write, 0, true, FaultKind::StorageFull);
    assert!(db.put(b"b", b"2").is_err(), "ENOSPC write failure fences");
    assert!(
        db.try_auto_resume().expect("auto tick"),
        "transient auto-resumes"
    );
    let rec = db.last_fence_recovery().expect("report recorded");
    assert_eq!(rec.fence.class, pedradb_core::FenceClass::Transient);
    assert!(rec.lost_writes);
    db.put(b"c", b"3").expect("put after auto-resume");
    assert_eq!(db.get(b"c").unwrap().as_deref(), Some(&b"3"[..]));
    let _ = std::fs::remove_dir_all(&dir);

    // (b) Generic write failure → Persistent → auto tick is a no-op,
    // manual `resume()` still works.
    let dir = tmp("resume-io", 0x4749);
    let env = FailingEnv::passing();
    let db = DB::open_cf_with_env(&g1_opts(), &dir, &[], env.clone()).expect("open");
    db.put(b"a", b"1").expect("put");
    env.arm_op_class(OpClass::Write, 0, true, FaultKind::IoError);
    assert!(db.put(b"b", b"2").is_err(), "write failure fences");
    assert!(
        !db.try_auto_resume().expect("auto tick"),
        "persistent fences never auto-resume"
    );
    assert!(db.put(b"x", b"y").is_err(), "still fenced (manual only)");
    db.resume().expect("manual resume");
    assert!(db.last_fence_recovery().is_some());
    db.put(b"c", b"3").expect("put after manual resume");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Snapshot iterator must not leak later puts (F180 class on the compat face).
#[test]
fn adversarial_snapshot_iterator_no_leak() {
    let dir = tmp("snap-iter", 7);
    let db = DB::open_cf_with_env(&g1_opts(), &dir, &["raft"], FailingEnv::passing()).unwrap();
    db.put(b"a", b"1").unwrap();
    let snap = db.snapshot();
    db.put(b"b", b"2").unwrap();
    assert_eq!(snap.get(b"a").unwrap().as_deref(), Some(&b"1"[..]));
    assert_eq!(snap.get(b"b").unwrap(), None, "snapshot leaked later put");
    let mut it = snap.iterator(IteratorMode::Start).unwrap();
    let mut keys = Vec::new();
    while it.valid() {
        keys.push(it.key().to_vec());
        it.next();
    }
    assert_eq!(keys, vec![b"a".to_vec()]);
    assert_eq!(db.get(b"b").unwrap().as_deref(), Some(&b"2"[..]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `delete_file_in_range` is tombstone+compact, never silent unlink.
#[test]
fn adversarial_delete_file_in_range_is_tombstone() {
    let dir = tmp("dfr", 1);
    let db = DB::open_cf_with_env(&g1_opts(), &dir, &[], FailingEnv::passing()).unwrap();
    db.put(b"a", b"1").unwrap();
    db.put(b"b", b"2").unwrap();
    db.put(b"c", b"3").unwrap();
    db.delete_file_in_range(b"a", b"c").unwrap();
    assert_eq!(db.get(b"a").unwrap(), None);
    assert_eq!(db.get(b"b").unwrap(), None);
    assert_eq!(db.get(b"c").unwrap().as_deref(), Some(&b"3"[..]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Merge operator through the compat face is get+full_merge+put (atomic).
#[test]
fn adversarial_merge_operator_roundtrip() {
    use rocksdb_compat::MergeOperands;

    let dir = tmp("merge", 2);
    let mut opts = g1_opts();
    opts.set_merge_operator_associative("concat", |_k, existing, ops: &MergeOperands| {
        let mut out = existing.unwrap_or(&[]).to_vec();
        for o in ops.iter() {
            out.extend_from_slice(o);
        }
        Some(out)
    });
    let db = DB::open_cf_with_env(&opts, &dir, &[], FailingEnv::passing()).unwrap();
    db.put(b"k", b"a").unwrap();
    db.merge(b"k", b"b").unwrap();
    db.merge(b"k", b"c").unwrap();
    assert_eq!(db.get(b"k").unwrap().as_deref(), Some(&b"abc"[..]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// RFC-0065 P0.3: one `WriteBatch` / one WAL — crash is all-or-nothing across CFs.
/// A 3-DB / 3-WAL layout is not the design (would let lock land without default).
#[test]
fn rfc0065_multi_cf_batch_crash_all_or_nothing() {
    fn wal_logs(dir: &std::path::Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n == "CURRENT.log" || n.starts_with("CURRENT.log."))
            .collect()
    }

    // Happy path: one apply, one WAL, three CFs visible after flush+reopen.
    let dir = tmp("p03-ok", 1);
    let env = FailingEnv::passing();
    let db = DB::open_cf_with_env(&g1_opts(), &dir, &["write", "lock"], env).unwrap();
    let lock = db.cf_handle("lock").unwrap();
    let write = db.cf_handle("write").unwrap();
    let mut wb = WriteBatch::new();
    wb.put(b"dk", b"dv");
    wb.put_cf(&write, b"wk", b"wv");
    wb.put_cf(&lock, b"lk", b"lv");
    db.write(&wb).unwrap();
    db.flush().unwrap();
    assert_eq!(
        wal_logs(&dir).len(),
        1,
        "one WAL, not 3 DBs: {:?}",
        wal_logs(&dir)
    );
    drop(db);
    let db =
        DB::open_cf_with_env(&g1_opts(), &dir, &["write", "lock"], FailingEnv::passing()).unwrap();
    let lock = db.cf_handle("lock").unwrap();
    let write = db.cf_handle("write").unwrap();
    assert_eq!(db.get(b"dk").unwrap().as_deref(), Some(&b"dv"[..]));
    assert_eq!(
        db.get_cf(&write, b"wk").unwrap().as_deref(),
        Some(&b"wv"[..])
    );
    assert_eq!(
        db.get_cf(&lock, b"lk").unwrap().as_deref(),
        Some(&b"lv"[..])
    );
    let _ = std::fs::remove_dir_all(&dir);

    // Sync-fail mid-commit: recover sees all three keys or none.
    let dir = tmp("p03-crash", 2);
    let env = FailingEnv::passing();
    let db = DB::open_cf_with_env(&g1_opts(), &dir, &["write", "lock"], env.clone()).unwrap();
    let lock = db.cf_handle("lock").unwrap();
    let write = db.cf_handle("write").unwrap();
    env.arm_with_kind(0, false, FaultKind::SyncFail);
    let mut wb = WriteBatch::new();
    wb.put(b"dk", b"dv");
    wb.put_cf(&write, b"wk", b"wv");
    wb.put_cf(&lock, b"lk", b"lv");
    let wrote = db.write(&wb);
    drop(db);
    let db =
        DB::open_cf_with_env(&g1_opts(), &dir, &["write", "lock"], FailingEnv::passing()).unwrap();
    let lock = db.cf_handle("lock").unwrap();
    let write = db.cf_handle("write").unwrap();
    let d = db.get(b"dk").unwrap();
    let w = db.get_cf(&write, b"wk").unwrap();
    let l = db.get_cf(&lock, b"lk").unwrap();
    let n = usize::from(d.is_some()) + usize::from(w.is_some()) + usize::from(l.is_some());
    assert!(
        n == 0 || n == 3,
        "partial multi-CF apply (3-DB smell): ok={wrote:?} d={d:?} w={w:?} l={l:?}"
    );
    if wrote.is_ok() {
        assert_eq!(n, 3, "G1 Ok must be durable on all three CFs");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
