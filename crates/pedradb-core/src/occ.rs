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
    read_set: BTreeSet<Bytes>,
    staging: BTreeMap<Bytes, Stage>,
    finished: bool,
}

impl<E: Env> OccTransaction<E> {
    pub(crate) fn new(db: ConcurrentDb<E>) -> Self {
        let snapshot = db.last_sequence();
        Self {
            db,
            snapshot,
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

    /// Read at snapshot; records the key in the OCC read set.
    #[must_use]
    pub fn get(&mut self, key: &[u8]) -> Option<Bytes> {
        self.read_set.insert(Bytes::copy_from_slice(key));
        if let Some(stage) = self.staging.get(key) {
            return match stage {
                Stage::Put(v) => Some(v.clone()),
                Stage::Delete => None,
            };
        }
        if self.snapshot == 0 {
            return None;
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

    /// Validate OCC then commit (one WAL record under the write lock).
    ///
    /// # Errors
    /// [`CoreError::TransactionConflict`], WAL I/O, or already finished.
    pub fn commit(self) -> Result<()> {
        self.commit_with(WriteOptions::default())
    }

    /// Commit with durability options.
    ///
    /// # Errors
    /// Conflict, WAL I/O, or finished.
    pub fn commit_with(mut self, durability: WriteOptions) -> Result<()> {
        self.ensure_open()?;
        if self.staging.is_empty() {
            self.finished = true;
            return Ok(());
        }

        let staging = mem::take(&mut self.staging);
        let read_set = mem::take(&mut self.read_set);
        let snapshot = self.snapshot;
        let db = self.db.clone();
        self.finished = true;

        db.with_write(|inner| {
            // Validate: no version with seq > snapshot on any read or write key.
            for key in read_set.iter().chain(staging.keys()) {
                if inner.key_has_write_after(key.as_ref(), snapshot) {
                    return Err(CoreError::TransactionConflict);
                }
            }

            let mut ops = Vec::with_capacity(staging.len());
            for (key, stage) in staging {
                match stage {
                    Stage::Put(value) => ops.push(BatchOp::Put { key, value }),
                    Stage::Delete => ops.push(BatchOp::Delete { key }),
                }
            }
            // apply_batch_with assigns sequences and commits one WAL record.
            inner.apply_batch_with(ops, durability)?;
            Ok(())
        })
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

    fn open_cdb(dir: &std::path::Path) -> ConcurrentDb {
        ConcurrentDb::open_with(
            dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
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
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: Some(512),
            },
        )
        .unwrap();
        db.put(b"huge", &big).unwrap();
        let mut tx = db.begin_occ();
        let v = tx.get(b"huge").expect("OCC get must see key");
        assert_eq!(v.as_ref(), big.as_slice());
        assert_eq!(v.len(), 2048, "OCC get must resolve VLG1, not return pointer");
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

    #[test]
    fn occ_write_write_conflict_on_same_key() {
        let dir = temp_dir();
        let db = open_cdb(&dir);
        db.put(b"k", b"v0").unwrap();

        // Two overlapping OCC txs: both read k, both try to write k.
        let mut tx1 = db.begin_occ();
        let mut tx2 = db.begin_occ();
        assert_eq!(tx1.get(b"k").as_deref(), Some(b"v0".as_ref()));
        assert_eq!(tx2.get(b"k").as_deref(), Some(b"v0".as_ref()));
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
        assert_eq!(tx.get(b"m").as_deref(), Some(b"v0".as_ref()));
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
        assert_eq!(tx.get(b"m").as_deref(), Some(b"v0".as_ref()));
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