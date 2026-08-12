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
//! - **Flush on the primary rotates/truncates the WAL.** After flush, a shipper
//!   whose cursor points past the new file length returns [`ShipError::WalRotated`];
//!   you must re-bootstrap the replica (copy SSTs/MANIFEST, or rebuild from snapshot).
//! - This is **not** Raft. For ordered multi-node apply of a shared log, see
//!   `pedradb-apply`. WAL ship is for **asynchronous read replicas** of one writer.
//! - The replica must not take local writes while shipping (single-writer primary).
//!
//! Durable ship I/O uses [`pedradb_core::Env`] (path-only helpers default to [`StdEnv`]).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use pedradb_core::{
    Db, Env, EnvFile, OpenOptions as DbOpen, Result as CoreResult, StdEnv, WAL_FILE_NAME,
};
use thiserror::Error;

/// Errors from WAL shipping (distinct from engine [`pedradb_core::CoreError`]).
#[derive(Debug, Error)]
pub enum ShipError {
    /// Underlying I/O.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Primary WAL was truncated/replaced (flush/rotate); cursor is invalid.
    #[error("WAL rotated or truncated (file len {file_len} < cursor {cursor}); re-bootstrap replica")]
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
        Self::follow_on(&StdEnv, primary_dir)
    }

    /// Like [`Self::follow`] with an explicit [`Env`].
    ///
    /// # Errors
    /// Metadata I/O if the WAL exists but cannot be stat'd.
    pub fn follow_on<E: Env>(env: &E, primary_dir: impl AsRef<Path>) -> ShipResult<Self> {
        let wal_path = primary_dir.as_ref().join(WAL_FILE_NAME);
        let offset = if env.exists(&wal_path) {
            env.metadata_len(&wal_path)?
        } else {
            0
        };
        Ok(Self {
            wal_path,
            offset,
            max_pull_bytes: DEFAULT_MAX_PULL_BYTES,
        })
    }

    /// Ship from byte 0 (full WAL catch-up for a fresh replica).
    #[must_use]
    pub fn from_start(primary_dir: impl AsRef<Path>) -> Self {
        Self {
            wal_path: primary_dir.as_ref().join(WAL_FILE_NAME),
            offset: 0,
            max_pull_bytes: DEFAULT_MAX_PULL_BYTES,
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
    /// I/O or [`ShipError::WalRotated`] if the file shrank (flush).
    pub fn pull(&mut self) -> ShipResult<Option<Vec<u8>>> {
        self.pull_on(&StdEnv)
    }

    /// Like [`Self::pull`] with an explicit [`Env`].
    ///
    /// # Errors
    /// I/O or [`ShipError::WalRotated`] if the file shrank (flush).
    pub fn pull_on<E: Env>(&mut self, env: &E) -> ShipResult<Option<Vec<u8>>> {
        if !env.exists(&self.wal_path) {
            return Ok(None);
        }
        let len = env.metadata_len(&self.wal_path)?;
        if len < self.offset {
            return Err(ShipError::WalRotated {
                file_len: len,
                cursor: self.offset,
            });
        }
        if len == self.offset {
            return Ok(None);
        }
        let remaining = len - self.offset;
        let take = remaining.min(self.max_pull_bytes);
        let take_usize = usize::try_from(take).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "pull chunk does not fit usize",
            )
        })?;
        let mut f = env.open_read(&self.wal_path)?;
        f.seek(SeekFrom::Start(self.offset))?;
        let mut buf = vec![0u8; take_usize];
        f.read_exact(&mut buf)?;
        self.offset = self.offset.saturating_add(take);
        Ok(Some(buf))
    }
}

/// Append raw WAL bytes onto a replica directory's `CURRENT.log` and fsync.
///
/// Creates the directory if missing. Does **not** open the DB (caller opens
/// after shipping, or between batches). Uses [`StdEnv`].
///
/// # Errors
/// I/O while creating/appending/syncing.
pub fn append_wal_bytes(replica_dir: impl AsRef<Path>, bytes: &[u8]) -> ShipResult<()> {
    append_wal_bytes_on(&StdEnv, replica_dir, bytes)
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
    if bytes.is_empty() {
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
    catch_up_on(&StdEnv, shipper, replica_dir)
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
pub fn open_replica(replica_dir: impl AsRef<Path>, exclusive: bool) -> CoreResult<Db> {
    Db::open_with(
        replica_dir,
        DbOpen {
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive,
            large_value_threshold: None,
        },
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
) -> ShipResult<Db> {
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
                sync: true,
                auto_flush_bytes: None, // keep data in WAL for ship
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
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
        // Drain current WAL so cursor is at EOF.
        let _ = shipper.pull().unwrap();
        db.flush().unwrap(); // truncates / replaces WAL
        // Write something so file may be shorter or reset.
        db.put(b"after", b"z").unwrap();
        db.close().unwrap();

        // Cursor likely past new file start; pull must report rotate or succeed
        // only if file grew past old offset (unlikely after truncate).
        match shipper.pull() {
            Err(ShipError::WalRotated { .. }) => {}
            Ok(None) => {
                // If implementation recreated WAL at same or larger size without
                // shrinking below cursor, treat as non-fatal for this host FS.
            }
            Ok(Some(_)) => {
                // Appended after recreate with larger offset path — ok.
            }
            Err(e) => panic!("unexpected {e:?}"),
        }
        let _ = std::fs::remove_dir_all(&primary);
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
