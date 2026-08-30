//! RFC-0151 P0.1/P1: FailingEnv plants that drive shipped kernels.
//!
//! One plant per kernel. No new World swarm seed.

use std::fs;
use std::ops::Bound;
use std::path::PathBuf;

use pedradb_core::batch::{
    write_record_count_ok, write_record_count_ok_as_is, WriteOp, WriteRecord,
};
use pedradb_core::cf_kernel::{key_in_cf_family, key_in_cf_family_as_is};
use pedradb_core::compact_kernel::{
    gc_oldest_from_pin, gc_oldest_from_pin_as_is, point_version_fate, VersionFate,
};
use pedradb_core::flush_kernel::{may_publish_manifest, may_publish_manifest_as_is};
use pedradb_core::key::ValueType;
use pedradb_core::merge::{iter_window_keep, iter_window_keep_as_is, visible_at, visible_at_as_is};
use pedradb_core::{BatchOp, Db, OpenOptions, WriteOptions, CURRENT_FILE};

use super::{FailingEnv, FaultKind, OpClass};

fn parent() -> PathBuf {
    std::env::temp_dir()
}

fn opts() -> OpenOptions {
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
    }
}

fn fresh_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = parent().join(format!("pedra-0151-{tag}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn key_in_cf_family_on_live_scan_is_not_ok() {
    assert!(!key_in_cf_family(b"lock\0k", "default"));
    assert!(
        key_in_cf_family_as_is(b"lock\0k", "default"),
        "AS-IS dente: CF scan leak — lock key treated as default"
    );
    let dir = fresh_dir("cf");
    let env = FailingEnv::passing();
    let mut db = Db::open_with_env(&dir, opts(), env).unwrap();
    db.set_physical_cfs(vec!["default".into(), "lock".into()]);
    db.set_defer_auto_compact(true);
    db.put(b"lock\0k", b"L").unwrap();
    db.put(b"default\0d", b"D").unwrap();
    db.flush().unwrap();
    let meta = db.live_sst_meta();
    let default_ssts: Vec<_> = meta.iter().filter(|m| m.cf == "default").collect();
    let lock_ssts: Vec<_> = meta.iter().filter(|m| m.cf == "lock").collect();
    assert!(
        !default_ssts.is_empty(),
        "flush must emit a default SST, meta={meta:?}"
    );
    assert!(
        !lock_ssts.is_empty(),
        "flush must emit a lock SST, meta={meta:?}"
    );
    for s in &default_ssts {
        assert!(
            key_in_cf_family(&s.start_key, "default")
                && key_in_cf_family(&s.end_key, "default"),
            "default SST bounds must not be the lock family: {s:?}"
        );
        assert!(
            !s.start_key.starts_with(b"lock\0") && !s.end_key.starts_with(b"lock\0"),
            "AS-IS leak would flush lock keys into the default SST"
        );
    }
    assert_eq!(db.get(b"lock\0k").as_deref(), Some(b"L".as_ref()));
    assert_eq!(db.get(b"default\0d").as_deref(), Some(b"D".as_ref()));
    db.close().unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn visible_at_on_live_range_del_is_not_ok() {
    let dir = fresh_dir("vis");
    let env = FailingEnv::passing();
    let mut db = Db::open_with_env(&dir, opts(), env).unwrap();
    db.put(b"a", b"1").unwrap();
    db.put(b"b", b"2").unwrap();
    db.put(b"c", b"3").unwrap();
    db.delete_range(b"a", b"c").unwrap();
    let live: Vec<u8> = db
        .range_limited(Bound::Unbounded, Bound::Unbounded, None)
        .into_iter()
        .map(|(k, _)| k[0])
        .collect();
    assert_eq!(live, vec![b'c'], "range-del must hide a,b");
    assert!(!visible_at(ValueType::Value, true));
    assert!(
        visible_at_as_is(ValueType::Value, true),
        "AS-IS dente: hidden value scans live"
    );
    assert!(!iter_window_keep(visible_at(ValueType::Value, true)));
    assert!(iter_window_keep_as_is(visible_at(ValueType::Value, true)));
    db.close().unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn write_record_count_ok_on_live_torn_batch_is_not_ok() {
    let dir = fresh_dir("wr");
    let env = FailingEnv::passing();
    let mut db = Db::open_with_env(&dir, opts(), env).unwrap();
    db.apply_batch([
        BatchOp::put(b"a", b"1"),
        BatchOp::put(b"b", b"2"),
        BatchOp::put(b"c", b"3"),
    ])
    .unwrap();
    let rec = WriteRecord {
        ops: vec![
            WriteOp::put(1, b"a".as_slice(), b"1".as_slice()),
            WriteOp::put(2, b"b".as_slice(), b"2".as_slice()),
            WriteOp::put(3, b"c".as_slice(), b"3".as_slice()),
        ],
    };
    let encoded = rec.encode();
    assert!(write_record_count_ok(
        3,
        WriteRecord::decode(&encoded).unwrap().ops.len()
    ));
    let mut truncated = encoded.clone();
    truncated.truncate(encoded.len().saturating_sub(4));
    assert!(
        WriteRecord::decode(&truncated).is_err(),
        "torn batch must not apply a prefix"
    );
    assert!(!write_record_count_ok(3, 2));
    assert!(
        write_record_count_ok_as_is(3, 2),
        "AS-IS dente: silent prefix"
    );
    db.close().unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn gc_oldest_from_pin_on_live_reclaim_is_not_ok() {
    let dir = fresh_dir("pin");
    let env = FailingEnv::passing();
    let mut db = Db::open_with_env(&dir, opts(), env).unwrap();
    db.put(b"k", b"old").unwrap();
    db.flush().unwrap();
    let pin = db.pin_snapshot();
    let snap = pin.snapshot();
    db.put(b"k", b"new").unwrap();
    db.flush().unwrap();
    db.compact_reclaim().unwrap();
    assert_eq!(
        db.get_at(snap, b"k").unwrap().as_deref(),
        Some(b"old".as_ref()),
        "pin must keep the old version"
    );
    let oldest = gc_oldest_from_pin(Some(pin.sequence()), 10, 9);
    assert_eq!(oldest, pin.sequence());
    assert_eq!(point_version_fate(1, Some(8), oldest), VersionFate::Keep);
    assert_eq!(
        point_version_fate(
            1,
            Some(8),
            gc_oldest_from_pin_as_is(Some(pin.sequence()), 10, 9)
        ),
        VersionFate::Drop,
        "AS-IS dente: compact over pin"
    );
    db.release_snapshot_pin(pin);
    db.close().unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn may_publish_manifest_on_live_unsynced_sst_is_not_ok() {
    let dir = fresh_dir("flush");
    let env = FailingEnv::passing();
    // L0 write skips file fdatasync; OpenOptions.sync=false skips dir-sync
    // after rename so the first Sync in flush is persist_manifest's SST
    // fsync — the catalog kernel's input.
    let mut open = opts();
    open.sync = false;
    let mut db = Db::open_with_env(&dir, open.clone(), env.clone()).unwrap();
    db.put_with(b"durable", b"yes", WriteOptions::sync())
        .unwrap();
    let current_path = dir.join(CURRENT_FILE);
    let current_before = fs::read(&current_path).ok();
    env.arm_op_class(OpClass::Sync, 0, true, FaultKind::SyncFail);
    let flush = db.flush();
    assert!(
        flush.is_err(),
        "SST fsync fail must refuse MANIFEST publish, got {flush:?}"
    );
    assert!(
        db.unsynced_sst_count() > 0,
        "L0 is installed in memory; SST fsync failed inside persist_manifest"
    );
    assert_eq!(
        fs::read(&current_path).ok(),
        current_before,
        "CURRENT must not name the unsynced SST"
    );
    let sst_durable = false;
    assert!(
        !may_publish_manifest(sst_durable),
        "kernel refuses MANIFEST while SST unsynced"
    );
    assert!(
        may_publish_manifest_as_is(sst_durable),
        "AS-IS dente: CURRENT names a torn/missing SST"
    );
    std::mem::forget(db);
    let env2 = FailingEnv::passing();
    let db = Db::open_with_env(&dir, open, env2).unwrap();
    assert_eq!(
        db.get(b"durable").as_deref(),
        Some(b"yes".as_ref()),
        "crash between SST fsync and MANIFEST recovers from WAL (prior/empty SST, not missing-file get)"
    );
    db.close().unwrap();
    let _ = fs::remove_dir_all(&dir);
}
