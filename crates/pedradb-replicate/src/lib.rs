//! WAL-shipped read replicas for PedraDB (RFC-0010 P1.2 / Rung 1.5 / RFC-0015 P1.3).
//!
//! # Model
//!
//! 1. **Primary** accepts writes (`Db::put` / TX / `apply_batch`) with durable WAL.
//! 2. [`WalShipper`] reads new bytes of `CURRENT.log` after a cursor offset.
//! 3. **Replica** directory appends those bytes to its own `CURRENT.log`.
//! 4. Opening the replica with [`Db::open`] recovers the same logical state
//!    (sequences preserved — physical ship, not re-apply).
//!
//! # Limits (honest)
//!
//! - **Flush on the primary rotates/truncates the WAL.** A shipper detects
//!   this via a prefix stamp (F165): shrink past the cursor, rewritten prefix
//!   (rotate-then-regrow), or a vanished file all return
//!   [`ShipError::WalRotated`]; you must re-bootstrap the replica (copy
//!   SSTs/MANIFEST, or rebuild from snapshot).
//! - This is **not** Raft. For ordered multi-node apply of a shared log, see
//!   `pedradb-apply`. WAL ship is for **asynchronous read replicas** of one writer.
//! - The replica must not take local writes while shipping (single-writer primary).
//!
//! Durable ship I/O uses [`pedradb_core::Env`] (path-only helpers default to production `IoUringEnv`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod ship_kernel;

use ship_kernel::{pull_plan, PullPlan, SHIP_STAMP_BYTES};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use pedradb_core::{Db, Env, EnvFile, OpenOptions as DbOpen, Result as CoreResult, WAL_FILE_NAME};
use pedradb_io_uring::IoUringEnv;
use thiserror::Error;

/// Errors from WAL shipping (distinct from engine [`pedradb_core::CoreError`]).
#[derive(Debug, Error)]
pub enum ShipError {
    /// Underlying I/O.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Primary WAL was truncated/replaced (flush/rotate); cursor is invalid.
    #[error(
        "WAL rotated or truncated (file len {file_len} < cursor {cursor}); re-bootstrap replica"
    )]
    WalRotated {
        /// Current primary WAL length.
        file_len: u64,
        /// Shipper cursor that is now past EOF.
        cursor: u64,
    },
    /// PedraDB open/recover on the replica failed.
    #[error("replica open: {0}")]
    ReplicaOpen(#[from] pedradb_core::CoreError),
}

/// Result alias for ship operations.
pub type ShipResult<T> = std::result::Result<T, ShipError>;

/// Default max bytes per [`WalShipper::pull`] (4 MiB).
///
/// Avoids `vec![0; full_delta]` OOM when the primary WAL is multi-GB.
pub const DEFAULT_MAX_PULL_BYTES: u64 = 4 * 1024 * 1024;

/// Tracks a byte cursor into the primary's `CURRENT.log`.
#[derive(Debug, Clone)]
pub struct WalShipper {
    wal_path: PathBuf,
    /// Next byte to read (inclusive).
    offset: u64,
    /// Max bytes returned by a single pull (chunked ship).
    max_pull_bytes: u64,
    /// Prefix stamp of the WAL when the cursor was established (F165 guard).
    ///
    /// `None` until the first non-empty pull (`from_start` on a fresh primary).
    stamp: Option<Vec<u8>>,
}

/// Read the first `n` bytes of `path` (caller guarantees `n <= len`).
///
/// # Errors
/// I/O.
fn read_prefix_on<E: Env>(env: &E, path: &Path, n: u64) -> ShipResult<Vec<u8>> {
    let n = usize::try_from(n).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "prefix does not fit usize",
        )
    })?;
    let mut f = env.open_read(path)?;
    let mut buf = vec![0u8; n];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

impl WalShipper {
    /// Watch the WAL of an open primary (or any DB directory).
    ///
    /// Starts at the **current end** of the file (only new writes after this
    /// call are shipped). Use [`Self::from_start`] to catch up the full log.
    ///
    /// # Errors
    /// Metadata I/O if the WAL exists but cannot be stat'd.
    pub fn follow(primary_dir: impl AsRef<Path>) -> ShipResult<Self> {
        Self::follow_on(&IoUringEnv::default(), primary_dir)
    }

    /// Like [`Self::follow`] with an explicit [`Env`].
    ///
    /// # Errors
    /// Metadata I/O if the WAL exists but cannot be stat'd.
    pub fn follow_on<E: Env>(env: &E, primary_dir: impl AsRef<Path>) -> ShipResult<Self> {
        let wal_path = primary_dir.as_ref().join(WAL_FILE_NAME);
        let mut stamp = None;
        let offset = if env.exists(&wal_path) {
            let len = env.metadata_len(&wal_path)?;
            if len > 0 {
                stamp = Some(read_prefix_on(
                    env,
                    &wal_path,
                    len.min(SHIP_STAMP_BYTES as u64),
                )?);
            }
            len
        } else {
            0
        };
        Ok(Self {
            wal_path,
            offset,
            max_pull_bytes: DEFAULT_MAX_PULL_BYTES,
            stamp,
        })
    }

    /// Ship from byte 0 (full WAL catch-up for a fresh replica).
    ///
    /// The prefix stamp is captured at the first non-empty pull: with the
    /// cursor at 0 there is no gap to miss, so a rotation before the first
    /// pull is not (and need not be) detected.
    #[must_use]
    pub fn from_start(primary_dir: impl AsRef<Path>) -> Self {
        Self {
            wal_path: primary_dir.as_ref().join(WAL_FILE_NAME),
            offset: 0,
            max_pull_bytes: DEFAULT_MAX_PULL_BYTES,
            stamp: None,
        }
    }

    /// Cap each [`Self::pull`] to at most `max` bytes (chunked ship).
    ///
    /// `max == 0` restores [`DEFAULT_MAX_PULL_BYTES`].
    pub fn set_max_pull_bytes(&mut self, max: u64) {
        self.max_pull_bytes = if max == 0 {
            DEFAULT_MAX_PULL_BYTES
        } else {
            max
        };
    }

    /// Current max pull chunk size.
    #[must_use]
    pub fn max_pull_bytes(&self) -> u64 {
        self.max_pull_bytes
    }

    /// Current cursor.
    #[must_use]
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Path of the primary WAL file.
    #[must_use]
    pub fn wal_path(&self) -> &Path {
        &self.wal_path
    }

    /// Read up to [`Self::max_pull_bytes`] new bytes from the primary WAL.
    ///
    /// Advances the cursor by the returned length. Call repeatedly (or use
    /// [`catch_up`]) until `Ok(None)`. Incomplete trailing records may be
    /// included; replica recovery skips a truncated tail (same as crash).
    ///
    /// # Errors
    /// I/O or [`ShipError::WalRotated`] when the primary WAL no longer
    /// continues the shipped stream: shrunk past the cursor, rewritten
    /// prefix (flush rotates `CURRENT.log` in place, F165), or missing file
    /// under an advanced cursor.
    pub fn pull(&mut self) -> ShipResult<Option<Vec<u8>>> {
        self.pull_on(&IoUringEnv::default())
    }

    /// Like [`Self::pull`] with an explicit [`Env`].
    ///
    /// # Errors
    /// I/O or [`ShipError::WalRotated`] (shrink, prefix rewrite, vanished file).
    pub fn pull_on<E: Env>(&mut self, env: &E) -> ShipResult<Option<Vec<u8>>> {
        let (file_len, stamp_now) = if env.exists(&self.wal_path) {
            let len = env.metadata_len(&self.wal_path)?;
            if self.stamp.is_none() && len > 0 {
                self.stamp = Some(read_prefix_on(
                    env,
                    &self.wal_path,
                    len.min(SHIP_STAMP_BYTES as u64),
                )?);
            }
            let stamp_now = match &self.stamp {
                Some(then) => read_prefix_on(env, &self.wal_path, (then.len() as u64).min(len))?,
                None => Vec::new(),
            };
            (Some(len), stamp_now)
        } else {
            (None, Vec::new())
        };
        match pull_plan(
            file_len,
            self.offset,
            self.max_pull_bytes,
            self.stamp.as_deref(),
            &stamp_now,
        ) {
            PullPlan::Rotated { file_len, cursor } => {
                Err(ShipError::WalRotated { file_len, cursor })
            }
            PullPlan::UpToDate => Ok(None),
            PullPlan::Ship { bytes } => {
                let take_usize = usize::try_from(bytes).map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "pull chunk does not fit usize",
                    )
                })?;
                let mut f = env.open_read(&self.wal_path)?;
                f.seek(SeekFrom::Start(self.offset))?;
                let mut buf = vec![0u8; take_usize];
                f.read_exact(&mut buf)?;
                self.offset = self.offset.saturating_add(bytes);
                Ok(Some(buf))
            }
        }
    }
}

/// Append raw WAL bytes onto a replica directory's `CURRENT.log` and fsync.
///
/// Creates the directory if missing. Does **not** open the DB (caller opens
/// after shipping, or between batches). Uses production Env.
///
/// # Errors
/// I/O while creating/appending/syncing.
pub fn append_wal_bytes(replica_dir: impl AsRef<Path>, bytes: &[u8]) -> ShipResult<()> {
    append_wal_bytes_on(&IoUringEnv::default(), replica_dir, bytes)
}

/// Append WAL bytes via [`Env`] (create/append/sync + dir sync).
///
/// # Errors
/// I/O while creating/appending/syncing.
pub fn append_wal_bytes_on<E: Env>(
    env: &E,
    replica_dir: impl AsRef<Path>,
    bytes: &[u8],
) -> ShipResult<()> {
    if pedradb_core::write_admission_kernel::batch_is_empty(bytes.len() as u64) {
        return Ok(());
    }
    let dir = replica_dir.as_ref();
    env.create_dir_all(dir)?;
    let path = dir.join(WAL_FILE_NAME);
    let mut f = env.open_append(&path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    // Directory durability for the new/updated name (RFC-0015 H5 — surface errors).
    env.sync_dir(dir)?;
    Ok(())
}

/// Pull from primary and append to replica until caught up (one shot).
///
/// # Errors
/// Ship I/O or WAL rotation.
pub fn catch_up(shipper: &mut WalShipper, replica_dir: impl AsRef<Path>) -> ShipResult<usize> {
    catch_up_on(&IoUringEnv::default(), shipper, replica_dir)
}

/// Like [`catch_up`] with an explicit [`Env`].
///
/// # Errors
/// Ship I/O or WAL rotation.
pub fn catch_up_on<E: Env>(
    env: &E,
    shipper: &mut WalShipper,
    replica_dir: impl AsRef<Path>,
) -> ShipResult<usize> {
    let mut total = 0usize;
    while let Some(chunk) = shipper.pull_on(env)? {
        let n = chunk.len();
        append_wal_bytes_on(env, replica_dir.as_ref(), &chunk)?;
        total += n;
    }
    Ok(total)
}

/// Open a replica DB after WAL ship (read-mostly; `exclusive` as given).
///
/// # Errors
/// PedraDB open/recover errors.
pub fn open_replica(replica_dir: impl AsRef<Path>, exclusive: bool) -> CoreResult<Db<IoUringEnv>> {
    Db::open_with_env(
        replica_dir,
        DbOpen {
            wal_full_fsync: true,
            history: Default::default(),
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive,
            large_value_threshold: None,
            wal_recovery: Default::default(),
            sst_payload_budget_bytes: None,
        },
        IoUringEnv::default(),
    )
}

/// High-level: primary dir → replica dir full catch-up from WAL start, then open.
///
/// Intended for demos/tests where the primary has **not** flushed (WAL still
/// holds all data).
///
/// # Errors
/// Ship or open errors.
pub fn bootstrap_replica_from_wal(
    primary_dir: impl AsRef<Path>,
    replica_dir: impl AsRef<Path>,
) -> ShipResult<Db<IoUringEnv>> {
    let mut shipper = WalShipper::from_start(primary_dir.as_ref());
    catch_up(&mut shipper, replica_dir.as_ref())?;
    Ok(open_replica(replica_dir.as_ref(), true)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pedradb_core::{Db, OpenOptions};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pedradb-repl-{tag}-{n}-{i}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn open_primary(dir: &Path) -> Db {
        Db::open_with(
            dir,
            OpenOptions {
                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
                sync: true,
                auto_flush_bytes: None, // keep data in WAL for ship
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
                sst_payload_budget_bytes: None,
            },
        )
        .unwrap()
    }

    #[test]
    fn ship_puts_replica_sees_keys() {
        let primary = temp_dir("p");
        let replica = temp_dir("r");

        {
            let mut db = open_primary(&primary);
            db.put(b"a", b"1").unwrap();
            db.put(b"b", b"2").unwrap();
            // leave open so WAL is the live file; ship after close is also fine
            db.close().unwrap();
        }

        let mut shipper = WalShipper::from_start(&primary);
        let n = catch_up(&mut shipper, &replica).unwrap();
        assert!(n > 0, "expected WAL bytes");

        let db = open_replica(&replica, true).unwrap();
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        db.close().unwrap();

        let _ = std::fs::remove_dir_all(&primary);
        let _ = std::fs::remove_dir_all(&replica);
    }

    /// Chunked pull: small max_pull_bytes still catches up fully.
    #[test]
    fn chunked_pull_catch_up_same_as_full() {
        let primary = temp_dir("pchunk");
        let replica = temp_dir("rchunk");
        {
            let mut db = open_primary(&primary);
            for i in 0..30u8 {
                db.put([b'k', i], [b'v', i]).unwrap();
            }
            db.close().unwrap();
        }
        let mut shipper = WalShipper::from_start(&primary);
        shipper.set_max_pull_bytes(64); // force many chunks
        let mut pulls = 0u32;
        let mut total = 0usize;
        while let Some(chunk) = shipper.pull().unwrap() {
            assert!(chunk.len() <= 64);
            append_wal_bytes(&replica, &chunk).unwrap();
            total += chunk.len();
            pulls += 1;
        }
        assert!(pulls > 1, "expected multi-chunk ship, got pulls={pulls}");
        assert!(total > 64);
        let db = open_replica(&replica, true).unwrap();
        for i in 0..30u8 {
            assert_eq!(
                db.get(&[b'k', i]).as_deref(),
                Some([b'v', i].as_slice()),
                "key {i}"
            );
        }
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&primary);
        let _ = std::fs::remove_dir_all(&replica);
    }

    #[test]
    fn incremental_ship_after_follow() {
        let primary = temp_dir("p2");
        let replica = temp_dir("r2");

        let mut db = open_primary(&primary);
        db.put(b"seed", b"0").unwrap();
        // Cursor at end after seed — seed must be shipped with from_start first.
        drop(db);

        // Full bootstrap of seed.
        let mut shipper = WalShipper::from_start(&primary);
        catch_up(&mut shipper, &replica).unwrap();
        {
            let db = open_replica(&replica, true).unwrap();
            assert_eq!(db.get(b"seed").as_deref(), Some(b"0".as_ref()));
            db.close().unwrap();
        }

        // New writes; follow-style pull from advanced cursor.
        {
            let mut db = open_primary(&primary);
            db.put(b"later", b"1").unwrap();
            db.close().unwrap();
        }
        catch_up(&mut shipper, &replica).unwrap();
        let db = open_replica(&replica, true).unwrap();
        assert_eq!(db.get(b"seed").as_deref(), Some(b"0".as_ref()));
        assert_eq!(db.get(b"later").as_deref(), Some(b"1".as_ref()));
        db.close().unwrap();

        let _ = std::fs::remove_dir_all(&primary);
        let _ = std::fs::remove_dir_all(&replica);
    }

    #[test]
    fn flush_rotates_wal_detected() {
        let primary = temp_dir("p3");
        let mut db = open_primary(&primary);
        db.put(b"x", b"y").unwrap();
        let mut shipper = WalShipper::from_start(&primary);
        // Drain current WAL so cursor is at EOF (and the stamp is captured).
        let _ = shipper.pull().unwrap();
        db.flush().unwrap(); // truncates / replaces WAL
        db.put(b"after", b"z").unwrap();
        db.close().unwrap();

        // F165: even when the fresh file regrows past the stale cursor, the
        // prefix stamp differs and pull must fail closed.
        match shipper.pull() {
            Err(ShipError::WalRotated { .. }) => {}
            other => panic!("expected WalRotated, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&primary);
    }

    /// F165 REAL: rotate (flush) then regrow past the stale cursor. Length-only
    /// detection returned `Ok(Some(misaligned bytes))` and the records below
    /// the cursor were never shipped — a silently stale replica.
    #[test]
    fn rotation_regrow_past_cursor_fails_closed() {
        let primary = temp_dir("f165");
        let mut db = open_primary(&primary);
        for i in 0..40u8 {
            db.put([b'k', i], [b'v', i]).unwrap();
        }
        let mut shipper = WalShipper::follow(&primary).unwrap();
        let cursor = shipper.offset();
        assert!(cursor > 0, "pre-flush WAL must be non-empty");

        db.flush().unwrap(); // rotate: CURRENT.log truncated to 0, same path
        let len_after_flush = std::fs::metadata(primary.join(WAL_FILE_NAME))
            .unwrap()
            .len();
        assert!(
            len_after_flush < cursor,
            "flush must rotate (truncate) the WAL for this hazard"
        );
        // Regrow the fresh log past the stale cursor (two rounds of writes).
        for i in 0..40u8 {
            db.put([b'j', i], [b'w', i]).unwrap();
        }
        for i in 0..40u8 {
            db.put([b'm', i], [b'u', i]).unwrap();
        }
        let len_now = std::fs::metadata(primary.join(WAL_FILE_NAME))
            .unwrap()
            .len();
        assert!(len_now > cursor, "regrow must pass the stale cursor");
        match shipper.pull() {
            Err(ShipError::WalRotated { .. }) => {}
            Ok(Some(bytes)) => panic!(
                "F165 AS-IS: shipped {} misaligned bytes; records below cursor {cursor} never shipped",
                bytes.len()
            ),
            other => panic!("expected WalRotated, got {other:?}"),
        }
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&primary);
    }

    /// Catalog three-teeth plant. Direct `rotation_regrow_past_cursor_fails_closed` is **not** this tooth.
    #[test]
    fn pull_plan_on_live_ship_is_not_ok() {
        let stamp = [7u8; SHIP_STAMP_BYTES];
        let mut new_stamp = [9u8; SHIP_STAMP_BYTES];
        new_stamp[0] ^= 0xff;
        assert_eq!(
            pull_plan(Some(500), 300, 4_000_000, Some(&stamp), &new_stamp),
            PullPlan::Rotated {
                file_len: 500,
                cursor: 300
            }
        );
        assert_eq!(
            crate::ship_kernel::pull_plan_as_is(
                Some(500),
                300,
                4_000_000,
                Some(&stamp),
                &new_stamp
            ),
            PullPlan::Ship { bytes: 200 },
            "AS-IS dente: length-only ships misaligned bytes after rotate-regrow"
        );
        let primary = temp_dir("ship-plant");
        let mut db = open_primary(&primary);
        for i in 0..40u8 {
            db.put([b'k', i], [b'v', i]).unwrap();
        }
        let mut shipper = WalShipper::follow(&primary).unwrap();
        let cursor = shipper.offset();
        assert!(cursor > 0);
        db.flush().unwrap();
        for i in 0..40u8 {
            db.put([b'j', i], [b'w', i]).unwrap();
        }
        for i in 0..40u8 {
            db.put([b'm', i], [b'u', i]).unwrap();
        }
        let len_now = std::fs::metadata(primary.join(WAL_FILE_NAME))
            .unwrap()
            .len();
        assert!(len_now > cursor, "regrow must pass the stale cursor");
        match shipper.pull() {
            Err(ShipError::WalRotated { .. }) => {}
            other => panic!("live pull_plan must fail closed on stamp change, got {other:?}"),
        }
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&primary);
    }

    /// F165: a WAL deleted under an advanced cursor must not read as "caught up".
    #[test]
    fn vanished_wal_under_cursor_fails_closed() {
        let primary = temp_dir("f165-gone");
        {
            let mut db = open_primary(&primary);
            db.put(b"a", b"1").unwrap();
            db.close().unwrap();
        }
        let mut shipper = WalShipper::follow(&primary).unwrap();
        assert!(shipper.offset() > 0);
        std::fs::remove_file(primary.join(WAL_FILE_NAME)).unwrap();
        match shipper.pull() {
            Err(ShipError::WalRotated { .. }) => {}
            Ok(None) => panic!("F165 AS-IS: vanished WAL silently reported up-to-date"),
            other => panic!("expected WalRotated, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&primary);
    }

    /// No false positive: rotation before the first `from_start` pull ships the
    /// whole fresh log (cursor 0 has no gap to miss).
    #[test]
    fn from_start_after_rotation_ships_fresh_log() {
        let primary = temp_dir("f165-fp");
        let replica = temp_dir("f165-fp-r");
        {
            let mut db = open_primary(&primary);
            for i in 0..10u8 {
                db.put([b'k', i], [b'v', i]).unwrap();
            }
            db.flush().unwrap(); // rotate
            for i in 0..10u8 {
                db.put([b'j', i], [b'w', i]).unwrap();
            }
            db.close().unwrap();
        }
        let mut shipper = WalShipper::from_start(&primary);
        catch_up(&mut shipper, &replica).unwrap();
        let db = open_replica(&replica, true).unwrap();
        for i in 0..10u8 {
            assert_eq!(
                db.get(&[b'j', i]).as_deref(),
                Some([b'w', i].as_slice()),
                "post-rotate key {i} must ship from a cursor-0 catch-up"
            );
        }
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&primary);
        let _ = std::fs::remove_dir_all(&replica);
    }

    #[test]
    fn bootstrap_helper() {
        let primary = temp_dir("p4");
        let replica = temp_dir("r4");
        {
            let mut db = open_primary(&primary);
            db.put(b"k", b"v").unwrap();
            db.close().unwrap();
        }
        let db = bootstrap_replica_from_wal(&primary, &replica).unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(b"v".as_ref()));
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&primary);
        let _ = std::fs::remove_dir_all(&replica);
    }

    /// RFC-0015 P1.4: append via Env surfaces FailingEnv injection.
    #[test]
    fn append_wal_bytes_on_failing_env() {
        use pedradb_sim::FailingEnv;

        let replica = temp_dir("fail-append");
        let env = FailingEnv::fail_after(0);
        let err = append_wal_bytes_on(&env, &replica, b"not-a-wal-record").unwrap_err();
        assert!(
            err.to_string().contains("injected") || err.to_string().contains("io"),
            "expected injected I/O, got {err}"
        );
        assert!(env.tripped());
        let _ = std::fs::remove_dir_all(&replica);
    }

    #[test]
    fn append_wal_bytes_on_sync_fail() {
        use pedradb_sim::{FailingEnv, FaultKind};

        let replica = temp_dir("sync-fail-append");
        // create_dir_all + open_append + write succeed under SyncFail; first sync fails.
        let env = FailingEnv::fail_after_kind(0, FaultKind::SyncFail);
        let err = append_wal_bytes_on(&env, &replica, b"payload").unwrap_err();
        assert!(
            err.to_string().contains("sync") || err.to_string().contains("io"),
            "expected sync failure, got {err}"
        );
        assert!(env.tripped());
        let _ = std::fs::remove_dir_all(&replica);
    }
}
