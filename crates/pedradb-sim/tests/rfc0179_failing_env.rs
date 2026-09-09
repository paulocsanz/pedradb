//! RFC-0179 P1.1: `FailingEnv` injects `available_bytes` so the live put
//! path matches `disk_pressure_admit` (refuse below hard; get stays Ok;
//! not a durability fence).

use pedradb_core::db::{Db, OpenOptions};
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
