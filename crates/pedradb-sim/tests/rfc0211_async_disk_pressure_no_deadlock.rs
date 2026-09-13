//! RFC-0211 P1.2 wave-hang regression: the first async (1c) put under
//! soft-band disk pressure must not deadlock.
//!
//! `commit_async_one` holds the WAL mutex across `encode_async_one`
//! (RFC-0185 P0.3). The encode's disk-pressure admit used to run the full
//! reclaim plan, whose `try_rotate_wal` re-locks `wal` — a parking_lot
//! self-deadlock on the held lock. It fired on the :p211s gate wave: the
//! container overlay upperdir is a 256 MiB tmpfs, so the DB dir is always
//! below `DISK_SOFT_FREE_BYTES` (reclaim band) and the very first async put
//! on an empty DB (WAL rotate-worthy) parked forever.

use pedradb_core::concurrent::ConcurrentDb;
use pedradb_core::db::OpenOptions;
use pedradb_core::disk_pressure_kernel::{
    disk_pressure_admit, disk_pressure_reclaim_plan_wal_held, DiskPressureAdmit,
    DISK_HARD_FREE_BYTES, DISK_SOFT_FREE_BYTES,
};
use pedradb_sim::FailingEnvArc;
use std::sync::mpsc;
use std::time::Duration;

fn tmp(tag: &str) -> std::path::PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let d = std::env::temp_dir().join(format!("rfc0211-hang-{tag}-{n}"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Mid soft band (hard ≤ free < soft): `Reclaim`, not `Refuse`.
const SOFT_BAND_FREE: u64 = (DISK_HARD_FREE_BYTES + DISK_SOFT_FREE_BYTES) / 2;

#[test]
fn rfc0211_wal_held_reclaim_plan_is_lock_free() {
    let plan = disk_pressure_reclaim_plan_wal_held();
    assert!(!plan.compact_sst && !plan.rotate_wal && !plan.compact_vlog);
    assert!(
        matches!(disk_pressure_admit(Some(SOFT_BAND_FREE)), DiskPressureAdmit::Reclaim),
        "fixture must sit in the reclaim band"
    );
}

/// Real path: `ConcurrentDb::put` with default write sync off (the wave's
/// `PEDRA_PARITY_ASYNC=1` class) on an empty DB whose env reports the soft
/// band. A regression re-introducing a lock-taking reclaim under the held
/// WAL mutex parks forever; the bounded channel turns that into a failure
/// instead of a hung suite.
#[test]
fn rfc0211_first_async_put_under_soft_pressure_no_deadlock() {
    let dir = tmp("async");
    let env = FailingEnvArc::passing();
    let handle = env.clone();
    let db = ConcurrentDb::open_with_env(
        &dir,
        OpenOptions {
            sync: false,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            ..OpenOptions::default()
        },
        env,
    )
    .unwrap();
    db.set_default_write_sync(false);
    handle.set_available_bytes(Some(SOFT_BAND_FREE));

    let (tx, rx) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        let r = db.put(b"rfc0211", b"wave");
        let _ = tx.send(r);
    });
    let put = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("async put deadlocked under soft-band disk pressure (wal lock re-entry)");
    put.expect("soft band must admit the write (only < hard refuses)");
    writer.join().unwrap();

    let dir2 = dir.clone();
    let env2 = FailingEnvArc::passing();
    let handle2 = env2.clone();
    let db2 = ConcurrentDb::open_with_env(
        &dir2,
        OpenOptions {
            sync: false,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            ..OpenOptions::default()
        },
        env2,
    )
    .unwrap();
    handle2.set_available_bytes(Some(SOFT_BAND_FREE));
    assert_eq!(
        db2.get(b"rfc0211").as_deref(),
        Some(b"wave".as_ref()),
        "acked async write must be readable after reopen (WAL replay)"
    );
    let _ = std::fs::remove_dir_all(dir);
}
