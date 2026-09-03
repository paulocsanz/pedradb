//! Optimistic multi-writer transactions (RFC-0014 P2.1).
//!
//! Unlike [`crate::tx::Transaction`] (exclusive `&mut Db`), an [`OccTransaction`]
//! holds only a snapshot + staging and shares a [`crate::concurrent::ConcurrentDb`].
//! Commit takes the write lock, validates that no key in the read/write set has a
//! newer version than the snapshot, then applies the batch atomically.
//!
//! # Semantics
//! - Snapshot isolation for reads at begin time.
//! - Conflict on any concurrent committed write to a **read or written** key
//!   with `sequence > snapshot` (fail-closed; no silent overwrite of concurrent work).
//! - Writes absorbed into the **same atomic group commit** are simultaneous
//!   (one write-lock hold, one WAL fsync): they never conflict with each
//!   other, and per-member sequence order inside a group is not
//!   serialization order.
//! - `ConcurrentDb`'s write lock still serialises the commit critical section
//!   (OCC validation + WAL fsync); the point is **detectable conflicts** when two
//!   txs overlap on keys, not lock-free LSM multi-writer amp.

use std::collections::{BTreeMap, BTreeSet};
use std::mem;

use bytes::Bytes;

use crate::batch::WriteOp;
use crate::concurrent::ConcurrentDb;
use crate::db::{BatchOp, WriteOptions};
use crate::env::Env;
use crate::error::{CoreError, Result};
use crate::key::SequenceNumber;

#[derive(Debug, Clone)]
enum Stage {
    Put(Bytes),
    Delete,
}

/// Optimistic TX over a shared [`ConcurrentDb`].
pub struct OccTransaction<E: Env = crate::env::StdEnv> {
    db: ConcurrentDb<E>,
    snapshot: SequenceNumber,
    /// Fold-GC registration for this TX's snapshot (see
    /// [`ConcurrentDb::occ_register_snapshot`]); released on drop.
    floor_id: u64,
    read_set: BTreeSet<Bytes>,
    staging: BTreeMap<Bytes, Stage>,
    finished: bool,
}

impl<E: Env> OccTransaction<E> {
    pub(crate) fn new(db: ConcurrentDb<E>) -> Self {
        // Prefer last_sequence when the write lock is free (sees just-applied
        // versions). If a commit holds the write lock, don't stall — snapshot
        // at published_seq (lock-free). Either way the registry lower bound
        // is installed first so fold-GC cannot pass this TX's snapshot.
        let floor_id = db.occ_register_snapshot();
        let snapshot = db.occ_snapshot();
        Self {
            db,
            snapshot,
            floor_id,
            read_set: BTreeSet::new(),
            staging: BTreeMap::new(),
            finished: false,
        }
    }

    /// Snapshot sequence at begin.
    #[must_use]
    pub fn snapshot(&self) -> SequenceNumber {
        self.snapshot
    }

    /// Record `key` in the OCC read set without reading (compat iterator
    /// yields — rust-rocksdb `Transaction::NewIterator` tracks every key).
    pub fn observe(&mut self, key: &[u8]) {
        if self.finished {
            return;
        }
        self.read_set.insert(Bytes::copy_from_slice(key));
    }

    /// Read at snapshot; records the key in the OCC read set.
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`] if version GC dropped history for this TX's snapshot.
    pub fn get(&mut self, key: &[u8]) -> crate::error::Result<Option<Bytes>> {
        self.read_set.insert(Bytes::copy_from_slice(key));
        if let Some(stage) = self.staging.get(key) {
            return Ok(match stage {
                Stage::Put(v) => Some(v.clone()),
                Stage::Delete => None,
            });
        }
        if self.snapshot == 0 {
            return Ok(None);
        }
        // Same double-checked point-cache hit as `get_at`, without the Db
        // read lock (OCC rmw under concurrency was stalling on apply).
        if self.db.visible_sequence() == self.snapshot {
            if let Some(v) = self.db.point_cache_get(key) {
                if self.db.visible_sequence() == self.snapshot {
                    return Ok(v);
                }
            }
        }
        // Use get_at so VLG1 pointers resolve (same as single-writer Transaction).
        self.db
            .with_read(|db| db.get_at(crate::db::Snapshot::at(self.snapshot), key))
    }

    /// Stage a put.
    ///
    /// # Errors
    /// [`CoreError::TransactionFinished`].
    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        self.ensure_open()?;
        self.staging.insert(
            Bytes::copy_from_slice(key.as_ref()),
            Stage::Put(Bytes::copy_from_slice(value.as_ref())),
        );
        Ok(())
    }

    /// Stage a delete.
    ///
    /// # Errors
    /// [`CoreError::TransactionFinished`].
    pub fn delete(&mut self, key: impl AsRef<[u8]>) -> Result<()> {
        self.ensure_open()?;
        self.staging
            .insert(Bytes::copy_from_slice(key.as_ref()), Stage::Delete);
        Ok(())
    }

    /// Staged writes, user-key sorted: `(key, Some(value))` = put,
    /// `(key, None)` = delete. Read-your-own-writes overlay for scan/count
    /// surfaces (compat `Transaction::scan_count`); non-consuming.
    #[must_use]
    pub fn staged_entries(&self) -> Vec<(Bytes, Option<Bytes>)> {
        self.staging
            .iter()
            .map(|(k, s)| {
                (
                    k.clone(),
                    match s {
                        Stage::Put(v) => Some(v.clone()),
                        Stage::Delete => None,
                    },
                )
            })
            .collect()
    }

    /// Validate OCC then commit (one WAL record under the write lock).
    ///
    /// # Errors
    /// [`CoreError::TransactionConflict`], WAL I/O, or already finished.
    pub fn commit(self) -> Result<()> {
        self.commit_with(WriteOptions::default())
    }

    /// Commit with durability options. RFC-0054 P2.1: `WriteOptions::sync`
    /// is honored — `no_sync` commits without a WAL barrier (same semantics
    /// as `put_with`); `None` keeps the open-time default.
    ///
    /// # Errors
    /// Conflict, [`CoreError::SnapshotTooOld`], WAL I/O, or finished.
    pub fn commit_with(mut self, durability: WriteOptions) -> Result<()> {
        self.ensure_open()?;
        if self.staging.is_empty() {
            self.finished = true;
            if self.read_set.is_empty() {
                return Ok(());
            }
            // Read-only commits still honour the documented read-set contract:
            // conflict on concurrent writes to keys this TX read (occ.rs
            // header), and `SnapshotTooOld` once GC moved past the snapshot.
            let read_set = mem::take(&mut self.read_set);
            let snapshot = self.snapshot;
            let db = self.db.clone();
            return db.with_write(|guard| {
                guard.ensure_snapshot_readable(crate::db::Snapshot::at(snapshot))?;
                if guard.last_sequence() != snapshot
                    && read_set
                        .iter()
                        .any(|k| guard.key_has_write_after(k.as_ref(), snapshot))
                {
                    return Err(CoreError::TransactionConflict);
                }
                Ok(())
            });
        }

        let staging = mem::take(&mut self.staging);
        let read_set = mem::take(&mut self.read_set);
        let snapshot = self.snapshot;
        let db = self.db.clone();
        self.finished = true;

        // No upfront key clones: `apply_batch_occ` validates the write set by
        // reference from `ops` and only walks it when a publish actually raced.
        let mut ops = Vec::with_capacity(staging.len());
        for (key, stage) in staging {
            ops.push(match stage {
                Stage::Put(value) => BatchOp::Put { key, value },
                Stage::Delete => BatchOp::Delete { key },
            });
        }
        db.apply_batch_occ_with(snapshot, read_set, ops, durability)
            .map(|_| ())
    }

    /// Discard staged changes.
    pub fn abort(mut self) {
        self.finished = true;
        self.staging.clear();
        self.read_set.clear();
    }

    fn ensure_open(&self) -> Result<()> {
        if self.finished {
            Err(CoreError::TransactionFinished)
        } else {
            Ok(())
        }
    }
}

impl<E: Env> Drop for OccTransaction<E> {
    fn drop(&mut self) {
        self.finished = true;
        self.db.occ_unregister_snapshot(self.floor_id);
    }
}

// Silence unused WriteOp import path if only BatchOp is used.
#[allow(dead_code)]
fn _write_op_ty(_: WriteOp) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::concurrent::ConcurrentDb;
    use crate::db::OpenOptions;
    use std::fs;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pedradb-occ-{n}-{i}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    /// RFC-0054 P2.1: `commit_with(WriteOptions)` must honor `sync` —
    /// `no_sync` commits the WAL record without a barrier, explicit `sync`
    /// barriers, and `None` resolves to the open-time default. Proven at the
    /// env seam (counting WAL barriers), not assumed.
    #[test]
    fn occ_write_options_sync_honored() {
        use crate::db::WAL_FILE_NAME;
        use crate::env::{Env, EnvFile, StdEnv};
        use std::cell::Cell;
        use std::io::{self, Read, Seek, SeekFrom, Write};
        use std::path::Path;
        use std::rc::Rc;

        #[derive(Default)]
        struct Counts {
            barriers: Cell<u64>,
        }
        #[derive(Clone)]
        struct CountingEnv {
            inner: StdEnv,
            counts: Rc<Counts>,
        }
        struct CountingFile {
            inner: <StdEnv as Env>::File,
            counts: Rc<Counts>,
            is_wal: bool,
        }
        impl Read for CountingFile {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.inner.read(buf)
            }
        }
        impl Write for CountingFile {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.inner.write(buf)
            }
            fn flush(&mut self) -> io::Result<()> {
                self.inner.flush()
            }
        }
        impl Seek for CountingFile {
            fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
                self.inner.seek(pos)
            }
        }
        impl EnvFile for CountingFile {
            fn sync_data(&mut self) -> io::Result<()> {
                if self.is_wal {
                    self.counts.barriers.set(self.counts.barriers.get() + 1);
                }
                self.inner.sync_data()
            }
            fn sync_data_strong(&mut self) -> io::Result<()> {
                if self.is_wal {
                    self.counts.barriers.set(self.counts.barriers.get() + 1);
                }
                self.inner.sync_data_strong()
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
        impl Env for CountingEnv {
            type File = CountingFile;
            fn create_dir_all(&self, path: &Path) -> io::Result<()> {
                self.inner.create_dir_all(path)
            }
            fn create(&self, path: &Path) -> io::Result<Self::File> {
                Ok(CountingFile {
                    inner: self.inner.create(path)?,
                    counts: Rc::clone(&self.counts),
                    is_wal: path.file_name().is_some_and(|n| n == WAL_FILE_NAME),
                })
            }
            fn open_append(&self, path: &Path) -> io::Result<Self::File> {
                Ok(CountingFile {
                    inner: self.inner.open_append(path)?,
                    counts: Rc::clone(&self.counts),
                    is_wal: path.file_name().is_some_and(|n| n == WAL_FILE_NAME),
                })
            }
            fn open_read(&self, path: &Path) -> io::Result<Self::File> {
                Ok(CountingFile {
                    inner: self.inner.open_read(path)?,
                    counts: Rc::clone(&self.counts),
                    is_wal: path.file_name().is_some_and(|n| n == WAL_FILE_NAME),
                })
            }
            fn sync_dir(&self, path: &Path) -> io::Result<()> {
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

        let dir = temp_dir();
        let env = CountingEnv {
            inner: StdEnv,
            counts: Rc::default(),
        };
        let db = ConcurrentDb::open_with_env(
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

        // no_sync: WAL record committed, no barrier on the WAL file.
        let before = env.counts.barriers.get();
        let mut tx = db.begin_occ();
        tx.put(b"a", b"1").unwrap();
        tx.commit_with(WriteOptions::no_sync()).unwrap();
        assert_eq!(
            env.counts.barriers.get(),
            before,
            "no_sync OCC commit must not barrier the WAL"
        );
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));

        // Explicit sync: one barrier.
        let before = env.counts.barriers.get();
        let mut tx = db.begin_occ();
        tx.put(b"b", b"2").unwrap();
        tx.commit_with(WriteOptions::sync()).unwrap();
        assert_eq!(
            env.counts.barriers.get(),
            before + 1,
            "sync OCC commit must barrier the WAL exactly once"
        );

        // None → open-time default (sync=true here): one barrier.
        let before = env.counts.barriers.get();
        let mut tx = db.begin_occ();
        tx.put(b"c", b"3").unwrap();
        tx.commit().unwrap();
        assert_eq!(
            env.counts.barriers.get(),
            before + 1,
            "default must resolve to the open-time sync policy"
        );

        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    fn open_cdb(dir: &std::path::Path) -> ConcurrentDb {
        ConcurrentDb::open_with(
            dir,
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
        )
        .unwrap()
    }

    #[test]
    fn occ_get_resolves_large_vlog_value() {
        let dir = temp_dir();
        let big = vec![0x11u8; 2048];
        let db = ConcurrentDb::open_with(
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
                large_value_threshold: Some(512),
                sst_payload_budget_bytes: None,
            },
        )
        .unwrap();
        db.put(b"huge", &big).unwrap();
        let mut tx = db.begin_occ();
        let v = tx
            .get(b"huge")
            .expect("OCC get result")
            .expect("OCC get must see key");
        assert_eq!(v.as_ref(), big.as_slice());
        assert_eq!(
            v.len(),
            2048,
            "OCC get must resolve VLG1, not return pointer"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn occ_single_writer_commit_ok() {
        let dir = temp_dir();
        let db = open_cdb(&dir);
        {
            let mut tx = db.begin_occ();
            tx.put(b"a", b"1").unwrap();
            tx.put(b"b", b"2").unwrap();
            tx.commit().unwrap();
        }
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    /// `get_at` fast path: snapshot == published may serve from the point
    /// cache; once a newer version is published (and the cache refilled with
    /// it), the older snapshot must not see it — and commit conflicts.
    #[test]
    fn occ_get_at_snapshot_is_stable_across_publish_and_cache_refill() {
        let dir = temp_dir();
        let db = open_cdb(&dir);
        db.put(b"k", b"v0").unwrap();
        // Fill the point cache with the latest value.
        assert_eq!(db.get(b"k").as_deref(), Some(b"v0".as_ref()));

        let mut tx = db.begin_occ(); // snapshot == published
        assert_eq!(tx.get(b"k").unwrap().as_deref(), Some(b"v0".as_ref()));
        tx.put(b"k2", b"w").unwrap(); // writer tx: commit validates the read set

        // Publish a newer version and refill the cache with it.
        db.put(b"k", b"v1").unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(b"v1".as_ref()));

        // TX still reads its own snapshot, not the refilled cache.
        assert_eq!(tx.get(b"k").unwrap().as_deref(), Some(b"v0".as_ref()));
        assert!(matches!(tx.commit(), Err(CoreError::TransactionConflict)));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Concurrent reclaim past OCC snapshot → commit fails SnapshotTooOld.
    #[test]
    fn occ_commit_too_old_after_reclaim() {
        let dir = temp_dir();
        let db = open_cdb(&dir);
        db.put(b"k", b"v0").unwrap();
        let mut tx = db.begin_occ();
        tx.put(b"k", b"v1").unwrap();
        // Reclaim without pins advances watermark to last_seq at reclaim time.
        db.compact_reclaim().unwrap();
        // Further writes raise last_seq; force a history-dropping latest_only after
        // more versions so watermark is strictly above the OCC snapshot.
        db.put(b"k", b"v2").unwrap();
        db.flush().unwrap();
        db.with_write(|inner| {
            inner
                .compact_with(crate::db::CompactOptions::latest_only())
                .unwrap();
        });
        assert!(db.earliest_readable_sequence() > 0);
        let err = tx.commit().unwrap_err();
        assert!(
            matches!(err, CoreError::SnapshotTooOld { .. }),
            "expected SnapshotTooOld on OCC commit, got {err:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn occ_write_write_conflict_on_same_key() {
        let dir = temp_dir();
        let db = open_cdb(&dir);
        db.put(b"k", b"v0").unwrap();

        // Two overlapping OCC txs: both read k, both try to write k.
        let mut tx1 = db.begin_occ();
        let mut tx2 = db.begin_occ();
        assert_eq!(tx1.get(b"k").unwrap().as_deref(), Some(b"v0".as_ref()));
        assert_eq!(tx2.get(b"k").unwrap().as_deref(), Some(b"v0".as_ref()));
        tx1.put(b"k", b"from1").unwrap();
        tx2.put(b"k", b"from2").unwrap();
        tx1.commit().unwrap();
        let err = tx2.commit().unwrap_err();
        assert!(
            matches!(err, CoreError::TransactionConflict),
            "expected conflict, got {err:?}"
        );
        // Winner's value only.
        assert_eq!(db.get(b"k").as_deref(), Some(b"from1".as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn occ_concurrent_threads_conflict_or_serialize() {
        let dir = temp_dir();
        let db = Arc::new(open_cdb(&dir));
        db.put(b"counter", b"0").unwrap();

        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for id in 0..2u8 {
            let db = Arc::clone(&db);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                let mut tx = db.begin_occ();
                let _ = tx.get(b"counter"); // read-set
                barrier.wait();
                // Both try to overwrite after overlapping snapshots.
                tx.put(b"counter", [id]).unwrap();
                tx.commit()
            }));
        }
        let mut ok = 0usize;
        let mut conflicts = 0usize;
        for h in handles {
            match h.join().unwrap() {
                Ok(()) => ok += 1,
                Err(CoreError::TransactionConflict) => conflicts += 1,
                Err(e) => panic!("unexpected {e:?}"),
            }
        }
        // At least one must succeed; if both raced with same snapshot, one conflicts.
        assert!(ok >= 1, "at least one commit must succeed");
        assert!(
            ok + conflicts == 2,
            "only Ok or Conflict, ok={ok} conflicts={conflicts}"
        );
        if conflicts == 1 {
            assert_eq!(ok, 1);
        }
        // Value is one of the two writers (or single winner).
        let v = db.get(b"counter").unwrap();
        assert!(v.as_ref() == [0] || v.as_ref() == [1] || v.as_ref() == b"0");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Concurrent delete_range covering a read key must conflict OCC.
    #[test]
    fn occ_conflicts_on_range_delete_covering_key() {
        let dir = temp_dir();
        let db = open_cdb(&dir);
        db.put(b"m", b"v0").unwrap();

        let mut tx = db.begin_occ();
        assert_eq!(tx.get(b"m").unwrap().as_deref(), Some(b"v0".as_ref()));
        // Concurrent writer range-deletes [a,z) which covers m
        db.delete_range(b"a", b"z").unwrap();
        // TX still tries to write m based on stale snapshot
        tx.put(b"m", b"from_occ").unwrap();
        let err = tx.commit();
        // Expected: TransactionConflict. Bug if Ok and m resurrects under range tomb.
        match err {
            Err(CoreError::TransactionConflict) => {
                assert!(db.get(b"m").is_none(), "range tomb must hide m");
            }
            Ok(()) => {
                panic!(
                    "BUG: OCC commit succeeded after covering delete_range; get={:?}",
                    db.get(b"m")
                );
            }
            Err(e) => panic!("unexpected error: {e:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn occ_conflicts_on_range_delete_after_flush() {
        let dir = temp_dir();
        let db = open_cdb(&dir);
        db.put(b"m", b"v0").unwrap();
        db.flush().unwrap();

        let mut tx = db.begin_occ();
        assert_eq!(tx.get(b"m").unwrap().as_deref(), Some(b"v0".as_ref()));
        db.delete_range(b"a", b"z").unwrap();
        db.flush().unwrap();
        tx.put(b"m", b"from_occ").unwrap();
        let err = tx.commit();
        match err {
            Err(CoreError::TransactionConflict) => {}
            Ok(()) => panic!(
                "BUG: OCC ok after flushed range-delete; get={:?}",
                db.get(b"m")
            ),
            Err(e) => panic!("unexpected {e:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// Write-only OCC (no prior get) must still conflict with covering range delete.
    #[test]
    fn occ_write_only_conflicts_on_range_delete() {
        let dir = temp_dir();
        let db = open_cdb(&dir);
        db.put(b"m", b"v0").unwrap();
        let mut tx = db.begin_occ();
        db.delete_range(b"a", b"z").unwrap();
        tx.put(b"m", b"from_occ").unwrap();
        let err = tx.commit().unwrap_err();
        assert!(
            matches!(err, CoreError::TransactionConflict),
            "write-only OCC must conflict: {err:?}"
        );
        assert!(db.get(b"m").is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
