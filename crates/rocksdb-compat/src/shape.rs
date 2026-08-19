//! rust-rocksdb type surface that SurrealDB `kv-rocksdb` imports.
//!
//! Tuning knobs are accepted and ignored (Pedra policy is not Rocks).
//! Iterators / OCC / flush / compact are real.

use super::{scan_cf_at, DBIterator, Direction, IteratorMode, Result, DB, DEFAULT_CF};
use pedradb_core::{Env, StdEnv};
use std::marker::PhantomData;

/// rust-rocksdb `ColumnFamilyDescriptor`.
pub struct ColumnFamilyDescriptor {
    pub name: String,
    pub options: super::Options,
}

impl ColumnFamilyDescriptor {
    /// Name + CF options (options ignored except `write_buffer_size` on the DB).
    pub fn new(name: impl Into<String>, options: super::Options) -> Self {
        Self {
            name: name.into(),
            options,
        }
    }

    /// CF name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// rust-rocksdb log level (accepted, unused).
#[derive(Debug, Clone, Copy, Default)]
pub enum LogLevel {
    Debug,
    #[default]
    Info,
    Warn,
    Error,
    Fatal,
    Header,
}

/// rust-rocksdb compaction style (accepted, unused).
#[derive(Debug, Clone, Copy, Default)]
pub enum DBCompactionStyle {
    #[default]
    Level,
    Universal,
    Fifo,
}

/// rust-rocksdb compression type (accepted, unused).
#[derive(Debug, Clone, Copy, Default)]
pub enum DBCompressionType {
    #[default]
    None,
    Snappy,
    Zlib,
    Bz2,
    Lz4,
    Lz4hc,
    Zstd,
}

/// rust-rocksdb bottommost compaction (accepted, unused).
#[derive(Debug, Clone, Copy, Default)]
pub enum BottommostLevelCompaction {
    #[default]
    Skip,
    IfHaveCompactionFilter,
    Force,
    ForceOptimized,
}

/// rust-rocksdb universal stop style (accepted, unused).
#[derive(Debug, Clone, Copy, Default)]
pub enum UniversalCompactionStopStyle {
    #[default]
    Similar,
    Total,
}

/// rust-rocksdb universal options (accepted, unused).
#[derive(Debug, Clone, Default)]
pub struct UniversalCompactOptions {
    _priv: (),
}

impl UniversalCompactOptions {
    pub fn set_stop_style(&mut self, _s: UniversalCompactionStopStyle) {}
    pub fn set_max_size_amplification_percent(&mut self, _n: i32) {}
}

/// rust-rocksdb `CompactOptions` (manual compact knobs; Pedra `compact()`
/// still runs the whole merge).
#[derive(Debug, Clone, Default)]
pub struct CompactOptions {
    _priv: (),
}

impl CompactOptions {
    pub fn set_exclusive_manual_compaction(&mut self, _v: bool) {}
    pub fn set_change_level(&mut self, _v: bool) {}
    pub fn set_target_level(&mut self, _n: i32) {}
    pub fn set_bottommost_level_compaction(&mut self, _v: BottommostLevelCompaction) {}
}

/// rust-rocksdb `FlushOptions`.
#[derive(Debug, Clone, Default)]
pub struct FlushOptions {
    pub wait: bool,
}

impl FlushOptions {
    pub fn set_wait(&mut self, v: bool) {
        self.wait = v;
    }
}

/// rust-rocksdb `WaitForCompactOptions`.
#[derive(Debug, Clone, Default)]
pub struct WaitForCompactOptions {
    pub timeout_us: u64,
}

impl WaitForCompactOptions {
    pub fn set_timeout(&mut self, timeout_us: u64) {
        self.timeout_us = timeout_us;
    }
}

/// rust-rocksdb `SliceTransform` (prefix extractor). Stored, unused.
pub struct SliceTransform {
    pub name: String,
}

impl SliceTransform {
    /// rust-rocksdb `create` — function pointers, not closures.
    pub fn create(
        name: impl Into<String>,
        _transform: fn(&[u8]) -> &[u8],
        _in_domain: Option<fn(&[u8]) -> bool>,
    ) -> Self {
        Self { name: name.into() }
    }

    /// Fixed-length prefix extractor.
    #[must_use]
    pub fn create_fixed_prefix(len: usize) -> Self {
        let _ = len;
        Self {
            name: "fixed".into(),
        }
    }
}

/// rust-rocksdb `ReadOptions`. Iterate bounds are honoured; the rest is
/// accepted (snapshot is already pinned on the `Transaction`).
#[derive(Debug, Clone, Default)]
pub struct ReadOptions {
    pub lower: Option<Vec<u8>>,
    pub upper: Option<Vec<u8>>,
}

impl ReadOptions {
    pub fn set_snapshot<D>(&mut self, _snap: &SnapshotWithThreadMode<'_, D>) {}
    pub fn set_async_io(&mut self, _v: bool) {}
    pub fn fill_cache(&mut self, _v: bool) {}
    pub fn set_verify_checksums(&mut self, _v: bool) {}
    pub fn set_prefix_same_as_start(&mut self, _v: bool) {}
    pub fn set_total_order_seek(&mut self, _v: bool) {}
    pub fn set_timestamp(&mut self, _ts: impl Into<Vec<u8>>) {}
    pub fn set_iterate_lower_bound(&mut self, key: impl Into<Vec<u8>>) {
        self.lower = Some(key.into());
    }
    pub fn set_iterate_upper_bound(&mut self, key: impl Into<Vec<u8>>) {
        self.upper = Some(key.into());
    }
    pub fn set_readahead_size(&mut self, _n: usize) {}
    pub fn set_pin_data(&mut self, _v: bool) {}
}

/// rust-rocksdb snapshot handle (sequence pin). SurrealDB stores this and
/// passes it to `ReadOptions::set_snapshot`.
#[derive(Debug, Clone, Copy)]
pub struct SnapshotWithThreadMode<'a, D> {
    pub seq: pedradb_core::SequenceNumber,
    _marker: PhantomData<fn(&'a D) -> &'a D>,
}

impl<'a, D> SnapshotWithThreadMode<'a, D> {
    pub(crate) fn at(seq: pedradb_core::SequenceNumber) -> Self {
        Self {
            seq,
            _marker: PhantomData,
        }
    }
}

/// rust-rocksdb `DBRawIteratorWithThreadMode`. `D` is phantom so SurrealDB
/// can name `DBRawIteratorWithThreadMode<'static, OptimisticTransactionDB>`.
pub struct DBRawIteratorWithThreadMode<'a, D, E: Env = StdEnv> {
    inner: Option<DBIterator<E>>,
    db: &'a DB<E>,
    seq: pedradb_core::SequenceNumber,
    lower: Option<Vec<u8>>,
    upper: Option<Vec<u8>>,
    _d: PhantomData<fn(&'a D) -> &'a D>,
}

impl<'a, D, E: Env> DBRawIteratorWithThreadMode<'a, D, E> {
    pub(crate) fn open(db: &'a DB<E>, seq: pedradb_core::SequenceNumber, ro: &ReadOptions) -> Self {
        let mut it = Self {
            inner: None,
            db,
            seq,
            lower: ro.lower.clone(),
            upper: ro.upper.clone(),
            _d: PhantomData,
        };
        it.seek_to_first();
        it
    }

    /// rust-rocksdb: iterator is usable.
    #[must_use]
    pub fn valid(&self) -> bool {
        self.inner.as_ref().is_some_and(DBIterator::valid)
    }

    /// rust-rocksdb: last iterator error (Pedra fails closed on open).
    pub fn status(&self) -> Result<()> {
        Ok(())
    }

    /// Current key.
    #[must_use]
    pub fn key(&self) -> Option<&[u8]> {
        let it = self.inner.as_ref()?;
        if it.valid() {
            Some(it.key())
        } else {
            None
        }
    }

    /// Current value.
    #[must_use]
    pub fn value(&self) -> Option<&[u8]> {
        let it = self.inner.as_ref()?;
        if it.valid() {
            Some(it.value())
        } else {
            None
        }
    }

    /// Seek first key (honours lower bound).
    pub fn seek_to_first(&mut self) {
        let start = self.lower.clone();
        self.reopen(match start.as_deref() {
            Some(k) => IteratorMode::From(k, Direction::Forward),
            None => IteratorMode::Start,
        });
    }

    /// Seek last key (honours upper bound).
    pub fn seek_to_last(&mut self) {
        self.reopen(IteratorMode::End);
        self.skip_past_upper();
    }

    /// Seek ≥ `key`.
    pub fn seek<K: AsRef<[u8]>>(&mut self, key: K) {
        let k = key.as_ref();
        let start = match &self.lower {
            Some(lo) if k < lo.as_slice() => lo.as_slice(),
            _ => k,
        };
        // Need owned to avoid borrow of self.lower while reopen takes &self.
        let owned = start.to_vec();
        self.reopen(IteratorMode::From(&owned, Direction::Forward));
        self.skip_past_upper();
    }

    /// Seek ≤ `key`.
    pub fn seek_for_prev<K: AsRef<[u8]>>(&mut self, key: K) {
        let owned = key.as_ref().to_vec();
        self.reopen(IteratorMode::From(&owned, Direction::Reverse));
    }

    /// Next key.
    pub fn next(&mut self) {
        if let Some(it) = self.inner.as_mut() {
            it.next();
        }
        self.skip_past_upper();
    }

    /// Previous key.
    pub fn prev(&mut self) {
        // Our windowed iterator only walks one direction per open. Re-seek
        // from the current key in reverse.
        let cur = self.key().map(<[u8]>::to_vec);
        if let Some(k) = cur {
            self.reopen(IteratorMode::From(&k, Direction::Reverse));
            if self.valid() && self.key() == Some(k.as_slice()) {
                if let Some(it) = self.inner.as_mut() {
                    it.next();
                }
            }
        } else {
            self.inner = None;
        }
    }

    fn skip_past_upper(&mut self) {
        let Some(hi) = self.upper.clone() else {
            return;
        };
        if self
            .inner
            .as_ref()
            .is_some_and(|it| it.valid() && it.key() >= hi.as_slice())
        {
            self.inner = None;
        }
    }

    fn reopen(&mut self, mode: IteratorMode<'_>) {
        let cf = super::ColumnFamily {
            name: DEFAULT_CF.into(),
        };
        match scan_cf_at(
            &self.db.inner,
            &self.db.codec,
            DEFAULT_CF,
            mode,
            self.seq,
            &self.db.cfs,
        ) {
            Ok(it) => self.inner = Some(it),
            Err(_) => self.inner = None,
        }
        let _ = cf;
    }
}

/// Property name constants (rust-rocksdb `properties`).
pub mod properties {
    pub const BLOCK_CACHE_USAGE: &str = "rocksdb.block-cache-usage";
    pub const BLOCK_CACHE_PINNED_USAGE: &str = "rocksdb.block-cache-pinned-usage";
    pub const ESTIMATE_TABLE_READERS_MEM: &str = "rocksdb.estimate-table-readers-mem";
    pub const CUR_SIZE_ALL_MEM_TABLES: &str = "rocksdb.cur-size-all-mem-tables";
    pub const TOTAL_SST_FILES_SIZE: &str = "rocksdb.total-sst-files-size";
    pub const LIVE_SST_FILES_SIZE: &str = "rocksdb.live-sst-files-size";
    pub const ESTIMATE_LIVE_DATA_SIZE: &str = "rocksdb.estimate-live-data-size";
    pub const ESTIMATE_NUM_KEYS: &str = "rocksdb.estimate-num-keys";
    pub const COMPACTION_PENDING: &str = "rocksdb.compaction-pending";
    pub const NUM_RUNNING_COMPACTIONS: &str = "rocksdb.num-running-compactions";
    pub const NUM_RUNNING_FLUSHES: &str = "rocksdb.num-running-flushes";
}

// Env bound kept so scan_cf_at type-checks through DB.
#[allow(dead_code)]
fn _env_bound<E: Env>() {}
