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
    assert!(
        snap >= 2,
        "begin after reopen must not reset to 0, snap={snap}"
    );
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
    let _ = tx
        .get_range(&c, &keys[0][..keys[0].len().saturating_sub(0)], b"\xff")
        .unwrap();
    // Concurrent multi-range write that includes the seed key.
    c.commit_tx([
        (keys[0].as_slice(), b"other".as_slice()),
        (keys[1].as_slice(), b"x".as_slice()),
    ])
    .unwrap();
    tx.set(b"zzz-out", b"1").unwrap();
    let err = tx
        .commit(&mut c)
        .expect_err("range OCC vs multi-range write");
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
    assert!(
        c.range_leader(1).is_some_and(|l| l != 1),
        "leader must not be partitioned node"
    );
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

/// Fold `changelog_after` must not read a partitioned `ids[0]` (F42 class).
#[test]
fn changelog_after_skips_lagging_first_node() {
    let dir = temp();
    let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
    c.elect_all(80).unwrap();
    c.put(b"/host/h1/old", b"v0").unwrap();
    c.set_participating(1, false).unwrap();
    c.elect_all(120).unwrap();
    assert!(c.range_leader(1).is_some_and(|l| l != 1));
    c.put(b"/host/h1/new", b"v1").unwrap();
    let feed = c.changelog_after(0);
    assert!(
        feed.iter().any(|e| e.key.as_ref() == b"/host/h1/new"),
        "changelog_after used lagging node 1, missing new key: {feed:?}"
    );
    let snap = c.read_version();
    let ranged = c
        .keys_in_range_at(b"/host/h1/", b"/host/h10", snap)
        .unwrap();
    assert!(
        ranged
            .iter()
            .any(|(k, v)| k.as_slice() == b"/host/h1/new" && v.as_slice() == b"v1"),
        "get_range/keys_in_range_at used lagging node 1, missing new key: {ranged:?}"
    );
    // F72: default get() must not read lagging ids[0] either.
    assert_eq!(
        c.get(b"/host/h1/new").unwrap().as_deref(),
        Some(b"v1".as_ref()),
        "get() used lagging node 1, missing new key"
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

/// F47: client heard finish fail; heal+reopen+elect must not majority-install the TX.
///
/// No `tx_cancel` before drop — that hid the soak (cancel after heal rewrote
/// Pedra while the committed `TxnCommit` stayed in the raft log).
#[test]
fn fail_after_mid_2pc_restores_preimage() {
    for seed in [0xF2C_FA17_u64, 0xF47_F47, 0x00C0_FFEE] {
        let dir = temp();
        let e1 = FailingEnv::passing();
        let e2 = FailingEnv::passing();
        let e3 = FailingEnv::passing();
        let keys;
        {
            let mut c = StoreCluster::open_with_envs_rng(
                &dir,
                3,
                3,
                [e1.clone(), e2.clone(), e3.clone()],
                pedradb_core::SeedRng::new(seed),
            )
            .unwrap();
            c.elect_all(80).unwrap();
            keys = keys_one_per_range(&c);
            assert!(keys.len() >= 2);
            c.put(&keys[0], b"old-a").unwrap();
            c.put(&keys[1], b"old-b").unwrap();
            let h = c
                .tx_start([
                    (keys[0].as_slice(), b"new-a".as_slice()),
                    (keys[1].as_slice(), b"new-b".as_slice()),
                ])
                .expect("prepare");
            e1.arm_op_class(OpClass::Write, 0, false, FaultKind::IoError);
            e2.arm_op_class(OpClass::Write, 0, false, FaultKind::IoError);
            let err = c.tx_finish(&h);
            assert!(
                err.is_err(),
                "seed {seed:#x}: finish under dead majority must fail, got {err:?}"
            );
            assert!(
                c.count_applied_eq(&keys[0], b"new-a") < 2,
                "seed {seed:#x}: new-a majority-applied after failed finish"
            );
            assert!(
                c.count_applied_eq(&keys[1], b"new-b") < 2,
                "seed {seed:#x}: new-b majority-applied after failed finish"
            );
            // Heal disks, then crash the process — no cancel, no elect.
            e1.arm(u64::MAX, false);
            e2.arm(u64::MAX, false);
            drop(c);
        }
        let mut c = StoreCluster::open_with_envs_rng(
            &dir,
            3,
            3,
            [e1.clone(), e2.clone(), e3.clone()],
            pedradb_core::SeedRng::new(seed ^ 0xA5A5_A5A5),
        )
        .unwrap();
        c.elect_all(80).unwrap();
        assert!(
            c.count_applied_eq(&keys[0], b"new-a") < 2,
            "seed {seed:#x}: reopen+elect majority-installed new-a (F47)"
        );
        assert!(
            c.count_applied_eq(&keys[1], b"new-b") < 2,
            "seed {seed:#x}: reopen+elect majority-installed new-b (F47)"
        );
        let got0 = c.get(&keys[0]).unwrap();
        assert_eq!(
            got0.as_deref(),
            Some(b"old-a".as_ref()),
            "seed {seed:#x}: preimage lost after reopen, got {got0:?}"
        );
        c.put(&keys[0], b"after")
            .expect("put after heal+reopen must not Conflict on leftover intent");
        assert!(c.count_applied_eq(&keys[0], b"after") >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
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
    // Flip a byte in the durable SI hist row on *every* node so no peer has
    // a good hist. CRC reject on all → key absent from key_history.
    {
        let opts = OpenOptions {
            wal_recovery: Default::default(),
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
        };
        let mut hk = b"\0store/hist/".to_vec();
        hk.extend_from_slice(b"hk");
        for nid in 1..=3u64 {
            let mut db = Db::open_with(dir.join(format!("store-node-{nid}")), opts).unwrap();
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
    }
    let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
    c.elect_all(40).unwrap();
    // Tip may still read Pedra; snapshot 0 must NOT invent tip as pre-history.
    let tip = c.get(b"hk").unwrap();
    assert_eq!(tip.as_deref(), Some(b"v1".as_ref()), "tip still v1");
    let at0 = c.get_at_version(b"hk", 0).unwrap();
    assert_ne!(
        at0.as_deref(),
        Some(b"v1".as_ref()),
        "F50: bitrot hist on all peers must not silently serve tip as snapshot 0, got {at0:?}"
    );
    assert!(
        at0.is_none(),
        "F50: expected None at snapshot 0 when hist unusable, got {at0:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// F56: store DCS TTL is an absolute `now_ms` deadline in the raft log, but
/// `now_ms` itself was RAM-only. After the clock passed the deadline the key
/// is absent; reopen reset the clock to 0 and the expired lock reanimated
/// (F7 class — HA fence comes back).
#[test]
fn dcs_ttl_expired_stays_dead_after_reopen() {
    let dir = temp();
    let key = pedradb_store::meta_key(b"leader-lock");
    {
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.set_ms_per_tick(0);
        c.elect_all(80).unwrap();
        c.dcs_create_ttl(&key, b"node-a", 100).unwrap();
        assert!(c.dcs_get_on(1, &key).unwrap().is_some());
        c.advance_now_ms(100);
        assert!(
            c.dcs_get_on(1, &key).unwrap().is_none(),
            "expired lease must be absent before crash"
        );
        drop(c);
    }
    {
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.set_ms_per_tick(0);
        assert!(
            c.dcs_get_on(1, &key).unwrap().is_none(),
            "expired DCS lease must not reanimate after reopen (now_ms reset)"
        );
        c.elect_all(80).unwrap();
        // Lock is free: a new holder can take it.
        let rev = c
            .dcs_create_ttl(&key, b"node-b", 500)
            .expect("expired lock must be reclaimable after reopen");
        assert!(rev >= 1);
        assert_eq!(c.dcs_get_on(2, &key).unwrap().unwrap().value, b"node-b");
        drop(c);
    }
    // Still-valid TTL must survive a crash (do not expire everything on open).
    {
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.set_ms_per_tick(0);
        c.elect_all(40).unwrap();
        let live = pedradb_store::meta_key(b"live-lock");
        c.dcs_create_ttl(&live, b"hold", 10_000).unwrap();
        drop(c);
        let c = StoreCluster::open(&dir, 3, 1).unwrap();
        let kv = c
            .dcs_get_on(1, &live)
            .unwrap()
            .expect("unexpired TTL lease must survive reopen");
        assert_eq!(kv.value.as_slice(), b"hold");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// F168 repro: a snapshot below the GC watermark must fail closed
/// (`TransactionTooOld`), not fabricate key absence. Before the fix,
/// `get_at_version` returned `Ok(None)` once GC pruned the entries that
/// covered the snapshot — an old TX silently observed committed data as
/// deleted while reads were still being served.
#[test]
fn snapshot_below_watermark_fails_closed_not_absent() {
    let dir = temp();
    let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
    c.elect_all(80).unwrap();
    c.put(b"f168", b"committed").unwrap();
    let snapshot = c.read_version();
    assert!(snapshot >= 1);
    // Sanity: the snapshot reads the committed value before GC.
    assert_eq!(
        c.get_at_version(b"f168", snapshot).unwrap().as_deref(),
        Some(b"committed".as_ref())
    );
    // Advance past VERSION_RETENTION (64) so GC prunes below the watermark.
    for i in 0..70u32 {
        c.put(format!("f168-churn-{i}").as_bytes(), b"x").unwrap();
    }
    assert!(
        c.safe_watermark() > snapshot,
        "GC must have advanced past the snapshot (wm={})",
        c.safe_watermark()
    );
    let got = c.get_at_version(b"f168", snapshot);
    match got {
        Err(StoreError::TransactionTooOld { .. }) => {}
        other => panic!(
            "snapshot {} below watermark {} must be TransactionTooOld, got {:?}",
            snapshot,
            c.safe_watermark(),
            other
        ),
    }
    drop(c);
    let _ = std::fs::remove_dir_all(&dir);
}
