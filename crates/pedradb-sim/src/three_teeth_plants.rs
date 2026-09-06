//! RFC-0151 P0.1/P1: FailingEnv plants that drive shipped kernels.
//!
//! One plant per kernel. No new World swarm seed.

use std::fs;
use std::ops::Bound;
use std::path::PathBuf;

use pedradb_core::batch::{
    write_record_count_ok, write_record_count_ok_as_is, WriteOp, WriteRecord,
};
use pedradb_core::cf_kernel::{
    cf_encode_effective, cf_encode_effective_as_is, cf_family_of, cf_family_of_as_is,
    compact_rewrites_sst_cf, compact_rewrites_sst_cf_as_is, decode_cf_key, decode_cf_key_as_is,
    encode_cf_key, encode_cf_key_as_is, infer_sst_cf, infer_sst_cf_as_is, key_in_cf_family,
    key_in_cf_family_as_is,
};
use pedradb_core::compact_kernel::{
    gc_oldest_from_pin, gc_oldest_from_pin_as_is, point_version_fate, VersionFate,
};
use pedradb_core::flush_kernel::{may_publish_manifest, may_publish_manifest_as_is};
use pedradb_core::key::ValueType;
use pedradb_core::merge::{iter_window_keep, iter_window_keep_as_is, visible_at, visible_at_as_is};
use pedradb_core::probe_order_kernel::{
    first_probe_on_equal_lo, first_probe_on_equal_lo_as_is, run_pairwise_disjoint_los,
    run_pairwise_disjoint_los_as_is,
};
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
        sst_payload_budget_bytes: None,
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
            key_in_cf_family(&s.start_key, "default") && key_in_cf_family(&s.end_key, "default"),
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
fn cf_family_of_on_live_sst_bounds_is_not_ok() {
    assert_eq!(cf_family_of(b"lock\0k"), "lock");
    assert_eq!(
        cf_family_of_as_is(b"lock\0k"),
        "default",
        "AS-IS dente: named family lost — every key reports default"
    );
    let dir = fresh_dir("cffam");
    let env = FailingEnv::passing();
    let mut db = Db::open_with_env(&dir, opts(), env).unwrap();
    db.set_physical_cfs(vec!["default".into(), "lock".into()]);
    db.set_defer_auto_compact(true);
    db.put(b"lock\0k", b"L").unwrap();
    db.flush().unwrap();
    let meta = db.live_sst_meta();
    let lock_ssts: Vec<_> = meta.iter().filter(|m| m.cf == "lock").collect();
    assert!(
        !lock_ssts.is_empty(),
        "lock-only flush must emit a lock SST, meta={meta:?}"
    );
    for s in &lock_ssts {
        assert_eq!(cf_family_of(&s.start_key), "lock", "bounds: {s:?}");
        assert_eq!(cf_family_of(&s.end_key), "lock", "bounds: {s:?}");
    }
    db.close().unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cf_encode_effective_on_live_default_raw_is_not_ok() {
    assert_eq!(cf_encode_effective("default", true), "");
    assert_eq!(cf_encode_effective("default", false), "default");
    assert_eq!(cf_encode_effective("lock", true), "lock");
    assert_eq!(
        cf_encode_effective_as_is("default", true),
        "default",
        "AS-IS dente: raw-default layout lost — default keys stored prefixed"
    );
    let dir = fresh_dir("cfeff");
    let env = FailingEnv::passing();
    let mut db = Db::open_with_env(&dir, opts(), env).unwrap();
    db.set_physical_cfs(vec!["default".into(), "lock".into()]);
    db.set_defer_auto_compact(true);
    db.put(b"default\0d", b"D").unwrap();
    db.put(b"lock\0k", b"L").unwrap();
    db.flush().unwrap();
    let meta = db.live_sst_meta();
    let default_bound = encode_cf_key("default", b"d", false);
    let lock_bound = encode_cf_key("lock", b"k", false);
    for s in meta.iter().filter(|m| m.cf == "default") {
        assert_eq!(
            s.start_key, default_bound,
            "engine physical-CF path is non-raw: bound is exactly encode_cf_key(default, d, false): {s:?}"
        );
    }
    for s in meta.iter().filter(|m| m.cf == "lock") {
        assert_eq!(
            s.start_key, lock_bound,
            "named CF carries its prefix (effective prefix non-empty): {s:?}"
        );
    }
    db.close().unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn encode_cf_key_on_live_sst_bounds_is_not_ok() {
    assert_eq!(encode_cf_key("lock", b"k", false), b"lock\0k".to_vec());
    assert_eq!(
        encode_cf_key_as_is("lock", b"k", false),
        b"k".to_vec(),
        "AS-IS dente: prefix dropped — lock k collides with default k"
    );
    let lock_prefix = encode_cf_key("lock", &[], false);
    let dir = fresh_dir("cfenc");
    let env = FailingEnv::passing();
    let mut db = Db::open_with_env(&dir, opts(), env).unwrap();
    db.set_physical_cfs(vec!["default".into(), "lock".into()]);
    db.set_defer_auto_compact(true);
    db.put(b"lock\0a", b"1").unwrap();
    db.put(b"lock\0z", b"2").unwrap();
    db.flush().unwrap();
    let meta = db.live_sst_meta();
    let lock_ssts: Vec<_> = meta.iter().filter(|m| m.cf == "lock").collect();
    assert!(
        !lock_ssts.is_empty(),
        "lock-only flush must emit a lock SST, meta={meta:?}"
    );
    for s in &lock_ssts {
        assert!(
            s.start_key.starts_with(&lock_prefix) && s.end_key.starts_with(&lock_prefix),
            "live lock bounds must be `lock\\0 ++ user`: {s:?}"
        );
    }
    db.close().unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn decode_cf_key_on_live_sst_bounds_is_not_ok() {
    let enc = encode_cf_key("lock", b"k", false);
    assert_eq!(decode_cf_key("lock", &enc, false), b"k".as_slice());
    assert_eq!(
        decode_cf_key_as_is("lock", &enc, false),
        enc.as_slice(),
        "AS-IS dente: cf prefix leaks into the user key"
    );
    let dir = fresh_dir("cfdec");
    let env = FailingEnv::passing();
    let mut db = Db::open_with_env(&dir, opts(), env).unwrap();
    db.set_physical_cfs(vec!["default".into(), "lock".into()]);
    db.set_defer_auto_compact(true);
    db.put(b"lock\0a", b"1").unwrap();
    db.put(b"lock\0z", b"2").unwrap();
    db.flush().unwrap();
    let meta = db.live_sst_meta();
    for s in meta.iter().filter(|m| m.cf == "lock") {
        let start_user = decode_cf_key("lock", &s.start_key, false);
        let end_user = decode_cf_key("lock", &s.end_key, false);
        assert!(
            !start_user.starts_with(b"lock\0") && !end_user.starts_with(b"lock\0"),
            "stripped live bounds must be pure user keys: {s:?} ({start_user:?}, {end_user:?})"
        );
    }
    db.close().unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn infer_sst_cf_on_live_flush_tag_is_not_ok() {
    assert_eq!(
        infer_sst_cf(Some(b"lock\0a".as_slice()), Some(b"lock\0z".as_slice())),
        "lock"
    );
    assert_eq!(
        infer_sst_cf(Some(b"aaa".as_slice()), Some(b"lock\0z".as_slice())),
        "",
        "mixed bounds tag empty"
    );
    assert_eq!(
        infer_sst_cf_as_is(Some(b"aaa".as_slice()), Some(b"lock\0z".as_slice())),
        "default",
        "AS-IS dente: mixed file tagged default — compacted as default"
    );
    let dir = fresh_dir("cfinf");
    let env = FailingEnv::passing();
    let mut db = Db::open_with_env(&dir, opts(), env).unwrap();
    db.set_physical_cfs(vec!["default".into(), "lock".into()]);
    db.set_defer_auto_compact(true);
    db.put(b"default\0d", b"D").unwrap();
    db.put(b"lock\0a", b"1").unwrap();
    db.put(b"lock\0z", b"2").unwrap();
    db.flush().unwrap();
    let meta = db.live_sst_meta();
    assert!(!meta.is_empty(), "flush must emit SSTs");
    for s in &meta {
        assert_eq!(
            infer_sst_cf(Some(&s.start_key), Some(&s.end_key)),
            s.cf,
            "engine-recorded tag must equal infer_sst_cf over live bounds: {s:?}"
        );
    }
    db.close().unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn compact_rewrites_sst_cf_on_live_meta_is_not_ok() {
    assert!(!compact_rewrites_sst_cf("default", "lock"));
    assert!(
        compact_rewrites_sst_cf_as_is("default", "lock"),
        "AS-IS dente: lock compact rewrites the default-tagged SST"
    );
    let dir = fresh_dir("cfcpt");
    let env = FailingEnv::passing();
    let mut db = Db::open_with_env(&dir, opts(), env).unwrap();
    db.set_physical_cfs(vec!["default".into(), "lock".into()]);
    db.set_defer_auto_compact(true);
    db.put(b"default\0d", b"D").unwrap();
    db.put(b"lock\0a", b"1").unwrap();
    db.flush().unwrap();
    let meta = db.live_sst_meta();
    assert!(
        meta.iter().any(|m| m.cf == "default") && meta.iter().any(|m| m.cf == "lock"),
        "flush must emit both families, meta={meta:?}"
    );
    for s in &meta {
        assert_eq!(
            compact_rewrites_sst_cf(&s.cf, "lock"),
            s.cf == "lock",
            "lock compact must select exactly the lock-tagged live files: {s:?}"
        );
    }
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

/// RFC-0164 P0.1: the live equal-`lo` tie — put@flush then delete@flush
/// leaves two single-key L0 tables over the same key, and the historical
/// descending-`lo` walk probes the OLDER (the put) first: `Found` wins and
/// the deleted value is resurrected
/// (findings/2026-09-04-reopen-delete-resurrected).
#[test]
fn probe_order_on_live_equal_lo_is_not_ok() {
    let dir = fresh_dir("probe-order");
    let env = FailingEnv::passing();
    let mut db = Db::open_with_env(&dir, opts(), env).unwrap();
    db.set_defer_auto_compact(true);
    db.put(b"k", b"v").unwrap();
    db.flush().unwrap();
    db.delete(b"k").unwrap();
    db.flush().unwrap();
    let meta = db.live_sst_meta();
    assert_eq!(
        meta.len(),
        2,
        "put-flush + delete-flush must leave two L0 tables, meta={meta:?}"
    );
    for m in &meta {
        assert!(
            &*m.start_key == b"k".as_slice() && &*m.end_key == b"k".as_slice(),
            "single-key tables must tie at lo = hi = k, got {m:?}"
        );
    }
    // Two covering candidates tied at lo: candidate 1 is the newest write
    // (the tombstone), candidate 0 the older put — the measured shape.
    let (newer, older) = (1usize, 0usize);
    assert_eq!(
        first_probe_on_equal_lo(newer, older),
        newer,
        "kernel probes the tombstone table first"
    );
    assert_eq!(
        first_probe_on_equal_lo_as_is(newer, older),
        older,
        "AS-IS dente: descending-lo walk probes the older put first — deleted value resurrected"
    );
    db.close().unwrap();
    let _ = fs::remove_dir_all(&dir);
}

/// RFC-0164 P1.2: the strict-disjoint bisect arm on the LIVE equal-lo
/// shape. Two single-key L0 tables tied at lo = hi = k — the kernel keeps
/// the single-candidate bisect path unarmed (spec `false`); the non-strict
/// AS-IS mutant arms it and, with the stable sort keeping newest-first
/// among ties, `by_lo[p-1]` is the OLDER put — the live get would
/// resurrect `v`. The live engine agrees with the kernel: get == None.
#[test]
fn run_disjoint_on_live_equal_lo_is_not_ok() {
    let dir = fresh_dir("run-disjoint");
    let env = FailingEnv::passing();
    let mut db = Db::open_with_env(&dir, opts(), env).unwrap();
    db.set_defer_auto_compact(true);
    db.put(b"k", b"v").unwrap();
    db.flush().unwrap();
    db.delete(b"k").unwrap();
    db.flush().unwrap();
    let meta = db.live_sst_meta();
    assert_eq!(
        meta.len(),
        2,
        "put-flush + delete-flush must leave two L0 tables, meta={meta:?}"
    );
    for m in &meta {
        assert!(
            &*m.start_key == b"k".as_slice() && &*m.end_key == b"k".as_slice(),
            "single-key tables must tie at lo = hi = k, got {m:?}"
        );
    }
    let los: Vec<&[u8]> = meta.iter().map(|m| m.start_key.as_slice()).collect();
    let his: Vec<&[u8]> = meta.iter().map(|m| m.end_key.as_slice()).collect();
    assert!(
        !run_pairwise_disjoint_los(&los, &his),
        "equal-lo tie must keep the bisect arm off (kernel)"
    );
    assert!(
        run_pairwise_disjoint_los_as_is(&los, &his),
        "AS-IS dente: the non-strict arm takes the bisect path onto the older put"
    );
    assert_eq!(
        db.get(b"k"),
        None,
        "live engine agrees with the kernel: the tombstone wins, no resurrection"
    );
    db.close().unwrap();
    let _ = fs::remove_dir_all(&dir);
}

/// RFC-0164 P1.1: the cold point-cache live get must agree with the
/// reopen ground truth on the measured shape (put→flush→delete→flush ⇒
/// two L0 tables tied at lo = hi = k). The original pass was VACUOUS: the
/// point-cache answered `(k → None)` without touching mem or SST
/// (`mem_hit=0 sst_fb=0` measured — findings/2026-09-04-reopen-delete-
/// resurrected). The first `get` on a fresh handle is cold by
/// construction; a broken probe order would return `Some("v")` live
/// while the reopen read stays `None`.
#[test]
fn cold_cache_live_get_agrees_with_reopen() {
    let dir = fresh_dir("cold-cache-get");
    let env = FailingEnv::passing();
    let mut db = Db::open_with_env(&dir, opts(), env).unwrap();
    db.set_defer_auto_compact(true);
    db.put(b"k", b"v").unwrap();
    db.flush().unwrap();
    db.delete(b"k").unwrap();
    db.flush().unwrap();
    // First get on this handle: the point-cache has no entry for k.
    let live_cold = db.get(b"k");
    assert_eq!(live_cold, None, "cold live get: the tombstone table must win");
    // The cached answer must stay consistent with the cold one.
    assert_eq!(db.get(b"k"), None);
    db.close().unwrap();
    let reopened = Db::open_with_env(&dir, opts(), FailingEnv::passing()).unwrap();
    let after_reopen = reopened.get(b"k");
    assert_eq!(after_reopen, None, "reopen ground truth: the delete wins on disk");
    assert_eq!(
        live_cold, after_reopen,
        "cold live get == reopen get (RFC-0164 P1.1)"
    );
    reopened.close().unwrap();
    let _ = fs::remove_dir_all(&dir);
}
