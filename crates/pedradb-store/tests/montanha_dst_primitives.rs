//! DST-style probes for Montanha primitives (FDB-class TX / 2PC / SI).
//!
//! These encode the same bug classes the determinismo portfolio hunts in
//! Pedra, RBS, and FDB: compensating-action data loss, immortal intents
//! after crash (F7 class), and snapshot isolation that evaporates on reopen.

use pedradb_core::{Db, OpenOptions};
use pedradb_sim::{FailingEnv, FaultKind, OpClass};
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

fn keys_one_per_range<E: pedradb_core::Env>(c: &StoreCluster<E>) -> Vec<Vec<u8>> {
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

/// Multi-range commit_tx must not expose intermediate SI generations that see
/// only a subset of the TX's keys (FDB commit version is one number).
#[test]
fn multi_range_commit_tx_single_generation_visibility() {
    let dir = temp();
    let mut c = StoreCluster::open(&dir, 3, 3).unwrap();
    c.elect_all(80).unwrap();
    let keys = keys_one_per_range(&c);
    assert!(keys.len() >= 2);
    let gen_before = c.read_version();
    c.commit_tx([
        (keys[0].as_slice(), b"a".as_slice()),
        (keys[1].as_slice(), b"b".as_slice()),
    ])
    .expect("cross-range commit");
    let gen_after = c.read_version();
    // Intermediate generations must not see only one key of the TX.
    for g in gen_before..=gen_after {
        let v0 = c.get_at_version(&keys[0], g).unwrap();
        let v1 = c.get_at_version(&keys[1], g).unwrap();
        let saw0 = v0.as_deref() == Some(b"a".as_ref());
        let saw1 = v1.as_deref() == Some(b"b".as_ref());
        assert_eq!(
            saw0, saw1,
            "gen {g}: partial multi-range visibility saw0={saw0} saw1={saw1} v0={v0:?} v1={v1:?} before={gen_before} after={gen_after}"
        );
    }
    // One logical TX → one generation bump (not one per range).
    assert_eq!(
        gen_after,
        gen_before + 1,
        "cross-range commit_tx must advance commit_generation once, before={gen_before} after={gen_after}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Duplicate keys in one commit_tx: last write wins, single intent, no double-apply mess.
#[test]
fn commit_tx_duplicate_keys_last_wins() {
    let dir = temp();
    let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
    c.elect_all(80).unwrap();
    c.commit_tx([
        (b"dup".as_slice(), b"first".as_slice()),
        (b"dup".as_slice(), b"second".as_slice()),
    ])
    .expect("dup keys in one TX");
    assert_eq!(
        c.get(b"dup").unwrap().as_deref(),
        Some(b"second".as_ref()),
        "last pair should win"
    );
    assert!(c.count_applied_eq(b"dup", b"second") >= 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// OCC range conflict must fire when a concurrent multi-range TX writes inside the range.
#[test]
fn multi_range_tx_conflicts_with_range_read() {
    let dir = temp();
    let mut c = StoreCluster::open(&dir, 3, 3).unwrap();
    c.elect_all(80).unwrap();
    let keys = keys_one_per_range(&c);
    assert!(keys.len() >= 2);
    c.put(&keys[0], b"seed").unwrap();
    let mut tx = c.begin();
    let _ = tx.get_range(&c, &keys[0][..keys[0].len().saturating_sub(0)], b"\xff").unwrap();
    // Concurrent multi-range write that includes the seed key.
    c.commit_tx([
        (keys[0].as_slice(), b"other".as_slice()),
        (keys[1].as_slice(), b"x".as_slice()),
    ])
    .unwrap();
    tx.set(b"zzz-out", b"1").unwrap();
    let err = tx.commit(&mut c).expect_err("range OCC vs multi-range write");
    assert!(
        matches!(err, StoreError::Conflict),
        "expected Conflict, got {err:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}


/// SI notes after commit_tx must not be sourced from a lagging `ids[0]` node.
///
/// `note_tx_commit` used `local_node_id().or(ids.first())` — with multi-node
/// in-process, that is always node 1. If node 1 is partitioned, majority commit
/// still succeeds on 2/3 but history records empty/stale values for the TX.
#[test]
fn note_tx_commit_reads_applied_not_lagging_first_node() {
    let dir = temp();
    let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
    c.elect_all(80).unwrap();
    c.put(b"k", b"old").unwrap();
    // Partition node 1 so it will not apply later commits.
    c.set_participating(1, false).unwrap();
    // Re-elect among 2/3 if needed.
    c.elect_all(120).unwrap();
    assert!(c.range_leader(1).is_some_and(|l| l != 1), "leader must not be partitioned node");
    let gen_before = c.read_version();
    c.commit_tx([(b"k".as_slice(), b"new".as_slice())])
        .expect("majority commit without node 1");
    let gen = c.read_version();
    assert_eq!(gen, gen_before + 1, "generation must advance");
    // Snapshot at new gen must see *new*, not empty/old from lagging node 1.
    let at = c.get_at_version(b"k", gen).unwrap();
    assert_eq!(
        at.as_deref(),
        Some(b"new".as_ref()),
        "SI history recorded lagging/first-node view: {at:?}"
    );
    // Strong path on live leader still sees new.
    assert_eq!(
        c.get_strong(b"k").unwrap().as_deref(),
        Some(b"new".as_ref())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `Transaction::clear` must Pedra-delete: get after commit is None, not Some([]).
#[test]
fn clear_is_real_pedra_delete() {
    let dir = temp();
    {
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"gone", b"here").unwrap();
        assert_eq!(c.get(b"gone").unwrap().as_deref(), Some(b"here".as_ref()));
        let mut tx = c.begin();
        tx.clear(b"gone").unwrap();
        tx.commit(&mut c).unwrap();
        assert_eq!(
            c.get(b"gone").unwrap().as_deref(),
            None,
            "clear must not leave an empty-value tombstone"
        );
        assert_eq!(c.get_at_version(b"gone", c.read_version()).unwrap(), None);
        drop(c);
    }
    let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
    c.elect_all(40).unwrap();
    assert_eq!(
        c.get(b"gone").unwrap().as_deref(),
        None,
        "delete must survive reopen"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// FailingEnv mid-2PC: finish cannot majority-apply; preimage stays; no stuck intents.
#[test]
fn fail_after_mid_2pc_restores_preimage() {
    let dir = temp();
    let e1 = FailingEnv::passing();
    let e2 = FailingEnv::passing();
    let e3 = FailingEnv::passing();
    let mut c = StoreCluster::open_with_envs_rng(
        &dir,
        3,
        3,
        [e1.clone(), e2.clone(), e3.clone()],
        pedradb_core::SeedRng::new(0xF2C_FA17),
    )
    .unwrap();
    c.elect_all(80).unwrap();
    let keys = keys_one_per_range(&c);
    assert!(keys.len() >= 2);
    c.put(&keys[0], b"old-a").unwrap();
    c.put(&keys[1], b"old-b").unwrap();
    let h = c
        .tx_start([
            (keys[0].as_slice(), b"new-a".as_slice()),
            (keys[1].as_slice(), b"new-b".as_slice()),
        ])
        .expect("prepare");
    // Majority disks dead for writes — finish must fail closed.
    e1.arm_op_class(OpClass::Write, 0, false, FaultKind::IoError);
    e2.arm_op_class(OpClass::Write, 0, false, FaultKind::IoError);
    let err = c.tx_finish(&h);
    assert!(err.is_err(), "finish under dead majority must fail, got {err:?}");
    // I-TX-2: fail ⇒ no *majority* new apply (a single leader replica may
    // have applied locally before replication died — LocalApplied, not I-MAJ).
    assert!(
        c.count_applied_eq(&keys[0], b"new-a") < 2,
        "new-a majority-applied after failed finish"
    );
    assert!(
        c.count_applied_eq(&keys[1], b"new-b") < 2,
        "new-b majority-applied after failed finish"
    );
    e1.arm(u64::MAX, false);
    e2.arm(u64::MAX, false);
    c.elect_all(80).unwrap();
    let _ = c.tx_cancel(&h);
    drop(c);
    // Reopen aborts leftover intents (F35) even if cancel could not write.
    let mut c = StoreCluster::open_with_envs_rng(
        &dir,
        3,
        3,
        [e1.clone(), e2.clone(), e3.clone()],
        pedradb_core::SeedRng::new(0xF2C_FA18),
    )
    .unwrap();
    c.elect_all(80).unwrap();
    assert!(
        c.count_applied_eq(&keys[0], b"new-a") < 2,
        "reopen must not majority-install new-a"
    );
    c.put(&keys[0], b"after")
        .expect("put after heal+reopen must not Conflict on leftover intent");
    assert!(c.count_applied_eq(&keys[0], b"after") >= 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Bitrot on `\0store/hist/` must not serve the tip as an old snapshot.
#[test]
fn hist_bitrot_does_not_silent_wrong_old_snapshot() {
    let dir = temp();
    {
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"hk", b"v0").unwrap();
        c.put(b"hk", b"v1").unwrap();
        assert!(c.read_version() >= 2);
        drop(c);
    }
    // Flip a byte in the durable SI hist row (CRC should reject on reload).
    {
        let opts = OpenOptions {
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
        };
        let mut db = Db::open_with(&dir.join("store-node-1"), opts).unwrap();
        let mut hk = b"\0store/hist/".to_vec();
        hk.extend_from_slice(b"hk");
        if let Some(raw) = db.get(&hk) {
            let mut flipped = raw.to_vec();
            if !flipped.is_empty() {
                let i = flipped.len() / 2;
                flipped[i] ^= 0xFF;
                db.put(&hk, &flipped).unwrap();
            }
        }
        drop(db);
    }
    let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
    c.elect_all(40).unwrap();
    // Tip read is fine; an *old* snapshot must not invent v1 from Pedra tip.
    let at0 = c.get_at_version(b"hk", 0).unwrap();
    assert_ne!(
        at0.as_deref(),
        Some(b"v1".as_ref()),
        "bitrot hist must not silently serve tip as snapshot 0, got {at0:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
