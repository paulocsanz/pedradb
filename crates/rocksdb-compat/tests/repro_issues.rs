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
