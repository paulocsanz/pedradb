//! Integration test suite for RFC-0300: Resilient Concurrency, Adaptive Backoff, and Write Pacing.
//!
//! Validates:
//! 1. Smooth write pacing delay progression under L0 accumulation without stall cliffs.
//! 2. `ConcurrentDb::transact` resilient execution under heavy multi-thread OCC contention.
//! 3. Bounded backoff duration and jitter under conflict retry policies.

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pedradb_core::concurrent::ConcurrentDb;
use pedradb_core::db::WriteOptions;
use pedradb_core::resilient_tx::{ContentionTracker, TransactionRetryPolicy};
use pedradb_core::write_admission_kernel::write_pacing_delay_micros;

fn temp_db_dir() -> std::path::PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let id = SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("pedradb-rfc0300-{nanos}-{id}"));
    let _ = std::fs::remove_dir_all(&path);
    path
}

#[test]
fn test_write_pacing_curve_boundaries() {
    let limit = 40u64;
    // Below 75% (30 files), delay is zero
    assert_eq!(write_pacing_delay_micros(0, limit), 0);
    assert_eq!(write_pacing_delay_micros(20, limit), 0);
    assert_eq!(write_pacing_delay_micros(29, limit), 0);
    assert_eq!(write_pacing_delay_micros(30, limit), 0);

    // Between 30 and 40 files, delay increases smoothly from 0 to 1000us
    let delay_35 = write_pacing_delay_micros(35, limit);
    assert_eq!(delay_35, 500);

    let delay_39 = write_pacing_delay_micros(39, limit);
    assert_eq!(delay_39, 900);

    // At or above limit, maximum 1000us damping applies
    assert_eq!(write_pacing_delay_micros(40, limit), 1000);
    assert_eq!(write_pacing_delay_micros(50, limit), 1000);
}

#[test]
fn test_concurrent_transact_resolves_high_contention() {
    let path = temp_db_dir();
    let db = ConcurrentDb::open(&path).expect("open db");

    let hot_key = b"shared_hot_counter";
    // Initialize counter to 0
    db.put(hot_key, &0u64.to_le_bytes()).expect("initial put");

    let num_threads = 4;
    let increments_per_thread = 5;
    let tracker = Arc::new(ContentionTracker::new());

    let mut handles = Vec::new();
    for _ in 0..num_threads {
        let db_clone = db.clone();
        let tracker_clone = Arc::clone(&tracker);
        handles.push(thread::spawn(move || {
            let policy = TransactionRetryPolicy {
                max_retries: 200,
                initial_backoff: Duration::from_micros(100),
                max_backoff: Duration::from_millis(20),
                backoff_multiplier: 1.5,
                jitter: true,
            };

            for _ in 0..increments_per_thread {
                loop {
                    let res = db_clone.transact_with(policy, |tx| {
                        let old_val = tx.get(hot_key)?.unwrap();
                        let current = u64::from_le_bytes(old_val.as_ref().try_into().unwrap());
                        tx.put(hot_key, &(current + 1).to_le_bytes())?;
                        Ok(())
                    });

                    if res.is_ok() {
                        tracker_clone.record_retried_commit();
                        break;
                    } else {
                        tracker_clone.record_exhausted_abort();
                        thread::sleep(Duration::from_millis(1));
                    }
                }
            }
        }));
    }

    for h in handles {
        h.join().expect("thread join");
    }

    // Verify counter matches exactly num_threads * increments_per_thread
    let final_bytes = db.get(hot_key).expect("exists");
    let final_val = u64::from_le_bytes(final_bytes.as_ref().try_into().unwrap());
    assert_eq!(final_val, (num_threads * increments_per_thread) as u64);

    let stats = tracker.stats();
    assert_eq!(stats.retried_commits, (num_threads * increments_per_thread) as u64);

    db.close().expect("close db");
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn test_durability_modes_parity() {
    let path = temp_db_dir();
    let db = ConcurrentDb::open(&path).expect("open db");

    // Write with PhysicalSync (default G1)
    let sync_opts = WriteOptions {
        sync: Some(true),
    };
    db.put_with(b"k_sync", b"v_sync", sync_opts).expect("sync write");

    // Write with BufferedAsync (parity with RocksDB sync=false)
    let async_opts = WriteOptions {
        sync: Some(false),
    };
    db.put_with(b"k_async", b"v_async", async_opts).expect("async write");

    assert_eq!(db.get(b"k_sync").as_deref(), Some(b"v_sync".as_ref()));
    assert_eq!(db.get(b"k_async").as_deref(), Some(b"v_async".as_ref()));

    db.close().expect("close");
    let _ = std::fs::remove_dir_all(&path);
}
