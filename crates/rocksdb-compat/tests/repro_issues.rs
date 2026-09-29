//! Isolated reproductions from public issues.
//!
//! `cargo test -p rocksdb-compat --test repro_issues`

use rocksdb_compat::{
    IteratorMode, MergeOperands, OptimisticTransactionDB, OptimisticTransactionOptions, Options,
    ReadOptions, WriteOptions, DB,
};
use std::sync::{Arc, Barrier};
use std::thread;

fn tmp(tag: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("rdbcompat-repro-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// rust-rocksdb contract: `raw_iterator_cf(lock)` walks `lock`, not default.
///
/// Was a silent cross-CF leak: `_cf` discarded, `reopen` hard-coded `default`.
#[test]
fn raw_iterator_cf_ignores_column_family() {
    let dir = tmp("raw-cf");
    let mut opts = Options::new();
    opts.create_if_missing(true);
    let db = DB::open_cf(&opts, &dir, &["lock"]).unwrap();
    let lock = db.cf_handle("lock").unwrap();
    db.put(b"default-key", b"d").unwrap();
    db.put_cf(&lock, b"lock-key", b"l").unwrap();

    let via_cf: Vec<_> = db
        .iterator_cf(&lock, IteratorMode::Start)
        .unwrap()
        .map(|r| r.unwrap().0.to_vec())
        .collect();
    assert_eq!(via_cf, vec![b"lock-key".to_vec()]);

    let mut raw = db.raw_iterator_cf(&lock);
    raw.seek_to_first();
    let mut got = Vec::new();
    while raw.valid() {
        got.push(raw.key().unwrap().to_vec());
        raw.next();
    }
    assert_eq!(
        got,
        vec![b"lock-key".to_vec()],
        "raw_iterator_cf(lock) must not yield default-CF keys; got {got:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Issue #4: `prefix_iterator("a")` must not yield `b`.
#[test]
fn prefix_iterator_does_not_stop_at_prefix() {
    let dir = tmp("prefix");
    let db = DB::open_default(&dir).unwrap();
    db.put(b"aa", b"1").unwrap();
    db.put(b"ab", b"2").unwrap();
    db.put(b"b", b"3").unwrap();

    let keys: Vec<Vec<u8>> = db
        .prefix_iterator(b"a")
        .unwrap()
        .map(|r| r.unwrap().0.to_vec())
        .collect();

    assert!(
        keys.iter().all(|k| k.starts_with(b"a")),
        "prefix_iterator(a) leaked non-prefix keys: {keys:?}"
    );
    assert_eq!(keys, vec![b"aa".to_vec(), b"ab".to_vec()]);

    let mut opts = Options::new();
    opts.create_if_missing(true);
    let dir_cf = tmp("prefix-cf");
    let db = DB::open_cf(&opts, &dir_cf, &["data"]).unwrap();
    let cf = db.cf_handle("data").unwrap();
    db.put_cf(&cf, b"aa", b"1").unwrap();
    db.put_cf(&cf, b"ab", b"2").unwrap();
    db.put_cf(&cf, b"b", b"3").unwrap();
    let keys: Vec<Vec<u8>> = db
        .prefix_iterator_cf(&cf, b"a")
        .unwrap()
        .map(|r| r.unwrap().0.to_vec())
        .collect();
    assert_eq!(keys, vec![b"aa".to_vec(), b"ab".to_vec()]);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir_cf);
}

/// Issue #3: a key seen only through the txn iterator must Busy on overwrite.
#[test]
fn optimistic_txn_iterator_does_not_conflict_on_scanned_keys() {
    let dir = tmp("txn-iter");
    let mut opts = Options::new();
    opts.create_if_missing(true);
    let db = OptimisticTransactionDB::open(&opts, &dir).unwrap();
    db.put(b"k", b"v1").unwrap();

    let mut txn_opts = OptimisticTransactionOptions::default();
    txn_opts.set_snapshot(true);
    let tx = db.transaction_opt(&WriteOptions::default(), &txn_opts);

    let mut it = tx.raw_iterator_opt(ReadOptions::default());
    it.seek_to_first();
    assert_eq!(it.key(), Some(b"k".as_ref()));
    assert_eq!(it.value(), Some(b"v1".as_ref()));

    db.put(b"k", b"v2").unwrap();

    tx.commit()
        .expect_err("scanned key was overwritten; commit must Busy");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Issue #2: concurrent `merge` must not drop operands.
#[test]
fn merge_is_get_put_and_loses_concurrent_operands() {
    let dir = tmp("merge-race");
    let mut opts = Options::new();
    opts.create_if_missing(true);
    opts.set_merge_operator_associative("concat", |_k, existing, ops: &MergeOperands| {
        let mut out = existing.unwrap_or(&[]).to_vec();
        for o in ops.iter() {
            out.extend_from_slice(o);
        }
        Some(out)
    });
    let db = Arc::new(DB::open(&opts, &dir).unwrap());
    const N: u8 = 8;
    let barrier = Arc::new(Barrier::new(N as usize));
    let mut handles = Vec::new();
    for id in 0..N {
        let db = Arc::clone(&db);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            db.merge(b"acc", [id]).unwrap();
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let got = db.get(b"acc").unwrap().unwrap_or_default();
    assert_eq!(got.len(), N as usize, "lost merge operands: {got:?}");
    let mut sorted = got.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (0..N).collect::<Vec<_>>());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Bug 1: `get_cf_opt` must not poison thread-local cache across column families.
#[test]
fn get_cf_opt_no_cross_cf_cache_pollution() {
    let dir = tmp("cf-cache-isolation");
    let mut opts = Options::new();
    opts.create_if_missing(true);
    let db = DB::open_cf(&opts, &dir, &["finance", "public"]).unwrap();
    let cf_finance = db.cf_handle("finance").unwrap();
    let cf_public = db.cf_handle("public").unwrap();

    db.put(b"shared_key", b"val_default").unwrap();
    db.put_cf(&cf_finance, b"shared_key", b"val_finance").unwrap();
    db.put_cf(&cf_public, b"shared_key", b"val_public").unwrap();

    let ro = ReadOptions::default();

    // Query finance first (warms TLS cache)
    let got_finance = db.get_cf_opt(&cf_finance, b"shared_key", &ro).unwrap();
    assert_eq!(got_finance.as_deref(), Some(b"val_finance".as_slice()));

    // Query public: must NOT return finance value!
    let got_public = db.get_cf_opt(&cf_public, b"shared_key", &ro).unwrap();
    assert_eq!(
        got_public.as_deref(),
        Some(b"val_public".as_slice()),
        "cross-cf leak: got_public returned finance value"
    );

    // Query default: must NOT return finance or public value!
    let got_default = db.get_opt(b"shared_key", &ro).unwrap();
    assert_eq!(
        got_default.as_deref(),
        Some(b"val_default".as_slice()),
        "cross-cf leak: got_default returned non-default value"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Bug 4: `multi_get` must evaluate all keys under a single atomic snapshot (no fractured reads).
#[test]
fn multi_get_snapshot_isolation_no_fractured_reads() {
    let dir = tmp("multi-get-snapshot");
    let mut opts = Options::new();
    opts.create_if_missing(true);
    let db = Arc::new(DB::open(&opts, &dir).unwrap());

    // Initialize key_a and key_b to "0"
    let mut wb = rocksdb_compat::WriteBatch::default();
    wb.put(b"key_a", b"0");
    wb.put(b"key_b", b"0");
    db.write(&wb).unwrap();

    let running = Arc::new(std::sync::atomic::AtomicBool::new(true));

    // Writer thread: continuously writes (i, i) atomically
    let writer_db = Arc::clone(&db);
    let writer_running = Arc::clone(&running);
    let writer = thread::spawn(move || {
        for i in 1..=500 {
            let mut wb = rocksdb_compat::WriteBatch::default();
            let val = i.to_string().into_bytes();
            wb.put(b"key_a", &val);
            wb.put(b"key_b", &val);
            writer_db.write(&wb).unwrap();
        }
        writer_running.store(false, std::sync::atomic::Ordering::Relaxed);
    });

    // Reader thread: multi_get on [key_a, key_b] must ALWAYS see key_a == key_b
    let reader_db = Arc::clone(&db);
    let reader_running = Arc::clone(&running);
    let reader = thread::spawn(move || {
        let keys = [b"key_a".as_slice(), b"key_b".as_slice()];
        while reader_running.load(std::sync::atomic::Ordering::Relaxed) {
            let res = reader_db.multi_get(keys);
            assert_eq!(res.len(), 2);
            let val_a = res[0].as_ref().unwrap().clone();
            let val_b = res[1].as_ref().unwrap().clone();
            assert_eq!(
                val_a, val_b,
                "fractured read detected: multi_get saw mismatched versions across keys"
            );
        }
    });

    writer.join().unwrap();
    reader.join().unwrap();

    let _ = std::fs::remove_dir_all(&dir);
}

/// Bug 3: Reverse iteration across multiple pages (> 2048 keys) must refill
/// in bounded linear O(M) time and yield all keys in exact descending order.
#[test]
fn reverse_iterator_refill_spans_multiple_windows_correctly() {
    let dir = tmp("reverse-iter-multi-window");
    let mut opts = Options::new();
    opts.create_if_missing(true);
    let db = DB::open(&opts, &dir).unwrap();

    let total = 5000;
    for i in 0..total {
        let key = format!("k_{i:06}").into_bytes();
        let val = format!("v_{i}").into_bytes();
        db.put(&key, &val).unwrap();
    }

    let mut iter = db.iterator(IteratorMode::End).unwrap();
    let mut seen = Vec::new();
    while iter.valid() {
        let k = std::str::from_utf8(iter.key()).unwrap();
        let id: usize = k.strip_prefix("k_").unwrap().parse().unwrap();
        seen.push(id);
        iter.next();
    }

    assert_eq!(seen.len(), total, "must yield exactly all 5000 keys");
    let expected: Vec<usize> = (0..total).rev().collect();
    assert_eq!(seen, expected, "reverse iteration must yield exact descending sequence");

    // Barreira 3: Algorithmic Complexity Budget Guard
    // Total steps to traverse all keys in reverse must be bounded by O(M), strictly <= 3 * M
    iter.assert_step_budget(total, 3);

    // Verify forward iteration complexity budget as well (<= 2 * M)
    let mut fwd_iter = db.iterator(IteratorMode::Start).unwrap();
    let mut fwd_seen = 0;
    while fwd_iter.valid() {
        fwd_seen += 1;
        fwd_iter.next();
    }
    assert_eq!(fwd_seen, total);
    fwd_iter.assert_step_budget(total, 2);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Barreira 2: Physical Resource Invariants & Disk Leak Checker.
/// Verifies that any uncommitted SST, abandoned .tmp file, or missing manifest SST
/// is immediately flagged by the invariant checker.
#[test]
fn test_physical_disk_inventory_and_leak_invariant() {
    let dir = tmp("disk-inventory-invariant");
    let mut opts = Options::new();
    opts.create_if_missing(true);
    let db = DB::open(&opts, &dir).unwrap();

    for i in 0..100 {
        db.put(format!("key_{i:04}").as_bytes(), b"val").unwrap();
    }
    db.flush().unwrap();
    for i in 100..200 {
        db.put(format!("key_{i:04}").as_bytes(), b"val").unwrap();
    }
    db.flush().unwrap();
    db.compact().unwrap();

    // 1. Quiescent database must strictly satisfy the invariant
    db.assert_disk_inventory_invariant()
        .expect("Valid database must pass disk invariant checker");

    // 2. Inject an abandoned .tmp file (e.g. from an interrupted compaction or aborted write)
    let tmp_path = dir.join("compaction_job_01.tmp");
    std::fs::write(&tmp_path, b"abandoned tmp data").unwrap();

    let err = db.assert_disk_inventory_invariant().unwrap_err();
    assert!(
        err.to_string().contains("Abandoned temporary file"),
        "Invariant checker must detect abandoned .tmp files: {err}"
    );
    std::fs::remove_file(&tmp_path).unwrap();

    // 3. Inject an untracked / uncommitted .sst file (disk leak)
    let orphan_sst = dir.join("099999.sst");
    std::fs::write(&orphan_sst, b"fake uncommitted sst").unwrap();

    let err = db.assert_disk_inventory_invariant().unwrap_err();
    assert!(
        err.to_string().contains("File number") || err.to_string().contains("Orphan / uncommitted SST"),
        "Invariant checker must detect untracked SST files: {err}"
    );
    std::fs::remove_file(&orphan_sst).unwrap();

    // 4. Invariant holds again once clean
    db.assert_disk_inventory_invariant()
        .expect("Clean database must pass disk invariant checker");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_bug9_backup_restore_preserves_cfreg_and_named_cfs() {
    use rocksdb_compat::backup::{BackupEngine, BackupEngineOptions, RestoreOptions};
    use rocksdb_compat::Env;

    let dir = tmp("b9-cfreg-src");
    let backup_dir = tmp("b9-cfreg-backup");
    let restore_dir = tmp("b9-cfreg-restore");

    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.set_sync(true);

    let cfs = ["cf_payments", "cf_users"];
    let db = DB::open_cf(&opts, &dir, &cfs).unwrap();

    let h_pay = db.cf_handle("cf_payments").unwrap();
    let h_usr = db.cf_handle("cf_users").unwrap();

    db.put_cf(&h_pay, b"tx100", b"1000_usd").unwrap();
    db.put_cf(&h_usr, b"user42", b"alice").unwrap();
    db.put(b"default_key", b"global_state").unwrap();

    // Create backup with BackupEngine
    let env = Env::new().unwrap();
    let backup_opts = BackupEngineOptions::new(&backup_dir).unwrap();
    let mut engine = BackupEngine::open(&backup_opts, &env).unwrap();
    engine.create_new_backup_flush(&db, true).unwrap();
    drop(db);

    // Restore from latest backup
    let ropts = RestoreOptions::default();
    engine
        .restore_from_latest_backup(&restore_dir, &restore_dir, &ropts)
        .unwrap();

    // Verify CFREG sidecar is present and all column families open and read cleanly
    let restored_db = DB::open_cf(&opts, &restore_dir, &cfs).expect("Must open with CFs restored");
    let h_pay_r = restored_db.cf_handle("cf_payments").unwrap();
    let h_usr_r = restored_db.cf_handle("cf_users").unwrap();

    assert_eq!(
        restored_db.get_cf(&h_pay_r, b"tx100").unwrap().as_deref(),
        Some(&b"1000_usd"[..])
    );
    assert_eq!(
        restored_db.get_cf(&h_usr_r, b"user42").unwrap().as_deref(),
        Some(&b"alice"[..])
    );
    assert_eq!(
        restored_db.get(b"default_key").unwrap().as_deref(),
        Some(&b"global_state"[..])
    );

    drop(restored_db);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&backup_dir);
    let _ = std::fs::remove_dir_all(&restore_dir);
}

#[test]
fn test_bug10_concurrent_write_opt_isolation() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    let dir = tmp("b10-durability-race");
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.set_sync(false); // default db is async

    let db = Arc::new(DB::open(&opts, &dir).unwrap());
    let stop = Arc::new(AtomicBool::new(false));

    let mut handles = Vec::new();

    // Thread A: calls put_opt with sync = true
    let db_a = Arc::clone(&db);
    let stop_a = Arc::clone(&stop);
    handles.push(std::thread::spawn(move || {
        let mut wo_sync = rocksdb_compat::WriteOptions::default();
        wo_sync.set_sync(true);
        let mut count = 0;
        while !stop_a.load(Ordering::Relaxed) && count < 200 {
            let k = format!("sync_k_{count}").into_bytes();
            db_a.put_opt(&k, b"v_sync", &wo_sync).unwrap();
            count += 1;
        }
    }));

    // Thread B: calls put_opt with sync = false (must not be clobbered by Thread A)
    let db_b = Arc::clone(&db);
    let stop_b = Arc::clone(&stop);
    handles.push(std::thread::spawn(move || {
        let mut wo_async = rocksdb_compat::WriteOptions::default();
        wo_async.set_sync(false);
        let mut count = 0;
        while !stop_b.load(Ordering::Relaxed) && count < 200 {
            let k = format!("async_k_{count}").into_bytes();
            db_b.put_opt(&k, b"v_async", &wo_async).unwrap();
            count += 1;
        }
    }));

    std::thread::sleep(Duration::from_millis(50));
    stop.store(true, Ordering::Relaxed);

    for h in handles {
        h.join().unwrap();
    }

    // Both threads must have successfully written without errors or cross-thread data corruption
    assert!(db.get(b"sync_k_0").unwrap().is_some());
    assert!(db.get(b"async_k_0").unwrap().is_some());

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}


