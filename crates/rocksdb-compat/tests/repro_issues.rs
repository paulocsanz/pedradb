//! Isolated reproductions from public issues.
//!
//! `cargo test -p rocksdb-compat --test repro_issues raw_iterator_cf_ignores_column_family`

use rocksdb_compat::{IteratorMode, Options, DB};

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
