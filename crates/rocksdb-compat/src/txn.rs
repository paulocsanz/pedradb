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
//! (F168). rust-rocksdb `Transaction::NewIterator` tracks every key the
//! iterator yields; `raw_iterator_opt` does the same (issue #3). `scan_count`
//! still counts without recording individual keys.
//!
//! Compile-shape (RFC-0043 P2.4): `open_cf_descriptors`, `ReadOptions`,
//! `raw_iterator_opt`, `property_int_value`, `flush_opt`/`flush_wal`,
//! `compact_range_opt`. Prefix extractor / UDT comparator are accepted
//! no-ops. Versioned CF timestamps remain a gap.

use super::locktab::{LockErr, LockTable};
use super::{
    ColumnFamily, DBRawIteratorWithThreadMode, Error, ErrorKind, KeyCodec, Result, DB, DEFAULT_CF,
};
use bytes::Bytes;
use parking_lot::Mutex;
use pedradb_core::{CoreError, Env, OccTransaction};
use pedradb_io_uring::IoUringEnv;
use std::collections::BTreeSet;
use std::ops::{Bound, Deref};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// rust-rocksdb `WriteOptions` subset. Durability follows the DB
/// [`super::Options::sync`] until [`Self::set_sync`] is called — then the
/// kernel per-call flag is honored.
#[derive(Debug, Clone, Default)]
pub struct WriteOptions {
    /// Rocks `WriteOptions.sync`. Meaningful after [`Self::set_sync`].
    pub sync: bool,
    /// `true` once [`Self::set_sync`] ran — default keeps the DB flag.
    sync_explicit: bool,
}

impl WriteOptions {
    /// Builder: Rocks per-write sync flag. Overrides [`super::Options::sync`].
    pub fn set_sync(&mut self, v: bool) -> &mut Self {
        self.sync = v;
        self.sync_explicit = true;
        self
    }

    /// Kernel durability for this write. `None` = open-time DB default.
    #[must_use]
    pub(crate) fn kernel_durability(&self) -> pedradb_core::WriteOptions {
        if self.sync_explicit {
            pedradb_core::WriteOptions {
                sync: Some(self.sync),
            }
        } else {
            pedradb_core::WriteOptions::default()
        }
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

/// rust-rocksdb `TransactionOptions` (pessimistic).
#[derive(Debug, Clone)]
pub struct TransactionOptions {
    snapshot: bool,
    deadlock_detect: bool,
    /// ms; 0 = no wait; negative = use [`TransactionDBOptions`] default.
    lock_timeout: i64,
    expiration: i64,
    deadlock_detect_depth: i64,
    max_write_batch_size: usize,
}

impl Default for TransactionOptions {
    fn default() -> Self {
        Self {
            snapshot: false,
            deadlock_detect: false,
            lock_timeout: -1,
            expiration: -1,
            deadlock_detect_depth: 50,
            max_write_batch_size: 0,
        }
    }
}

impl TransactionOptions {
    /// rust-rocksdb `TransactionOptions::new`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Skip 2PC prepare (Pedra is 1PC; accepted).
    pub fn set_skip_prepare(&mut self, _skip_prepare: bool) {}

    /// Pin a snapshot at begin (stricter isolation).
    pub fn set_snapshot(&mut self, snapshot: bool) {
        self.snapshot = snapshot;
    }

    /// Check wait-for cycle before blocking. Default false.
    pub fn set_deadlock_detect(&mut self, deadlock_detect: bool) {
        self.deadlock_detect = deadlock_detect;
    }

    /// Lock wait in ms. `0` = try-once; negative = DB default (1s).
    pub fn set_lock_timeout(&mut self, lock_timeout: i64) {
        self.lock_timeout = lock_timeout;
    }

    /// Txn wall-clock expiration (ms). Stored; not enforced yet.
    pub fn set_expiration(&mut self, expiration: i64) {
        self.expiration = expiration;
    }

    /// Deadlock BFS depth. Stored.
    pub fn set_deadlock_detect_depth(&mut self, depth: i64) {
        self.deadlock_detect_depth = depth;
    }

    /// Write-batch byte cap. `0` = unlimited. Stored.
    pub fn set_max_write_batch_size(&mut self, size: usize) {
        self.max_write_batch_size = size;
    }
}

/// rust-rocksdb `TransactionDBOptions`.
#[derive(Debug, Clone)]
pub struct TransactionDBOptions {
    default_lock_timeout: i64,
    txn_lock_timeout: i64,
    max_num_locks: i64,
    num_stripes: usize,
}

impl Default for TransactionDBOptions {
    fn default() -> Self {
        Self {
            default_lock_timeout: 1000,
            txn_lock_timeout: 1000,
            max_num_locks: -1,
            num_stripes: 16,
        }
    }
}

impl TransactionDBOptions {
    /// rust-rocksdb `TransactionDBOptions::new`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Timeout for `TransactionDB::put` outside a txn (ms). Default 1000.
    pub fn set_default_lock_timeout(&mut self, default_lock_timeout: i64) {
        self.default_lock_timeout = default_lock_timeout;
    }

    /// Default txn lock wait (ms). Default 1000.
    pub fn set_txn_lock_timeout(&mut self, txn_lock_timeout: i64) {
        self.txn_lock_timeout = txn_lock_timeout;
    }

    /// Max keys locked per CF. Negative = unlimited.
    pub fn set_max_num_locks(&mut self, max_num_locks: i64) {
        self.max_num_locks = max_num_locks;
    }

    /// Lock-table stripes. Stored; table is one mutex.
    pub fn set_num_stripes(&mut self, num_stripes: usize) {
        self.num_stripes = num_stripes;
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

/// rust-rocksdb `TransactionDB` — pessimistic 2PL on the same Pedra KV.
///
/// Puts / `get_for_update` take an exclusive per-key lock (wait or
/// `Busy`/`TimedOut`). Commit is still one WAL group. Not 2PC: `prepare`
/// is 1PC Ok; `prepared_transactions` is empty.
pub struct TransactionDB<E: Env = IoUringEnv> {
    db: DB<E>,
    locks: Arc<LockTable>,
    txn_lock_timeout: Duration,
    default_lock_timeout: Duration,
}

impl TransactionDB<IoUringEnv> {
    /// rust-rocksdb `TransactionDB::open_default`.
    ///
    /// # Errors
    /// Pedra open errors.
    pub fn open_default(path: impl AsRef<Path>) -> Result<Self> {
        let mut opts = super::Options::new();
        opts.create_if_missing(true);
        Self::open(&opts, &TransactionDBOptions::default(), path)
    }

    /// rust-rocksdb `TransactionDB::open`.
    ///
    /// # Errors
    /// Pedra open errors.
    pub fn open(
        opts: &super::Options,
        txn_db_opts: &TransactionDBOptions,
        path: impl AsRef<Path>,
    ) -> Result<Self> {
        Self::open_cf(opts, txn_db_opts, path, None::<&str>)
    }

    /// rust-rocksdb `TransactionDB::open_cf`.
    ///
    /// # Errors
    /// Pedra open errors.
    pub fn open_cf(
        opts: &super::Options,
        txn_db_opts: &TransactionDBOptions,
        path: impl AsRef<Path>,
        cfs: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<Self> {
        let names: Vec<String> = cfs.into_iter().map(|s| s.as_ref().to_string()).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        Ok(Self {
            db: DB::open_cf(opts, path, &refs)?,
            locks: Arc::new(LockTable::new()),
            txn_lock_timeout: ms_to_dur(txn_db_opts.txn_lock_timeout),
            default_lock_timeout: ms_to_dur(txn_db_opts.default_lock_timeout),
        })
    }

    /// rust-rocksdb `TransactionDB::open_cf_descriptors`.
    ///
    /// # Errors
    /// Pedra open errors.
    pub fn open_cf_descriptors(
        opts: &super::Options,
        txn_db_opts: &TransactionDBOptions,
        path: impl AsRef<Path>,
        cfs: impl IntoIterator<Item = super::ColumnFamilyDescriptor>,
    ) -> Result<Self> {
        Ok(Self {
            db: DB::open_cf_descriptors(opts, path, cfs)?,
            locks: Arc::new(LockTable::new()),
            txn_lock_timeout: ms_to_dur(txn_db_opts.txn_lock_timeout),
            default_lock_timeout: ms_to_dur(txn_db_opts.default_lock_timeout),
        })
    }
}

fn ms_to_dur(ms: i64) -> Duration {
    if ms < 0 {
        Duration::from_secs(u32::MAX as u64)
    } else if ms == 0 {
        Duration::ZERO
    } else {
        Duration::from_millis(ms as u64)
    }
}

fn lock_error(e: LockErr) -> Error {
    match e {
        LockErr::Deadlock => Error {
            msg: "Busy: deadlock".into(),
            kind: ErrorKind::Busy,
        },
        LockErr::TimedOut => Error {
            msg: "TimedOut: lock wait".into(),
            kind: ErrorKind::TimedOut,
        },
    }
}

impl<E: Env> TransactionDB<E> {
    /// Begin a pessimistic transaction (default options).
    #[must_use]
    pub fn transaction(&self) -> Transaction<'_, E> {
        self.transaction_opt(&WriteOptions::default(), &TransactionOptions::default())
    }

    /// Begin with rust-rocksdb option objects.
    #[must_use]
    pub fn transaction_opt(
        &self,
        writeopts: &WriteOptions,
        txn_opts: &TransactionOptions,
    ) -> Transaction<'_, E> {
        let timeout = if txn_opts.lock_timeout < 0 {
            self.txn_lock_timeout
        } else {
            ms_to_dur(txn_opts.lock_timeout)
        };
        let mut t = Transaction::new_pessimistic(
            &self.db,
            Arc::clone(&self.locks),
            timeout,
            txn_opts.deadlock_detect,
        );
        t.durability = writeopts.kernel_durability();
        t
    }

    /// 2PC recovered txns. Pedra is 1PC — always empty.
    #[must_use]
    pub fn prepared_transactions(&self) -> Vec<Transaction<'_, E>> {
        Vec::new()
    }

    /// Direct put: exclusive-lock the key, write, unlock (Rocks non-txn write).
    ///
    /// # Errors
    /// Lock wait or Pedra write.
    pub fn put(&self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        self.put_named(DEFAULT_CF, key.as_ref(), value.as_ref())
    }

    /// Direct put on a named CF.
    ///
    /// # Errors
    /// Unknown CF, lock wait, or Pedra write.
    pub fn put_cf(
        &self,
        cf: &ColumnFamily,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<()> {
        self.put_named(cf.name(), key.as_ref(), value.as_ref())
    }

    fn put_named(&self, cf: &str, key: &[u8], value: &[u8]) -> Result<()> {
        let enc = Bytes::from(self.db.codec.encode(cf, key));
        let id = self.locks.alloc_id();
        self.locks
            .lock(enc.clone(), id, self.default_lock_timeout, false)
            .map_err(lock_error)?;
        let r = if cf == DEFAULT_CF {
            self.db.put(key, value)
        } else {
            match self.db.cf_handle(cf) {
                Some(h) => self.db.put_cf(&h, key, value),
                None => Err(Error {
                    msg: format!("unknown column family {cf}"),
                    kind: ErrorKind::InvalidArgument,
                }),
            }
        };
        self.locks.unlock_all(&[enc], id);
        r
    }

    /// Direct delete with the same lock as [`Self::put`].
    ///
    /// # Errors
    /// Lock wait or Pedra write.
    pub fn delete(&self, key: impl AsRef<[u8]>) -> Result<()> {
        let enc = Bytes::from(self.db.codec.encode(DEFAULT_CF, key.as_ref()));
        let id = self.locks.alloc_id();
        self.locks
            .lock(enc.clone(), id, self.default_lock_timeout, false)
            .map_err(lock_error)?;
        let r = self.db.delete(key);
        self.locks.unlock_all(&[enc], id);
        r
    }
}

impl<E: Env> Deref for TransactionDB<E> {
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
    /// `Some` = pessimistic 2PL ([`TransactionDB`]). `None` = OCC.
    pess: Option<Pess>,
    /// Keys yielded by [`Self::raw_iterator_opt`] (issue #3). Shared so the
    /// iterator can outlive a borrow of `self` until `commit` consumes us.
    scan_reads: Arc<Mutex<BTreeSet<Bytes>>>,
    durability: pedradb_core::WriteOptions,
}

struct Pess {
    table: Arc<LockTable>,
    id: u64,
    held: Mutex<Vec<Bytes>>,
    timeout: Duration,
    detect: bool,
}

impl<E: Env> Drop for Transaction<'_, E> {
    fn drop(&mut self) {
        if let Some(p) = &self.pess {
            let held = std::mem::take(&mut *p.held.lock());
            p.table.unlock_all(&held, p.id);
        }
        self.db.inner.release_snapshot_pin(self.pin);
    }
}

impl<'a, E: Env> Transaction<'a, E> {
    pub(crate) fn new(db: &'a DB<E>) -> Self {
        Self::new_with_durability(db, pedradb_core::WriteOptions::default())
    }

    pub(crate) fn new_with_durability(
        db: &'a DB<E>,
        durability: pedradb_core::WriteOptions,
    ) -> Self {
        let pin = db.inner.pin_snapshot();
        Self {
            occ: Mutex::new(db.inner.begin_occ()),
            pin,
            codec: db.codec.clone(),
            db,
            pess: None,
            scan_reads: Arc::new(Mutex::new(BTreeSet::new())),
            durability,
        }
    }

    pub(crate) fn new_pessimistic(
        db: &'a DB<E>,
        table: Arc<LockTable>,
        timeout: Duration,
        detect: bool,
    ) -> Self {
        let pin = db.inner.pin_snapshot();
        let id = table.alloc_id();
        Self {
            occ: Mutex::new(db.inner.begin_occ()),
            pin,
            codec: db.codec.clone(),
            db,
            pess: Some(Pess {
                table,
                id,
                held: Mutex::new(Vec::new()),
                timeout,
                detect,
            }),
            scan_reads: Arc::new(Mutex::new(BTreeSet::new())),
            durability: pedradb_core::WriteOptions::default(),
        }
    }

    fn lock_enc(&self, enc: Bytes) -> Result<()> {
        let Some(p) = self.pess.as_ref() else {
            return Ok(());
        };
        p.table
            .lock(enc.clone(), p.id, p.timeout, p.detect)
            .map_err(lock_error)?;
        p.held.lock().push(enc);
        Ok(())
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
        let enc = Bytes::from(self.encode(cf, key.as_ref()));
        self.lock_enc(enc.clone())?;
        self.occ
            .lock()
            .put(enc.as_ref(), value.as_ref())
            .map_err(Error::from)
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
        let enc = Bytes::from(self.encode(cf, key.as_ref()));
        self.lock_enc(enc.clone())?;
        self.occ.lock().delete(enc).map_err(Error::from)
    }

    /// rust-rocksdb `get_for_update` — exclusive lock then snapshot get.
    /// `exclusive` is accepted; Pedra always takes exclusive (stricter).
    ///
    /// # Errors
    /// Lock wait, deadlock, or Pedra read.
    pub fn get_for_update(
        &self,
        key: impl AsRef<[u8]>,
        exclusive: bool,
    ) -> Result<Option<Vec<u8>>> {
        let _ = exclusive;
        self.get_for_update_cf_name(DEFAULT_CF, key.as_ref())
    }

    /// rust-rocksdb `get_for_update_cf`.
    ///
    /// # Errors
    /// Lock wait, deadlock, unknown CF, or Pedra read.
    pub fn get_for_update_cf(
        &self,
        cf: &ColumnFamily,
        key: impl AsRef<[u8]>,
        exclusive: bool,
    ) -> Result<Option<Vec<u8>>> {
        let _ = exclusive;
        self.get_for_update_cf_name(cf.name(), key.as_ref())
    }

    fn get_for_update_cf_name(&self, cf: &str, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let enc = Bytes::from(self.encode(cf, key));
        self.lock_enc(enc)?;
        self.get_cf_name(cf, key)
    }

    /// Count live keys in `[start, end)` at the txn snapshot **with the
    /// txn's own staged writes overlaid** (rust-rocksdb `Transaction` reads
    /// see the uncommitted write batch — F179: a staged put must count, a
    /// staged delete must not).
    ///
    /// Snapshot-pinned count. Individual keys in the range are not added
    /// to the OCC read set (unlike [`Self::raw_iterator_opt`]).
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
    /// Rocks `Transaction` iterators see the transaction's own uncommitted
    /// writes (write-batch overlay, WriteBatchWithIndex semantics): a
    /// staged put must be returned by a mid-txn scan and a staged delete
    /// must hide the committed key (F183). The merged iterator overlays
    /// `staged_entries` on the snapshot read.
    ///
    /// Keys the iterator yields enter the OCC read set (issue #3 /
    /// rust-rocksdb `Transaction::NewIterator`).
    #[must_use]
    pub fn raw_iterator_opt(&self, ro: super::ReadOptions) -> TxnRawIterator<'_, Self, E> {
        let (seq, staged) = {
            let g = self.occ.lock();
            (g.snapshot(), g.staged_entries())
        };
        let staged = staged
            .into_iter()
            .map(|(k, v)| (k.to_vec(), v.map(|b| b.to_vec())))
            .collect();
        TxnRawIterator::new(self.db, seq, staged, ro, Arc::clone(&self.scan_reads))
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

    /// Validate OCC and commit (one WAL record; durability = DB `Options::sync`).
    ///
    /// # Errors
    /// `Busy` (write conflict), snapshot-too-old, or WAL I/O.
    pub fn commit(mut self) -> Result<()> {
        // Drop (F186 pin release) needs `self` intact; swap in a finished
        // stub instead of a partial move out of a Drop type.
        let stub = self.db.inner.begin_occ();
        let mut occ = std::mem::replace(&mut self.occ, Mutex::new(stub)).into_inner();
        {
            let extra = self.scan_reads.lock();
            for k in extra.iter() {
                occ.observe(k);
            }
        }
        occ.commit_with(self.durability).map_err(|e| match e {
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
        if let Some(p) = &self.pess {
            let held = std::mem::take(&mut *p.held.lock());
            p.table.unlock_all(&held, p.id);
        }
        Ok(())
    }
}

/// Merged raw iterator for [`Transaction`]: the txn's staged writes
/// (last-write-wins per key, sorted) overlaid on the snapshot read — the
/// rust-rocksdb `Transaction` iterator contract (F183). Forward operations
/// (`seek`, `seek_to_first`, `next`) are lazy merge walks; reverse
/// operations materialize the merged view once (SurrealDB scans only walk
/// forward; the reverse path is correctness, not speed).
pub struct TxnRawIterator<'a, D, E: Env = IoUringEnv> {
    db: DBRawIteratorWithThreadMode<'a, D, E>,
    /// Sorted, deduped staged entries (`None` = staged delete).
    staged: Vec<(Vec<u8>, Option<Vec<u8>>)>,
    idx: usize,
    /// Current merged head (forward path): staged rows are an index into
    /// `staged`, db rows delegate to the db walk's current position — zero
    /// copies per row (F222; the old head cloned key+value for every row
    /// into an owned `cur`).
    cur: CurRow,
    lower: Option<Vec<u8>>,
    upper: Option<Vec<u8>>,
    /// Materialized merged view (reverse-path only).
    mat: Option<Vec<(Vec<u8>, Vec<u8>)>>,
    mat_at: usize,
    /// OCC read-set sidecar (issue #3). `None` only if constructed without.
    scan_reads: Arc<Mutex<BTreeSet<Bytes>>>,
    /// False while [`Self::materialize`] walks internally (don't record
    /// keys the caller never positioned on).
    tracking: bool,
}

/// Source of the current merged head. A staged head is an index into
/// `TxnRawIterator::staged` (immutable after `new`); a db head is the db
/// walk's current row, borrowed on read — the walk only advances on `next`,
/// so the position is stable between `key()`/`value()` calls.
enum CurRow {
    Db,
    Staged(usize),
    End,
}

impl<'a, D, E: Env> TxnRawIterator<'a, D, E> {
    pub(crate) fn new(
        db: &'a super::DB<E>,
        seq: pedradb_core::SequenceNumber,
        mut staged: Vec<(Vec<u8>, Option<Vec<u8>>)>,
        ro: super::ReadOptions,
        scan_reads: Arc<Mutex<BTreeSet<Bytes>>>,
    ) -> Self {
        staged.sort_by(|a, b| a.0.cmp(&b.0));
        staged.dedup_by(|a, b| a.0 == b.0);
        let db = super::DBRawIteratorWithThreadMode::open(db, seq, &ro);
        Self {
            db,
            staged,
            idx: 0,
            cur: CurRow::End,
            lower: ro.lower,
            upper: ro.upper,
            mat: None,
            mat_at: 0,
            scan_reads,
            tracking: true,
        }
    }

    fn track_if_valid(&self) {
        if !self.tracking {
            return;
        }
        let Some(k) = self.key() else {
            return;
        };
        self.scan_reads.lock().insert(Bytes::copy_from_slice(k));
    }

    fn in_window(&self, k: &[u8]) -> bool {
        if let Some(lo) = &self.lower {
            if k < lo.as_slice() {
                return false;
            }
        }
        if let Some(hi) = &self.upper {
            if k >= hi.as_slice() {
                return false;
            }
        }
        true
    }

    /// Merged head: smallest visible key from db walk + staged overlay.
    /// Returns the head's source; db-row bytes land in the reusable
    /// buffers, staged rows are an index — no per-row allocation (F222).
    fn head(&mut self) -> CurRow {
        loop {
            let db_k = self.db.key();
            let st = self.staged.get(self.idx);
            match (db_k, st) {
                (None, None) => return CurRow::End,
                (Some(dk), None) => {
                    if !self.in_window(dk) {
                        return CurRow::End;
                    }
                    return CurRow::Db;
                }
                (None, Some((sk, sv))) => {
                    let idx = self.idx;
                    self.idx += 1;
                    if !self.in_window(sk) {
                        continue;
                    }
                    if sv.is_some() {
                        return CurRow::Staged(idx);
                    }
                }
                (Some(dk), Some((sk, sv))) => {
                    if sk.as_slice() < dk {
                        let idx = self.idx;
                        self.idx += 1;
                        if sv.is_some() && self.in_window(sk) {
                            return CurRow::Staged(idx);
                        }
                        continue;
                    }
                    if sk.as_slice() > dk {
                        if !self.in_window(dk) {
                            return CurRow::End;
                        }
                        return CurRow::Db;
                    }
                    // Equal: staged shadows db (put or delete).
                    let idx = self.idx;
                    self.idx += 1;
                    self.db.next();
                    if sv.is_some() && self.in_window(sk) {
                        return CurRow::Staged(idx);
                    }
                }
            }
        }
    }

    /// rust-rocksdb: iterator is usable.
    #[must_use]
    pub fn valid(&self) -> bool {
        if let Some(m) = &self.mat {
            return self.mat_at < m.len();
        }
        !matches!(self.cur, CurRow::End)
    }

    /// Current key.
    #[must_use]
    pub fn key(&self) -> Option<&[u8]> {
        if let Some(m) = &self.mat {
            return m.get(self.mat_at).map(|(k, _)| k.as_slice());
        }
        match &self.cur {
            CurRow::Db => self.db.key(),
            CurRow::Staged(i) => Some(self.staged[*i].0.as_slice()),
            CurRow::End => None,
        }
    }

    /// Current value.
    #[must_use]
    pub fn value(&self) -> Option<&[u8]> {
        if let Some(m) = &self.mat {
            return m.get(self.mat_at).map(|(_, v)| v.as_slice());
        }
        match &self.cur {
            CurRow::Db => self.db.value(),
            CurRow::Staged(i) => self.staged[*i].1.as_deref(),
            CurRow::End => None,
        }
    }

    /// Seek ≥ `key`.
    pub fn seek<K: AsRef<[u8]>>(&mut self, key: K) {
        let k = key.as_ref();
        self.mat = None;
        self.db.seek(k);
        self.idx = self.staged.partition_point(|(sk, _)| sk.as_slice() < k);
        self.advance_head();
        self.track_if_valid();
    }

    /// Seek first key.
    pub fn seek_to_first(&mut self) {
        self.mat = None;
        self.db.seek_to_first();
        self.idx = 0;
        self.advance_head();
        self.track_if_valid();
    }

    /// Next visible key (ascending).
    pub fn next(&mut self) {
        if let Some(m) = &mut self.mat {
            if self.mat_at < m.len() {
                self.mat_at += 1;
            }
            self.track_if_valid();
            return;
        }
        match std::mem::replace(&mut self.cur, CurRow::End) {
            CurRow::Db => {
                self.db.next();
            }
            // Staged head: its index already advanced inside `head`, and an
            // equal db key (shadowed put) was consumed there too.
            CurRow::Staged(_) | CurRow::End => {}
        }
        self.advance_head();
        self.track_if_valid();
    }

    fn advance_head(&mut self) {
        self.cur = self.head();
    }

    /// Last error (Pedra fails closed on open).
    pub fn status(&self) -> Result<()> {
        self.db.status()
    }

    fn materialize(&mut self) {
        if self.mat.is_some() {
            return;
        }
        self.tracking = false;
        // Full merged forward walk (reverse path: correctness over speed).
        let mut out = Vec::new();
        let saved_cur = std::mem::replace(&mut self.cur, CurRow::End);
        let saved_idx = self.idx;
        self.seek_to_first();
        while !matches!(self.cur, CurRow::End) {
            if let (Some(k), Some(v)) = (self.key(), self.value()) {
                out.push((k.to_vec(), v.to_vec()));
            }
            self.next();
        }
        self.cur = saved_cur;
        self.idx = saved_idx;
        self.mat = Some(out);
        self.mat_at = 0;
        self.tracking = true;
    }

    /// Seek last key (reverse path — materialized).
    pub fn seek_to_last(&mut self) {
        self.materialize();
        if let Some(m) = &self.mat {
            self.mat_at = m.len().saturating_sub(1);
        }
        self.track_if_valid();
    }

    /// Previous key (reverse path — materialized).
    pub fn prev(&mut self) {
        self.materialize();
        if self.mat_at > 0 {
            self.mat_at -= 1;
        } else {
            self.mat_at = usize::MAX; // exhausted
        }
        self.track_if_valid();
    }

    /// Seek ≤ `key` (reverse path — materialized).
    pub fn seek_for_prev<K: AsRef<[u8]>>(&mut self, key: K) {
        self.materialize();
        let k = key.as_ref();
        if let Some(m) = &self.mat {
            self.mat_at = m
                .partition_point(|(mk, _)| mk.as_slice() <= k)
                .saturating_sub(1);
        }
        self.track_if_valid();
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
    fn txn_iterator_conflict_on_scanned_key() {
        let dir = tmp("iter-occ");
        let db = OptimisticTransactionDB::open_default(&dir).unwrap();
        db.put(b"k", b"v1").unwrap();
        let tx = db.transaction();
        let mut it = tx.raw_iterator_opt(crate::ReadOptions::default());
        it.seek_to_first();
        assert_eq!(it.key(), Some(b"k".as_ref()));
        db.put(b"k", b"v2").unwrap();
        let err = tx.commit().unwrap_err();
        assert!(
            err.to_string().contains("Busy") || err.to_string().contains("conflict"),
            "{err}"
        );
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

    #[test]
    fn transaction_db_put_commit_get() {
        let dir = tmp("txn-db");
        let db = TransactionDB::open_default(&dir).unwrap();
        let tx = db.transaction();
        tx.put(b"k", b"v").unwrap();
        assert_eq!(tx.get(b"k").unwrap().as_deref(), Some(&b"v"[..]));
        tx.commit().unwrap();
        assert_eq!(db.get(b"k").unwrap().as_deref(), Some(&b"v"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transaction_db_second_writer_times_out() {
        let dir = tmp("txn-lock");
        let db = TransactionDB::open_default(&dir).unwrap();
        let mut to = TransactionOptions::new();
        to.set_lock_timeout(0);
        let a = db.transaction();
        a.put(b"k", b"1").unwrap();
        let b = db.transaction_opt(&WriteOptions::default(), &to);
        let err = b.put(b"k", b"2").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::TimedOut);
        a.commit().unwrap();
        let c = db.transaction_opt(&WriteOptions::default(), &to);
        c.put(b"k", b"3").unwrap();
        c.commit().unwrap();
        assert_eq!(db.get(b"k").unwrap().as_deref(), Some(&b"3"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transaction_db_rollback_releases_lock() {
        let dir = tmp("txn-rb");
        let db = TransactionDB::open_default(&dir).unwrap();
        let a = db.transaction();
        a.put(b"k", b"1").unwrap();
        a.rollback().unwrap();
        drop(a);
        let b = db.transaction();
        b.put(b"k", b"2").unwrap();
        b.commit().unwrap();
        assert_eq!(db.get(b"k").unwrap().as_deref(), Some(&b"2"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transaction_db_get_for_update_locks() {
        let dir = tmp("txn-gfu");
        let db = TransactionDB::open_default(&dir).unwrap();
        db.put(b"k", b"v").unwrap();
        let a = db.transaction();
        assert_eq!(
            a.get_for_update(b"k", true).unwrap().as_deref(),
            Some(&b"v"[..])
        );
        let mut to = TransactionOptions::new();
        to.set_lock_timeout(0);
        let b = db.transaction_opt(&WriteOptions::default(), &to);
        assert_eq!(b.put(b"k", b"x").unwrap_err().kind(), ErrorKind::TimedOut);
        drop(a);
        b.put(b"k", b"x").unwrap();
        b.commit().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_and_sst_file_manager_roundtrip() {
        let mut env = crate::Env::new().unwrap();
        env.set_background_threads(4);
        env.set_high_priority_background_threads(2);
        env.join_all_threads();
        assert_eq!(env.background_threads(), 4);
        let mgr = crate::SstFileManager::new(&env).unwrap();
        mgr.set_max_allowed_space_usage(1 << 30);
        mgr.set_delete_rate_bytes_per_second(1024);
        assert!(!mgr.is_max_allowed_space_reached());
        let mut opts = Options::new();
        opts.set_env(&env);
        opts.set_sst_file_manager(&mgr);
        opts.create_if_missing(true);
        let dir = tmp("env-sst");
        let _db = TransactionDB::open(&opts, &TransactionDBOptions::new(), &dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
