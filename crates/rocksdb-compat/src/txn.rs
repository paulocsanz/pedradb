//! rust-rocksdb-shaped optimistic transactions on Pedra `OccTransaction`.
//!
//! SurrealDB `kv-rocksdb` (`surrealdb/core/src/kvs/rocksdb/mod.rs`) opens
//! `OptimisticTransactionDB`, calls `transaction_opt` with
//! `OptimisticTransactionOptions::set_snapshot(true)` and
//! `WriteOptions::set_sync(false)`, then `get`/`put`/`delete`/`iterator`
//! on the `Transaction` and `commit()`. Conflict at commit is Rocks
//! `Busy`. We map Pedra `TransactionConflict` to that string.
//!
//! OCC read-set policy (RFC-0048 P1.4): `get`/`get_cf` record the key in
//! the read set and `commit()` validates it — read-only commits included
//! (F168). Real Rocks `Transaction::NewIterator` also tracks every key its
//! iterator yields; our `scan_count`/`raw_iterator_opt` read at the txn
//! snapshot but are **untracked**: a commit after a concurrent overwrite of
//! a key that was only seen through a scan still reports `Ok`. Reads that
//! need conflict detection must `get` the key (guard-key pattern); scan
//! reads are advisory. Bounded scan tracking is the P2 follow-up if a
//! caller needs scan-wide OCC.
//!
//! Compile-shape (RFC-0043 P2.4): `open_cf_descriptors`, `ReadOptions`,
//! `raw_iterator_opt`, `property_int_value`, `flush_opt`/`flush_wal`,
//! `compact_range_opt`. Prefix extractor / UDT comparator are accepted
//! no-ops. Versioned CF timestamps remain a gap.

use super::{ColumnFamily, Error, KeyCodec, Result, DB, DEFAULT_CF};
use parking_lot::Mutex;
use pedradb_core::{CoreError, Env, OccTransaction};
use pedradb_io_uring::IoUringEnv;
use std::ops::{Bound, Deref};
use std::path::Path;

/// rust-rocksdb `WriteOptions` subset. Pedra still `fdatasync`s before Ok
/// (G1); `sync` is accepted so SurrealDB's `set_sync(false)` compiles.
#[derive(Debug, Clone, Default)]
pub struct WriteOptions {
    /// Rocks `WriteOptions.sync`. Ignored: Pedra G1 always syncs.
    pub sync: bool,
}

impl WriteOptions {
    /// Builder: Rocks per-write sync flag (ignored; G1).
    pub fn set_sync(&mut self, v: bool) -> &mut Self {
        self.sync = v;
        self
    }
}

/// rust-rocksdb `OptimisticTransactionOptions` subset.
#[derive(Debug, Clone, Default)]
pub struct OptimisticTransactionOptions {
    /// When true (SurrealDB always sets this), reads are snapshot-pinned.
    /// Pedra OCC always snapshots; the flag is accepted for API shape.
    pub snapshot: bool,
}

impl OptimisticTransactionOptions {
    /// Builder: pin a snapshot at `transaction_opt` (always-on here).
    pub fn set_snapshot(&mut self, v: bool) -> &mut Self {
        self.snapshot = v;
        self
    }
}

/// rust-rocksdb `OptimisticTransactionDB` — `DB` plus `transaction()`.
pub struct OptimisticTransactionDB<E: Env = IoUringEnv> {
    db: DB<E>,
}

impl OptimisticTransactionDB<IoUringEnv> {
    /// Open with only the default CF.
    ///
    /// # Errors
    /// Pedra open errors.
    pub fn open_default(path: impl AsRef<Path>) -> Result<Self> {
        // F192: create if missing — rust-rocksdb `open_default` parity.
        let mut opts = super::Options::new();
        opts.create_if_missing(true);
        Self::open(&opts, path)
    }

    /// Open with explicit options.
    ///
    /// # Errors
    /// Pedra open errors.
    pub fn open(opts: &super::Options, path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            db: DB::open(opts, path)?,
        })
    }

    /// Open with named CFs.
    ///
    /// # Errors
    /// Pedra open errors.
    pub fn open_cf(opts: &super::Options, path: impl AsRef<Path>, cfs: &[&str]) -> Result<Self> {
        Ok(Self {
            db: DB::open_cf(opts, path, cfs)?,
        })
    }

    /// rust-rocksdb `open_cf_descriptors`.
    ///
    /// # Errors
    /// Pedra open errors.
    pub fn open_cf_descriptors(
        opts: &super::Options,
        path: impl AsRef<Path>,
        cfs: impl IntoIterator<Item = super::ColumnFamilyDescriptor>,
    ) -> Result<Self> {
        Ok(Self {
            db: DB::open_cf_descriptors(opts, path, cfs)?,
        })
    }
}

impl<E: Env> OptimisticTransactionDB<E> {
    /// Open with an injected [`Env`].
    ///
    /// # Errors
    /// Pedra open errors.
    pub fn open_cf_with_env(
        opts: &super::Options,
        path: impl AsRef<Path>,
        cfs: &[&str],
        env: E,
    ) -> Result<Self> {
        Ok(Self {
            db: DB::open_cf_with_env(opts, path, cfs, env)?,
        })
    }

    /// Begin an optimistic transaction (default options).
    #[must_use]
    pub fn transaction(&self) -> Transaction<'_, E> {
        self.db.transaction()
    }

    /// Begin with rust-rocksdb option objects. Pedra always snapshots
    /// and always `fdatasync`s; flags are accepted for API shape.
    #[must_use]
    pub fn transaction_opt(
        &self,
        writeopts: &WriteOptions,
        otxn_opts: &OptimisticTransactionOptions,
    ) -> Transaction<'_, E> {
        self.db.transaction_opt(writeopts, otxn_opts)
    }
}

impl<E: Env> Deref for OptimisticTransactionDB<E> {
    type Target = DB<E>;
    fn deref(&self) -> &DB<E> {
        &self.db
    }
}

/// rust-rocksdb `Transaction` over Pedra [`OccTransaction`].
///
/// Methods take `&self` (Rocks FFI is internally mutable). Commit
/// consumes the handle.
pub struct Transaction<'a, E: Env = IoUringEnv> {
    occ: Mutex<OccTransaction<E>>,
    /// Version-GC pin held from begin to drop (F186) — see [`Self::new`].
    pin: pedradb_core::SnapshotPin,
    codec: KeyCodec,
    db: &'a DB<E>,
}

impl<E: Env> Drop for Transaction<'_, E> {
    fn drop(&mut self) {
        self.db.inner.release_snapshot_pin(self.pin);
    }
}

impl<'a, E: Env> Transaction<'a, E> {
    pub(crate) fn new(db: &'a DB<E>) -> Self {
        // F186: pin the version-GC floor for the transaction's lifetime
        // (rust-rocksdb: a txn's snapshot is pinned; `auto_reclaim` must
        // never abort a live txn with `SnapshotTooOld`). Pin BEFORE
        // `begin_occ` so the OCC snapshot is always ≥ the pinned floor.
        let pin = db.inner.pin_snapshot();
        Self {
            occ: Mutex::new(db.inner.begin_occ()),
            pin,
            codec: db.codec.clone(),
            db,
        }
    }

    fn encode(&self, cf: &str, key: &[u8]) -> Vec<u8> {
        self.codec.encode(cf, key)
    }

    fn with_encoded<R>(
        &self,
        cf: &str,
        key: &[u8],
        f: impl FnOnce(&mut OccTransaction<E>, &[u8]) -> R,
    ) -> R {
        self.codec.encode_with(cf, key, |enc| {
            let mut occ = self.occ.lock();
            f(&mut occ, enc)
        })
    }

    /// Point get at the transaction snapshot (own writes first).
    ///
    /// # Errors
    /// Pedra read / snapshot-too-old.
    pub fn get(&self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        self.get_cf_name(DEFAULT_CF, key)
    }

    /// Point get on a named CF.
    ///
    /// # Errors
    /// Unknown CF or Pedra read.
    pub fn get_cf(&self, cf: &ColumnFamily, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        self.get_cf_name(cf.name(), key)
    }

    fn get_cf_name(&self, cf: &str, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        self.with_encoded(cf, key.as_ref(), |occ, enc| {
            occ.get(enc)
                .map(|v| v.map(|b| b.to_vec()))
                .map_err(Error::from)
        })
    }

    /// Stage a put (visible to later gets in this txn).
    ///
    /// # Errors
    /// Transaction already finished.
    pub fn put(&self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        self.put_cf_name(DEFAULT_CF, key, value)
    }

    /// Stage a put on a named CF.
    ///
    /// # Errors
    /// Transaction already finished.
    pub fn put_cf(
        &self,
        cf: &ColumnFamily,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<()> {
        self.put_cf_name(cf.name(), key, value)
    }

    fn put_cf_name(&self, cf: &str, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        self.with_encoded(cf, key.as_ref(), |occ, enc| {
            occ.put(enc, value.as_ref()).map_err(Error::from)
        })
    }

    /// Stage a delete.
    ///
    /// # Errors
    /// Transaction already finished.
    pub fn delete(&self, key: impl AsRef<[u8]>) -> Result<()> {
        self.delete_cf_name(DEFAULT_CF, key)
    }

    /// Stage a delete on a named CF.
    ///
    /// # Errors
    /// Transaction already finished.
    pub fn delete_cf(&self, cf: &ColumnFamily, key: impl AsRef<[u8]>) -> Result<()> {
        self.delete_cf_name(cf.name(), key)
    }

    fn delete_cf_name(&self, cf: &str, key: impl AsRef<[u8]>) -> Result<()> {
        let enc = self.encode(cf, key.as_ref());
        self.occ.lock().delete(enc).map_err(Error::from)
    }

    /// Count live keys in `[start, end)` at the txn snapshot **with the
    /// txn's own staged writes overlaid** (rust-rocksdb `Transaction` reads
    /// see the uncommitted write batch — F179: a staged put must count, a
    /// staged delete must not).
    ///
    /// Untracked OCC read (see module policy): the count is snapshot-pinned
    /// but the scanned keys do not enter the read set.
    ///
    /// # Errors
    /// Snapshot-too-old or Pedra scan.
    pub fn scan_count(
        &self,
        start: impl AsRef<[u8]>,
        end: impl AsRef<[u8]>,
        cap: usize,
    ) -> Result<usize> {
        let (snap, staged) = {
            let g = self.occ.lock();
            (g.snapshot(), g.staged_entries())
        };
        let lo = self.encode(DEFAULT_CF, start.as_ref());
        let hi = self.encode(DEFAULT_CF, end.as_ref());
        self.db
            .inner
            .with_read(|db| {
                let mut n = db.count_in_range(
                    snap,
                    Bound::Included(lo.as_slice()),
                    Bound::Excluded(hi.as_slice()),
                    Some(cap),
                )?;
                for (key, val) in &staged {
                    let k: &[u8] = &key[..];
                    if k < lo.as_slice() || k >= hi.as_slice() {
                        continue;
                    }
                    let at_snap = db
                        .get_at(pedradb_core::db::Snapshot::at(snap), k)?
                        .is_some();
                    match val {
                        Some(_) => {
                            if !at_snap {
                                n += 1;
                            }
                        }
                        None => {
                            if at_snap {
                                n = n.saturating_sub(1);
                            }
                        }
                    }
                }
                Ok::<usize, pedradb_core::error::CoreError>(n.min(cap))
            })
            .map_err(Error::from)
    }

    /// rust-rocksdb `get_opt` (ReadOptions snapshot already pinned at begin).
    ///
    /// # Errors
    /// Pedra read / snapshot-too-old.
    pub fn get_opt(
        &self,
        key: impl AsRef<[u8]>,
        _readopts: &super::ReadOptions,
    ) -> Result<Option<Vec<u8>>> {
        self.get(key)
    }

    /// rust-rocksdb raw iterator at this txn's snapshot (SurrealDB scan).
    ///
    /// Untracked OCC read (see module policy): unlike Rocks
    /// `Transaction::NewIterator`, keys yielded here do not enter the read
    /// set — a later `commit()` does not conflict on them.
    #[must_use]
    pub fn raw_iterator_opt(
        &self,
        ro: super::ReadOptions,
    ) -> super::DBRawIteratorWithThreadMode<'_, Self, E> {
        let seq = self.occ.lock().snapshot();
        super::DBRawIteratorWithThreadMode::open(self.db, seq, &ro)
    }

    /// rust-rocksdb `snapshot()` — sequence pin matching this txn's begin.
    #[must_use]
    pub fn snapshot(&self) -> super::SnapshotWithThreadMode<'_, Self> {
        super::SnapshotWithThreadMode::at(self.occ.lock().snapshot())
    }

    /// SurrealDB versioning hook. No-op (UDT not implemented).
    pub fn set_read_timestamp_for_validation(&self, _ts: u64) {}

    /// SurrealDB versioning hook. No-op (UDT not implemented).
    pub fn set_commit_timestamp(&self, _ts: u64) -> Result<()> {
        Ok(())
    }

    /// Validate OCC and commit (one WAL record; G1 fdatasync).
    ///
    /// # Errors
    /// `Busy` (write conflict), snapshot-too-old, or WAL I/O.
    pub fn commit(mut self) -> Result<()> {
        // Drop (F186 pin release) needs `self` intact; swap in a finished
        // stub instead of a partial move out of a Drop type.
        let stub = self.db.inner.begin_occ();
        let occ = std::mem::replace(&mut self.occ, Mutex::new(stub)).into_inner();
        occ.commit().map_err(|e| match e {
            CoreError::TransactionConflict => Error {
                msg: "Busy: transaction conflict: key changed since snapshot".into(),
                kind: crate::ErrorKind::TransactionConflict,
            },
            other => Error::from(other),
        })
    }

    /// Discard staged writes (rust-rocksdb `rollback`).
    ///
    /// # Errors
    /// Never — signature matches rust-rocksdb.
    pub fn rollback(&self) -> Result<()> {
        // OccTransaction::abort consumes; drop the mutex contents by
        // replacing with a fresh txn so further ops don't see staging.
        let mut g = self.occ.lock();
        let old = std::mem::replace(&mut *g, self.db.inner.begin_occ());
        old.abort();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Options;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pedra-compat-txn-{}-{}",
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn txn_put_get_commit_visible() {
        let dir = tmp("commit");
        let db = OptimisticTransactionDB::open_default(&dir).unwrap();
        let tx = db.transaction();
        tx.put(b"k", b"v").unwrap();
        assert_eq!(tx.get(b"k").unwrap().as_deref(), Some(&b"v"[..]));
        // Not visible outside until commit.
        assert_eq!(db.get(b"k").unwrap(), None);
        tx.commit().unwrap();
        assert_eq!(db.get(b"k").unwrap().as_deref(), Some(&b"v"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn txn_rollback_discards() {
        let dir = tmp("rollback");
        let db = OptimisticTransactionDB::open_default(&dir).unwrap();
        let tx = db.transaction();
        tx.put(b"k", b"v").unwrap();
        tx.rollback().unwrap();
        tx.put(b"other", b"x").unwrap();
        tx.commit().unwrap();
        assert_eq!(db.get(b"k").unwrap(), None);
        assert_eq!(db.get(b"other").unwrap().as_deref(), Some(&b"x"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn txn_write_write_conflict() {
        let dir = tmp("conflict");
        let db = OptimisticTransactionDB::open_default(&dir).unwrap();
        db.put(b"k", b"0").unwrap();
        let a = db.transaction();
        let b = db.transaction();
        a.put(b"k", b"a").unwrap();
        b.put(b"k", b"b").unwrap();
        a.commit().unwrap();
        let err = b.commit().unwrap_err();
        assert!(
            err.to_string().contains("Busy") || err.to_string().contains("conflict"),
            "{err}"
        );
        assert_eq!(db.get(b"k").unwrap().as_deref(), Some(&b"a"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn txn_snapshot_hides_later_writes() {
        let dir = tmp("snap");
        let db = OptimisticTransactionDB::open_default(&dir).unwrap();
        db.put(b"k", b"old").unwrap();
        let tx = db.transaction();
        db.put(b"k", b"new").unwrap();
        assert_eq!(tx.get(b"k").unwrap().as_deref(), Some(&b"old"[..]));
        // Read-only commits validate the read set (occ.rs contract): the
        // concurrent overwrite of the key this tx read must surface as Busy.
        let err = tx.commit().unwrap_err();
        assert!(
            err.to_string().contains("Busy") || err.to_string().contains("conflict"),
            "{err}"
        );
        assert_eq!(db.get(b"k").unwrap().as_deref(), Some(&b"new"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn txn_scan_count_at_snapshot() {
        let dir = tmp("scan");
        let db = OptimisticTransactionDB::open_default(&dir).unwrap();
        for i in 0..10u8 {
            db.put([b'k', i], [i]).unwrap();
        }
        let tx = db.transaction();
        db.put(b"k\x0a", b"later").unwrap();
        let n = tx.scan_count(b"k", b"k\x0a", 25).unwrap();
        assert_eq!(n, 10, "snapshot must not see the later key");
        tx.commit().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn surrealdb_open_scan_shutdown_shape() {
        use crate::{
            properties, BottommostLevelCompaction, ColumnFamilyDescriptor, CompactOptions,
            DBCompressionType, FlushOptions, IteratorMode, LogLevel, Options, ReadOptions,
            SliceTransform, WaitForCompactOptions,
        };
        let dir = tmp("shape");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_use_fsync(false);
        opts.set_log_level(LogLevel::Warn);
        opts.set_bottommost_compression_type(DBCompressionType::Zstd);
        opts.set_prefix_extractor(SliceTransform::create("t", |k| k, None));
        let db = OptimisticTransactionDB::open_cf_descriptors(
            &opts,
            &dir,
            [ColumnFamilyDescriptor::new("default", Options::new())],
        )
        .unwrap();
        db.put(b"a", b"1").unwrap();
        db.put(b"b", b"2").unwrap();
        let mut ro = ReadOptions::default();
        ro.set_iterate_lower_bound(b"a".to_vec());
        ro.set_iterate_upper_bound(b"c".to_vec());
        let mut it = db.raw_iterator_opt(ro);
        it.seek(b"a");
        assert_eq!(it.key(), Some(&b"a"[..]));
        it.next();
        assert_eq!(it.key(), Some(&b"b"[..]));
        it.status().unwrap();
        let _ = db.property_int_value(properties::ESTIMATE_NUM_KEYS);
        let mut fo = FlushOptions::default();
        fo.set_wait(true);
        db.flush_wal(true).unwrap();
        db.flush_opt(&fo).unwrap();
        let mut co = CompactOptions::default();
        co.set_bottommost_level_compaction(BottommostLevelCompaction::Force);
        db.compact_range_opt::<&[u8], &[u8]>(None, None, &co);
        db.wait_for_compact(&WaitForCompactOptions::default())
            .unwrap();
        db.cancel_all_background_work(true);
        let _ = IteratorMode::Start;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn surrealdb_option_shape() {
        // Exact calls SurrealDB kv-rocksdb makes at begin.
        let dir = tmp("opts");
        let mut to = OptimisticTransactionOptions::default();
        to.set_snapshot(true);
        let mut wo = WriteOptions::default();
        wo.set_sync(false);
        let db = OptimisticTransactionDB::open(&Options::new(), &dir).unwrap();
        let tx = db.transaction_opt(&wo, &to);
        tx.put(b"s/1", b"doc").unwrap();
        tx.commit().unwrap();
        assert_eq!(db.get(b"s/1").unwrap().as_deref(), Some(&b"doc"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
