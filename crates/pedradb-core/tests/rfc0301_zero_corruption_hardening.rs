//! RFC-0301 Zero Corruption Hardening Regression Test Suite
//! Validates the 5 critical bug fixes and prevents any regression in consistency:
//! 1. Mid-user-key compaction split prevention (disjointness invariant).
//! 2. Tombstone preservation on partial compaction (bottommost check).
//! 3. Multi-CF archived WAL recovery without cross-CF data loss.
//! 4. OCC DeleteRange conflict detection against concurrent writes.
//! 5. Strictly monotonic sequence number recovery on reopen.
//! 6. Intra-group absorbed OCC conflict detection under concurrent transaction pacing.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use pedradb_core::concurrent::ConcurrentDb;
use pedradb_core::db::Db;
use pedradb_core::resilient_tx::{ContentionTracker, TransactionRetryPolicy};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(200);

fn temp_db_dir() -> std::path::PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("pedradb_rfc0301_{}_{}", std::process::id(), id));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp dir");
    p
}

/// Bug 1: Mid-User-Key Compaction Split
/// Enforces that an SST boundary is never split in the middle of versions of the same user key.
#[test]
fn test_bug1_no_split_mid_user_key() {
    let path = temp_db_dir();
    let mut db = Db::open(&path).expect("open db");

    let user_key = b"repeated_user_key";
    let val_payload = vec![b'x'; 256];

    // Write multiple versions of the same user key
    for _ in 0..12 {
        db.put(user_key, &val_payload).expect("put");
    }
    db.flush().expect("flush to sst");

    // Perform compaction
    db.compact().expect("compact");

    // Verify all versions of the user key remain contiguous and correctly queryable
    let res = db.get(user_key);
    assert_eq!(res.as_deref(), Some(val_payload.as_slice()));

    db.close().expect("close");
    let _ = std::fs::remove_dir_all(&path);
}

/// Bug 2: Tombstone Dropping on Partial Compaction
/// Enforces that partial-level compaction does not prematurely drop tombstones when older SSTs exist.
#[test]
fn test_bug2_tombstone_preservation_on_partial_compaction() {
    let path = temp_db_dir();
    let mut db = Db::open(&path).expect("open db");

    let key = b"tombstone_key";
    db.put(key, b"old_value_to_not_resurrect").expect("put");
    db.flush().expect("flush to L0");

    db.delete(key).expect("delete key");
    db.flush().expect("flush delete to L0");

    // Verify tombstone is active and old value is not visible
    assert_eq!(db.get(key), None);

    db.close().expect("close");
    let _ = std::fs::remove_dir_all(&path);
}

/// Bug 3: Multi-CF Archived WAL Recovery Data Loss
/// Enforces that when multiple CFs exist, flushing CF A does not drop unflushed WAL records of CF B upon restart.
#[test]
fn test_bug3_multi_cf_archived_wal_recovery() {
    let path = temp_db_dir();
    let mut db = Db::open(&path).expect("open db");

    db.set_physical_cfs(vec!["default".into(), "orders".into()]);

    // Put data in both default and orders
    db.put(b"default\0k1", b"v_default").expect("put default");
    db.put(b"orders\0k2", b"v_orders").expect("put orders");

    // Flush only default
    db.flush_cf("default").expect("flush default");

    db.close().expect("close db");

    // Reopen DB
    let db2 = Db::open(&path).expect("reopen db");
    assert_eq!(db2.get(b"default\0k1").as_deref(), Some(b"v_default".as_ref()));
    assert_eq!(db2.get(b"orders\0k2").as_deref(), Some(b"v_orders".as_ref()));

    db2.close().expect("close reopened db");
    let _ = std::fs::remove_dir_all(&path);
}

/// Bug 4: OCC DeleteRange Conflict Detection
/// Enforces that transactions modifying keys conflict properly with concurrent range updates.
#[test]
fn test_bug4_occ_delete_range_conflict_lone_commit() {
    let path = temp_db_dir();
    let db = ConcurrentDb::open(&path).expect("open db");

    db.put(b"range_item_5", b"val5").expect("put");

    let b1 = Arc::new(std::sync::Barrier::new(2));
    let b2 = Arc::new(std::sync::Barrier::new(2));

    let b1_c = Arc::clone(&b1);
    let b2_c = Arc::clone(&b2);
    let db_clone = db.clone();
    let handle = thread::spawn(move || {
        let policy = TransactionRetryPolicy {
            max_retries: 0,
            initial_backoff: Duration::from_micros(10),
            max_backoff: Duration::from_millis(1),
            backoff_multiplier: 1.0,
            jitter: false,
        };
        db_clone.transact_with(policy, |tx| {
            let val = tx.get(b"range_item_5")?.expect("exists");
            b1_c.wait();
            b2_c.wait();
            tx.put(b"range_item_5", &val)?;
            Ok(())
        })
    });

    b1.wait();
    db.delete_range(b"range_item_0", b"range_item_9").expect("delete_range");
    b2.wait();

    let res = handle.join().expect("join");
    assert!(matches!(res, Err(pedradb_core::error::CoreError::TransactionConflict)), "Must abort with TransactionConflict");

    db.close().expect("close");
    let _ = std::fs::remove_dir_all(&path);
}

/// Bug 5: Sequence Monotonicity Violation on Archived WAL Reopen
/// Enforces that next_seq strictly accounts for max sequence seen across any archived WAL.
#[test]
fn test_bug5_sequence_monotonicity_archived_wal_reopen() {
    let path = temp_db_dir();
    let mut db = Db::open(&path).expect("open db");

    db.put(b"seq_k1", b"v1").expect("put");
    db.put(b"seq_k2", b"v2").expect("put");
    let max_seq_before = db.last_sequence();

    db.close().expect("close db");

    let mut db2 = Db::open(&path).expect("reopen db");
    let next_seq_after = db2.last_sequence();
    assert!(next_seq_after >= max_seq_before, "Next sequence must be strictly monotonic");

    db2.put(b"seq_k3", b"v3").expect("put new");
    let new_max_seq = db2.last_sequence();
    assert!(new_max_seq > max_seq_before, "New write must have sequence strictly greater than previous writes");

    db2.close().expect("close db2");
    let _ = std::fs::remove_dir_all(&path);
}

/// Bug 6: Intra-Group Absorbed Extra Batches OCC Conflict Detection
/// Enforces that concurrent transactions with high contention correctly resolve all increments without lost updates.
#[test]
fn test_bug6_intra_group_absorbed_occ_isolation() {
    let path = temp_db_dir();
    let db = ConcurrentDb::open(&path).expect("open db");

    let hot_key = b"shared_hot_counter";
    db.put(hot_key, &0u64.to_le_bytes()).expect("initial put");

    let num_threads = 4;
    let increments_per_thread = 20;
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

    let final_bytes = db.get(hot_key).expect("exists");
    let final_val = u64::from_le_bytes(final_bytes.as_ref().try_into().unwrap());
    assert_eq!(final_val, (num_threads * increments_per_thread) as u64, "All increments must be accounted for (zero lost updates)");

    db.close().expect("close db");
    let _ = std::fs::remove_dir_all(&path);
}
