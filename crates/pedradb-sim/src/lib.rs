//! Deterministic fault injection for PedraDB recovery tests (P2.1+).
//!
//! # Crash classes (path-level)
//! 1. **Process kill after durable commit** — `mem::forget` after sync write;
//!    reopen must recover committed keys.
//! 2. **Lost unsynced / truncated tail** — after a known durable prefix, truncate
//!    `CURRENT.log` so the last complete record(s) vanish; recovery must keep
//!    the prefix and not invent keys from the lost tail.
//!
//! # Disk fault classes (Env seam — RBS `FailingMedia` pattern)
//! [`FailingEnv`] implements [`pedradb_core::Env`] and injects `io::Error` on the
//! Nth op (`fail_after` / `arm` / `arm_with_kind` / `from_seed`). Open the DB with
//! [`Db::open_with_env`](pedradb_core::Db::open_with_env).
//!
//! # Recording / lying / short-write (RFC-0011 P2)
//! [`RecordingEnv`] buffers writes until honest sync; [`SyncPolicy::Lying`] lies on
//! fsync; [`RecordingEnv::arm_short_write`] injects partial writes.
//!
//! # Thread-safe faults
//! [`FailingEnvArc`] is `Send + Sync` for multi-thread stress.
//!
//! This is not a full FoundationDB-scale clock/disk simulator; it is a small,
//! reproducible injection surface over the real [`pedradb_core::Db`] recovery path.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod failing;
mod failing_arc;
mod recording;

pub use failing::{FailingEnv, FaultKind};
pub use failing_arc::FailingEnvArc;
pub use recording::{RecordingEnv, SyncPolicy};

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use pedradb_core::{BatchOp, Db, OpenOptions as DbOpen, Result, WAL_FILE_NAME};

/// Working directory for one fault experiment.
#[derive(Debug)]
pub struct FaultEnv {
    dir: PathBuf,
}

impl FaultEnv {
    /// Create a fresh empty data directory under `parent`.
    ///
    /// # Errors
    /// I/O creating the directory.
    pub fn new(parent: impl AsRef<Path>) -> Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let i = N.fetch_add(1, Ordering::Relaxed);
        let dir = parent
            .as_ref()
            .join(format!("pedradb-sim-{n}-{i}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    /// Data directory path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// Open a DB with the given sync policy.
    ///
    /// # Errors
    /// Propagates [`Db::open_with`].
    pub fn open(&self, sync: bool) -> Result<Db> {
        Db::open_with(
            &self.dir,
            DbOpen {
                sync,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                exclusive: true,
            },
        )
    }

    /// Path of the active WAL file.
    #[must_use]
    pub fn wal_path(&self) -> PathBuf {
        self.dir.join(WAL_FILE_NAME)
    }

    /// Current WAL file length in bytes (0 if missing).
    ///
    /// # Errors
    /// Metadata I/O.
    pub fn wal_len(&self) -> Result<u64> {
        let p = self.wal_path();
        if !p.exists() {
            return Ok(0);
        }
        Ok(fs::metadata(p)?.len())
    }

    /// Truncate the WAL to `len` bytes (simulates power-loss dropping a tail).
    ///
    /// # Errors
    /// I/O on open/set_len.
    pub fn truncate_wal_to(&self, len: u64) -> Result<()> {
        let p = self.wal_path();
        let f = OpenOptions::new().write(true).open(p)?;
        f.set_len(len)?;
        f.sync_all()?;
        Ok(())
    }

    /// Remove the environment directory.
    pub fn cleanup(self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Run: durable put → process-style crash → reopen; key must survive.
///
/// # Errors
/// DB I/O.
pub fn scenario_crash_after_sync_survives(parent: impl AsRef<Path>) -> Result<()> {
    let env = FaultEnv::new(parent)?;
    {
        let mut db = env.open(true)?;
        db.put(b"durable", b"yes")?;
        // Process kill after Ok (no close).
        std::mem::forget(db);
    }
    let db = env.open(true)?;
    if db.get(b"durable").as_deref() != Some(b"yes".as_ref()) {
        return Err(pedradb_core::CoreError::Internal(
            "durable key missing after crash-after-sync".into(),
        ));
    }
    env.cleanup();
    Ok(())
}

/// Run: durable prefix, then extra write, truncate WAL to prefix → reopen
/// loses only the truncated tail.
///
/// # Errors
/// DB I/O or truncation failure.
pub fn scenario_truncated_tail_loses_unsynced_suffix(
    parent: impl AsRef<Path>,
) -> Result<()> {
    let env = FaultEnv::new(parent)?;
    // Durable prefix.
    {
        let mut db = env.open(true)?;
        db.apply_batch([
            BatchOp::put(b"keep", b"1"),
            BatchOp::put(b"keep2", b"2"),
        ])?;
        db.close()?;
    }
    let prefix_len = env.wal_len()?;

    // Additional durable-looking writes, then surgically drop them from the file.
    {
        let mut db = env.open(true)?;
        db.put(b"lost", b"x")?;
        db.close()?;
    }
    // Simulate crash that loses the second epoch of WAL bytes.
    env.truncate_wal_to(prefix_len)?;

    let db = env.open(true)?;
    if db.get(b"keep").as_deref() != Some(b"1".as_ref()) {
        return Err(pedradb_core::CoreError::Internal(
            "prefix key keep missing after truncate".into(),
        ));
    }
    if db.get(b"keep2").as_deref() != Some(b"2".as_ref()) {
        return Err(pedradb_core::CoreError::Internal(
            "prefix key keep2 missing after truncate".into(),
        ));
    }
    if db.get(b"lost").is_some() {
        return Err(pedradb_core::CoreError::Internal(
            "truncated tail key still visible".into(),
        ));
    }
    env.cleanup();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parent() -> PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn crash_after_sync_recovers_committed() {
        scenario_crash_after_sync_survives(parent()).unwrap();
    }

    #[test]
    fn truncate_wal_drops_tail_keeps_prefix() {
        scenario_truncated_tail_loses_unsynced_suffix(parent()).unwrap();
    }

    #[test]
    fn failing_env_fail_after_trips_and_open_errors() {
        use pedradb_core::{Db, OpenOptions};

        let dir = parent().join(format!(
            "pedradb-fail-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);

        // fail_after(0): first fallible op (create_dir_all or first file create) fails.
        let env = FailingEnv::fail_after(0);
        let r = Db::open_with_env(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                exclusive: true,
            },
            env,
        );
        assert!(r.is_err(), "must inject");
        let err = r.err().unwrap();
        assert!(
            matches!(err, pedradb_core::CoreError::Io(_)),
            "got {err:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn failing_env_nth_put_then_reopen_recovers_prefix() {
        use pedradb_core::{Db, OpenOptions};

        let dir = parent().join(format!(
            "pedradb-fail-n-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);

        // Generous budget so open + first put succeed; then arm one failure.
        let env = FailingEnv::passing();
        let mut db = Db::open_with_env(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                exclusive: true,
            },
            env.clone(),
        )
        .unwrap();
        db.put(b"keep", b"1").unwrap();
        assert!(!env.tripped());

        // Next write path fails (append or sync).
        env.arm_one_failure();
        let r = db.put(b"lost", b"x");
        assert!(r.is_err(), "expected injected fault, got {r:?}");
        assert!(env.tripped());
        drop(db);

        // Heal and reopen: only the acked key must survive.
        env.disarm();
        let db = Db::open_with_env(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                exclusive: true,
            },
            env,
        )
        .unwrap();
        assert_eq!(db.get(b"keep").as_deref(), Some(b"1".as_ref()));
        // "lost" may or may not be present depending on whether fault hit
        // before or after WAL append+sync; contract: keep must survive.
        let _ = db.close();
        let _ = fs::remove_dir_all(&dir);
    }

    /// F1 regression: fault during flush must not leave a final `*.sst` that
    /// blocks open while acked WAL data is still present.
    #[test]
    fn failed_flush_does_not_block_reopen_with_acked_wal() {
        use pedradb_core::{Db, Env, OpenOptions, StdEnv};

        let dir = parent().join(format!(
            "pedradb-f1-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);

        let env = FailingEnv::passing();
        let mut db = Db::open_with_env(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                exclusive: true,
            },
            env.clone(),
        )
        .unwrap();
        db.put(b"a", b"1").unwrap();
        db.put(b"b", b"2").unwrap();
        // Arm permanent dead disk for the flush write path.
        env.arm(0, false);
        let flush_err = db.flush();
        assert!(flush_err.is_err(), "expected flush to fail under fault");
        drop(db);

        // No final SST should remain (tmp cleaned on error). Orphan final = F1.
        let std = StdEnv;
        let names = std.read_dir_names(&dir).unwrap_or_default();
        for name in &names {
            assert!(
                !name.ends_with(".sst") || name.ends_with(".sst.tmp"),
                "partial final SST must not remain: {name}"
            );
        }

        env.disarm();
        let db = Db::open_with_env(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                exclusive: true,
            },
            env,
        )
        .expect("open must succeed and recover WAL after failed flush");
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        let _ = db.close();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn from_seed_is_deterministic() {
        let a = FailingEnv::seed_to_fail_after(42);
        let b = FailingEnv::seed_to_fail_after(42);
        assert_eq!(a, b);
        assert!((1..=32).contains(&a));
        let c = FailingEnv::seed_to_fail_after(43);
        // different seeds usually differ (not required for all pairs)
        let _ = c;
    }

    #[test]
    fn seed_fail_after_open_may_fail_or_succeed_consistently() {
        use pedradb_core::{Db, OpenOptions};
        let seed = 7u64;
        let n = FailingEnv::seed_to_fail_after(seed);
        let dir = parent().join(format!(
            "pedradb-seed-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        let env = FailingEnv::from_seed(seed);
        let r = Db::open_with_env(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                exclusive: true,
            },
            env,
        );
        // n >= 1 so open often succeeds; if it fails, that is also deterministic.
        let _ = (n, r);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn failing_env_storage_full_kind() {
        use pedradb_core::{Db, OpenOptions};

        let dir = parent().join(format!(
            "pedradb-enospc-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        let env = FailingEnv::fail_after_kind(0, FaultKind::StorageFull);
        let r = Db::open_with_env(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                exclusive: true,
            },
            env,
        );
        let err = r.err().expect("must inject StorageFull");
        match err {
            pedradb_core::CoreError::Io(io) => {
                assert_eq!(io.kind(), std::io::ErrorKind::StorageFull);
            }
            other => panic!("expected Io StorageFull, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn failing_env_compact_then_reopen_keeps_data() {
        use pedradb_core::{Db, OpenOptions};

        let dir = parent().join(format!(
            "pedradb-compact-fault-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);

        let env = FailingEnv::passing();
        let mut db = Db::open_with_env(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                exclusive: true,
            },
            env.clone(),
        )
        .unwrap();
        db.put(b"a", b"1").unwrap();
        db.flush().unwrap();
        db.put(b"b", b"2").unwrap();
        db.flush().unwrap();
        assert!(db.sst_count() >= 2);

        // Fail during compact I/O (write/rename/manifest).
        env.arm(0, false);
        let r = db.compact();
        assert!(r.is_err(), "expected compact fault, got {r:?}");
        drop(db);

        env.disarm();
        let db = Db::open_with_env(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                exclusive: true,
            },
            env,
        )
        .expect("reopen after failed compact");
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        let _ = db.close();
        let _ = fs::remove_dir_all(&dir);
    }

    /// F18: after a durable put, auto-flush I/O failure must not make `put` return Err.
    #[test]
    fn auto_flush_fault_does_not_fail_acked_put() {
        use pedradb_core::{Db, OpenOptions};

        let opts = OpenOptions {
            sync: true,
            auto_flush_bytes: Some(64),
            auto_compact_sst_count: None,
            exclusive: true,
        };
        let big = vec![b'x'; 128];

        // Allow a few Env ops for the big put's WAL append+sync, then fail so
        // auto-flush (create SST / rename / MANIFEST) faults after durability.
        // Scan a small window of budgets so the test is not tied to exact op counts.
        let mut saw_ok_with_fault = false;
        for allow in 1..40u64 {
            let dir = parent().join(format!(
                "pedradb-f18-{allow}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let _ = fs::remove_dir_all(&dir);
            let env = FailingEnv::passing();
            let mut db = Db::open_with_env(&dir, opts, env.clone()).unwrap();
            db.put(b"s", b"1").unwrap();
            env.arm(allow, false);
            let r = db.put(b"big", &big);
            if r.is_ok() && env.tripped() {
                assert_eq!(db.get(b"big").as_deref(), Some(big.as_slice()));
                drop(db);
                env.disarm();
                let db = Db::open_with_env(&dir, opts, FailingEnv::passing()).unwrap();
                assert_eq!(
                    db.get(b"big").as_deref(),
                    Some(big.as_slice()),
                    "allow={allow}: acked put must reopen"
                );
                let _ = db.close();
                let _ = fs::remove_dir_all(&dir);
                saw_ok_with_fault = true;
                break;
            }
            drop(db);
            let _ = fs::remove_dir_all(&dir);
        }
        assert!(
            saw_ok_with_fault,
            "F18: expected some Env budget where put Ok but auto-flush tripped"
        );
    }

    /// RFC-0011 P1.1-lite: for each budget n, put+flush under `fail_after(n)`;
    /// if the op path fails, heal and reopen — previously acked keys must remain.
    #[test]
    fn failing_env_nth_op_sweep_put_flush_no_silent_loss() {
        use pedradb_core::{Db, OpenOptions};

        let opts = OpenOptions {
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            exclusive: true,
        };

        for n in 0..40u64 {
            let dir = parent().join(format!(
                "pedradb-sweep-{n}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let _ = fs::remove_dir_all(&dir);

            // Seed a durable key with a healthy env first.
            {
                let mut db = Db::open_with_env(&dir, opts, FailingEnv::passing()).unwrap();
                db.put(b"seed", b"ok").unwrap();
                db.close().unwrap();
            }

            let env = FailingEnv::fail_after(n);
            let open = Db::open_with_env(&dir, opts, env.clone());
            match open {
                Err(_) => {
                    // Failed during open/recover — seed must still reopen healthy.
                    env.disarm();
                    let db = Db::open_with_env(&dir, opts, FailingEnv::passing()).unwrap();
                    assert_eq!(
                        db.get(b"seed").as_deref(),
                        Some(b"ok".as_ref()),
                        "n={n}: seed lost after open fault"
                    );
                    let _ = db.close();
                }
                Ok(mut db) => {
                    assert_eq!(db.get(b"seed").as_deref(), Some(b"ok".as_ref()));
                    let put_r = db.put(b"extra", b"x");
                    if put_r.is_ok() {
                        let _ = db.flush();
                    }
                    drop(db);
                    env.disarm();
                    let db = Db::open_with_env(&dir, opts, FailingEnv::passing()).unwrap();
                    assert_eq!(
                        db.get(b"seed").as_deref(),
                        Some(b"ok".as_ref()),
                        "n={n}: seed lost after put/flush fault"
                    );
                    // If put was Ok, extra should be durable under sync=true.
                    if put_r.is_ok() {
                        assert_eq!(
                            db.get(b"extra").as_deref(),
                            Some(b"x".as_ref()),
                            "n={n}: acked put missing after reopen"
                        );
                    }
                    let _ = db.close();
                }
            }
            let _ = fs::remove_dir_all(&dir);
        }
    }
}
