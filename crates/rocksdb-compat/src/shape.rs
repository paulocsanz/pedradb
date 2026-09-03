//! rust-rocksdb type surface that SurrealDB `kv-rocksdb` imports.
//!
//! Tuning knobs are accepted and ignored (Pedra policy is not Rocks).
//! Iterators / OCC / flush / compact are real.

use super::{scan_cf_at, DBIterator, Direction, IteratorMode, Result, DB, DEFAULT_CF};
use pedradb_core::{Env, StdEnv};
use std::marker::PhantomData;
use std::sync::Arc;

/// rust-rocksdb `ColumnFamilyDescriptor`.
pub struct ColumnFamilyDescriptor {
    pub name: String,
    pub options: super::Options,
}

impl ColumnFamilyDescriptor {
    /// Name + CF options. `write_buffer_size` is the per-CF memtable cap
    /// (RFC-0065 P1.1); other knobs stay ignored.
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

/// rust-rocksdb `ReadOptions`. Iterate bounds are honoured; the snapshot is
/// honoured by `DB::raw_iterator_opt` / `iterator_opt` / `get_opt` when
/// attached via `set_snapshot` (F180 — was a no-op and the iterator read
/// latest, leaking post-snapshot writes into a "pinned" scan).
#[derive(Debug, Clone)]
pub struct ReadOptions {
    pub lower: Option<Vec<u8>>,
    pub upper: Option<Vec<u8>>,
    /// Sequence pinned by `set_snapshot` (`None` = read latest).
    pub(crate) snap: Option<pedradb_core::SequenceNumber>,
    /// rust-rocksdb default is `true`. `false` is G2 — [`Self::refuse_checksums_off`].
    pub(crate) verify_checksums: bool,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            lower: None,
            upper: None,
            snap: None,
            verify_checksums: true,
        }
    }
}

impl ReadOptions {
    pub fn set_snapshot<D>(&mut self, snap: &SnapshotWithThreadMode<'_, D>) {
        self.snap = Some(snap.seq);
    }
    pub fn set_async_io(&mut self, _v: bool) {}
    pub fn fill_cache(&mut self, _v: bool) {}
    /// rust-rocksdb `set_verify_checksums`. `false` does **not** disable CRC:
    /// subsequent `get_opt` / `iterator_opt` return [`ErrorKind::NotSupported`].
    pub fn set_verify_checksums(&mut self, v: bool) {
        self.verify_checksums = v;
    }
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

    pub(crate) fn refuse_checksums_off(&self) -> Result<()> {
        if self.verify_checksums {
            Ok(())
        } else {
            Err(super::Error::not_supported(
                "ReadOptions::set_verify_checksums(false) is NotSupported (G2: CRC stays on)",
            ))
        }
    }
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
    /// Column family this raw iterator walks. `reopen` must use this, not
    /// a hard-coded default — `raw_iterator_cf` is otherwise a silent
    /// cross-CF leak (issue #1).
    cf: Arc<str>,
    lower: Option<Vec<u8>>,
    upper: Option<Vec<u8>>,
    /// Whether the windowed iterator is currently walking reverse (fix C2):
    /// `next` must always ascend, so from a reverse position it re-seeks
    /// forward instead of decrementing the window.
    rev: bool,
    _d: PhantomData<fn(&'a D) -> &'a D>,
}

impl<'a, D, E: Env> DBRawIteratorWithThreadMode<'a, D, E> {
    pub(crate) fn open(db: &'a DB<E>, seq: pedradb_core::SequenceNumber, ro: &ReadOptions) -> Self {
        Self::open_cf(db, DEFAULT_CF, seq, ro)
    }

    pub(crate) fn open_cf(
        db: &'a DB<E>,
        cf: &str,
        seq: pedradb_core::SequenceNumber,
        ro: &ReadOptions,
    ) -> Self {
        let mut it = Self {
            inner: None,
            db,
            seq,
            cf: Arc::from(cf),
            lower: ro.lower.clone(),
            upper: ro.upper.clone(),
            rev: true,
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
        self.rev = false;
        self.reopen(match start.as_deref() {
            Some(k) => IteratorMode::From(k, Direction::Forward),
            None => IteratorMode::Start,
        });
    }

    /// Seek last key (honours upper bound — F181: the bound is exclusive,
    /// so position on the last key **below** it, walking back instead of
    /// invalidating).
    pub fn seek_to_last(&mut self) {
        self.rev = true;
        match self.upper.clone() {
            Some(hi) => {
                self.reopen(IteratorMode::From(&hi, Direction::Reverse));
                self.step_prev_past_upper();
                self.clamp_to_lower();
            }
            None => self.reopen(IteratorMode::End),
        }
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
        self.rev = false;
        self.reopen(IteratorMode::From(&owned, Direction::Forward));
        self.skip_past_upper();
    }

    /// Seek ≤ `key` (honours lower bound — fix C3; honours the exclusive
    /// upper bound by stepping previous past any key ≥ it — F181).
    pub fn seek_for_prev<K: AsRef<[u8]>>(&mut self, key: K) {
        let owned = key.as_ref().to_vec();
        self.rev = true;
        self.reopen(IteratorMode::From(&owned, Direction::Reverse));
        self.step_prev_past_upper();
        self.clamp_to_lower();
    }

    /// Next key (ascending, regardless of how positioned — fix C2).
    pub fn next(&mut self) {
        if self.rev {
            // Re-seek forward from the current key (inclusive), then advance
            // one so the net move is "next ascending key".
            let cur = self.key().map(<[u8]>::to_vec);
            if let Some(k) = cur {
                let owned = match &self.lower {
                    Some(lo) if k.as_slice() < lo.as_slice() => lo.as_slice().to_vec(),
                    _ => k,
                };
                self.reopen(IteratorMode::From(&owned, Direction::Forward));
                self.rev = false;
                if self.valid() && self.key() == Some(owned.as_slice()) {
                    if let Some(it) = self.inner.as_mut() {
                        it.next();
                    }
                }
                self.skip_past_upper();
            } else {
                self.inner = None;
            }
            return;
        }
        if let Some(it) = self.inner.as_mut() {
            it.next();
        }
        self.skip_past_upper();
    }

    /// Previous key (descending; honours lower bound — fix C3).
    pub fn prev(&mut self) {
        // Our windowed iterator only walks one direction per open. Re-seek
        // from the current key in reverse.
        self.rev = true;
        let cur = self.key().map(<[u8]>::to_vec);
        if let Some(k) = cur {
            self.reopen(IteratorMode::From(&k, Direction::Reverse));
            if self.valid() && self.key() == Some(k.as_slice()) {
                if let Some(it) = self.inner.as_mut() {
                    it.next();
                }
            }
            self.clamp_to_lower();
        } else {
            self.inner = None;
        }
    }

    fn clamp_to_lower(&mut self) {
        let Some(lo) = self.lower.clone() else {
            return;
        };
        if self
            .inner
            .as_ref()
            .is_some_and(|it| it.valid() && it.key() < lo.as_slice())
        {
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

    /// Reverse-positioned walk below an exclusive upper bound (F181): while
    /// the current key is ≥ the bound, step to the previous key (reverse
    /// iteration) instead of invalidating — `seek_to_last`/`seek_for_prev`
    /// must land on the last key inside `[lower, upper)`.
    fn step_prev_past_upper(&mut self) {
        let Some(hi) = self.upper.clone() else {
            return;
        };
        while self
            .inner
            .as_ref()
            .is_some_and(|it| it.valid() && it.key() >= hi.as_slice())
        {
            match self.inner.as_mut() {
                Some(it) => it.next(), // reverse mode: steps to the previous key
                None => break,
            }
        }
    }

    fn reopen(&mut self, mode: IteratorMode<'_>) {
        let names = self.db.cf_names();
        match scan_cf_at(
            &self.db.inner,
            &self.db.codec,
            self.cf.as_ref(),
            mode,
            self.seq,
            &names,
            // The raw iterator applies its own bounds per step (F181); the
            // window scan stays unbounded here.
            super::IterBounds::none(),
        ) {
            Ok(it) => self.inner = Some(it),
            Err(_) => self.inner = None,
        }
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
