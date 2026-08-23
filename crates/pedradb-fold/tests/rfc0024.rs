//! RFC-0024 named acceptance tests (drive shipped fold API).

use pedradb_core::{Db, OpenOptions};
use pedradb_fold::{
    caixote_host_filter, cursor_expired, export_fold, fold_get_local, follow_prefix,
    follow_store_prefix, import_fold, last_per_key, resume_window_ok, resync_expired, ship_pull,
    state_sync_then_tail, watch_applied, watch_applied_prefix, FoldCursor, FoldRole, FoldStore,
    FoldUpdate, IntentObservedDelta, PedraFold, PrefixSet, SeqSyncState, WatchApplied,
};
use pedradb_journal::JournalConsumer;
use pedradb_replicate::WalShipper;
use pedradb_sim::{FailingEnv, FaultKind, OpClass};
use pedradb_store::StoreCluster;
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
    let d = std::env::temp_dir().join(format!("rfc0024-{n}-{i}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn opts() -> OpenOptions {
    OpenOptions {
        wal_full_fsync: true,
        history: Default::default(),
        sync: true,
        auto_flush_bytes: None,
        auto_compact_sst_count: None,
        auto_compact_sst_bytes: None,
        exclusive: true,
        large_value_threshold: None,
        wal_recovery: Default::default(),
    }
}

/// Crash between recv and apply: pin stays; resume does not skip.
#[test]
fn watch_applied_pin_after_apply() {
    let dir = temp();
    let src = dir.join("src");
    let fd = dir.join("fold");
    let mut db = Db::open_with(&src, opts()).unwrap();
    db.put(b"k", b"v1").unwrap();
    let (_c, fold) = PedraFold::open(&fd).unwrap();
    let mut wa = WatchApplied::new(fold);
    wa.recv(FoldUpdate::Put {
        key: b"k".to_vec(),
        value: b"v1".to_vec(),
        seq: 1,
    });
    // Simulated crash: drop without flush. Pin still 0.
    assert_eq!(wa.store().cursor().seq(), 0);
    assert!(!wa.pending().is_empty());
    let pending = wa.pending().to_vec();
    drop(wa);
    let (_c, fold) = PedraFold::open(&fd).unwrap();
    assert_eq!(fold.cursor().seq(), 0);
    let mut wa = WatchApplied::new(fold);
    for u in pending {
        wa.recv(u);
    }
    wa.flush().unwrap();
    assert_eq!(
        wa.store().get(b"k").unwrap().as_deref(),
        Some(b"v1".as_ref())
    );
    assert_eq!(wa.store().cursor().seq(), 1);
    // Second flush of empty pending does not skip.
    wa.flush().unwrap();
    assert_eq!(wa.store().cursor().seq(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Torn / failed apply cannot leave cursor ahead of data.
#[test]
fn fold_apply_cursor_atomic() {
    let dir = temp();
    let fd = dir.join("fold");
    let env = FailingEnv::passing();
    let (_c, mut fold) = PedraFold::open_role_env(&fd, FoldRole::Storage, env.clone()).unwrap();
    env.arm_op_class(OpClass::Write, 0, false, FaultKind::IoError);
    let err = fold.apply_updates(
        &[FoldUpdate::Put {
            key: b"k".to_vec(),
            value: b"v".to_vec(),
            seq: 3,
        }],
        FoldCursor(3),
    );
    assert!(err.is_err(), "apply under dead disk must fail");
    assert_eq!(
        fold.applied_cursor().seq(),
        0,
        "pin must not advance on failed apply"
    );
    env.arm(u64::MAX, false);
    drop(fold);
    let (_c, fold) = PedraFold::open(&fd).unwrap();
    assert_eq!(fold.cursor().seq(), 0);
    assert!(fold.get(b"k").unwrap().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Transient apply re-queues; pin unchanged (F47 class).
#[test]
fn fold_transient_apply_does_not_advance_pin() {
    struct FailOnce {
        inner: PedraFold,
        fail: bool,
    }
    impl FoldStore for FailOnce {
        fn open(_path: &std::path::Path) -> pedradb_fold::Result<(FoldCursor, Self)> {
            Err(pedradb_fold::FoldError::Msg("test store".into()))
        }
        fn apply(&mut self, batch: &[FoldUpdate], cursor: FoldCursor) -> pedradb_fold::Result<()> {
            if self.fail {
                return Err(pedradb_fold::FoldError::TransientApply("injected".into()));
            }
            self.inner.apply(batch, cursor)
        }
        fn get(&self, key: &[u8]) -> pedradb_fold::Result<Option<Vec<u8>>> {
            self.inner.get(key)
        }
        fn range(&self, prefix: &[u8]) -> pedradb_fold::Result<Vec<(Vec<u8>, Vec<u8>)>> {
            self.inner.range(prefix)
        }
        fn cursor(&self) -> FoldCursor {
            self.inner.cursor()
        }
    }

    let dir = temp();
    let fd = dir.join("fold");
    let (_c, fold) = PedraFold::open(&fd).unwrap();
    let mut wa = WatchApplied::new(FailOnce {
        inner: fold,
        fail: true,
    });
    wa.recv(FoldUpdate::Put {
        key: b"a".to_vec(),
        value: b"1".to_vec(),
        seq: 2,
    });
    assert!(wa.flush().is_err(), "injected apply failure");
    assert_eq!(wa.store().cursor().seq(), 0, "pin must not advance");
    assert_eq!(wa.pending().len(), 1, "batch stays queued");
    wa.store_mut().fail = false;
    wa.flush().unwrap();
    assert!(wa.pending().is_empty());
    assert_eq!(wa.store().cursor().seq(), 2);
    assert_eq!(
        wa.store().get(b"a").unwrap().as_deref(),
        Some(b"1".as_ref())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// F53: missing CHANGELOG after flush still seeds fold (SST last-per-key rebuild).
#[test]
fn fold_follow_after_changelog_deleted_post_flush() {
    let dir = temp();
    let src = dir.join("src");
    {
        let mut db = Db::open_with(&src, opts()).unwrap();
        db.put(b"/host/h1/a", b"1").unwrap();
        db.put(b"/host/h1/b", b"2").unwrap();
        db.flush().unwrap();
        drop(db);
    }
    std::fs::remove_file(src.join(pedradb_core::CHANGELOG_FILE_NAME)).unwrap();
    let db = Db::open_with(&src, opts()).unwrap();
    let prefixes = PrefixSet::one(b"/host/h1/");
    let ups = follow_prefix(&db, &prefixes, FoldCursor::none());
    assert!(
        ups.iter().any(|u| u.key() == b"/host/h1/a"),
        "F53 rebuild must feed fold, got {ups:?}"
    );
    assert!(ups.iter().any(|u| u.key() == b"/host/h1/b"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Only in-prefix keys appear.
#[test]
fn fold_follow_montanha_prefix() {
    let dir = temp();
    let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
    c.elect_all(80).unwrap();
    c.put(b"/host/h1/cap", b"ok").unwrap();
    c.put(b"/vm/vm-a", b"spec").unwrap();
    c.put(b"/other/x", b"nope").unwrap();
    let prefixes = caixote_host_filter("h1", &["vm-a"]);
    let ups = follow_store_prefix(&c, &prefixes, FoldCursor::none());
    assert!(
        ups.iter().any(|u| u.key() == b"/host/h1/cap"),
        "missing host key {ups:?}"
    );
    assert!(ups.iter().any(|u| u.key() == b"/vm/vm-a"));
    assert!(
        ups.iter().all(|u| u.key() != b"/other/x"),
        "out-of-prefix leaked {ups:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// F83: `/vm/{id}` was a raw `starts_with` prefix. Host owning `vm-a` also
/// followed `vm-ab` / `vm-a2` (and the same on `/assign/`).
#[test]
fn caixote_host_filter_does_not_include_vm_id_prefix_sibling() {
    let dir = temp();
    let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
    c.elect_all(80).unwrap();
    c.put(b"/host/h1/cap", b"ok").unwrap();
    c.put(b"/vm/vm-a", b"mine").unwrap();
    c.put(b"/vm/vm-ab", b"sib").unwrap();
    c.put(b"/vm/vm-a2", b"sib2").unwrap();
    c.put(b"/vm/vm-a/disk", b"child").unwrap();
    c.put(b"/assign/vm-a", b"hold").unwrap();
    c.put(b"/assign/vm-ab", b"leak").unwrap();
    let prefixes = caixote_host_filter("h1", &["vm-a"]);
    let ups = follow_store_prefix(&c, &prefixes, FoldCursor::none());
    let keys: Vec<&[u8]> = ups.iter().map(|u| u.key()).collect();
    assert!(
        keys.iter().any(|k| *k == b"/vm/vm-a"),
        "own vm missing: {keys:?}"
    );
    assert!(
        keys.iter().any(|k| *k == b"/assign/vm-a"),
        "own assign missing: {keys:?}"
    );
    assert!(
        keys.iter().any(|k| *k == b"/vm/vm-a/disk"),
        "child path under own vm should stay: {keys:?}"
    );
    assert!(
        !keys
            .iter()
            .any(|k| *k == b"/vm/vm-ab" || *k == b"/vm/vm-a2"),
        "vm-a filter included sibling vm id: {keys:?}"
    );
    assert!(
        !keys.iter().any(|k| *k == b"/assign/vm-ab"),
        "assign/vm-a filter included assign/vm-ab: {keys:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Host/VM fixture: reopen delivers only seq > pin.
#[test]
fn fold_resume_after_reopen() {
    let dir = temp();
    let src = dir.join("src");
    let fd = dir.join("fold");
    let prefixes = caixote_host_filter("h1", &["vm-a", "vm-b"]);
    let pin;
    {
        let mut db = Db::open_with(&src, opts()).unwrap();
        db.put(b"/host/h1/cap", b"c").unwrap();
        db.put(b"/vm/vm-a", b"a").unwrap();
        db.put(b"/vm/vm-b", b"b").unwrap();
        db.put(b"/other/z", b"no").unwrap();
        let (_c, mut fold) = PedraFold::open(&fd).unwrap();
        let mut cons = JournalConsumer::new();
        watch_applied_prefix(&db, &mut cons, &mut fold, Some(&prefixes)).unwrap();
        pin = fold.cursor();
        assert!(fold.get(b"/host/h1/cap").unwrap().is_some());
        assert!(fold.get(b"/other/z").unwrap().is_none());
        drop(fold);
        drop(db);
    }
    let mut db = Db::open_with(&src, opts()).unwrap();
    db.put(b"/vm/vm-a", b"a2").unwrap();
    let (_c, mut fold) = PedraFold::open(&fd).unwrap();
    assert_eq!(fold.cursor(), pin);
    let mut cons = JournalConsumer { pin: pin.seq() };
    watch_applied_prefix(&db, &mut cons, &mut fold, Some(&prefixes)).unwrap();
    let tail = follow_prefix(&db, &prefixes, pin);
    assert!(
        tail.iter().all(|u| u.seq() > pin.seq()),
        "reopen must only see seq > pin, got {tail:?}"
    );
    assert_eq!(
        fold.get(b"/vm/vm-a").unwrap().as_deref(),
        Some(b"a2".as_ref())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn watch_state_sync_then_tail() {
    let dir = temp();
    let mut db = Db::open_with(&dir.join("src"), opts()).unwrap();
    db.put(b"/route/a", b"1").unwrap();
    db.put(b"/route/a", b"2").unwrap();
    db.put(b"/route/b", b"x").unwrap();
    let p = pedradb_fold::PrefixSet::one(b"/route/");
    let seed = state_sync_then_tail(&db, &p, None, 0).unwrap();
    // Last-per-key: /route/a is 2, not 1.
    let a = seed.iter().find(|u| u.key() == b"/route/a").unwrap();
    match a {
        FoldUpdate::Put { value, .. } => assert_eq!(value, b"2"),
        _ => panic!("{a:?}"),
    }
    db.put(b"/route/c", b"live").unwrap();
    let high = seed.iter().map(FoldUpdate::seq).max().unwrap();
    let tail = state_sync_then_tail(&db, &p, Some(FoldCursor(high)), 0).unwrap();
    assert!(tail.iter().any(|u| u.key() == b"/route/c"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn watch_seed_then_watch_gap_closed() {
    let dir = temp();
    let mut db = Db::open_with(&dir.join("src"), opts()).unwrap();
    db.put(b"/route/old", b"1").unwrap();
    let p = pedradb_fold::PrefixSet::one(b"/route/");
    // Gap write between "list" and "subscribe" is included in state-sync.
    db.put(b"/route/gap", b"seen").unwrap();
    let all = state_sync_then_tail(&db, &p, None, 0).unwrap();
    assert!(
        all.iter().any(|u| u.key() == b"/route/gap"),
        "write between list and subscribe must be visible"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cursor_expired_synthetic_deletes() {
    let dir = temp();
    let src = dir.join("src");
    let fd = dir.join("fold");
    let p = pedradb_fold::PrefixSet::one(b"/vm/");
    let mut db = Db::open_with(&src, opts()).unwrap();
    db.put(b"/vm/gone", b"1").unwrap();
    db.put(b"/vm/stay", b"2").unwrap();
    let (_c, mut fold) = PedraFold::open(&fd).unwrap();
    let mut cons = JournalConsumer::new();
    watch_applied_prefix(&db, &mut cons, &mut fold, Some(&p)).unwrap();
    db.delete(b"/vm/gone").unwrap();
    db.put(b"/vm/new", b"3").unwrap();
    assert!(cursor_expired(FoldCursor(0), 99));
    assert!(resume_window_ok(5, 6));
    let pin = fold.cursor();
    let batch = resync_expired(&fold, &db, &p, pin).unwrap();
    let dels: Vec<_> = batch
        .iter()
        .take_while(|u| matches!(u, FoldUpdate::Delete { .. }))
        .collect();
    assert!(
        dels.iter().any(|u| u.key() == b"/vm/gone"),
        "synthetic delete first: {batch:?}"
    );
    fold.apply(&batch, FoldCursor(db.last_sequence())).unwrap();
    assert!(fold.get(b"/vm/gone").unwrap().is_none());
    assert_eq!(
        fold.get(b"/vm/new").unwrap().as_deref(),
        Some(b"3".as_ref())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fold_export_import_cursor_eq() {
    let dir = temp();
    let fd = dir.join("fold");
    let dest = dir.join("export");
    let (_c, mut fold) = PedraFold::open(&fd).unwrap();
    fold.apply(
        &[FoldUpdate::Put {
            key: b"/route/x".to_vec(),
            value: b"v".to_vec(),
            seq: 7,
        }],
        FoldCursor(7),
    )
    .unwrap();
    let live = fold.cursor();
    let got = export_fold(&mut fold, &dest).unwrap();
    assert_eq!(got, live);
    drop(fold);
    let (imp, store) = import_fold(&dest, FoldRole::Storage).unwrap();
    assert_eq!(imp, live);
    assert_eq!(
        store.get(b"/route/x").unwrap().as_deref(),
        Some(b"v".as_ref())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wal_rotate_is_cursor_expired() {
    let dir = temp();
    let src = dir.join("src");
    let mut db = Db::open_with(&src, opts()).unwrap();
    db.put(b"a", b"1").unwrap();
    let mut ship = WalShipper::from_start(&src);
    let _ = ship.pull().unwrap();
    db.flush().unwrap(); // rotates / truncates WAL
    let err = ship_pull(&mut ship);
    match err {
        Err(pedradb_fold::FoldError::CursorExpired { .. }) => {}
        other => panic!("expected CursorExpired, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn seq_sync_delta_primary_dump_backstop() {
    let dir = temp();
    let (_c, mut fold) = PedraFold::open(&dir.join("fold")).unwrap();
    let mut st = SeqSyncState::default();
    let mut d = IntentObservedDelta::empty(3);
    d.puts
        .push((b"/host/h1/observed/vm-a".to_vec(), b"up".to_vec()));
    fold_get_local(&fold, b"/host/h1/observed/vm-a")
        .unwrap()
        .ok_or(())
        .err();
    st.apply_delta(&mut fold, &d).unwrap();
    assert_eq!(
        fold_get_local(&fold, b"/host/h1/observed/vm-a")
            .unwrap()
            .as_deref(),
        Some(b"up".as_ref())
    );
    assert!(!st.should_full_dump(100));
    assert!(st.should_full_dump(6000));
    st.note_full_dump(6000, 3);
    assert_eq!(st.sync_full_dump, 1);
    assert!(st.sync_delta_keys >= 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fold_read_is_not_get_strong() {
    let dir = temp();
    let (_c, mut fold) = PedraFold::open(&dir.join("fold")).unwrap();
    fold.apply(
        &[FoldUpdate::Put {
            key: b"/route/svc/80".to_vec(),
            value: b"10.0.0.1".to_vec(),
            seq: 1,
        }],
        FoldCursor(1),
    )
    .unwrap();
    // LocalApplied fold get — no StoreCluster::get_strong in this path.
    let v = fold_get_local(&fold, b"/route/svc/80").unwrap();
    assert_eq!(v.as_deref(), Some(b"10.0.0.1".as_ref()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn proxy_keeps_keys_evicts_values() {
    let dir = temp();
    let (_c, mut fold) =
        PedraFold::open_role(&dir.join("fold"), FoldRole::Proxy { evict_values: true }).unwrap();
    fold.apply(
        &[FoldUpdate::Put {
            key: b"/route/a".to_vec(),
            value: b"secret".to_vec(),
            seq: 1,
        }],
        FoldCursor(1),
    )
    .unwrap();
    assert!(fold.contains_key(b"/route/a"));
    assert!(fold.get(b"/route/a").unwrap().is_none());
    let (_c, mut relay) = PedraFold::open_role(&dir.join("relay"), FoldRole::Relay).unwrap();
    relay
        .apply(
            &[FoldUpdate::Put {
                key: b"/route/a".to_vec(),
                value: b"x".to_vec(),
                seq: 1,
            }],
            FoldCursor(1),
        )
        .unwrap();
    assert!(relay.get(b"/route/a").unwrap().is_none());
    assert_eq!(relay.cursor().seq(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn watch_applied_from_changelog() {
    let dir = temp();
    let mut db = Db::open_with(&dir.join("src"), opts()).unwrap();
    db.put(b"k", b"1").unwrap();
    let (_c, mut fold) = PedraFold::open(&dir.join("fold")).unwrap();
    let mut cons = JournalConsumer::new();
    watch_applied(&db, &mut cons, &mut fold).unwrap();
    assert_eq!(cons.pin, fold.cursor().seq());
    assert_eq!(fold.get(b"k").unwrap().as_deref(), Some(b"1".as_ref()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Prefix scan must include keys whose next byte is 0xff (exclusive end is
/// successor, not `prefix || 0xff`).
///
/// `range_user` used `end = prefix + [0xff]` exclusive, so `prefix || 0xff…`
/// vanished from fold range / resync (silent missing keys).
#[test]
fn fold_range_includes_ff_suffix_keys() {
    let dir = temp();
    let (_c, mut fold) = PedraFold::open(&dir.join("fold")).unwrap();
    let prefix = b"/host/h1/".as_slice();
    let mid = {
        let mut k = prefix.to_vec();
        k.extend_from_slice(b"abc");
        k
    };
    let ff = {
        let mut k = prefix.to_vec();
        k.push(0xff);
        k.extend_from_slice(b"z");
        k
    };
    let outside = b"/host/h2/x".to_vec();
    fold.apply(
        &[
            FoldUpdate::Put {
                key: mid.clone(),
                value: b"1".to_vec(),
                seq: 1,
            },
            FoldUpdate::Put {
                key: ff.clone(),
                value: b"2".to_vec(),
                seq: 2,
            },
            FoldUpdate::Put {
                key: outside.clone(),
                value: b"3".to_vec(),
                seq: 3,
            },
        ],
        FoldCursor(3),
    )
    .unwrap();
    assert_eq!(fold.get(&ff).unwrap().as_deref(), Some(b"2".as_ref()));
    let rows = fold.range(prefix).unwrap();
    let keys: Vec<&[u8]> = rows.iter().map(|(k, _)| k.as_slice()).collect();
    assert!(
        keys.contains(&mid.as_slice()),
        "plain suffix missing: {keys:?}"
    );
    assert!(
        keys.contains(&ff.as_slice()),
        "0xff-suffix key missing from prefix range (end=prefix||0xff): {keys:?}"
    );
    assert!(
        !keys.contains(&outside.as_slice()),
        "must not leak sibling prefix: {keys:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// User keys that start with `0x00` are valid (FDB tuples / packed keys).
/// `range_user` skipped every `\0…` key as fold meta, so get saw them and
/// prefix range / resync did not.
#[test]
fn fold_range_includes_nul_prefixed_user_keys() {
    let dir = temp();
    let (_c, mut fold) = PedraFold::open(&dir.join("fold")).unwrap();
    let nul_key = {
        let mut k = vec![0x00];
        k.extend_from_slice(b"user");
        k
    };
    fold.apply(
        &[FoldUpdate::Put {
            key: nul_key.clone(),
            value: b"v".to_vec(),
            seq: 1,
        }],
        FoldCursor(1),
    )
    .unwrap();
    assert_eq!(
        fold.get(&nul_key).unwrap().as_deref(),
        Some(b"v".as_ref()),
        "point get must see 0x00-prefixed user key"
    );
    let rows = fold.range(&[0x00]).unwrap();
    assert!(
        rows.iter().any(|(k, v)| k == &nul_key && v == b"v"),
        "0x00-prefixed user key missing from range (skipped as meta): {rows:?}"
    );
    // Fold meta must stay hidden.
    assert!(
        !rows
            .iter()
            .any(|(k, _)| k == b"\0fold/cursor" || k.starts_with(b"\0fold/keyset/")),
        "range leaked fold meta: {rows:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// CHANGELOG follow / last_per_key used raw prefix match, so `\0fold/cursor`
/// and `\0fold/keyset/…` leaked into state-sync when the prefix was `0x00`.
#[test]
fn fold_changelog_sync_skips_fold_meta() {
    let dir = temp();
    let (_c, mut fold) = PedraFold::open(&dir.join("fold")).unwrap();
    let nul_key = {
        let mut k = vec![0x00];
        k.extend_from_slice(b"user");
        k
    };
    fold.apply(
        &[FoldUpdate::Put {
            key: nul_key.clone(),
            value: b"v".to_vec(),
            seq: 1,
        }],
        FoldCursor(1),
    )
    .unwrap();
    let prefs = PrefixSet::one([0x00]);
    let sync = last_per_key(fold.db_mut(), &prefs);
    let keys: Vec<&[u8]> = sync.iter().map(FoldUpdate::key).collect();
    assert!(
        keys.iter().any(|k| *k == nul_key.as_slice()),
        "state-sync must include 0x00 user key: {keys:?}"
    );
    assert!(
        !keys.iter().any(|k| k.starts_with(b"\0fold/")),
        "last_per_key leaked fold meta under 0x00 prefix: {keys:?}"
    );
    let tail = follow_prefix(fold.db_mut(), &prefs, FoldCursor(0));
    assert!(
        !tail.iter().any(|u| u.key().starts_with(b"\0fold/")),
        "follow_prefix leaked fold meta: {tail:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// F69: applying a user key under `\0fold/` must fail (would clobber cursor).
#[test]
fn fold_rejects_reserved_meta_user_keys() {
    let dir = temp();
    let (_c, mut fold) = PedraFold::open(&dir.join("fold")).unwrap();
    let err = fold
        .apply(
            &[FoldUpdate::Put {
                key: b"\0fold/cursor".to_vec(),
                value: b"evil".to_vec(),
                seq: 1,
            }],
            FoldCursor(1),
        )
        .expect_err("must reject reserved fold meta key");
    let msg = format!("{err}");
    assert!(
        msg.contains("reserved") || msg.contains("fold meta"),
        "unexpected err: {msg}"
    );
    // Cursor must still be 0 / not corrupted by evil put.
    assert_eq!(fold.cursor().0, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// F169: a range tombstone must hide every covered key in last-per-key
/// state-sync, not just the range-start key. Before the fix, `entry_to_update`
/// mapped `DeleteRange` to a point delete of `e.key` (the start) and
/// `last_per_key` last-write-wins per key, so a covered put survived as live.
#[test]
fn last_per_key_applies_range_tombstone_coverage() {
    let dir = temp();
    let src = dir.join("src");
    let mut db = Db::open_with(&src, opts()).unwrap();
    db.put(b"k-a", b"va").unwrap();
    db.put(b"k-c", b"vc").unwrap();
    db.put(b"k-e", b"ve").unwrap();
    db.delete_range(b"k-b", b"k-d").unwrap();
    assert_eq!(db.get(b"k-a").as_deref(), Some(b"va".as_ref()));
    assert_eq!(
        db.get(b"k-c"),
        None,
        "source hides k-c under the range tombstone"
    );
    assert_eq!(db.get(b"k-e").as_deref(), Some(b"ve".as_ref()));

    let prefs = PrefixSet::one(b"k-");
    let sync = last_per_key(&db, &prefs);
    let live: Vec<&[u8]> = sync
        .iter()
        .filter(|u| matches!(u, FoldUpdate::Put { .. }))
        .map(FoldUpdate::key)
        .collect();
    assert!(
        !live.iter().any(|k| *k == b"k-c"),
        "last_per_key must not resurrect k-c after delete_range [k-b, k-d); live={live:?}"
    );
    assert!(
        live.iter().any(|k| *k == b"k-a"),
        "k-a is outside the range and must stay"
    );
    assert!(
        live.iter().any(|k| *k == b"k-e"),
        "k-e is outside the range and must stay"
    );

    // Dest already holding the covered key must drop it on apply (tail / resume).
    let (_c, mut fold) = PedraFold::open(&dir.join("fold")).unwrap();
    fold.apply(
        &[
            FoldUpdate::Put {
                key: b"k-a".to_vec(),
                value: b"va".to_vec(),
                seq: 1,
            },
            FoldUpdate::Put {
                key: b"k-c".to_vec(),
                value: b"vc".to_vec(),
                seq: 2,
            },
            FoldUpdate::Put {
                key: b"k-e".to_vec(),
                value: b"ve".to_vec(),
                seq: 3,
            },
        ],
        FoldCursor(3),
    )
    .unwrap();
    fold.apply(
        &[FoldUpdate::DeleteRange {
            start: b"k-b".to_vec(),
            end: b"k-d".to_vec(),
            seq: 4,
        }],
        FoldCursor(4),
    )
    .unwrap();
    assert_eq!(fold.get(b"k-a").unwrap().as_deref(), Some(b"va".as_ref()));
    assert_eq!(
        fold.get(b"k-c").unwrap(),
        None,
        "dest apply of DeleteRange must drop covered k-c"
    );
    assert_eq!(fold.get(b"k-e").unwrap().as_deref(), Some(b"ve".as_ref()));
    let _ = std::fs::remove_dir_all(&dir);
}
