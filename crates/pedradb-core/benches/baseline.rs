//! Baseline benches for put / get / commit + sync vs batched (RFC-0009 P0.4).
//!
//! Run: `cargo bench -p pedradb-core --bench baseline`

use std::time::{Duration, Instant};

use pedradb_core::{Db, OpenOptions, WriteOptions};

fn temp_dir() -> std::path::PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("pedradb-bench-{n}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn report(name: &str, n: u64, elapsed: Duration) {
    let secs = elapsed.as_secs_f64().max(1e-12);
    let ops = n as f64 / secs;
    let ns_per = (elapsed.as_nanos() as f64) / n as f64;
    println!("{name}: {n} ops in {elapsed:?} → {ops:.0} ops/s, {ns_per:.0} ns/op");
}

fn open_nosync_autoflush(path: impl AsRef<std::path::Path>) -> Db {
    Db::open_with(
        path,
        OpenOptions {
            wal_full_fsync: true,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
        },
    )
    .expect("open")
}

fn main() {
    let dir = temp_dir();
    let n = 2_000u64;

    // --- put (auto-commit, sync every write) ---
    {
        let mut db = open_nosync_autoflush(dir.join("put_sync"));
        let t0 = Instant::now();
        for i in 0..n {
            let k = format!("k{i:08}");
            let v = format!("v{i}");
            db.put(k.as_bytes(), v.as_bytes()).expect("put");
        }
        report("put_sync_each", n, t0.elapsed());
        db.close().ok();
    }

    // --- put no_sync + one sync at end (group fsync) ---
    {
        let mut db = open_nosync_autoflush(dir.join("put_batch"));
        let t0 = Instant::now();
        for i in 0..n {
            let k = format!("k{i:08}");
            let v = format!("v{i}");
            db.put_with(k.as_bytes(), v.as_bytes(), WriteOptions::no_sync())
                .expect("put");
        }
        db.sync().expect("sync");
        report("put_nosync_then_sync", n, t0.elapsed());
        db.close().ok();
    }

    // --- get ---
    {
        let mut db = open_nosync_autoflush(dir.join("get"));
        for i in 0..n {
            let k = format!("k{i:08}");
            db.put_with(k.as_bytes(), b"x", WriteOptions::no_sync())
                .expect("put");
        }
        db.sync().expect("sync");
        let t0 = Instant::now();
        for i in 0..n {
            let k = format!("k{i:08}");
            let _ = db.get(k.as_bytes());
        }
        report("get", n, t0.elapsed());
        db.close().ok();
    }

    // --- commit (multi-key TX, 2 puts per commit, sync each) ---
    {
        let mut db = open_nosync_autoflush(dir.join("commit"));
        let t0 = Instant::now();
        for i in 0..n {
            let mut tx = db.begin();
            let k1 = format!("r{i:08}");
            let k2 = format!("i{i:08}");
            tx.put(k1.as_bytes(), b"row").expect("put");
            tx.put(k2.as_bytes(), b"idx").expect("put");
            tx.commit().expect("commit");
        }
        report("commit", n, t0.elapsed());
        db.close().ok();
    }

    let _ = std::fs::remove_dir_all(&dir);
}
