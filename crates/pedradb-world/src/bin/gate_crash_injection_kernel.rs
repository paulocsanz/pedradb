//! RFC-0187 P0.3 → RFC-0188 P2.1 — exhaustive crash-injection gate over
//! a WORKLOAD FAMILY (blocking, self-verifying).
//!
//! Enumerates every crash point of a family of workloads through the
//! real fault seam (`FailingEnvArc`), one injection per op index, and
//! checks the fail-closed recovery oracle after each:
//!
//! 1. **count**: per workload, the total fallible-op count T (and the
//!    sync-class count S) is MEASURED by binary search over `tripped()`
//!    on the same `FailingEnvArc` that injects — zero drift with the
//!    gated op set by construction. Injections loop the full `0..T` and
//!    each must actually FIRE (`tripped()`), so a shrunk or drifting op
//!    stream goes red.
//! 2. **family grid** (P2.1): the measured (T,S) points must form a
//!    grid — ≥3 distinct T values AND ≥2 distinct S values — so the ∀
//!    is over a family, not one shape restated. The measured boundary
//!    (max T, max S) is named in `docs/verification-ledger.md`.
//! 3. **crash oracle** (per workload, per injected index): abrupt death
//!    at the first failing op (`mem::forget` — no Drop flush), reopen
//!    with `StdEnv` must be `Ok` (the fault returned an error; nothing
//!    was torn), and — acked writes survive (TX committed Ok ⇒ both
//!    members present, correct value); — every TX is all-or-nothing
//!    (present or absent, never half); — every present value is correct
//!    (no silent-wrong). Unacked keys may be present or absent
//!    (group-commit may have durably grouped them before the fault).
//! 4. **CRC fail-closed** (corruption phase): byte flips at fixed
//!    offsets of a fully-acked WAL copy must reopen refused or Ok with
//!    correct values only — a flipped frame may never surface as wrong
//!    data (CRC fail-closed, not fail-silent).
//!
//! `--selftest` proves oracle redness without touching production code:
//! acked-lost, half-TX, wrong-value must each be caught; healthy early
//! and late crash states must pass (an always-red oracle is also a
//! bug); a degenerate (single-point) family grid must be flagged.
//!
//! Piso: this proves recovery under injected op failure + process death
//! at every op INDEX of this workload family — it is not a theorem over
//! torn sector writes (TCG nightly), nor ∀π (R-pct / R-glue stand), nor
//! over workloads with other shapes (the grid boundary is the ledger's
//! named frontier).

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use pedradb_core::{ConcurrentDb, OpenOptions, WAL_FILE_NAME};
use pedradb_sim::{FailingEnvArc, FaultKind};

static PROBE_N: AtomicU64 = AtomicU64::new(0);

/// RFC-0188 P2.1 — the workload FAMILY. One shape per axis the grid
/// spans: op count (T grows with puts/commits), commit mix (S grows
/// with commit count), and record width (corruption phase flips longer
/// WALs without changing the op stream shape).
struct Spec {
    tag: &'static str,
    txs: usize,
    tx_puts: usize,
    singles: usize,
    value_len: usize,
}

const FAMILY: [Spec; 4] = [
    Spec { tag: "w0-baseline", txs: 1, tx_puts: 2, singles: 1, value_len: 1 },
    Spec { tag: "w1-double", txs: 2, tx_puts: 2, singles: 2, value_len: 1 },
    Spec { tag: "w2-wide-values", txs: 1, tx_puts: 2, singles: 1, value_len: 4096 },
    Spec { tag: "w3-singles", txs: 0, tx_puts: 0, singles: 3, value_len: 1 },
];

impl Spec {
    /// TX groups: `txs` groups of `tx_puts` keys each.
    fn tx_groups(&self) -> Vec<Vec<String>> {
        (0..self.txs)
            .map(|t| {
                (0..self.tx_puts)
                    .map(|j| format!("crash/{}/tx{t}k{j}", self.tag))
                    .collect()
            })
            .collect()
    }

    fn single_keys(&self) -> Vec<String> {
        (0..self.singles).map(|i| format!("crash/{}/s{i}", self.tag)).collect()
    }

    fn all_keys(&self) -> Vec<String> {
        let mut ks: Vec<String> = self.tx_groups().into_iter().flatten().collect();
        ks.extend(self.single_keys());
        ks
    }

    /// Uniform correct value (width varies per spec; content is '1's).
    fn value(&self) -> String {
        "1".repeat(self.value_len.max(1))
    }
}

/// Workload progress when the op stream died. `acked` = the commits
/// that returned Ok BEFORE the fault (those MUST survive any later
/// crash); `groups` = the TX memberships for all-or-nothing.
#[derive(Default, Debug)]
struct Outcomes {
    acked: Vec<String>,
    groups: Vec<Vec<String>>,
    crashed: bool,
    detail: String,
}

/// Drive one family workload sequentially; stop at the first failing op
/// (the injected fault = the crash point). Panics are caught by the
/// caller; a panic loses the partial ack state and is treated as
/// "nothing acked" (conservative for the acked-survives rule).
fn drive_workload(db: &ConcurrentDb<FailingEnvArc>, spec: &Spec) -> Outcomes {
    let mut o = Outcomes::default();
    let value = spec.value();
    for (t, group) in spec.tx_groups().iter().enumerate() {
        let mut tx = db.begin_occ();
        for k in group {
            if let Err(e) = tx.put(k.as_bytes(), value.as_bytes()) {
                o.crashed = true;
                o.detail = format!("tx{t}.put: {e}");
                return o;
            }
        }
        match tx.commit() {
            Ok(()) => {
                o.acked.extend(group.iter().cloned());
                o.groups.push(group.clone());
            }
            Err(e) => {
                o.crashed = true;
                o.detail = format!("tx{t}.commit: {e}");
                return o;
            }
        }
    }
    for k in spec.single_keys() {
        match db.put(k.as_bytes(), value.as_bytes()) {
            Ok(()) => o.acked.push(k.clone()),
            Err(e) => {
                o.crashed = true;
                o.detail = format!("single.put: {e}");
                return o;
            }
        }
    }
    o
}

fn fresh_dir(base: &Path, tag: &str) -> PathBuf {
    let dir = base.join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// One probe run: fresh dir, open passing, arm AFTER open, drive the
/// workload (stopping at the fault), report whether the fault FIRED.
fn probe(base: &Path, after: u64, kind: FaultKind, spec: &Spec) -> bool {
    let dir = fresh_dir(base, &format!("probe-{}", PROBE_N.fetch_add(1, Ordering::Relaxed)));
    let env = FailingEnvArc::passing();
    let opts = OpenOptions { sync: true, ..OpenOptions::default() };
    let db = ConcurrentDb::open_with_env(&dir, opts, env.clone()).expect("probe open");
    db.set_write_group_catchup_window(std::time::Duration::ZERO);
    env.arm_with_kind(after, true, kind);
    let _ = catch_unwind(AssertUnwindSafe(|| drive_workload(&db, spec)));
    let tripped = env.tripped();
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
    tripped
}

/// Total gated ops for this workload+kind, measured with the SAME
/// `FailingEnvArc` that the injections use (binary search over
/// `tripped()`): T = 1 + max{ n : workload has more than n gated ops }.
fn count_ops(base: &Path, kind: FaultKind, spec: &Spec) -> Result<usize, String> {
    if !probe(base, 0, kind, spec) {
        return Err(format!("workload {} performs no gated ops — seam bypassed", spec.tag));
    }
    let mut hi: u64 = 1;
    while probe(base, hi, kind, spec) {
        hi = hi.checked_mul(2).ok_or("probe bound overflow")?;
        if hi > 8192 {
            return Err(format!("workload {} exceeds {hi} gated ops — not a bounded gate scenario", spec.tag));
        }
    }
    let mut lo: u64 = 0; // probe(0) is true (checked above)
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if probe(base, mid, kind, spec) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok((lo + 1) as usize)
}

/// Pure crash oracle. `observed` = (key, Some(value) | None) for every
/// key of THIS workload after reopen. Checks, in order: acked-survives,
/// TX all-or-nothing, every-present-value-correct.
fn check_crash_state(
    spec: &Spec,
    observed: &[(String, Option<String>)],
    o: &Outcomes,
) -> Result<(), String> {
    let present = |k: &str| {
        observed.iter().find(|(ok, _)| ok == k).is_some_and(|(_, v)| v.is_some())
    };
    let value = |k: &str| observed.iter().find(|(ok, _)| ok == k).and_then(|(_, v)| v.clone());
    let want = spec.value();

    for k in &o.acked {
        if !present(k) {
            return Err(format!("acked write lost: {k}"));
        }
    }
    // All-or-nothing is over the spec's TX membership, not only the
    // groups that committed Ok: an unacked TX must still never surface
    // half (the original P0.3 oracle checked KEYS_TX1/KEYS_TX2
    // regardless of ack).
    for group in spec.tx_groups() {
        let heads = group.iter().filter(|k| present(k)).count();
        if heads != 0 && heads != group.len() {
            return Err(format!(
                "tx half-present ({}/{} of [{}] present)",
                heads,
                group.len(),
                group.join(",")
            ));
        }
    }
    for k in spec.all_keys() {
        if present(&k) && value(&k).as_deref() != Some(want.as_str()) {
            return Err(format!("silent-wrong: {k} = {:?}", value(&k).map(|v| v.len())));
        }
    }
    Ok(())
}

/// Pure corruption oracle (CRC fail-closed): after a byte flip anywhere
/// in the WAL, reopen may refuse or serve — but a served key must carry
/// the correct value, and a TX must never surface half.
fn check_corrupt_state(spec: &Spec, observed: &[(String, Option<String>)]) -> Result<(), String> {
    let present = |k: &str| {
        observed.iter().find(|(ok, _)| ok == k).is_some_and(|(_, v)| v.is_some())
    };
    let value = |k: &str| observed.iter().find(|(ok, _)| ok == k).and_then(|(_, v)| v.clone());
    let want = spec.value();
    for group in spec.tx_groups() {
        let heads = group.iter().filter(|k| present(k)).count();
        if heads != 0 && heads != group.len() {
            return Err(format!(
                "half TX after corruption ({}/{} of [{}] present)",
                heads,
                group.len(),
                group.join(",")
            ));
        }
    }
    for k in spec.all_keys() {
        if present(&k) && value(&k).as_deref() != Some(want.as_str()) {
            return Err(format!("silent-wrong after corruption: {k} = {:?}", value(&k).map(|v| v.len())));
        }
    }
    Ok(())
}

fn read_state(dir: &Path, spec: &Spec) -> Result<Vec<(String, Option<String>)>, String> {
    let re = ConcurrentDb::open_with_env(dir, OpenOptions::default(), pedradb_core::StdEnv)
        .map_err(|e| format!("reopen failed: {e}"))?;
    let mut out = Vec::new();
    for k in spec.all_keys() {
        out.push((k.clone(), re.get(k.as_bytes()).map(|v| String::from_utf8_lossy(&v).into_owned())));
    }
    Ok(out)
}

/// One injected crash: op `i` fails, workload stops, abrupt death
/// (`mem::forget` — Drop never flushes), reopen + oracle.
fn crash_run(base: &Path, i: u64, spec: &Spec) -> Result<(), String> {
    let dir = fresh_dir(base, &format!("crash-{}-{i}", spec.tag));
    let env = FailingEnvArc::passing();
    let opts = OpenOptions { sync: true, ..OpenOptions::default() };
    let db = ConcurrentDb::open_with_env(&dir, opts, env.clone())
        .map_err(|e| format!("open: {e}"))?;
    db.set_write_group_catchup_window(std::time::Duration::ZERO);
    // Alternating errno classes for coverage; both are non-sync ops in
    // the same gated stream, so the index space is identical.
    let kind = if i % 2 == 0 { FaultKind::IoError } else { FaultKind::ShortWrite };
    env.arm_with_kind(i, true, kind);
    let outcomes = catch_unwind(AssertUnwindSafe(|| drive_workload(&db, spec))).unwrap_or_default();
    if !env.tripped() {
        return Err(format!(
            "injection {i}: fault never fired — op stream shorter than counted (drift)"
        ));
    }
    // Abrupt death: skip Drop so no flush path runs after the crash.
    std::mem::forget(db);
    let observed = read_state(&dir, spec)?;
    check_crash_state(spec, &observed, &outcomes)?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// Corruption phase: flip one byte of a fully-acked WAL at `off`, reopen,
/// fail-closed oracle. `Ok(true)` = reopened; `Ok(false)` = refused.
fn corrupt_run(base: &Path, ref_dir: &Path, off: u64, spec: &Spec) -> Result<bool, String> {
    let dir = fresh_dir(base, &format!("corrupt-{}-{off}", spec.tag));
    copy_dir(ref_dir, &dir)?;
    let wal = dir.join(WAL_FILE_NAME);
    let len = std::fs::metadata(&wal).map_err(|e| format!("wal stat: {e}"))?.len();
    if off >= len {
        return Ok(true); // offset beyond this WAL — nothing to flip
    }
    let mut bytes = std::fs::read(&wal).map_err(|e| format!("wal read: {e}"))?;
    bytes[off as usize] ^= 0xFF;
    std::fs::write(&wal, &bytes).map_err(|e| format!("wal write: {e}"))?;
    match read_state(&dir, spec) {
        Ok(observed) => {
            check_corrupt_state(spec, &observed)?;
            Ok(true)
        }
        // Refused reopen is fail-closed and allowed under corruption.
        Err(_) => Ok(false),
    }
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("mkdir: {e}"))?;
    for entry in std::fs::read_dir(from).map_err(|e| format!("readdir: {e}"))? {
        let entry = entry.map_err(|e| format!("dirent: {e}"))?;
        std::fs::copy(entry.path(), to.join(entry.file_name()))
            .map_err(|e| format!("copy: {e}"))?;
    }
    Ok(())
}

/// Pure family-grid verdict (P2.1): the measured (T,S) points must span
/// a grid — ≥3 distinct T values and ≥2 distinct S values — and every
/// workload must have executed its barrier stream (S ≥ 1).
fn check_family_grid(grid: &[(String, usize, usize)]) -> Result<(), String> {
    if grid.is_empty() {
        return Err("family grid is empty".into());
    }
    let mut errs = Vec::new();
    for (tag, t, s) in grid {
        if *t == 0 {
            errs.push(format!("{tag}: T=0 — no fallible ops counted"));
        }
        if *s == 0 {
            errs.push(format!("{tag}: S=0 — barrier ops never executed"));
        }
    }
    let distinct_t = grid.iter().map(|(_, t, _)| *t).collect::<std::collections::BTreeSet<_>>();
    let distinct_s = grid.iter().map(|(_, _, s)| *s).collect::<std::collections::BTreeSet<_>>();
    if distinct_t.len() < 3 {
        errs.push(format!(
            "family spans only {} distinct T values — a family grid needs >=3 (got {:?})",
            distinct_t.len(),
            distinct_t
        ));
    }
    if distinct_s.len() < 2 {
        errs.push(format!(
            "family spans only {} distinct S values — a family grid needs >=2 (got {:?})",
            distinct_s.len(),
            distinct_s
        ));
    }
    if errs.is_empty() { Ok(()) } else { Err(errs.join("; ")) }
}

/// Reference pass: passing env, whole workload acks. Returns the dir
/// (kept for the corruption phase) and the measured sync-class count S.
fn reference_pass(base: &Path, spec: &Spec) -> Result<(PathBuf, usize), String> {
    let dir = fresh_dir(base, &format!("reference-{}", spec.tag));
    let env = FailingEnvArc::passing();
    let opts = OpenOptions { sync: true, ..OpenOptions::default() };
    let db = ConcurrentDb::open_with_env(&dir, opts, env.clone())
        .map_err(|e| format!("reference open: {e}"))?;
    db.set_write_group_catchup_window(std::time::Duration::ZERO);
    let o = drive_workload(&db, spec);
    if o.crashed || o.acked.len() != spec.all_keys().len() {
        return Err(format!("reference workload {} did not fully ack: {:?}", spec.tag, o));
    }
    let syncs = count_ops(base, FaultKind::SyncFail, spec)?;
    drop(db);
    Ok((dir, syncs))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--selftest") {
        std::process::exit(selftest());
    }

    // Keep gate logs clean: panics are outcomes (caught), not noise.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let base = std::env::temp_dir().join(format!("pedra-gate-crash-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();

    let mut grid: Vec<(String, usize, usize)> = Vec::new();
    let mut failures = 0usize;
    let mut grand_total = 0usize;
    let mut max_sync = 0usize;
    let mut flips_served = 0usize;
    let mut flips_refused = 0usize;

    for spec in &FAMILY {
        let (ref_dir, sync_ops) = match reference_pass(&base, spec) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("GATE crash: FAIL — {}: {e}", spec.tag);
                failures += 1;
                continue;
            }
        };
        let t_total = match count_ops(&base, FaultKind::IoError, spec) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("GATE crash: FAIL — count {}: {e}", spec.tag);
                failures += 1;
                continue;
            }
        };
        println!(
            "CRASH_FAMILY {} T={t_total} S={sync_ops} (binary-search over tripped)",
            spec.tag
        );
        grid.push((spec.tag.to_string(), t_total, sync_ops));
        grand_total += t_total;
        max_sync = max_sync.max(sync_ops);

        for i in 0..t_total as u64 {
            if let Err(e) = crash_run(&base, i, spec) {
                eprintln!("GATE crash: FAIL — {} injection {i}/{t_total}: {e}", spec.tag);
                failures += 1;
            }
        }

        // Corruption phase: flips at fixed fractions of this fully-acked WAL.
        let wal_len =
            std::fs::metadata(ref_dir.join(WAL_FILE_NAME)).map(|m| m.len()).unwrap_or(0);
        let offsets: Vec<u64> = [
            1u64,
            8,
            wal_len / 4,
            wal_len / 2,
            3 * wal_len / 4,
            wal_len.saturating_sub(8),
            wal_len.saturating_sub(1),
        ]
        .into_iter()
        .filter(|&o| o < wal_len)
        .collect();
        for off in offsets {
            match corrupt_run(&base, &ref_dir, off, spec) {
                Ok(true) => flips_served += 1,
                Ok(false) => flips_refused += 1,
                Err(e) => {
                    eprintln!("GATE crash: FAIL — {} corruption @{off}: {e}", spec.tag);
                    failures += 1;
                }
            }
        }
        let _ = std::fs::remove_dir_all(&ref_dir);
    }

    println!(
        "CORRUPTION flips={} served={} refused={}",
        flips_served + flips_refused,
        flips_served,
        flips_refused
    );

    if let Err(e) = check_family_grid(&grid) {
        eprintln!("GATE crash: FAIL — family grid: {e}");
        failures += 1;
    }

    // Aggregate summary (format pinned by check_barrier_floor.py's
    // dynamic tie): total = every injected index across the family,
    // sync = the largest per-workload barrier stream measured.
    println!("CRASH_OPS total={grand_total} sync={max_sync}");

    std::panic::set_hook(prev_hook);
    let _ = std::fs::remove_dir_all(&base);
    if failures > 0 {
        eprintln!("GATE crash: RED");
        std::process::exit(1);
    }
    let injected: usize = grid.iter().map(|(_, t, _)| *t).sum();
    println!(
        "GATE crash: GREEN — {injected}/{injected} crash points injected across {} workloads (each fired), fail-closed oracle clean, family grid asserted",
        grid.len()
    );
}

/// Prove the oracles can fail (and pass) on synthetic states.
fn selftest() -> i32 {
    let mut caught = 0;
    let mut total = 0;
    let spec = &FAMILY[0];
    let value = spec.value();
    let group: Vec<String> = spec.tx_groups().into_iter().flatten().collect();
    let singles = spec.single_keys();
    let mut all = group.clone();
    all.extend(singles.clone());

    fn state(pairs: &[(String, Option<String>)]) -> Vec<(String, Option<String>)> {
        pairs.to_vec()
    }
    let ok_outcomes = Outcomes {
        acked: all.clone(),
        groups: spec.tx_groups(),
        crashed: false,
        detail: String::new(),
    };

    // S1: acked write lost (one tx member gone after full ack).
    total += 1;
    let mut s: Vec<(String, Option<String>)> =
        all.iter().map(|k| (k.clone(), Some(value.clone()))).collect();
    s[0].1 = None;
    if check_crash_state(spec, &state(&s), &ok_outcomes).is_err() {
        println!("SELFTEST crash: caught=acked-lost");
        caught += 1;
    } else {
        eprintln!("SELFTEST crash: MISSED acked-lost");
    }

    // S2: half TX (unacked — all-or-nothing must still hold).
    total += 1;
    let mut s: Vec<(String, Option<String>)> =
        all.iter().map(|k| (k.clone(), None)).collect();
    s[0].1 = Some(value.clone());
    if check_crash_state(spec, &state(&s), &Outcomes::default()).is_err() {
        println!("SELFTEST crash: caught=half-tx");
        caught += 1;
    } else {
        eprintln!("SELFTEST crash: MISSED half-tx");
    }

    // S3: silent-wrong value (right width, wrong byte).
    total += 1;
    let mut wrong = value.clone();
    wrong.replace_range(0..1, "X");
    let mut s: Vec<(String, Option<String>)> =
        all.iter().map(|k| (k.clone(), Some(value.clone()))).collect();
    let last = s.len() - 1;
    s[last].1 = Some(wrong);
    let acked_single = Outcomes {
        acked: vec![singles[0].clone()],
        groups: Vec::new(),
        crashed: false,
        detail: String::new(),
    };
    if check_crash_state(spec, &state(&s), &acked_single).is_err() {
        println!("SELFTEST crash: caught=silent-wrong");
        caught += 1;
    } else {
        eprintln!("SELFTEST crash: MISSED silent-wrong");
    }

    // S4: healthy late crash (everything acked and present) must PASS.
    total += 1;
    let s: Vec<(String, Option<String>)> =
        all.iter().map(|k| (k.clone(), Some(value.clone()))).collect();
    if check_crash_state(spec, &state(&s), &ok_outcomes).is_ok() {
        println!("SELFTEST crash: passed=healthy-late (oracle not always-red)");
        caught += 1;
    } else {
        eprintln!("SELFTEST crash: healthy-late state REJECTED — oracle always-red");
    }

    // S5: healthy early crash (nothing acked, one unacked single present) must PASS.
    total += 1;
    let mut s: Vec<(String, Option<String>)> =
        all.iter().map(|k| (k.clone(), None)).collect();
    let last = s.len() - 1;
    s[last].1 = Some(value.clone());
    if check_crash_state(spec, &s, &Outcomes::default()).is_ok() {
        println!("SELFTEST crash: passed=healthy-early");
        caught += 1;
    } else {
        eprintln!("SELFTEST crash: healthy-early state REJECTED — oracle always-red");
    }

    // S6: corruption oracle — silent-wrong after flip.
    total += 1;
    let mut s: Vec<(String, Option<String>)> =
        all.iter().map(|k| (k.clone(), Some(value.clone()))).collect();
    s[2].1 = Some("9".to_string());
    if check_corrupt_state(spec, &state(&s)).is_err() {
        println!("SELFTEST crash: caught=corrupt-silent-wrong");
        caught += 1;
    } else {
        eprintln!("SELFTEST crash: MISSED corrupt-silent-wrong");
    }

    // S7 (P2.1): a degenerate grid — every workload at the same (T,S) —
    // must be flagged (a family of clones is not a family).
    total += 1;
    let degenerate = vec![
        ("a".to_string(), 8, 3),
        ("b".to_string(), 8, 3),
        ("c".to_string(), 8, 3),
        ("d".to_string(), 8, 3),
    ];
    match check_family_grid(&degenerate) {
        Err(e) if e.contains("distinct") => {
            println!("SELFTEST crash: caught=degenerate-family-grid");
            caught += 1;
        }
        other => eprintln!("SELFTEST crash: degenerate grid not flagged: {other:?}"),
    }

    // S8 (P2.1): a zero-barrier workload (S=0) must be flagged.
    total += 1;
    let no_barrier = vec![
        ("a".to_string(), 8, 0),
        ("b".to_string(), 12, 3),
        ("c".to_string(), 16, 4),
    ];
    match check_family_grid(&no_barrier) {
        Err(e) if e.contains("S=0") => {
            println!("SELFTEST crash: caught=zero-barrier-workload");
            caught += 1;
        }
        other => eprintln!("SELFTEST crash: S=0 workload not flagged: {other:?}"),
    }

    // S9 (P2.1): the honest family grid (spread T, spread S) must PASS.
    total += 1;
    let honest = vec![
        ("a".to_string(), 8, 3),
        ("b".to_string(), 16, 5),
        ("c".to_string(), 24, 3),
        ("d".to_string(), 40, 7),
    ];
    if check_family_grid(&honest).is_ok() {
        println!("SELFTEST crash: passed=honest-family-grid (checker not always-red)");
        caught += 1;
    } else {
        eprintln!("SELFTEST crash: honest family grid REJECTED — checker always-red");
    }

    println!("SELFTEST crash: {caught}/{total} oracle checks caught");
    if caught == total { 0 } else { 1 }
}
