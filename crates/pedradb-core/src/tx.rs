//! Multi-key transactions (P0.4).
//!
//! Single-writer: [`Db::begin`] borrows the database mutably for the life of
//! the transaction. Commit writes one [`WriteRecord`] so recovery is atomic
//! across all keys in the TX.

use std::collections::BTreeMap;
use std::mem;

use bytes::Bytes;

use crate::batch::WriteOp;
use crate::db::{Db, WriteOptions};
use crate::error::{CoreError, Result};
use crate::key::SequenceNumber;
use crate::memtable::Lookup;

/// Staging entry: put value or delete.
#[derive(Debug, Clone)]
enum Stage {
    Put(Bytes),
    Delete,
}

/// A write transaction with snapshot reads and staged mutations.
///
/// Dropping without [`commit`](Self::commit) aborts (discards staging).
pub struct Transaction<'db, E: crate::env::Env = crate::env::StdEnv> {
    db: &'db mut Db<E>,
    /// Committed sequence visible to reads (set at begin).
    snapshot: SequenceNumber,
    /// User key → staged put/delete (last write wins per key).
    staging: BTreeMap<Bytes, Stage>,
    finished: bool,
}

impl<'db, E: crate::env::Env> Transaction<'db, E> {
    pub(crate) fn new(db: &'db mut Db<E>) -> Self {
        let snapshot = db.last_sequence();
        Self {
            db,
            snapshot,
            staging: BTreeMap::new(),
            finished: false,
        }
    }

    /// Snapshot sequence for this transaction (committed state at begin).
    #[must_use]
    pub fn snapshot(&self) -> SequenceNumber {
        self.snapshot
    }

    /// Read a key: staging first, then MemTable at snapshot.
    #[must_use]
    pub fn get(&self, key: &[u8]) -> Option<Bytes> {
        if let Some(stage) = self.staging.get(key) {
            return match stage {
                Stage::Put(v) => Some(v.clone()),
                Stage::Delete => None,
            };
        }
        if self.snapshot == 0 {
            return None;
        }
        match self.db.lookup(key, self.snapshot) {
            Lookup::Found(v) => Some(v),
            Lookup::Deleted | Lookup::NotFound => None,
        }
    }

    /// Stage a put (visible to later `get` in this TX; durable only after commit).
    ///
    /// # Errors
    /// [`CoreError::TransactionFinished`] if already committed/aborted.
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
    /// [`CoreError::TransactionFinished`] if already committed/aborted.
    pub fn delete(&mut self, key: impl AsRef<[u8]>) -> Result<()> {
        self.ensure_open()?;
        self.staging
            .insert(Bytes::copy_from_slice(key.as_ref()), Stage::Delete);
        Ok(())
    }

    /// Commit all staged ops in one WAL record (multi-key atomic).
    ///
    /// Empty transactions succeed without writing.
    ///
    /// # Errors
    /// WAL I/O, sequence exhaustion, or already finished.
    pub fn commit(self) -> Result<()> {
        self.commit_with(WriteOptions::default())
    }

    /// Commit with explicit [`WriteOptions`] (e.g. `no_sync` + later [`Db::sync`]).
    ///
    /// # Errors
    /// WAL I/O, sequence exhaustion, or already finished.
    pub fn commit_with(mut self, durability: WriteOptions) -> Result<()> {
        self.ensure_open()?;
        if self.staging.is_empty() {
            self.finished = true;
            return Ok(());
        }

        let staging = mem::take(&mut self.staging);
        let mut records = Vec::with_capacity(staging.len());
        for (key, stage) in staging {
            let seq = self.db.alloc_seq()?;
            match stage {
                Stage::Put(value) => records.push(WriteOp::put(seq, key, value)),
                Stage::Delete => records.push(WriteOp::delete(seq, key)),
            }
        }
        self.db.commit_ops_with(records, durability)?;
        // F18: TX is durable after commit_ops; auto-flush must not fail the commit.
        self.db.maybe_auto_flush_best_effort();
        self.finished = true;
        Ok(())
    }

    /// Discard staged changes.
    pub fn abort(mut self) {
        self.finished = true;
        self.staging.clear();
    }

    fn ensure_open(&self) -> Result<()> {
        if self.finished {
            Err(CoreError::TransactionFinished)
        } else {
            Ok(())
        }
    }
}

impl<E: crate::env::Env> Drop for Transaction<'_, E> {
    fn drop(&mut self) {
        // Uncommitted TX: staging drops with us (abort).
        self.finished = true;
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Db;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pedradb-tx-test-{n}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn multi_key_commit() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        {
            let mut tx = db.begin();
            tx.put(b"row", b"data").unwrap();
            tx.put(b"idx", b"ptr").unwrap();
            tx.commit().unwrap();
        }
        assert_eq!(db.get(b"row").as_deref(), Some(b"data".as_ref()));
        assert_eq!(db.get(b"idx").as_deref(), Some(b"ptr".as_ref()));
        assert_eq!(db.last_sequence(), 2);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn abort_discards_staging() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"k", b"old").unwrap();
        {
            let mut tx = db.begin();
            tx.put(b"k", b"new").unwrap();
            tx.put(b"x", b"y").unwrap();
            tx.abort();
        }
        assert_eq!(db.get(b"k").as_deref(), Some(b"old".as_ref()));
        assert_eq!(db.get(b"x"), None);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn drop_aborts() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        {
            let mut tx = db.begin();
            tx.put(b"a", b"1").unwrap();
            // drop without commit
        }
        assert_eq!(db.get(b"a"), None);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_your_writes() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"a", b"1").unwrap();
        {
            let mut tx = db.begin();
            assert_eq!(tx.get(b"a").as_deref(), Some(b"1".as_ref()));
            tx.put(b"a", b"2").unwrap();
            assert_eq!(tx.get(b"a").as_deref(), Some(b"2".as_ref()));
            tx.delete(b"a").unwrap();
            assert_eq!(tx.get(b"a"), None);
            tx.put(b"a", b"3").unwrap();
            assert_eq!(tx.get(b"a").as_deref(), Some(b"3".as_ref()));
            tx.commit().unwrap();
        }
        assert_eq!(db.get(b"a").as_deref(), Some(b"3".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_ignores_unrelated_auto_commit_after_begin_impossible() {
        // With &mut exclusive borrow, auto-commit cannot interleave mid-TX.
        // Snapshot still equals last_sequence at begin.
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"a", b"1").unwrap();
        let tx = db.begin();
        assert_eq!(tx.snapshot(), 1);
        assert_eq!(tx.get(b"a").as_deref(), Some(b"1".as_ref()));
        tx.commit().unwrap();
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn multi_key_survives_reopen() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            let mut tx = db.begin();
            tx.put(b"row", b"R").unwrap();
            tx.put(b"idx", b"I").unwrap();
            tx.delete(b"gone").unwrap(); // delete of missing is fine
            tx.commit().unwrap();
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"row").as_deref(), Some(b"R".as_ref()));
        assert_eq!(db.get(b"idx").as_deref(), Some(b"I".as_ref()));
        assert_eq!(db.get(b"gone"), None);
        // 2 puts + 1 delete
        assert_eq!(db.last_sequence(), 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_commit_ok() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        let tx = db.begin();
        tx.commit().unwrap();
        assert_eq!(db.last_sequence(), 0);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}
