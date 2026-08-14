//! DST-style probes for Montanha primitives (FDB-class TX / 2PC / SI).
//!
//! These encode the same bug classes the determinismo portfolio hunts in
//! Pedra, RBS, and FDB: compensating-action data loss, immortal intents
//! after crash (F7 class), and snapshot isolation that evaporates on reopen.

use pedradb_store::{StoreCluster, StoreError};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let i = N.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("montanha-dst-{n}-{i}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn keys_one_per_range(c: &StoreCluster) -> Vec<Vec<u8>> {
    c.range_metas()
        .iter()
        .map(|r| {
            if r.start.is_empty() {
                vec![0x00, b'k']
            } else {
                let mut k = r.start.clone();
                k.push(b'k');
                k
            }
        })
        .collect()
}

/// Failed 2PC finish must restore the *preimage*, not delete the user key.
///
/// FDB-class I-TX-2 is "fail ⇒ no partial *new* apply", not "fail ⇒ drop
/// whatever used to be there". Revert-as-delete is silent data loss.
#[test]
fn revert_restores_preimage_on_partial_commit() {
    let dir = temp();
    let mut c = StoreCluster::open(&dir, 3, 3).unwrap();
    c.elect_all(80).unwrap();
    let keys = keys_one_per_range(&c);
    assert!(keys.len() >= 2);
    c.put(&keys[0], b"old-a").unwrap();
    c.put(&keys[1], b"old-b").unwrap();
    assert!(c.count_applied_eq(&keys[0], b"old-a") >= 2);

    let h = c
        .tx_start([
            (keys[0].as_slice(), b"new-a".as_slice()),
            (keys[1].as_slice(), b"new-b".as_slice()),
        ])
        .expect("prepare");
    let last = *h.ranges.last().unwrap();
    let _ = c.step_down_range_leader(last);
    let err = c.tx_finish(&h).expect_err("finish without last leader");
    assert!(
        matches!(
            err,
            StoreError::NotLeader { .. } | StoreError::NotCommitted { .. }
        ),
        "got {err:?}"
    );

    // New values must not be majority-applied.
    assert_eq!(c.count_applied_eq(&keys[0], b"new-a"), 0);
    assert_eq!(c.count_applied_eq(&keys[1], b"new-b"), 0);

    // Preimages must survive. Delete-on-revert is silent data loss.
    let got0 = c.get(&keys[0]).unwrap();
    assert_eq!(
        got0.as_deref(),
        Some(b"old-a".as_ref()),
        "revert must restore preimage of key0, got {got0:?}"
    );
    let got1 = c.get(&keys[1]).unwrap();
    assert_eq!(
        got1.as_deref(),
        Some(b"old-b".as_ref()),
        "revert must restore preimage of key1, got {got1:?}"
    );

    c.elect_all(120).unwrap();
    assert_eq!(
        c.get(&keys[0]).unwrap().as_deref(),
        Some(b"old-a".as_ref()),
        "preimage must still be there after re-elect"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Crash after durable prepare must not immortalize intents (F7 class).
///
/// `next_txn_id` reset to 1 plus leftover intents also collides with the
/// next TX id (self-id match skips conflict).
#[test]
fn crash_after_prepare_does_not_immortalize_intents() {
    let dir = temp();
    let keys;
    {
        let mut c = StoreCluster::open(&dir, 3, 3).unwrap();
        c.elect_all(80).unwrap();
        keys = keys_one_per_range(&c);
        let _h = c
            .tx_start([
                (keys[0].as_slice(), b"p0".as_slice()),
                (keys[1].as_slice(), b"p1".as_slice()),
            ])
            .expect("prepare durable");
        // Process crash: drop without tx_finish / tx_cancel.
        drop(c);
    }
    let mut c = StoreCluster::open(&dir, 3, 3).unwrap();
    c.elect_all(80).unwrap();
    c.put(&keys[0], b"after-crash")
        .expect("put after crash-reopen must not Conflict on leftover intent");
    assert!(c.count_applied_eq(&keys[0], b"after-crash") >= 2);
    c.commit_tx([
        (keys[0].as_slice(), b"tx0".as_slice()),
        (keys[1].as_slice(), b"tx1".as_slice()),
    ])
    .expect("commit_tx after crash-reopen must not collide on reused txn id");
    assert!(c.count_applied_eq(&keys[0], b"tx0") >= 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Snapshot isolation + OCC must survive process reopen (RFC-0023).
///
/// In-memory `key_history` / `commit_generation` evaporating means:
/// begin() resets to 0, get_at_version falls back to Pedra tip, and a
/// concurrent overwrite is visible inside the TX (read skew).
#[test]
fn snapshot_isolation_survives_reopen() {
    let dir = temp();
    {
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"sk", b"v0").unwrap();
        c.put(b"sk", b"v1").unwrap();
        assert!(c.read_version() >= 2);
        drop(c);
    }
    let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
    c.elect_all(80).unwrap();
    assert!(
        c.read_version() >= 2,
        "commit_generation must be durable, got {}",
        c.read_version()
    );
    let mut tx = c.begin();
    let snap = tx.snapshot_version();
    assert!(snap >= 2, "begin after reopen must not reset to 0, snap={snap}");
    assert_eq!(
        tx.get(&c, b"sk").unwrap().as_deref(),
        Some(b"v1".as_ref()),
        "snapshot at tip after reopen"
    );
    // Concurrent overwrite after begin.
    c.put(b"sk", b"v2").unwrap();
    assert_eq!(
        tx.get(&c, b"sk").unwrap().as_deref(),
        Some(b"v1".as_ref()),
        "read skew after reopen: TX saw post-begin commit"
    );
    tx.set(b"other", b"x").unwrap();
    let err = tx.commit(&mut c).expect_err("OCC after reopen");
    assert!(matches!(err, StoreError::Conflict), "got {err:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
