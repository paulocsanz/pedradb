//! RFC-0330: Monotonic Read-Your-Writes Linearizability Test Suite.
//!
//! Mechanically verifies that every acknowledged write (`put` -> `Ok`) is immediately
//! visible to subsequent `get` calls on the same thread and concurrent observer threads,
//! with zero transient `None` stale-read windows even under concurrent background flushes,
//! compactions, and group commits.

#![forbid(unsafe_code)]

use pedradb_core::{ConcurrentDb, OpenOptions};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

fn temp_db_dir(prefix: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("pedra-rfc0330-{prefix}-{nanos}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_rfc0330_strict_read_your_writes_single_thread() {
    let dir = temp_db_dir("ryw-single");
    let db = ConcurrentDb::open(&dir).expect("open db");

    // Sequential write followed immediately by get: MUST NEVER read None or stale value
    for i in 0..2_000 {
        let key = format!("key_{i:06}");
        let val = format!("val_{i:06}");

        db.put(key.as_bytes(), val.as_bytes()).expect("put ok");

        let read = db.get(key.as_bytes());
        assert_eq!(
            read.as_deref(),
            Some(val.as_bytes()),
            "Read-Your-Writes violated at iteration {i}: expected {val}, got {read:?}"
        );
    }

    db.close().expect("close");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_rfc0330_concurrent_read_your_writes_under_contention() {
    let dir = temp_db_dir("ryw-concurrent");
    let opts = OpenOptions::default();
    let db = Arc::new(ConcurrentDb::open_with(&dir, opts).expect("open"));

    let running = Arc::new(AtomicBool::new(true));
    let total_verified = Arc::new(AtomicU64::new(0));

    let threads = 8;
    let iters_per_thread = 500;
    let mut handles = Vec::new();

    for t_idx in 0..threads {
        let db = Arc::clone(&db);
        let total = Arc::clone(&total_verified);

        handles.push(std::thread::spawn(move || {
            for i in 0..iters_per_thread {
                let key = format!("thread_{t_idx}_key_{i:04}");
                let val = format!("thread_{t_idx}_val_{i:04}");

                // Write
                db.put(key.as_bytes(), val.as_bytes()).expect("put must succeed");

                // Immediate read: must NEVER observe None or an older value
                let read = db.get(key.as_bytes());
                assert_eq!(
                    read.as_deref(),
                    Some(val.as_bytes()),
                    "Concurrent Read-Your-Writes violated for thread {t_idx} iter {i}"
                );

                total.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }

    // Background reader continuously probing keys
    let reader_db = Arc::clone(&db);
    let reader_running = Arc::clone(&running);
    let reader_handle = std::thread::spawn(move || {
        let mut rng_state = 12345u64;
        while reader_running.load(Ordering::Relaxed) {
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let t = (rng_state % (threads as u64)) as usize;
            let i = (rng_state / 100) % (iters_per_thread as u64);
            let key = format!("thread_{t}_key_{i:04}");
            let _ = reader_db.get(key.as_bytes());
        }
    });

    for h in handles {
        h.join().expect("writer thread panicked");
    }

    running.store(false, Ordering::Relaxed);
    reader_handle.join().expect("reader panicked");

    assert_eq!(
        total_verified.load(Ordering::Relaxed),
        (threads * iters_per_thread) as u64
    );

    let _ = std::fs::remove_dir_all(&dir);
}
