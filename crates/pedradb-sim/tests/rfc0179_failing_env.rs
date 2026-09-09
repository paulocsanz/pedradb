//! RFC-0179 P1.1: `FailingEnv` injects `available_bytes` so the live put
//! path matches `disk_pressure_admit` (refuse below hard; get stays Ok;
//! not a durability fence).

use pedradb_core::concurrent::ConcurrentDb;
use pedradb_core::db::{copy_db_directory, Db, OpenOptions};
use pedradb_core::env::Env;
use pedradb_core::disk_pressure_kernel::{
    disk_pressure_admit, DiskPressureAdmit, DISK_HARD_FREE_BYTES,
};
use pedradb_core::error::CoreError;
use pedradb_sim::{FailingEnv, FailingEnvArc};

fn tmp() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let i = N.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("failing-0179-{n}-{i}"));
    let _ = std::fs::create_dir_all(&d);
    d
}

#[test]
fn failing_env_hard_floor_refuses_put_get_ok() {
    let dir = tmp();
    let env = FailingEnv::passing();
    let handle = env.clone();
    let mut db = Db::open_with_env(
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
    db.put(b"k", b"v").unwrap();
    assert_eq!(db.get(b"k").as_deref(), Some(b"v".as_ref()));

    handle.set_available_bytes(Some(1024));
    assert!(
        matches!(
            disk_pressure_admit(Some(1024)),
            DiskPressureAdmit::Refuse { .. }
        ),
        "kernel must refuse 1024 < hard floor"
    );
    let err = db.put(b"k2", b"v2").unwrap_err();
    assert!(
        matches!(
            err,
            CoreError::DiskPressure {
                available: 1024,
                need: DISK_HARD_FREE_BYTES,
            }
        ),
        "expected DiskPressure, got {err:?}"
    );
    assert_eq!(db.get(b"k").as_deref(), Some(b"v".as_ref()));
    assert!(db.get(b"k2").is_none());
    assert!(!db.is_durability_fenced());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failing_env_arc_hard_floor_refuses_put_get_ok() {
    let dir = tmp();
    let env = FailingEnvArc::passing();
    let handle = env.clone();
    let mut db = Db::open_with_env(
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
    db.put(b"k", b"v").unwrap();
    handle.set_available_bytes(Some(1024));
    let err = db.put(b"k2", b"v2").unwrap_err();
    assert!(
        matches!(
            err,
            CoreError::DiskPressure {
                available: 1024,
                need: DISK_HARD_FREE_BYTES,
            }
        ),
        "expected DiskPressure, got {err:?}"
    );
    assert_eq!(db.get(b"k").as_deref(), Some(b"v".as_ref()));
    assert!(db.get(b"k2").is_none());
    assert!(!db.is_durability_fenced());
    let _ = std::fs::remove_dir_all(&dir);
}

/// RFC-0179 P1.2: between hard and soft the write is Reclaim, not Refuse —
/// put still Ok after the named reclaim plan (WAL recycle + vlog GC + SST).
#[test]
fn failing_env_reclaim_band_put_ok() {
    use pedradb_core::disk_pressure_kernel::{
        compact_allowed_under_pressure, disk_pressure_admit, disk_pressure_reclaim_plan,
        DiskPressureAdmit, DISK_HARD_FREE_BYTES,
    };

    assert!(matches!(
        disk_pressure_admit(Some(DISK_HARD_FREE_BYTES)),
        DiskPressureAdmit::Reclaim
    ));
    assert!(compact_allowed_under_pressure(Some(DISK_HARD_FREE_BYTES)));
    let plan = disk_pressure_reclaim_plan(true);
    assert!(plan.rotate_wal && plan.compact_vlog && plan.compact_sst);

    let dir = tmp();
    let env = FailingEnv::passing();
    let handle = env.clone();
    let mut db = Db::open_with_env(
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
    db.put(b"k", b"v").unwrap();
    handle.set_available_bytes(Some(DISK_HARD_FREE_BYTES));
    db.put(b"k2", b"v2").unwrap();
    assert_eq!(db.get(b"k2").as_deref(), Some(b"v2".as_ref()));
    assert!(!db.is_durability_fenced());
    let _ = std::fs::remove_dir_all(&dir);
}

/// RFC-0179: ConcurrentDb put under hard floor is DiskPressure; get Ok;
/// not a durability fence (write-lock client, not dump of parking_lot).
#[test]
fn concurrent_db_hard_floor_refuses_put_get_ok() {
    let dir = tmp();
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
    db.put(b"k", b"v").unwrap();
    handle.set_available_bytes(Some(1024));
    let err = db.put(b"k2", b"v2").unwrap_err();
    assert!(
        matches!(
            err,
            CoreError::DiskPressure {
                available: 1024,
                need: DISK_HARD_FREE_BYTES,
            }
        ),
        "expected DiskPressure, got {err:?}"
    );
    assert_eq!(db.get(b"k").as_deref(), Some(b"v".as_ref()));
    assert!(db.get(b"k2").is_none());
    assert!(!db.is_durability_fenced());
    let _ = std::fs::remove_dir_all(&dir);
}

/// RFC-0179: `copy_db_directory` admits before copy; dest is not created
/// when the probe is below the hard floor.
#[test]
fn copy_db_directory_under_hard_floor_does_not_create_dest() {
    use std::io::Write;

    let src = tmp();
    let dest_parent = tmp();
    let dest = dest_parent.join("copy");
    let env = FailingEnv::passing();
    env.create_dir_all(&src).unwrap();
    {
        let mut f = env.create(&src.join("CURRENT")).unwrap();
        f.write_all(b"x").unwrap();
    }
    env.set_available_bytes(Some(1024));
    assert!(!env.exists(&dest));
    let err = copy_db_directory(&env, &src, &dest).unwrap_err();
    assert!(
        matches!(
            err,
            CoreError::DiskPressure {
                available: 1024,
                need: DISK_HARD_FREE_BYTES,
            }
        ),
        "expected DiskPressure, got {err:?}"
    );
    assert!(!env.exists(&dest), "refused copy must not create dest");
    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dest_parent);
}
