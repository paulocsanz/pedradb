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
//! # Non-determinism seams (plug DST later)
//! Re-exports [`pedradb_core::{Clock, ManualClock, Rng, SeedRng, Host, DetHost}`]
//! so harnesses depend on one place. Keep full schedules in out-of-tree
//! `determinismo/`; this crate only supplies media models + trait re-exports.
//!
//! This is not a full FoundationDB-scale clock/disk simulator; it is a small,
//! reproducible injection surface over the real [`pedradb_core::Db`] recovery path.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod failing;
mod failing_arc;
mod recording;

#[cfg(test)]
mod three_teeth_plants;

pub use failing::{FailingEnv, FaultKind, OpClass};
pub use failing_arc::FailingEnvArc;
/// EXPLODE recover injection (byte-level `choose` on the WAL image).
pub use pedradb_core::wal::recover_choose::RecoverChoice;
pub use recording::{RecordingEnv, SyncPolicy};

// DST / non-determinism primitives (implemented in core; sim is the usual import path).
pub use pedradb_core::{
    Clock, DetHost, Host, ManualClock, Rng, SeedRng, StdHost, SystemClock, SystemRng,
};

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
        let dir = parent.as_ref().join(format!("pedradb-sim-{n}-{i}"));
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
                wal_full_fsync: true,
                history: Default::default(),
                sync,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
                wal_recovery: Default::default(),
                sst_payload_budget_bytes: None,
            },
        )
    }

    /// Open a DB with the **verified profile** options (RFC-0058 P0.2):
    /// `OpenOptions::verified()` — sync forced true, strongest WAL data
    /// class, fail-closed recovery. Same media faults apply.
    ///
    /// # Errors
    /// Propagates [`Db::open_with`].
    pub fn open_verified(&self) -> Result<Db> {
        Db::open_with(&self.dir, DbOpen::verified())
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

    /// EXPLODE `choose`: apply a named recover mutation to the on-disk WAL.
    ///
    /// Bytes + fsync of this write are the experiment setup, not the engine
    /// contract. Reopen goes through production [`pedradb_core::wal::Wal::recover_on`].
    ///
    /// # Errors
    /// Missing WAL, I/O, or a choice that does not fit the image.
    pub fn choose_wal_recover(&self, choice: RecoverChoice) -> Result<()> {
        use pedradb_core::wal::recover_choose::apply_recover_choice;
        let p = self.wal_path();
        if !p.exists() {
            return Err(pedradb_core::CoreError::Internal(
                "choose_wal_recover: WAL missing".into(),
            ));
        }
        let mut buf = fs::read(&p)?;
        if !apply_recover_choice(&mut buf, choice) {
            return Err(pedradb_core::CoreError::Internal(
                "choose_wal_recover: choice does not apply".into(),
            ));
        }
        fs::write(&p, buf)?;
        Ok(())
    }

    /// Remove the environment directory.
    pub fn cleanup(self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Run: durable put → process-style crash → reopen; key must survive.
///
/// RFC-0053 crash-dictionary tooth: acked prefix ⊆ map after reopen
/// (`docs/formal/crash-dictionary.md`).
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
pub fn scenario_truncated_tail_loses_unsynced_suffix(parent: impl AsRef<Path>) -> Result<()> {
    let env = FaultEnv::new(parent)?;
    // Durable prefix.
    {
        let mut db = env.open(true)?;
        db.apply_batch([BatchOp::put(b"keep", b"1"), BatchOp::put(b"keep2", b"2")])?;
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
    use pedradb_core::{Db, DetHost, Host, OpenOptions};

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

    /// RFC-0078 P0: production Db on Lying Env; crash must drop the put.
    /// AS-IS `fsync_promotes_pending` would recover the key.
    #[test]
    fn lying_fsync_does_not_promote_pending() {
        assert!(!pedradb_core::group_commit_kernel::fsync_promotes_pending(
            false
        ));
        assert!(
            pedradb_core::group_commit_kernel::fsync_promotes_pending_as_is(false),
            "AS-IS dente: promote on a lying fsync"
        );
        let dir = parent().join(format!(
            "pedradb-lying-0078-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let rec = RecordingEnv::lying();
        {
            let mut db = Db::open_with_env(&dir, opts(), rec.clone()).unwrap();
            db.put(b"k", b"pending").unwrap();
            db.close().unwrap();
        }
        rec.crash();
        let db = Db::open_with_env(&dir, opts(), rec).unwrap();
        assert_eq!(
            db.get(b"k"),
            None,
            "lying fsync must not retain after crash"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0018 inventory trial_ref: SyncFail + RecordingEnv lying.
    #[test]
    fn sync_fail_and_recording_lying() {
        use pedradb_core::env::{Env, EnvFile};
        use std::io::Write;

        let dir = parent().join(format!(
            "pedradb-syncfail-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let env = FailingEnv::passing();
        env.arm_op_class(OpClass::Sync, 0, true, FaultKind::SyncFail);
        {
            let mut f = env.create(&dir.join("x")).unwrap();
            f.write_all(b"data").unwrap();
            assert!(f.sync_all().is_err(), "SyncFail must trip sync");
        }
        assert!(env.tripped());
        // Lying fsync: put under RecordingEnv, crash, key must not recover.
        let rec = RecordingEnv::lying();
        {
            let mut db = Db::open_with_env(&dir.join("rec"), opts(), rec.clone()).unwrap();
            db.put(b"k", b"pending").unwrap();
            db.close().unwrap();
        }
        rec.crash();
        let db = Db::open_with_env(&dir.join("rec"), opts(), rec).unwrap();
        assert_eq!(
            db.get(b"k"),
            None,
            "lying fsync must not retain after crash"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn det_host_open_with_host_put_get() {
        let dir = parent().join(format!(
            "pedradb-dethost-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        let host = DetHost::with_seed(FailingEnv::passing(), 0xC0FFEE);
        host.clock().advance(std::time::Duration::from_millis(
            1 + host.rng().gen_range(9),
        ));
        let mut db = Db::open_with_host(&dir, opts(), &host).unwrap();
        db.put(b"via-host", b"ok").unwrap();
        assert_eq!(db.get(b"via-host").as_deref(), Some(b"ok".as_ref()));
        db.close().unwrap();
        // Reopen still via host seam.
        let db = Db::open_with_host(&dir, opts(), &host).unwrap();
        assert_eq!(db.get(b"via-host").as_deref(), Some(b"ok".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn det_host_failing_env_trips_put() {
        let dir = parent().join(format!(
            "pedradb-dethost-fail-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        // Budget small so a later put fails; seed path still durable under passing reopen.
        {
            let host = DetHost::with_seed(FailingEnv::passing(), 1);
            let mut db = Db::open_with_host(&dir, opts(), &host).unwrap();
            db.put(b"seed", b"1").unwrap();
            db.close().unwrap();
        }
        let host = DetHost::with_seed(FailingEnv::fail_after(3), 1);
        let open = Db::open_with_host(&dir, opts(), &host);
        // May open or fail depending on op count; either way no panic.
        if let Ok(mut db) = open {
            let _ = db.put(b"x", b"y");
            drop(db);
        }
        host.env().disarm();
        let host = DetHost::with_seed(FailingEnv::passing(), 2);
        let db = Db::open_with_host(&dir, opts(), &host).unwrap();
        assert_eq!(db.get(b"seed").as_deref(), Some(b"1".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0016 P0.1: **every** FailingEnv arm budget for `compact_vlog` must leave
    /// either correct in-process large gets (not silent None) or a durability fence
    /// (fail-closed writes). Reopen under a clean env always recovers live keys.
    ///
    /// Does **not** break on the first Err — covers early (pre-MANIFEST) and late
    /// (post-inventory / handle open) fault windows.
    #[test]
    fn fail_mid_vlog_gc_then_continue_large_values_ok() {
        let big_a = vec![0xAAu8; 2048];
        let big_b = vec![0xBBu8; 2048];
        let big_c = vec![0xCCu8; 2048];

        let vlog_opts = OpenOptions {
            wal_full_fsync: true,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: Some(512),
            sst_payload_budget_bytes: None,
        };

        let mut saw_gc_err = false;
        // Cover pre-MANIFEST and late post-commit ops (rewrite + install + open + promote).
        for n in 0..=80u64 {
            let dir = parent().join(format!(
                "pedradb-fail-mid-gc-n{n}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let _ = fs::remove_dir_all(&dir);

            // Seed under healthy env.
            {
                let mut db = Db::open_with_env(&dir, vlog_opts, FailingEnv::passing()).unwrap();
                db.put(b"keep_a", &big_a).unwrap();
                db.put(b"keep_b", &big_b).unwrap();
                db.put(b"drop_me", &big_c).unwrap();
                db.flush().unwrap();
                db.delete(b"drop_me").unwrap();
                db.flush().unwrap();
                db.compact_with(pedradb_core::CompactOptions::latest_only())
                    .unwrap();
                db.close().unwrap();
            }

            let env = FailingEnv::passing();
            {
                let mut db = Db::open_with_env(&dir, vlog_opts, env.clone()).unwrap();
                // Reclaim work so GC is non-trivial.
                db.put(b"junk", &big_c).unwrap();
                db.delete(b"junk").unwrap();
                db.flush().unwrap();

                env.disarm();
                env.arm(n, true);
                let res = db.compact_vlog();
                env.disarm();

                match res {
                    Ok(_stats) => {
                        assert!(
                            !db.is_durability_fenced(),
                            "arm={n}: successful GC must not fence"
                        );
                        assert_eq!(
                            db.get(b"keep_a").as_deref(),
                            Some(big_a.as_slice()),
                            "arm={n}: post-Ok keep_a"
                        );
                        assert_eq!(
                            db.get(b"keep_b").as_deref(),
                            Some(big_b.as_slice()),
                            "arm={n}: post-Ok keep_b"
                        );
                    }
                    Err(_) => {
                        saw_gc_err = true;
                        if db.is_durability_fenced() {
                            // Fail-closed: further puts refuse; do not require get.
                            assert!(
                                db.put(b"after_fence", b"x").is_err(),
                                "arm={n}: fenced Db must reject puts"
                            );
                        } else {
                            // Still serving: large keys must not be silent-None.
                            assert_eq!(
                                db.get(b"keep_a").as_deref(),
                                Some(big_a.as_slice()),
                                "arm={n}: after Err, unfenced keep_a must resolve"
                            );
                            assert_eq!(
                                db.get(b"keep_b").as_deref(),
                                Some(big_b.as_slice()),
                                "arm={n}: after Err, unfenced keep_b must resolve"
                            );
                            db.put(b"keep_c", &big_c).unwrap();
                            db.flush().unwrap();
                            assert_eq!(db.get(b"keep_c").as_deref(), Some(big_c.as_slice()));
                        }
                    }
                }
                let _ = db.close();
            }

            // Clean reopen: live large keys always correct (MANIFEST-consistent).
            let db = Db::open_with_env(&dir, vlog_opts, FailingEnv::passing()).unwrap();
            assert_eq!(
                db.get(b"keep_a").as_deref(),
                Some(big_a.as_slice()),
                "arm={n}: reopen keep_a"
            );
            assert_eq!(
                db.get(b"keep_b").as_deref(),
                Some(big_b.as_slice()),
                "arm={n}: reopen keep_b"
            );
            db.close().unwrap();
            let _ = fs::remove_dir_all(&dir);
        }
        assert!(
            saw_gc_err,
            "expected at least one compact_vlog Err across arm 0..=80"
        );
    }

    /// RFC-0016 P0.4: fixed-seed soak on FailingEnv — silent_wrong=0 on acked prefix.
    #[test]
    fn soak_failing_env_fixed_seed_silent_wrong_zero() {
        use pedradb_core::{rng::Rng, CompactOptions, SeedRng};
        use std::collections::HashMap;

        let dir = parent().join(format!(
            "pedradb-soak-fail-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        let seed = 0x00C0_FFEE_u64;
        let env = FailingEnv::passing(); // start healthy; inject transient faults by schedule
        let host = DetHost::with_seed(env.clone(), seed);
        let rng = SeedRng::new(seed);
        let mut model: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
        let mut silent_wrong = 0u64;

        let vlog_opts = OpenOptions {
            wal_full_fsync: true,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: true,
            auto_flush_bytes: Some(16 * 1024),
            auto_compact_sst_count: Some(6),
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: Some(256),
            sst_payload_budget_bytes: None,
        };

        {
            let mut db = Db::open_with_host(&dir, vlog_opts, &host).unwrap();
            for step in 0..280u64 {
                // Occasional one-shot I/O fault (heals after one fail).
                if step > 0 && step % 40 == 0 {
                    env.arm(0, true);
                }
                let op = rng.next_u64() % 100;
                let k = format!("k{:04}", rng.next_u64() % 32);
                let key = k.as_bytes();
                if op < 50 {
                    let sz = 8 + (rng.next_u64() % 400) as usize;
                    let mut val = vec![0u8; sz];
                    for b in &mut val {
                        *b = (rng.next_u64() & 0xff) as u8;
                    }
                    match db.put(key, &val) {
                        Ok(()) => {
                            model.insert(key.to_vec(), val);
                        }
                        Err(_) => {
                            // Fault/fence: only acked prefix is in model.
                            env.disarm();
                        }
                    }
                } else if op < 70 {
                    match db.delete(key) {
                        Ok(()) => {
                            model.remove(key);
                        }
                        Err(_) => env.disarm(),
                    }
                } else if op < 85 {
                    match db.get(key) {
                        Some(got) => {
                            let expect = model.get(key).map(Vec::as_slice);
                            if Some(got.as_ref()) != expect {
                                silent_wrong += 1;
                            }
                        }
                        None => {
                            if model.contains_key(key) {
                                silent_wrong += 1;
                            }
                        }
                    }
                } else if op < 92 {
                    let _ = db.flush();
                    env.disarm();
                } else if op < 97 {
                    let _ = db.compact();
                    env.disarm();
                } else {
                    let _ = db.compact_vlog();
                    env.disarm();
                }
            }
            env.disarm();
            // Final check of acked model under healed env.
            for i in 0..32u64 {
                let kk = format!("k{i:04}");
                let got = db.get(kk.as_bytes());
                let expect = model.get(kk.as_bytes()).map(Vec::as_slice);
                if got.as_deref() != expect {
                    silent_wrong += 1;
                }
            }
            // Optional latest_only reclaim path is avoided (tombstone footgun).
            let _ = CompactOptions::default();
            let _ = db.close();
        }

        // Reopen on clean Env (same seed host with passing).
        env.disarm();
        let host2 = DetHost::with_seed(FailingEnv::passing(), seed);
        let db = Db::open_with_host(&dir, vlog_opts, &host2).unwrap();
        for i in 0..32u64 {
            let kk = format!("k{i:04}");
            let got = db.get(kk.as_bytes());
            let expect = model.get(kk.as_bytes()).map(Vec::as_slice);
            if got.as_deref() != expect {
                silent_wrong += 1;
            }
        }
        db.close().unwrap();
        assert_eq!(silent_wrong, 0, "FailingEnv soak silent_wrong must be 0");
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0019: CHANGELOG persist fail after durable WAL must not fail the client,
    /// roll sequences, skip mem apply, or invent feed-ahead-of-get.
    #[test]
    fn rfc19_changelog_store_fail_after_wal_still_ok() {
        use pedradb_core::{ChangeKind, ConcurrentDb};

        let dir = parent().join(format!(
            "pedradb-rfc19-chlog-fail-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        let env = FailingEnv::passing();
        let opts = OpenOptions {
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
        };

        // --- Path A: commit_ops_with (Db::put) ---
        {
            let mut db = Db::open_with_env(&dir, opts, env.clone()).unwrap();
            let s1 = db.put_with_seq(b"a", b"1").unwrap();
            assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));

            // Next CHANGELOG rewrite fails at rename; WAL put still durable.
            env.arm_op_class(OpClass::Rename, 0, true, FaultKind::IoError);
            let s2 = db
                .put_with_seq(b"b", b"2")
                .expect("CHANGELOG store must not gate durable put");
            env.disarm();

            assert!(
                s2 > s1,
                "sequence must advance (no reuse after durable WAL)"
            );
            assert_eq!(
                db.get(b"b").as_deref(),
                Some(b"2".as_ref()),
                "mem must apply once WAL is durable"
            );
            let feed = db.changes_after(s1);
            assert_eq!(feed.len(), 1);
            assert_eq!(feed[0].sequence, s2);
            assert_eq!(feed[0].kind, ChangeKind::Put);
            assert_eq!(feed[0].key.as_ref(), b"b");
            assert!(
                feed.iter().all(|e| e.sequence <= db.last_sequence()),
                "feed must not show seq beyond durable last"
            );

            // Next put advances again; still consistent.
            let s3 = db.put_with_seq(b"c", b"3").unwrap();
            assert!(s3 > s2);
            assert_eq!(db.get(b"c").as_deref(), Some(b"3".as_ref()));
            db.close().unwrap();
        }

        // Reopen: rebuild from WAL even if CHANGELOG was incomplete.
        {
            let db = Db::open_with_env(&dir, opts, FailingEnv::passing()).unwrap();
            assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
            assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
            assert_eq!(db.get(b"c").as_deref(), Some(b"3".as_ref()));
            let all = db.changes_after(0);
            assert!(all.iter().any(|e| e.key.as_ref() == b"b"));
            assert!(all.iter().all(|e| e.sequence <= db.last_sequence()));
            db.close().unwrap();
        }

        // --- Path B: group_commit (ConcurrentDb) ---
        let dir2 = parent().join(format!(
            "pedradb-rfc19-chlog-group-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir2);
        let env2 = FailingEnv::passing();
        {
            let inner = Db::open_with_env(&dir2, opts, env2.clone()).unwrap();
            let db = ConcurrentDb::from_db(inner);
            db.put(b"x", b"1").unwrap();
            env2.arm_op_class(OpClass::Rename, 0, true, FaultKind::IoError);
            db.put(b"y", b"2")
                .expect("group_commit must not fail client after durable WAL");
            env2.disarm();
            assert_eq!(db.get(b"y").as_deref(), Some(b"2".as_ref()));
            let after = db.changes_after(0);
            assert!(after.iter().any(|e| e.key.as_ref() == b"y"));
            assert!(after.iter().all(|e| e.sequence <= db.last_sequence()));
            // Sequence advances on next write.
            let before = db.last_sequence();
            db.put(b"z", b"3").unwrap();
            assert!(db.last_sequence() > before);
        }

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dir2);
    }

    /// RFC-0019 P1.3: apply_batch + CAS under FailingEnv — silent_wrong=0 on acked prefix.
    #[test]
    fn rfc19_apply_cas_failing_env_silent_wrong_zero() {
        use pedradb_core::{rng::Rng, BatchOp, SeedRng};
        use std::collections::HashMap;

        let dir = parent().join(format!(
            "pedradb-rfc19-apply-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        let seed = 0x0019_00A1_u64;
        let env = FailingEnv::passing();
        let host = DetHost::with_seed(env.clone(), seed);
        let rng = SeedRng::new(seed);
        let mut model: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
        let mut silent_wrong = 0u64;
        let opts = OpenOptions {
            wal_full_fsync: true,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: true,
            auto_flush_bytes: Some(8 * 1024),
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
            sst_payload_budget_bytes: None,
        };

        {
            let mut db = Db::open_with_host(&dir, opts, &host).unwrap();
            for step in 0..200u64 {
                if step > 0 && step % 35 == 0 {
                    env.arm(0, true);
                }
                let k = format!("k{:03}", rng.next_u64() % 24);
                let key = k.as_bytes().to_vec();
                let op = rng.next_u64() % 100;
                if op < 45 {
                    let val = format!("v{step}").into_bytes();
                    let batch = [
                        BatchOp::put(key.clone(), val.clone()),
                        BatchOp::put(format!("idx-{}", step % 8), key.clone()),
                    ];
                    match db.apply_batch(batch) {
                        Ok(_) => {
                            model.insert(key, val);
                        }
                        Err(_) => env.disarm(),
                    }
                } else if op < 60 {
                    match db.put_if_absent(&key, b"cas-first") {
                        Ok(_) => {
                            model.entry(key).or_insert_with(|| b"cas-first".to_vec());
                        }
                        Err(pedradb_core::CoreError::CasMismatch) => {}
                        Err(_) => env.disarm(),
                    }
                } else if op < 75 {
                    match db.delete(&key) {
                        Ok(()) => {
                            model.remove(&key);
                        }
                        Err(_) => env.disarm(),
                    }
                } else {
                    let got = db.get(&key);
                    let expect = model.get(&key).map(Vec::as_slice);
                    if got.as_deref() != expect {
                        silent_wrong += 1;
                    }
                }
            }
            env.disarm();
            for (k, v) in &model {
                if db.get(k).as_deref() != Some(v.as_slice()) {
                    silent_wrong += 1;
                }
            }
            let syncs = db.wal_sync_count();
            eprintln!(
                "rfc19_apply_cas_failing_env_silent_wrong_zero wal_sync_count={syncs} silent_wrong={silent_wrong}"
            );
            let _ = db.close();
        }
        assert_eq!(silent_wrong, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn crash_after_sync_recovers_committed() {
        use pedradb_core::wal::reopen_kernel::{reopen_outcome, ReopenDamage, ReopenOutcome};
        scenario_crash_after_sync_survives(parent()).unwrap();
        assert_eq!(
            reopen_outcome(ReopenDamage::None, false, false),
            ReopenOutcome::ServeAll
        );
    }

    #[test]
    fn truncate_wal_drops_tail_keeps_prefix() {
        scenario_truncated_tail_loses_unsynced_suffix(parent()).unwrap();
    }

    #[test]
    fn recover_collect_act_on_live_exploded_crc_is_not_ok() {
        use pedradb_core::wal::recover_kernel::{
            recover_collect_act, recover_collect_act_as_is, RecoverAct, RecoverKind,
        };
        explode_choose_crc_fail_stops_reopen();
        assert_eq!(
            recover_collect_act(RecoverKind::Crc, 1, true, 0, false),
            RecoverAct::FailStop
        );
        assert_eq!(
            recover_collect_act_as_is(RecoverKind::Crc, 1, true, 0),
            RecoverAct::Resync
        );
    }

    #[test]
    fn explode_choose_crc_fail_stops_reopen() {
        let env = FaultEnv::new(parent()).unwrap();
        {
            let mut db = env.open(true).unwrap();
            db.put(b"a", b"1").unwrap();
            db.put(b"b", b"2").unwrap();
            db.put(b"c", b"3").unwrap();
            db.close().unwrap();
        }
        env.choose_wal_recover(RecoverChoice::FlipCrc { index: 1 })
            .unwrap();
        match env.open(true) {
            Ok(_) => panic!("CRC choose must fail-stop open"),
            Err(err) => {
                assert!(
                    matches!(err, pedradb_core::CoreError::Crc { .. })
                        || err.to_string().contains("crc"),
                    "CRC choose must fail-stop open, got {err}"
                );
            }
        }
        env.cleanup();
    }

    #[test]
    fn explode_choose_length_not_silent_wrong() {
        let env = FaultEnv::new(parent()).unwrap();
        {
            let mut db = env.open(true).unwrap();
            db.put(b"a", b"1").unwrap();
            db.put(b"b", b"2").unwrap();
            db.put(b"c", b"3").unwrap();
            db.close().unwrap();
        }
        env.choose_wal_recover(RecoverChoice::FlipLength { index: 1 })
            .unwrap();
        // Tiny Cursor WALs resync and keep the suffix (recover_choose sweep).
        // A real Db WAL may hit a false alignment whose CRC fails — fail-stop.
        // Either is fine. Silent empty / wrong `a` is not.
        match env.open(true) {
            Ok(db) => {
                assert_eq!(
                    db.get(b"a").as_deref(),
                    Some(b"1".as_ref()),
                    "durable prefix must survive length choose"
                );
                if let Some(v) = db.get(b"c") {
                    assert_eq!(v.as_ref(), b"3", "suffix must not be silent-wrong");
                }
                let _ = db.close();
            }
            Err(err) => {
                let msg = err.to_string();
                assert!(
                    matches!(err, pedradb_core::CoreError::Crc { .. })
                        || matches!(err, pedradb_core::CoreError::Truncated(_))
                        || msg.contains("length")
                        || msg.contains("crc")
                        // F171: a resync that skipped damaged mid-log bytes
                        // fail-stops the open with this typed internal error
                        // (journaled for escalation) — fail-stop is in-contract.
                        || msg.contains("WAL resync skipped damaged region"),
                    "length choose must fail-stop or resync, got {err}"
                );
            }
        }
        env.cleanup();
    }

    #[test]
    fn explode_choose_orphan_fail_stops_reopen() {
        let env = FaultEnv::new(parent()).unwrap();
        {
            let mut db = env.open(true).unwrap();
            db.put(b"a", b"1").unwrap();
            db.put(b"b", b"2").unwrap();
            db.put(b"c", b"3").unwrap();
            db.close().unwrap();
        }
        env.choose_wal_recover(RecoverChoice::ForgeOrphanMiddle { index: 1 })
            .unwrap();
        match env.open(true) {
            Ok(_) => panic!("orphan choose must fail-stop open"),
            Err(err) => {
                let msg = err.to_string();
                assert!(
                    msg.contains("orphan") || msg.contains("crc"),
                    "orphan choose must fail-stop open, got {msg}"
                );
            }
        }
        env.cleanup();
    }

    /// Multi-key TX Ok under sync + process kill → both keys recovered (all-or-nothing).
    #[test]
    fn multi_key_tx_ok_survives_crash_reopen() {
        let env = FaultEnv::new(parent()).unwrap();
        {
            let mut db = env.open(true).unwrap();
            let mut tx = db.begin();
            tx.put(b"row", b"R").unwrap();
            tx.put(b"idx", b"I").unwrap();
            tx.commit().unwrap();
            std::mem::forget(db);
        }
        let db = env.open(true).unwrap();
        assert_eq!(db.get(b"row").as_deref(), Some(b"R".as_ref()));
        assert_eq!(db.get(b"idx").as_deref(), Some(b"I".as_ref()));
        env.cleanup();
    }

    /// Uncommitted multi-key TX leaves no half-visible state after crash.
    #[test]
    fn multi_key_tx_uncommitted_no_half_after_crash() {
        let env = FaultEnv::new(parent()).unwrap();
        {
            let mut db = env.open(true).unwrap();
            db.put(b"base", b"0").unwrap();
            let mut tx = db.begin();
            tx.put(b"h1", b"1").unwrap();
            tx.put(b"h2", b"2").unwrap();
            std::mem::forget(tx);
            std::mem::forget(db);
        }
        let db = env.open(true).unwrap();
        assert_eq!(db.get(b"base").as_deref(), Some(b"0".as_ref()));
        assert_eq!(db.get(b"h1"), None);
        assert_eq!(db.get(b"h2"), None);
        env.cleanup();
    }

    /// fail_after schedule: acked prefix survives; no wrong recovered values for known acks.
    #[test]
    fn fail_after_schedule_no_silent_wrong_on_acked_prefix() {
        use pedradb_core::{Db, OpenOptions};
        use std::collections::BTreeMap;

        let dir = parent().join(format!(
            "pedradb-fail-sched-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        let env = FailingEnv::passing();
        let mut acked: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        let mut db = Db::open_with_env(
            &dir,
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
            },
            env.clone(),
        )
        .unwrap();
        for i in 0..12u8 {
            let k = vec![b'k', i];
            let v = vec![b'v', i];
            db.put(&k, &v).unwrap();
            acked.insert(k, v);
        }
        // Inject fault for subsequent ops.
        env.arm(2, false);
        for i in 12..20u8 {
            let k = vec![b'k', i];
            let v = vec![b'v', i];
            if db.put(&k, &v).is_ok() {
                acked.insert(k, v);
            } else {
                break;
            }
        }
        drop(db);
        env.disarm();
        let db = Db::open_with_env(
            &dir,
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
            },
            env,
        )
        .unwrap();
        for (k, v) in &acked {
            assert_eq!(
                db.get(k).as_deref(),
                Some(v.as_slice()),
                "silent wrong/missing for acked key {k:?}"
            );
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
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
            },
            env,
        );
        assert!(r.is_err(), "must inject");
        let err = r.err().unwrap();
        assert!(matches!(err, pedradb_core::CoreError::Io(_)), "got {err:?}");
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
            wal_full_fsync: true,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: true,
            auto_flush_bytes: Some(64),
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
            sst_payload_budget_bytes: None,
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

    /// Failed multi-key TX must not burn sequence numbers or leave partial keys.
    #[test]
    fn tx_fail_mid_commit_no_partial_and_seq_not_burned() {
        let dir = parent().join(format!(
            "pedradb-tx-fail-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Open + seed put under generous budget, then arm fail for next multi-op commit.
        let env = FailingEnv::passing();
        let mut db = Db::open_with_env(&dir, opts(), env.clone()).unwrap();
        db.put(b"seed", b"ok").unwrap();
        let seq_before = db.last_sequence();
        assert_eq!(seq_before, 1);

        // Arm: next fallible env ops fail (WAL append for TX).
        env.arm(0, true);
        {
            let mut tx = db.begin();
            tx.put(b"a", b"1").unwrap();
            tx.put(b"b", b"2").unwrap();
            tx.put(b"c", b"3").unwrap();
            let err = tx.commit().expect_err("commit must fail under arm(0)");
            let _ = err;
        }
        // No partial apply.
        assert!(db.get(b"a").is_none());
        assert!(db.get(b"b").is_none());
        assert!(db.get(b"c").is_none());
        assert_eq!(db.get(b"seed").as_deref(), Some(b"ok".as_ref()));
        // Sequence not burned: next successful put should use 2, not 5.
        assert_eq!(
            db.last_sequence(),
            seq_before,
            "failed TX must restore next_seq (got {})",
            db.last_sequence()
        );

        env.disarm();
        {
            let mut tx = db.begin();
            tx.put(b"a", b"1").unwrap();
            tx.put(b"b", b"2").unwrap();
            tx.commit().unwrap();
        }
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        assert_eq!(db.last_sequence(), seq_before + 2);
        db.close().unwrap();

        // Reopen: still no phantom sequences / partial keys.
        let db = Db::open_with_env(&dir, opts(), FailingEnv::passing()).unwrap();
        assert_eq!(db.get(b"seed").as_deref(), Some(b"ok".as_ref()));
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        assert!(db.get(b"c").is_none());
        assert_eq!(db.last_sequence(), seq_before + 2);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// apply_batch under fail_after: sequences restored on Err.
    #[test]
    fn batch_fail_mid_commit_restores_sequence() {
        let dir = parent().join(format!(
            "pedradb-batch-fail-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let env = FailingEnv::passing();
        let mut db = Db::open_with_env(&dir, opts(), env.clone()).unwrap();
        db.put(b"x", b"1").unwrap();
        let seq = db.last_sequence();
        env.arm(0, true);
        let r = db.apply_batch([
            pedradb_core::BatchOp::put(b"y", b"2"),
            pedradb_core::BatchOp::put(b"z", b"3"),
        ]);
        assert!(r.is_err());
        assert!(db.get(b"y").is_none());
        assert_eq!(db.last_sequence(), seq);
        env.disarm();
        // If the injection hit after append+required sync, the handle is fenced
        // and further puts refuse until reopen (RFC-0015 H1). Heal path: reopen.
        if db.is_durability_fenced() {
            db.close().unwrap();
            let mut db = Db::open_with_env(&dir, opts(), env).unwrap();
            db.put(b"y", b"2").unwrap();
            assert!(db.last_sequence() >= seq + 1);
            db.close().unwrap();
        } else {
            db.put(b"y", b"2").unwrap();
            assert_eq!(db.last_sequence(), seq + 1);
            db.close().unwrap();
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0015 P0.1/P0.3: required WAL sync fail → fence; further puts refuse; reopen consistent.
    #[test]
    fn sync_fail_after_append_fences_until_reopen() {
        use pedradb_core::{CoreError, Db, OpenOptions};

        let dir = parent().join(format!(
            "pedradb-fence-{}",
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
            },
            env.clone(),
        )
        .unwrap();
        db.put(b"a", b"1").unwrap();
        assert!(!db.is_durability_fenced());

        // SyncFail: writes land; next sync_data (put path) fails after append.
        env.arm_with_kind(0, false, FaultKind::SyncFail);
        let err = db.put(b"b", b"2").unwrap_err();
        assert!(
            matches!(err, CoreError::Io(_)) || err.to_string().contains("sync"),
            "first failure should be the injected sync err, got {err:?}"
        );
        assert!(db.is_durability_fenced());
        // In-process mem must not show the unacked write.
        assert!(db.get(b"b").is_none());

        let fenced = db.put(b"c", b"3").unwrap_err();
        assert!(
            matches!(fenced, CoreError::DurabilityFenced),
            "expected DurabilityFenced, got {fenced:?}"
        );

        drop(db);
        env.disarm();
        let db = Db::open_with_env(
            &dir,
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
            },
            env,
        )
        .unwrap();
        assert!(!db.is_durability_fenced());
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        // Append succeeded before sync fail → WAL recovery may surface `b`.
        assert_eq!(
            db.get(b"b").as_deref(),
            Some(b"2".as_ref()),
            "failed-sync write still recoverable from WAL after reopen"
        );
        assert!(db.get(b"c").is_none(), "fenced put must not appear");
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// ConcurrentDb group path: apply-before-fd must not make the failed
    /// write visible in-process (published_seq stays behind).
    #[test]
    fn concurrent_sync_fail_does_not_publish() {
        use pedradb_core::{ConcurrentDb, CoreError, OpenOptions};

        let dir = parent().join(format!(
            "pedradb-cenc-fence-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        let env = FailingEnv::passing();
        let opts = OpenOptions {
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
        };
        let db = ConcurrentDb::open_with_env(&dir, opts, env.clone()).unwrap();
        db.put(b"a", b"1").unwrap();
        env.arm_with_kind(0, false, FaultKind::SyncFail);
        let err = db.put(b"b", b"2").unwrap_err();
        assert!(
            matches!(err, CoreError::Io(_))
                || err.to_string().contains("sync")
                || err.to_string().contains("group wal sync"),
            "expected sync err, got {err:?}"
        );
        assert!(
            db.get(b"b").is_none(),
            "unpublished apply must stay invisible after fd fail"
        );
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0015 P0.2/P0.3: sync_dir failure under sync=true fails flush/MANIFEST path.
    #[test]
    fn sync_dir_fail_propagates_on_flush() {
        use pedradb_core::{Db, Env, EnvFile, OpenOptions, Result as CoreResult, StdEnv};
        use std::cell::Cell;
        use std::io::{self, Read, Seek, SeekFrom, Write};
        use std::path::Path;
        use std::rc::Rc;

        /// Env that only bombs `sync_dir` when armed (writes/sync_all still work).
        #[derive(Clone)]
        struct SyncDirBomb {
            inner: StdEnv,
            fail: Rc<Cell<bool>>,
        }

        struct BombFile {
            inner: <StdEnv as Env>::File,
        }

        impl Read for BombFile {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.inner.read(buf)
            }
        }
        impl Write for BombFile {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.inner.write(buf)
            }
            fn flush(&mut self) -> io::Result<()> {
                self.inner.flush()
            }
        }
        impl Seek for BombFile {
            fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
                self.inner.seek(pos)
            }
        }
        impl EnvFile for BombFile {
            fn sync_data(&mut self) -> io::Result<()> {
                self.inner.sync_data()
            }
            fn sync_all(&mut self) -> io::Result<()> {
                self.inner.sync_all()
            }
            fn set_len(&mut self, len: u64) -> io::Result<()> {
                self.inner.set_len(len)
            }
            fn len(&mut self) -> io::Result<u64> {
                self.inner.len()
            }
        }

        impl Env for SyncDirBomb {
            type File = BombFile;
            fn create_dir_all(&self, path: &Path) -> io::Result<()> {
                self.inner.create_dir_all(path)
            }
            fn create(&self, path: &Path) -> io::Result<Self::File> {
                Ok(BombFile {
                    inner: self.inner.create(path)?,
                })
            }
            fn open_append(&self, path: &Path) -> io::Result<Self::File> {
                Ok(BombFile {
                    inner: self.inner.open_append(path)?,
                })
            }
            fn open_read(&self, path: &Path) -> io::Result<Self::File> {
                Ok(BombFile {
                    inner: self.inner.open_read(path)?,
                })
            }
            fn sync_dir(&self, path: &Path) -> io::Result<()> {
                if self.fail.get() {
                    return Err(io::Error::other("injected sync_dir failure"));
                }
                self.inner.sync_dir(path)
            }
            fn read_dir_names(&self, path: &Path) -> io::Result<Vec<String>> {
                self.inner.read_dir_names(path)
            }
            fn remove_file(&self, path: &Path) -> io::Result<()> {
                self.inner.remove_file(path)
            }
            fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
                self.inner.rename(from, to)
            }
            fn exists(&self, path: &Path) -> bool {
                self.inner.exists(path)
            }
            fn metadata_len(&self, path: &Path) -> io::Result<u64> {
                self.inner.metadata_len(path)
            }
        }

        let dir = parent().join(format!(
            "pedradb-syncdir-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);

        let fail = Rc::new(Cell::new(false));
        let env = SyncDirBomb {
            inner: StdEnv,
            fail: Rc::clone(&fail),
        };
        let mut db = Db::open_with_env(
            &dir,
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
            },
            env.clone(),
        )
        .unwrap();
        db.put(b"k", b"v").unwrap();
        fail.set(true);
        let err = db.flush().unwrap_err();
        assert!(
            err.to_string().contains("sync_dir") || matches!(err, pedradb_core::CoreError::Io(_)),
            "expected sync_dir propagation, got {err:?}"
        );
        // Heal: disarm and reopen — acked put must still recover from WAL.
        fail.set(false);
        drop(db);
        let db: CoreResult<Db<_>> = Db::open_with_env(
            &dir,
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
            },
            env,
        );
        let db = db.expect("reopen after failed flush");
        assert_eq!(db.get(b"k").as_deref(), Some(b"v".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0015 P1.1: Db close/drop releases LOCK via Env (remove_file counted).
    #[test]
    fn dir_lock_release_via_env_on_close() {
        use pedradb_core::{Db, Env, EnvFile, OpenOptions, StdEnv, LOCK_FILE};
        use std::cell::Cell;
        use std::io::{self, Read, Seek, SeekFrom, Write};
        use std::path::Path;
        use std::rc::Rc;

        #[derive(Clone)]
        struct CountRemove {
            inner: StdEnv,
            removes: Rc<Cell<u64>>,
        }
        struct F {
            inner: <StdEnv as Env>::File,
        }
        impl Read for F {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.inner.read(buf)
            }
        }
        impl Write for F {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.inner.write(buf)
            }
            fn flush(&mut self) -> io::Result<()> {
                self.inner.flush()
            }
        }
        impl Seek for F {
            fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
                self.inner.seek(pos)
            }
        }
        impl EnvFile for F {
            fn sync_data(&mut self) -> io::Result<()> {
                self.inner.sync_data()
            }
            fn sync_all(&mut self) -> io::Result<()> {
                self.inner.sync_all()
            }
            fn set_len(&mut self, len: u64) -> io::Result<()> {
                self.inner.set_len(len)
            }
            fn len(&mut self) -> io::Result<u64> {
                self.inner.len()
            }
        }
        impl Env for CountRemove {
            type File = F;
            fn create_dir_all(&self, path: &Path) -> io::Result<()> {
                self.inner.create_dir_all(path)
            }
            fn create(&self, path: &Path) -> io::Result<Self::File> {
                Ok(F {
                    inner: self.inner.create(path)?,
                })
            }
            fn open_append(&self, path: &Path) -> io::Result<Self::File> {
                Ok(F {
                    inner: self.inner.open_append(path)?,
                })
            }
            fn open_read(&self, path: &Path) -> io::Result<Self::File> {
                Ok(F {
                    inner: self.inner.open_read(path)?,
                })
            }
            fn sync_dir(&self, path: &Path) -> io::Result<()> {
                self.inner.sync_dir(path)
            }
            fn read_dir_names(&self, path: &Path) -> io::Result<Vec<String>> {
                self.inner.read_dir_names(path)
            }
            fn remove_file(&self, path: &Path) -> io::Result<()> {
                self.removes.set(self.removes.get() + 1);
                self.inner.remove_file(path)
            }
            fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
                self.inner.rename(from, to)
            }
            fn exists(&self, path: &Path) -> bool {
                self.inner.exists(path)
            }
            fn metadata_len(&self, path: &Path) -> io::Result<u64> {
                self.inner.metadata_len(path)
            }
        }

        let dir = parent().join(format!(
            "pedradb-unlock-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        let removes = Rc::new(Cell::new(0));
        let env = CountRemove {
            inner: StdEnv,
            removes: Rc::clone(&removes),
        };
        let db = Db::open_with_env(
            &dir,
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
            },
            env,
        )
        .unwrap();
        assert!(dir.join(LOCK_FILE).exists());
        let before = removes.get();
        db.close().unwrap();
        assert!(
            removes.get() > before,
            "close must unlock via Env::remove_file"
        );
        assert!(!dir.join(LOCK_FILE).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    fn sim_dir(tag: &str) -> PathBuf {
        let dir = parent().join(format!(
            "pedradb-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    /// RFC-0050 P0.3: ENOSPC during SST flush fences Transient; acked prefix survives.
    #[test]
    fn enospc_mid_flush_fences_transient() {
        use pedradb_core::{Db, FenceClass};

        let dir = sim_dir("enospc-flush");
        let env = FailingEnv::passing();
        let mut db = Db::open_with_env(&dir, opts(), env.clone()).unwrap();
        db.put(b"seed", b"ok").unwrap();
        db.put(b"a", b"1").unwrap();
        env.arm_op_class(OpClass::Write, 0, true, FaultKind::StorageFull);
        assert!(db.flush().is_err(), "injected ENOSPC on SST write");
        assert!(db.is_durability_fenced());
        assert_eq!(
            db.fence_report().expect("fence").class,
            FenceClass::Transient
        );
        assert!(db.put(b"after", b"x").is_err(), "fenced writer");
        drop(db);
        env.disarm();
        let db = Db::open_with_env(&dir, opts(), FailingEnv::passing()).unwrap();
        assert_eq!(db.get(b"seed").as_deref(), Some(b"ok".as_ref()));
        let _ = db.close();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0050 P0.3: EIO during compact fails closed; live L0 unchanged.
    #[test]
    fn eio_mid_compact_fail_closed() {
        use pedradb_core::Db;

        let dir = sim_dir("eio-compact");
        let env = FailingEnv::passing();
        let mut db = Db::open_with_env(&dir, opts(), env.clone()).unwrap();
        db.put(b"seed", b"ok").unwrap();
        db.flush().unwrap();
        let ssts = db.sst_count();
        assert!(ssts >= 1, "flush must land an L0");
        env.arm_op_class(OpClass::Write, 0, true, FaultKind::IoError);
        assert!(db.compact().is_err(), "injected EIO on compact SST write");
        assert_eq!(
            db.sst_count(),
            ssts,
            "failed compact must not install a new SST"
        );
        drop(db);
        env.disarm();
        let db = Db::open_with_env(&dir, opts(), FailingEnv::passing()).unwrap();
        assert_eq!(db.get(b"seed").as_deref(), Some(b"ok".as_ref()));
        let _ = db.close();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0050 P0.3: ENOSPC on MANIFEST/CURRENT rename rolls back; CURRENT stays valid.
    #[test]
    fn enospc_mid_manifest_rename() {
        use pedradb_core::{Db, CURRENT_FILE};

        let dir = sim_dir("enospc-manifest");
        let env = FailingEnv::passing();
        let mut db = Db::open_with_env(&dir, opts(), env.clone()).unwrap();
        db.put(b"seed", b"ok").unwrap();
        db.flush().unwrap();
        let current_before = fs::read(dir.join(CURRENT_FILE)).unwrap_or_default();
        let ssts = db.sst_count();
        let mut hit = false;
        for n in 0..6u64 {
            env.arm_op_class(OpClass::Rename, n, true, FaultKind::StorageFull);
            if db.compact().is_err() {
                hit = true;
                assert_eq!(
                    db.sst_count(),
                    ssts,
                    "inventory rolled back on MANIFEST fail"
                );
                break;
            }
            env.disarm();
        }
        assert!(hit, "expected some Rename budget to fail compact");
        let current_after = fs::read(dir.join(CURRENT_FILE)).unwrap_or_default();
        assert!(
            !current_after.is_empty(),
            "CURRENT must remain a valid pointer"
        );
        let _ = current_before;
        drop(db);
        env.disarm();
        let db = Db::open_with_env(&dir, opts(), FailingEnv::passing()).unwrap();
        assert_eq!(db.get(b"seed").as_deref(), Some(b"ok".as_ref()));
        let _ = db.close();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0050 P0.3: range-delete + compact loop terminates; L0 stall bounds files.
    #[test]
    fn range_delete_compact_terminates() {
        use pedradb_core::Db;

        let dir = sim_dir("range-compact");
        let mut db = Db::open_with_env(&dir, opts(), FailingEnv::passing()).unwrap();
        db.set_write_stall_l0(Some(12));
        for i in 0..64u32 {
            let k = format!("k{i:04}");
            match db.put(k.as_bytes(), b"v") {
                Ok(()) => {}
                Err(e) => {
                    assert!(
                        e.to_string().contains("stall") || e.to_string().contains("Stall"),
                        "unexpected put err {e}"
                    );
                }
            }
            if i % 8 == 7 {
                let start = format!("k{:04}", i.saturating_sub(7));
                let end = format!("k{:04}", i + 1);
                let _ = db.delete_range(start.as_bytes(), end.as_bytes());
                let _ = db.flush();
                db.compact()
                    .expect("range-delete compact must return (not hang)");
            }
        }
        assert!(
            db.stats().l0_files <= 12,
            "L0 stall must bound files, got {}",
            db.stats().l0_files
        );
        let _ = db.close();
        let _ = fs::remove_dir_all(&dir);
    }

    // ---- RFC-0058 P0.2: the FailingEnv battery on the verified profile
    // (`OpenOptions::verified()`). Same fault classes as the twins above;
    // oracles must hold identically (silent_wrong = 0, fail-closed).

    fn vrf_dir(tag: &str) -> PathBuf {
        parent().join(format!(
            "pedradb-vrf-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// Verified twin of `scenario_crash_after_sync_survives`: durable put →
    /// process kill → reopen keeps the acked key.
    #[test]
    fn verified_crash_after_sync_survives() {
        let env = FaultEnv::new(parent()).unwrap();
        {
            let mut db = env.open_verified().unwrap();
            db.put(b"durable", b"yes").unwrap();
            std::mem::forget(db);
        }
        let db = env.open_verified().unwrap();
        assert_eq!(db.get(b"durable").as_deref(), Some(b"yes".as_ref()));
        env.cleanup();
    }

    /// Verified twin of `scenario_truncated_tail_loses_unsynced_suffix`:
    /// power-loss tail drop keeps the durable prefix, invents nothing.
    #[test]
    fn verified_truncated_tail_loses_unsynced_suffix() {
        let env = FaultEnv::new(parent()).unwrap();
        {
            let mut db = env.open_verified().unwrap();
            db.apply_batch([BatchOp::put(b"keep", b"1"), BatchOp::put(b"keep2", b"2")])
                .unwrap();
            db.close().unwrap();
        }
        let prefix_len = env.wal_len().unwrap();
        {
            let mut db = env.open_verified().unwrap();
            db.put(b"lost", b"x").unwrap();
            db.close().unwrap();
        }
        env.truncate_wal_to(prefix_len).unwrap();
        let db = env.open_verified().unwrap();
        assert_eq!(db.get(b"keep").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"keep2").as_deref(), Some(b"2".as_ref()));
        assert_eq!(db.get(b"lost"), None);
        env.cleanup();
    }

    /// Verified twin of `failing_env_nth_put_then_reopen_recovers_prefix`:
    /// injected EIO on the write path → Err; reopen keeps the acked prefix.
    #[test]
    fn verified_nth_put_eio_reopen_keeps_prefix() {
        use pedradb_core::OpenOptions;
        let dir = vrf_dir("eio");
        let _ = fs::remove_dir_all(&dir);
        let env = FailingEnv::passing();
        let mut db = Db::open_with_env(&dir, OpenOptions::verified(), env.clone()).unwrap();
        db.put(b"keep", b"1").unwrap();
        env.arm_one_failure();
        assert!(db.put(b"lost", b"x").is_err(), "must inject");
        assert!(env.tripped());
        drop(db);
        env.disarm();
        let db = Db::open_with_env(&dir, OpenOptions::verified(), env).unwrap();
        assert_eq!(db.get(b"keep").as_deref(), Some(b"1".as_ref()));
        let _ = db.close();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Verified twin of `sync_fail_after_append_fences_until_reopen`: WAL
    /// sync EIO → fence (fail-closed), unacked write invisible, further
    /// puts refused, reopen heals with the acked prefix only.
    #[test]
    fn verified_sync_fail_fences_fail_closed() {
        use pedradb_core::{CoreError, Db, OpenOptions};
        let dir = vrf_dir("fence");
        let _ = fs::remove_dir_all(&dir);
        let env = FailingEnv::passing();
        let mut db = Db::open_with_env(&dir, OpenOptions::verified(), env.clone()).unwrap();
        db.put(b"a", b"1").unwrap();
        assert!(!db.is_durability_fenced());
        env.arm_with_kind(0, false, FaultKind::SyncFail);
        let err = db.put(b"b", b"2").unwrap_err();
        assert!(
            matches!(err, CoreError::Io(_)) || err.to_string().contains("sync"),
            "injected sync err expected, got {err:?}"
        );
        assert!(db.is_durability_fenced());
        assert!(db.get(b"b").is_none(), "unacked write must be invisible");
        assert!(matches!(
            db.put(b"c", b"3"),
            Err(CoreError::DurabilityFenced)
        ));
        drop(db);
        env.disarm();
        let db = Db::open_with_env(&dir, OpenOptions::verified(), env).unwrap();
        assert!(!db.is_durability_fenced());
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        // Append succeeded before the sync fail → WAL recovery may surface
        // `b` (same as the non-verified twin); `c` was refused by the
        // fence and must never appear.
        assert_eq!(
            db.get(b"b").as_deref(),
            Some(b"2".as_ref()),
            "failed-sync write still recoverable from WAL after reopen"
        );
        assert!(db.get(b"c").is_none(), "fenced put must not appear");
        let _ = db.close();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Verified twins of the multi-key TX crash pair: committed TX is
    /// all-visible after a kill; uncommitted TX leaves no half state.
    #[test]
    fn verified_multi_key_tx_crash_no_half() {
        let env = FaultEnv::new(parent()).unwrap();
        {
            let mut db = env.open_verified().unwrap();
            let mut tx = db.begin();
            tx.put(b"row", b"R").unwrap();
            tx.put(b"idx", b"I").unwrap();
            tx.commit().unwrap();
            std::mem::forget(db);
        }
        let db = env.open_verified().unwrap();
        assert_eq!(db.get(b"row").as_deref(), Some(b"R".as_ref()));
        assert_eq!(db.get(b"idx").as_deref(), Some(b"I".as_ref()));
        let _ = db.close();
        env.cleanup();

        let env = FaultEnv::new(parent()).unwrap();
        {
            let mut db = env.open_verified().unwrap();
            db.put(b"base", b"0").unwrap();
            let mut tx = db.begin();
            tx.put(b"h1", b"1").unwrap();
            tx.put(b"h2", b"2").unwrap();
            std::mem::forget(tx);
            std::mem::forget(db);
        }
        let db = env.open_verified().unwrap();
        assert_eq!(db.get(b"base").as_deref(), Some(b"0".as_ref()));
        assert_eq!(db.get(b"h1"), None);
        assert_eq!(db.get(b"h2"), None);
        env.cleanup();
    }
}
