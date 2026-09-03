//! `rocksdb-compat` — rust-rocksdb **0.22 API** on pedradb-core
//! [`ConcurrentDb`](pedradb_core::ConcurrentDb).
//!
//! Swap `rocksdb = { package = "rocksdb-compat", path = ... }` and compile
//! against the rust-rocksdb surface. Semantics that would be silent-wrong
//! (skip-any WAL, dropping SST files without tombstones) are implemented
//! the Pedra-correct way, not omitted.
//!
//! Column families are prefix-encoded (`cf_name \x00 key`) in **one** WAL.
//! Flush emits one SST per CF (RFC-0065 P0); `compact_range_cf` rewrites
//! only that family. `create_cf` / `drop_cf` / `list_cf` /
//! `ingest_external_file` / `SstFileWriter` / `WriteBatchWithIndex` /
//! compaction filters / `delete_file_in_range` all exist and work.

#![forbid(unsafe_code)]

mod api;
mod env;
mod iter_kernel;
mod knobs;
mod locktab;
mod txn;
pub use api::{
    AsColumnFamilyRef, BlockBasedOptions, Cache, ChecksumType, CompactionDecision, DBPinnableSlice,
    IngestExternalFileOptions, LiveFile, MergeOperands, SstFileWriter, WriteBatchWithIndex,
    DEFAULT_COLUMN_FAMILY_NAME,
};
pub mod backup;
pub mod checkpoint;
pub use backup::{BackupEngine, BackupEngineInfo, BackupEngineOptions, RestoreOptions};
pub use checkpoint::Checkpoint;
pub use env::{Env, SstFileManager};
pub use knobs::{g2_not_supported, KnobClass, KnobEntry, KNOB_INVENTORY};
pub use txn::{
    OptimisticTransactionDB, OptimisticTransactionOptions, Transaction, TransactionDB,
    TransactionDBOptions, TransactionOptions, WriteOptions,
};

use bytes::Bytes;
use parking_lot::{Mutex, RwLock};
mod shape;
pub use shape::{
    properties, BottommostLevelCompaction, ColumnFamilyDescriptor, CompactOptions,
    DBCompactionStyle, DBCompressionType, DBRawIteratorWithThreadMode, FlushOptions, LogLevel,
    ReadOptions, SliceTransform, SnapshotWithThreadMode, UniversalCompactOptions,
    UniversalCompactionStopStyle, WaitForCompactOptions,
};

use pedradb_core::{
    cf_encode_effective, decode_cf_key, encode_cf_key, key_in_cf_family, prefix_exclusive_end,
    BatchOp, CompactOptions as CoreCompactOptions, ConcurrentDb, CoreError, Env as PedraEnv,
    Snapshot as CoreSnapshot, SnapshotPin, StdEnv, DEFAULT_SST_PAYLOAD_BUDGET_BYTES,
    L0_COMPACTION_TRIGGER,
};
use pedradb_io_uring::IoUringEnv;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt;
use std::ops::Bound;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Process-wide DB-instance discriminator for TLS read caches (fix C1/C1b):
/// `base + read_cache_epoch()` is unique per live instance, so the thread-local
/// last-get / last-count tables can never answer for another instance.
static CACHE_ID: AtomicU64 = AtomicU64::new(0);

enum CompactCmd {
    Run,
    Shutdown,
}

/// Machine-readable class of a compat [`Error`] (RFC-0047 P0.1).
///
/// rust-rocksdb exposes one opaque `Error`; a drop-in host still needs to
/// program availability policy, so the kind survives the string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Write refused after a failed required WAL sync (`CoreError::DurabilityFenced`).
    /// Outcome of the failed write is uncertain; `resume()`/reopen recovers.
    Fenced,
    /// WAL/SST integrity failure (CRC and friends).
    Corruption,
    /// Repeated corruption tripped the CORRUPTLOG escalation limit — open is
    /// refused in every recovery mode (RFC-0038).
    CorruptionEscalated,
    /// Filesystem I/O failure.
    Io,
    /// OCC conflict (`TransactionConflict`).
    TransactionConflict,
    /// rust-rocksdb `Busy` — 2PL deadlock / lock refused.
    Busy,
    /// rust-rocksdb `TimedOut` — 2PL lock wait expired.
    TimedOut,
    /// CAS precondition failed.
    CasMismatch,
    /// Snapshot older than the version-GC watermark.
    SnapshotTooOld,
    /// L0/memtable write stall.
    WriteStall,
    /// The directory is already open elsewhere.
    AlreadyOpen,
    /// Caller-side misuse (unknown column family, bad path, …).
    InvalidArgument,
    /// API the kernel will not implement (ingest, delete_files_in_range, …).
    /// Never Ok: faking these is silent-wrong (RFC-0050 P0.6).
    NotSupported,
    /// Anything else.
    Other,
}

/// Compatibility error surface (rust-rocksdb exposes one opaque `Error`).
#[derive(Debug, Clone)]
pub struct Error {
    pub(crate) msg: String,
    pub(crate) kind: ErrorKind,
}

impl Error {
    /// Stable class of this error for host-side policy (RFC-0047 P0.1).
    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for Error {}

impl From<CoreError> for Error {
    fn from(e: CoreError) -> Self {
        let kind = match &e {
            CoreError::DurabilityFenced => ErrorKind::Fenced,
            CoreError::Crc { .. }
            | CoreError::Truncated(_)
            | CoreError::WalZeroHeader { .. }
            | CoreError::CorruptManifest(_)
            | CoreError::CorruptHistory(_)
            | CoreError::CorruptValue(_) => ErrorKind::Corruption,
            CoreError::CorruptionEscalated { .. } => ErrorKind::CorruptionEscalated,
            CoreError::Io(_) => ErrorKind::Io,
            CoreError::TransactionConflict => ErrorKind::TransactionConflict,
            CoreError::CasMismatch => ErrorKind::CasMismatch,
            CoreError::SnapshotTooOld { .. } => ErrorKind::SnapshotTooOld,
            CoreError::WriteStall { .. } | CoreError::WriteStallMem { .. } => ErrorKind::WriteStall,
            CoreError::AlreadyOpen { .. } => ErrorKind::AlreadyOpen,
            CoreError::Internal(_) | CoreError::TransactionFinished | CoreError::Transaction(_) => {
                ErrorKind::Other
            }
            // F196: post-commit manifest unsynced — surfaced by off-lock
            // host persisters; an I/O durability condition.
            CoreError::ManifestCommittedUnsynced { .. } => ErrorKind::Io,
        };
        Self {
            msg: e.to_string(),
            kind,
        }
    }
}

/// Result alias matching rust-rocksdb's shape.
pub type Result<T> = std::result::Result<T, Error>;

fn map_property_int<E: PedraEnv>(db: &ConcurrentDb<E>, name: &str) -> Option<u64> {
    const LEVEL_PREFIX: &str = "rocksdb.num-files-at-level";
    if let Some(rest) = name.strip_prefix(LEVEL_PREFIX) {
        let lvl: u32 = rest.parse().ok()?;
        return Some(db.level_file_count(lvl) as u64);
    }
    let s = db.stats();
    match name {
        properties::ESTIMATE_NUM_KEYS => {
            Some((s.mem_entries as u64).saturating_add(s.sst_entries as u64))
        }
        properties::TOTAL_SST_FILES_SIZE | properties::LIVE_SST_FILES_SIZE => Some(s.sst_bytes),
        properties::CUR_SIZE_ALL_MEM_TABLES => Some(s.mem_approx_bytes as u64),
        properties::ESTIMATE_LIVE_DATA_SIZE => Some(
            s.sst_bytes
                .saturating_add(s.mem_approx_bytes as u64)
                .saturating_add(s.vlog_live_bytes),
        ),
        properties::COMPACTION_PENDING => {
            Some(u64::from(s.l0_files >= L0_COMPACTION_TRIGGER as u64))
        }
        properties::NUM_RUNNING_COMPACTIONS | properties::NUM_RUNNING_FLUSHES => Some(0),
        properties::BLOCK_CACHE_USAGE => Some(s.block_cache_bytes),
        properties::BLOCK_CACHE_PINNED_USAGE => Some(0),
        properties::ESTIMATE_TABLE_READERS_MEM => {
            Some(s.table_cache_hits.saturating_add(s.table_cache_misses))
        }
        _ => None,
    }
}

/// Open options (builder subset). `create_if_missing` mirrors rust-rocksdb;
/// Pedra always requires the directory to be creatable.
pub struct Options {
    /// Whether to create the database directory when absent.
    pub create_if_missing: bool,
    /// Memtable flush threshold. Default **64 MiB** (`0x4000000`) — rust-rocksdb
    /// / Rocks C++ factory. `0` disables auto-flush (manual [`DB::flush`] only).
    /// Hosts that `set_write_buffer_size` get exactly that many bytes.
    pub write_buffer_size: usize,
    /// Merged compaction output split target (`set_target_file_size_base`
    /// role). `0` keeps the kernel default (256 MiB): the SST writer buffers
    /// one output file in memory, so this bounds compaction peak RAM.
    pub target_file_size_base: u64,
    /// WAL barrier before Ok. Default **`false`** (RFC-0054): rust-rocksdb
    /// `WriteOptions.sync=false` — the factory config every Rocks host
    /// actually runs. Kernel `OpenOptions.sync` stays `true` (Pedra G1).
    pub sync: bool,
    /// Version GC on auto-compact (Pedra `auto_reclaim`): drops versions
    /// older than the oldest open snapshot pin, like RocksDB compaction
    /// dropping unpinned obsolete versions. Default **`true`** (RFC-0047
    /// P0.3): the drop-in ships the Rocks storage profile — disk ≈ live
    /// set + pins. `false` is the Pedra kernel default (RFC-0009 F20: keep
    /// all versions, PITR included) as an explicit opt-out for hosts that
    /// want it.
    pub auto_reclaim: bool,
    /// RFC-0047 P1.2: auto-resume a durability fence whose typed class is
    /// `Transient` (ENOSPC-like — heals on its own), via the host compact
    /// worker. Every other class stays **manual** ([`DB::resume`]) — never
    /// an untyped flag; the recovery outcome is always on
    /// [`DB::last_fence_recovery`]. Default `true` (Rocks-shaped
    /// background-error profile).
    pub auto_resume_transient: bool,
    /// RFC-0047 P2.1: `on_background_error` listener — fired when the
    /// engine durability-fences (the Pedra background-error class), by the
    /// host compact worker within one poll tick. `None` (default) = no
    /// listener.
    pub background_error_listener: Option<BackgroundErrorListener>,
    /// WAL recovery at open (RFC-0047 P0.2). Rust-rocksdb
    /// `WalRecoveryMode`-shaped; drop-in default is
    /// [`WalRecoveryMode::PointInTime`] (serve the prefix, report the
    /// discard). The kernel default is fail-closed.
    pub wal_recovery: WalRecoveryMode,
    /// Strongest WAL barrier (`F_FULLFSYNC` on Darwin). Default **`true`**:
    /// upstream Rocks CMake on macOS sets `HAVE_FULLFSYNC` (`PosixWritableFile::Sync`
    /// → `fcntl(F_FULLFSYNC)`). crates.io `librocksdb-sys` 0.16 `build.rs`
    /// omits that define (CMakeLists.txt does `check_cxx_symbol_exists`);
    /// later rust-rocksdb forks hardcode it. Drop-in matches **C++ Rocks on
    /// Darwin**, not the crippled sys crate. `false` = `fdatasync` (Linux
    /// class / the sys-crate accident).
    pub wal_full_fsync: bool,
    /// rust-rocksdb `enable_blob_files`. Default `false` (Rocks default).
    /// When true, values ≥ [`Self::min_blob_size`] spill to `VALUES.vlog`
    /// (WiscKey / BlobDB-shaped).
    pub enable_blob_files: bool,
    /// rust-rocksdb `min_blob_size`. Titan's 4096. Ignored unless blob files
    /// are enabled. `0` with blob files on spills every value.
    pub min_blob_size: u64,
    /// rust-rocksdb / Titan `blob_file_size` (rotate cap). `None` = single
    /// `VALUES.vlog` (no numbered blob generation).
    pub blob_file_size: Option<u64>,
    /// Rocks `NewLRUCache` / `optimize_for_point_lookup`. `None` = Pedra
    /// 8192-entry block-cache default. `Some(n)` (RFC-0042 v18) bounds the
    /// resident SST payload pool — Pedra's equivalent of Rocks' compressed
    /// block cache — to `n` bytes; the decoded-block cache stays small.
    pub block_cache_bytes: Option<u64>,
    compaction_filter: Option<CompactionFilterFn>,
    merge_operator: Option<MergeOperatorFn>,
    /// RFC-0062 P1.6: `set_paranoid_checks(false)` recorded; open refuses.
    paranoid_off: bool,
    /// RFC-0062 P1.6: `ChecksumType::NoChecksum` recorded; open refuses.
    checksum_off: bool,
    /// RFC-0062 P1.6: skip-any WAL recorded; open refuses.
    skip_any: bool,
    /// rust-rocksdb `Options::set_env`. Pedra I/O stays [`pedradb_core::Env`]
    /// at open; this is kept so `set_env` + `BackupEngine::open` compile.
    env: Option<Env>,
    /// rust-rocksdb / C++ `SstFileManager` (not in crates.io 0.22; we export it).
    sst_file_manager: Option<SstFileManager>,
}

type CompactionFilterFn =
    Arc<Mutex<Box<dyn FnMut(u32, &[u8], &[u8]) -> CompactionDecision + Send>>>;
type MergeOperatorFn =
    Arc<dyn Fn(&[u8], Option<&[u8]>, &MergeOperands) -> Option<Vec<u8>> + Send + Sync>;

/// WAL recovery mode at open (rust-rocksdb `WalRecoveryMode` / `DBRecoveryMode`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WalRecoveryMode {
    /// Rocks `kPointInTimeRecovery`-shaped: recover every complete record
    /// before the damage and keep serving; the discarded suffix is
    /// reported via [`DB::last_recovery_report`]. Repeated corruption
    /// still escalates (open refused) — CORRUPTLOG is not bypassed.
    #[default]
    PointInTime,
    /// Pedra kernel default: mid-WAL integrity failure fails the open.
    FailClosed,
    /// rust-rocksdb `AbsoluteConsistency` — mapped to [`Self::FailClosed`].
    AbsoluteConsistency,
    /// rust-rocksdb `TolerateCorruptedTailRecords` — mapped to [`Self::FailClosed`]
    /// (Rocks also refuses open on mid-WAL CRC).
    TolerateCorruptedTailRecords,
    /// rust-rocksdb `SkipAnyCorruptedRecord`. **Not implemented.** Open
    /// returns [`ErrorKind::NotSupported`] (G2).
    SkipAnyCorruptedRecord,
}

/// rust-rocksdb `DBRecoveryMode` name.
pub type DBRecoveryMode = WalRecoveryMode;

impl fmt::Debug for Options {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Options")
            .field("create_if_missing", &self.create_if_missing)
            .field("write_buffer_size", &self.write_buffer_size)
            .field("sync", &self.sync)
            .field("auto_reclaim", &self.auto_reclaim)
            .field("auto_resume_transient", &self.auto_resume_transient)
            .field("wal_recovery", &self.wal_recovery)
            .field("wal_full_fsync", &self.wal_full_fsync)
            .field("enable_blob_files", &self.enable_blob_files)
            .field("min_blob_size", &self.min_blob_size)
            .field("blob_file_size", &self.blob_file_size)
            .field("block_cache_bytes", &self.block_cache_bytes)
            .field(
                "background_error_listener",
                &self.background_error_listener.is_some(),
            )
            .field("env", &self.env.is_some())
            .field("sst_file_manager", &self.sst_file_manager.is_some())
            .finish()
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            create_if_missing: false,
            write_buffer_size: 64 * 1024 * 1024,
            target_file_size_base: 0,
            sync: false,
            auto_reclaim: true,
            auto_resume_transient: true,
            background_error_listener: None,
            wal_recovery: WalRecoveryMode::PointInTime,
            wal_full_fsync: true,
            enable_blob_files: false,
            min_blob_size: 4096,
            blob_file_size: None,
            block_cache_bytes: None,
            compaction_filter: None,
            merge_operator: None,
            paranoid_off: false,
            checksum_off: false,
            skip_any: false,
            env: None,
            sst_file_manager: None,
        }
    }
}

/// Retryability class of a durability fence mirrored from the kernel
/// (RFC-0047 P1.2): hosts program auto-resume on the class, never on
/// parsing strings. Maps to the RocksDB background-error severity split:
/// `Transient` ≈ retryable/soft (ENOSPC-like), the rest ≈ hard.
pub type FenceClass = pedradb_core::FenceClass;

/// RFC-0047 P2.1: RocksDB `EventListener::on_background_error`-shaped
/// payload. Pedra has one background-failure class — the durability fence
/// (WAL write/sync failure, vlog promote failure) — reported here with
/// its typed severity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundError {
    /// The engine is fenced (every other field describes why).
    pub kind: ErrorKind,
    /// Retryability severity ([`FenceClass`]).
    pub class: FenceClass,
    /// The I/O error that tripped the fence (for logs).
    pub message: String,
}

impl BackgroundError {
    pub(crate) fn from_fence(report: &pedradb_core::FenceReport) -> Self {
        Self {
            kind: ErrorKind::Fenced,
            class: report.class,
            message: report.io_error.clone(),
        }
    }
}

/// RFC-0047 P2.1: `on_background_error` listener. Fired by the host
/// compact worker within one poll tick (~5 ms) of a fence — never on the
/// calling thread of the failed write (that caller already got the typed
/// error). See [`Options::background_error_listener`].
pub type BackgroundErrorListener = Arc<dyn Fn(BackgroundError) + Send + Sync>;

impl Options {
    /// New default options (4 MiB write buffer — Pedra core default).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder: create the DB directory when missing.
    pub fn create_if_missing(&mut self, v: bool) -> &mut Self {
        self.create_if_missing = v;
        self
    }

    /// Builder: RFC-0047 P2.1 `on_background_error` listener (fired by the
    /// host compact worker on a durability fence).
    pub fn set_background_error_listener(
        &mut self,
        listener: BackgroundErrorListener,
    ) -> &mut Self {
        self.background_error_listener = Some(listener);
        self
    }

    /// WAL barrier before Ok. Default `false` (Rocks-shaped). `true` = G1
    /// (and `F_FULLFSYNC` on Darwin when [`Self::wal_full_fsync`] is on).
    pub fn set_sync(&mut self, v: bool) -> &mut Self {
        self.sync = v;
        self
    }

    /// rust-rocksdb `Options::set_env`. Stored; open still uses the Pedra
    /// [`pedradb_core::Env`] passed to [`DB::open_cf_with_env`].
    pub fn set_env(&mut self, env: &Env) -> &mut Self {
        self.env = Some(env.clone());
        self
    }

    /// C++ `Options::sst_file_manager`. Stored; compact does not stall yet.
    pub fn set_sst_file_manager(&mut self, mgr: &SstFileManager) -> &mut Self {
        self.sst_file_manager = Some(mgr.clone());
        self
    }

    /// WAL barrier class switch for the whole DB. See
    /// [`Options::wal_full_fsync`] (default **true** = upstream Darwin Rocks).
    pub fn set_wal_full_fsync(&mut self, v: bool) -> &mut Self {
        self.wal_full_fsync = v;
        self
    }

    /// rust-rocksdb: create named CFs that are not on disk. Pedra always
    /// registers the names passed to `open_cf` / `open_cf_descriptors`.
    pub fn create_missing_column_families(&mut self, _v: bool) -> &mut Self {
        self
    }

    /// Builder: memtable flush threshold in bytes. Rocks default is 64 MiB.
    pub fn set_write_buffer_size(&mut self, n: usize) -> &mut Self {
        self.write_buffer_size = n;
        self
    }

    /// Accepted no-ops: SurrealDB `kv-rocksdb` sets these on open. Pedra
    /// G1 / memtable / compact policy are not Rocks knobs.
    pub fn set_use_fsync(&mut self, _v: bool) {}
    pub fn set_manual_wal_flush(&mut self, _v: bool) {}
    pub fn set_wal_bytes_per_sync(&mut self, _n: u64) {}
    pub fn increase_parallelism(&mut self, _n: i32) {}
    pub fn set_max_background_jobs(&mut self, _n: i32) {}
    pub fn set_max_open_files(&mut self, _n: i32) {}
    pub fn set_keep_log_file_num(&mut self, _n: usize) {}
    pub fn set_compaction_readahead_size(&mut self, _n: usize) {}
    pub fn set_max_subcompactions(&mut self, _n: u32) {}
    pub fn set_enable_pipelined_write(&mut self, _v: bool) {}
    pub fn set_wal_size_limit_mb(&mut self, _n: u64) {}
    pub fn set_allow_concurrent_memtable_write(&mut self, _v: bool) {}
    pub fn set_avoid_unnecessary_blocking_io(&mut self, _v: bool) {}
    pub fn set_enable_write_thread_adaptive_yield(&mut self, _v: bool) {}
    pub fn set_log_level(&mut self, _l: LogLevel) {}
    pub fn set_target_file_size_base(&mut self, n: u64) {
        self.target_file_size_base = n;
    }
    pub fn set_target_file_size_multiplier(&mut self, _n: i32) {}
    pub fn set_bottommost_compression_type(&mut self, _c: DBCompressionType) {}
    pub fn set_bottommost_zstd_max_train_bytes(&mut self, _n: i32, _enabled: bool) {}
    pub fn set_prefix_extractor(&mut self, _t: SliceTransform) {}
    pub fn set_memtable_prefix_bloom_ratio(&mut self, _r: f64) {}
    pub fn set_compression_per_level(&mut self, _c: &[DBCompressionType]) {}
    pub fn set_compaction_style(&mut self, _s: DBCompactionStyle) {}
    pub fn set_level_compaction_dynamic_level_bytes(&mut self, _v: bool) {}
    pub fn set_bytes_per_sync(&mut self, _n: u64) {}
    pub fn set_max_write_buffer_number(&mut self, _n: i32) {}
    pub fn set_min_write_buffer_number_to_merge(&mut self, _n: i32) {}
    pub fn set_level_zero_file_num_compaction_trigger(&mut self, _n: i32) {}
    pub fn set_level_zero_slowdown_writes_trigger(&mut self, _n: i32) {}
    pub fn set_level_zero_stop_writes_trigger(&mut self, _n: i32) {}
    pub fn set_max_bytes_for_level_base(&mut self, _n: u64) {}
    pub fn set_max_bytes_for_level_multiplier(&mut self, _n: f64) {}
    pub fn set_disable_auto_compactions(&mut self, _v: bool) {}
    pub fn set_report_bg_io_stats(&mut self, _v: bool) {}
    pub fn set_optimize_filters_for_hits(&mut self, _v: bool) {}
    pub fn set_enable_blob_files(&mut self, v: bool) {
        self.enable_blob_files = v;
    }
    pub fn set_min_blob_size(&mut self, n: u64) {
        self.min_blob_size = n;
    }
    pub fn set_blob_file_size(&mut self, n: u64) {
        self.blob_file_size = if n == 0 { None } else { Some(n) };
    }
    pub fn set_enable_blob_gc(&mut self, _v: bool) {}
    pub fn set_blob_gc_age_cutoff(&mut self, _n: f64) {}
    pub fn set_blob_compression_type(&mut self, _c: DBCompressionType) {}
    pub fn set_universal_compaction_options(&mut self, _o: &UniversalCompactOptions) {}
    /// rust-rocksdb `set_block_based_table_factory`.
    /// [`ChecksumType::NoChecksum`] is G2 — recorded so [`DB::open`] refuses.
    pub fn set_block_based_table_factory(&mut self, b: &BlockBasedOptions) {
        self.checksum_off = matches!(b.checksum, ChecksumType::NoChecksum);
        if let Some(n) = b.block_cache_bytes {
            self.block_cache_bytes = Some(n);
        }
    }

    /// rust-rocksdb `set_paranoid_checks`. `false` is G2 (open refuses).
    pub fn set_paranoid_checks(&mut self, enabled: bool) {
        self.paranoid_off = !enabled;
    }

    /// rust-rocksdb `set_wal_recovery_mode`.
    pub fn set_wal_recovery_mode(&mut self, mode: DBRecoveryMode) {
        self.skip_any = matches!(mode, WalRecoveryMode::SkipAnyCorruptedRecord);
        self.wal_recovery = match mode {
            WalRecoveryMode::SkipAnyCorruptedRecord => WalRecoveryMode::PointInTime,
            WalRecoveryMode::AbsoluteConsistency
            | WalRecoveryMode::TolerateCorruptedTailRecords
            | WalRecoveryMode::FailClosed => WalRecoveryMode::FailClosed,
            WalRecoveryMode::PointInTime => WalRecoveryMode::PointInTime,
        };
    }

    pub(crate) fn refuse_g2(&self) -> Result<()> {
        if self.paranoid_off {
            return Err(Error::not_supported(
                "set_paranoid_checks(false) is NotSupported (G2: Pedra never writes through unrecognized corruption)",
            ));
        }
        if self.checksum_off {
            return Err(Error::not_supported(
                "ChecksumType::NoChecksum is NotSupported (G2: SST CRC stays on)",
            ));
        }
        if self.skip_any || matches!(self.wal_recovery, WalRecoveryMode::SkipAnyCorruptedRecord) {
            return Err(Error::not_supported(
                "DBRecoveryMode::SkipAnyCorruptedRecord is NotSupported (G2)",
            ));
        }
        Ok(())
    }
    /// rust-rocksdb `optimize_for_point_lookup`: size the SST block cache
    /// to `block_cache_mb` MiB (RFC-0153). Hash-index / bloom extras stay
    /// Pedra's SST layout.
    pub fn optimize_for_point_lookup(&mut self, block_cache_mb: u64) {
        self.block_cache_bytes = Some(block_cache_mb.saturating_mul(1024 * 1024));
    }

    /// rust-rocksdb `Options::set_block_cache`.
    pub fn set_block_cache(&mut self, c: &Cache) {
        self.block_cache_bytes = Some(c.capacity() as u64);
    }
    /// rust-rocksdb `increase_parallelism` already exists; `prepare_for_bulk_load`.
    pub fn prepare_for_bulk_load(&mut self) {}
    /// rust-rocksdb compaction filter (applied on [`DB::compact`] / range compact).
    pub fn set_compaction_filter<F>(&mut self, _name: impl Into<String>, filter: F)
    where
        F: FnMut(u32, &[u8], &[u8]) -> CompactionDecision + Send + 'static,
    {
        self.compaction_filter = Some(Arc::new(Mutex::new(Box::new(filter))));
    }
    /// Associative merge operator. `merge()` is get + full_merge + put.
    pub fn set_merge_operator_associative<F>(&mut self, _name: impl Into<String>, full: F)
    where
        F: Fn(&[u8], Option<&[u8]>, &MergeOperands) -> Option<Vec<u8>> + Send + Sync + 'static,
    {
        self.merge_operator = Some(Arc::new(full));
    }
    /// rust-rocksdb `set_merge_operator` (partial merge ignored; full merge is used).
    pub fn set_merge_operator<F, P>(&mut self, name: impl Into<String>, full: F, _partial: P)
    where
        F: Fn(&[u8], Option<&[u8]>, &MergeOperands) -> Option<Vec<u8>> + Send + Sync + 'static,
    {
        self.set_merge_operator_associative(name, full);
    }
    /// UDT comparator (SurrealDB versioning). Accepted; Pedra keys stay
    /// raw — versioned CF is a documented remaining gap.
    pub fn set_comparator_with_ts(
        &mut self,
        _name: impl AsRef<str>,
        _ts_size: usize,
        _cmp: Box<dyn Fn(&[u8], &[u8]) -> std::cmp::Ordering + Send + Sync>,
        _cmp_ts: Box<dyn Fn(&[u8], &[u8]) -> std::cmp::Ordering + Send + Sync>,
        _cmp_without_ts: Box<dyn Fn(&[u8], bool, &[u8], bool) -> std::cmp::Ordering + Send + Sync>,
    ) {
    }
}

/// Column family handle (name-keyed emulation). The name is an `Arc` clone
/// of the registry entry, so `cf_handle` hands out handles without a
/// per-call allocation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ColumnFamily {
    name: Arc<str>,
}

impl ColumnFamily {
    /// CF name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Default CF name (raw keys when it is the only CF; prefixed otherwise).
pub const DEFAULT_CF: &str = "default";

/// F185: persisted CF registry (`CFREG` next to the DB). The CF↔keyspace
/// codec used to be derived from the list *supplied at open*
/// (`default_raw = cfs.len() <= 1`), process-local: reopening without the
/// named CFs (or adding a new CF to a default-only DB) flipped the codec
/// and silently read a different keyspace — committed keys answered `None`.
/// The registry freezes `default_raw` at first creation and reconciles the
/// CF set on every open (an existing CF omitted from the open list is an
/// error, like rocksdb's "column families not opened").
const CFREG_FILE_NAME: &str = "CFREG";
const CFREG_MAGIC: &[u8] = b"COMPATCF1\n";

fn cfreg_path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join(CFREG_FILE_NAME)
}

fn validate_cf_name(name: &str) -> Result<()> {
    if name.contains('\n') || name.contains('\0') {
        return Err(Error {
            msg: format!("invalid column family name {name:?} (NUL/newline reserved)"),
            kind: ErrorKind::InvalidArgument,
        });
    }
    Ok(())
}

/// `(default_raw, non-default CF names)`, or `None` when no registry exists
/// yet (first compat open / pre-F185 DB).
///
/// # Errors
/// Fail-closed on a corrupt registry (a silent recreate could flip the
/// codec and hide committed keys).
fn load_cf_registry(dir: &std::path::Path) -> Result<Option<(bool, Vec<String>)>> {
    let path = cfreg_path(dir);
    let raw = match std::fs::read(&path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(Error {
                msg: format!("read {}: {e}", path.display()),
                kind: ErrorKind::Io,
            })
        }
    };
    let bad = |what: &str| {
        Err(Error {
            msg: format!("CFREG corrupt ({what}): {}", path.display()),
            kind: ErrorKind::InvalidArgument,
        })
    };
    let payload = match cfreg_payload(&raw) {
        Ok(p) => p,
        Err(what) => return bad(&what),
    };
    let mut lines = payload[CFREG_MAGIC.len()..].split(|&b| b == b'\n');
    let default_raw = match lines.next() {
        Some(b"R") => true,
        Some(b"P") => false,
        _ => return bad("codec flag"),
    };
    let mut names = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        match std::str::from_utf8(line) {
            Ok(s) => names.push(s.to_string()),
            Err(_) => return bad("name not utf-8"),
        }
    }
    Ok(Some((default_raw, names)))
}

/// RFC-0060 P2.26: optional last line `c:` + 8 hex CRC32C of the prefix.
/// Legacy files without that line still load.
fn cfreg_payload(raw: &[u8]) -> std::result::Result<&[u8], String> {
    if !raw.starts_with(CFREG_MAGIC) {
        return Err("bad magic".into());
    }
    let s = std::str::from_utf8(raw).map_err(|_| "not utf-8".to_string())?;
    let trimmed = s.trim_end_matches('\n');
    if let Some((head, last)) = trimmed.rsplit_once('\n') {
        if let Some(hex) = last.strip_prefix("c:") {
            if hex.len() == 8 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                let expect = u32::from_str_radix(hex, 16).map_err(|_| "crc not hex".to_string())?;
                let payload = raw
                    .get(..head.len() + 1)
                    .ok_or_else(|| "crc payload".to_string())?;
                let got = crc32c::crc32c(payload);
                if !pedradb_core::wal::crc::crc_match_ok(expect, got) {
                    return Err(format!(
                        "crc mismatch stored={expect:#010x} computed={got:#010x}"
                    ));
                }
                return Ok(payload);
            }
        }
    }
    Ok(raw)
}

/// Persist the registry atomically (tmp + rename + dir fsync). Written
/// BEFORE the core DB opens so a crash mid-open never leaves a CF'd DB
/// without its registry.
///
/// # Errors
/// I/O.
fn store_cf_registry(
    dir: &std::path::Path,
    default_raw: bool,
    non_default: &[String],
) -> Result<()> {
    use std::io::Write as _;
    let path = cfreg_path(dir);
    let tmp = dir.join(format!("{CFREG_FILE_NAME}.tmp"));
    let mut buf = CFREG_MAGIC.to_vec();
    buf.extend_from_slice(if default_raw { b"R\n" } else { b"P\n" });
    for n in non_default {
        buf.extend_from_slice(n.as_bytes());
        buf.push(b'\n');
    }
    // RFC-0060 P2.26: CRC32C hex of the prefix (CURRENT-shaped trailer).
    let crc = crc32c::crc32c(&buf);
    buf.extend_from_slice(format!("c:{crc:08x}\n").as_bytes());
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| Error {
            msg: format!("create {}: {e}", tmp.display()),
            kind: ErrorKind::Io,
        })?;
        f.write_all(&buf).map_err(|e| Error {
            msg: format!("write {}: {e}", tmp.display()),
            kind: ErrorKind::Io,
        })?;
        f.sync_all().map_err(|e| Error {
            msg: format!("sync {}: {e}", tmp.display()),
            kind: ErrorKind::Io,
        })?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| Error {
        msg: format!("rename {}: {e}", path.display()),
        kind: ErrorKind::Io,
    })?;
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// CF↔keyspace codec. `default` is raw **only** when no named CF existed at
/// first creation; otherwise it is prefixed too, so full-CF range scans
/// never leak another CF's encoded keys. The flag is frozen in `CFREG`
/// (F185) — never recomputed from the supplied open list.
#[derive(Debug, Clone)]
pub(crate) struct KeyCodec {
    default_raw: bool,
}

impl KeyCodec {
    pub(crate) fn encode(&self, cf: &str, key: &[u8]) -> Vec<u8> {
        let enc = encode_cf_key(cf, key, self.default_raw);
        debug_assert!(
            key_in_cf_family(&enc, cf),
            "encoded key must belong to family {cf}"
        );
        enc
    }

    /// Encode without the family `debug_assert`. Iterator resume keys come
    /// from `decode`, which in a default-raw DB hands back user keys with
    /// embedded NULs — legal input that re-encodes to the exact original
    /// bytes, but that the family check (built for user-supplied names)
    /// would reject.
    pub(crate) fn encode_resume(&self, cf: &str, key: &[u8]) -> Vec<u8> {
        encode_cf_key(cf, key, self.default_raw)
    }

    /// Append `cf\\0key` onto `pool` and freeze a shared `Bytes` (one backing
    /// alloc per `write()` instead of one malloc per op).
    fn encode_pooled(&self, cf: &str, key: &[u8], pool: &mut bytes::BytesMut) -> Bytes {
        let effective = cf_encode_effective(cf, self.default_raw);
        if effective.is_empty() {
            pool.reserve(key.len());
            pool.extend_from_slice(key);
            return pool.split_to(key.len()).freeze();
        }
        let n = effective.len() + 1 + key.len();
        pool.reserve(n);
        pool.extend_from_slice(effective.as_bytes());
        pool.extend_from_slice(&[0]);
        pool.extend_from_slice(key);
        pool.split_to(n).freeze()
    }

    /// `cf\0` run prefix for [`Self::encode_run`] — fill once per same-CF
    /// run (RFC-0054 P1.4 / RFC-0149 P1.1).
    fn fill_run_prefix(&self, cf: &str, pfx: &mut Vec<u8>) {
        pfx.clear();
        let effective = cf_encode_effective(cf, self.default_raw);
        if effective.is_empty() {
            return;
        }
        pfx.extend_from_slice(effective.as_bytes());
        pfx.push(0);
    }

    /// [`Self::encode_pooled`] with a prefix from [`Self::fill_run_prefix`].
    fn encode_run(&self, prefix: &[u8], key: &[u8], pool: &mut bytes::BytesMut) -> Bytes {
        if prefix.is_empty() {
            pool.reserve(key.len());
            pool.extend_from_slice(key);
            return pool.split_to(key.len()).freeze();
        }
        let n = prefix.len() + key.len();
        pool.reserve(n);
        pool.extend_from_slice(prefix);
        pool.extend_from_slice(key);
        pool.split_to(n).freeze()
    }

    /// Default-CF raw: copy user key; otherwise `cf\\0key` via the pool.
    fn encode_owned(&self, cf: &str, key: &[u8], pool: &mut bytes::BytesMut) -> Bytes {
        if cf_encode_effective(cf, self.default_raw).is_empty() {
            Bytes::copy_from_slice(key)
        } else {
            self.encode_pooled(cf, key, pool)
        }
    }

    /// Encode into a stack buffer when the key fits (RFC-0035 P1.2).
    pub(crate) fn encode_with<R>(&self, cf: &str, key: &[u8], f: impl FnOnce(&[u8]) -> R) -> R {
        const STACK: usize = 192;
        let effective = cf_encode_effective(cf, self.default_raw);
        if effective.is_empty() {
            return f(key);
        }
        let n = effective.len() + 1 + key.len();
        if n <= STACK {
            let mut buf = [0u8; STACK];
            buf[..effective.len()].copy_from_slice(effective.as_bytes());
            buf[effective.len()] = 0;
            buf[effective.len() + 1..n].copy_from_slice(key);
            f(&buf[..n])
        } else {
            let mut v = Vec::with_capacity(n);
            v.extend_from_slice(effective.as_bytes());
            v.push(0);
            v.extend_from_slice(key);
            f(&v)
        }
    }

    fn decode<'a>(&self, cf: &str, encoded: &'a [u8]) -> &'a [u8] {
        decode_cf_key(cf, encoded, self.default_raw)
    }

    /// [`Self::decode`] on a materialized window key: the suffix slice is
    /// the same, so a `Bytes` handle decodes by re-slicing (refcount bump,
    /// no copy, no per-row allocation) — the scan page path hands out
    /// handles into the cached blocks instead of `to_vec` copies. A key
    /// shorter than the family prefix decodes to empty, matching
    /// [`decode_cf_key`]'s `unwrap_or(&[])` instead of panicking.
    /// Decode on a materialized window key by consuming the handle: the
    /// family prefix is dropped by `advance` (pointer slide — no refcount
    /// op) or the handle is returned as-is, so the scan page path pays zero
    /// refcount RMWs per row.
    fn decode_bytes_owned(&self, cf: &str, mut encoded: Bytes) -> Bytes {
        let effective = cf_encode_effective(cf, self.default_raw);
        if effective.is_empty() {
            return encoded;
        }
        if encoded.len() > effective.len() {
            use bytes::Buf as _;
            encoded.advance(effective.len() + 1);
            return encoded;
        }
        Bytes::new()
    }
}

fn bound_as_ref(b: &Bound<Vec<u8>>) -> Bound<&[u8]> {
    match b {
        Bound::Included(k) => Bound::Included(k.as_slice()),
        Bound::Excluded(k) => Bound::Excluded(k.as_slice()),
        Bound::Unbounded => Bound::Unbounded,
    }
}

/// Per-thread direct-mapped hot set (RFC-0041). Official YCSB is zipfian
/// θ=0.99 / 4096 keys / 2000 ops — a few hundred unique keys. 1024 slots
/// keep the working set so a hit skips CF-prefix encode + the point-cache
/// mutex (~the 39 ns C still needs for 2.0). 8-probe hash; named CF writes
/// also land in a 16-slot ring (raftlog idx-1). Fat apply bumps the TLS
/// epoch; a 1-key put bumps only that key's gen (RFC-0154 P1.5).
const LAST_N: usize = 4096;
/// Hash probe for LAST_GET (default CF) and LAST_CF miss after the write
/// ring. 8 is enough for zipfian; raftlog idx-1 lives in `LAST_RING`.
const LAST_PROBE: usize = 8;
/// `write_cf_owned` write-through is this ring, not the 16-probe hash
/// (p11h hashed AND ringed — extra tax, dirty min 0.843). Newest-first
/// get of idx-1 is 2 compares.
const LAST_RING: usize = 16;
const TINY: usize = 128;

fn fx_mix(hash: u64, word: u64) -> u64 {
    (hash.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95)
}

fn fx_bytes(mut hash: u64, bytes: &[u8]) -> u64 {
    let n = bytes.len();
    // YCSB keys are 11 B (`ycsb/000042`). One padded load beats the byte loop.
    if n <= 16 {
        let mut tmp = [0u8; 16];
        tmp[..n].copy_from_slice(bytes);
        hash = fx_mix(hash, u64::from_le_bytes(tmp[0..8].try_into().unwrap()));
        hash = fx_mix(hash, u64::from_le_bytes(tmp[8..16].try_into().unwrap()));
        return fx_mix(hash, n as u64);
    }
    let mut bytes = bytes;
    while bytes.len() >= 8 {
        let (chunk, rest) = bytes.split_at(8);
        hash = fx_mix(hash, u64::from_le_bytes(chunk.try_into().unwrap()));
        bytes = rest;
    }
    if bytes.len() >= 4 {
        let (chunk, rest) = bytes.split_at(4);
        hash = fx_mix(hash, u32::from_le_bytes(chunk.try_into().unwrap()) as u64);
        bytes = rest;
    }
    for &b in bytes {
        hash = fx_mix(hash, u64::from(b));
    }
    fx_mix(hash, n as u64)
}

fn last_slot(hash: u64, probe: usize) -> usize {
    (hash as usize).wrapping_add(probe) & (LAST_N - 1)
}

#[derive(Clone, Copy)]
struct TinyBuf {
    data: [u8; TINY],
    len: u8,
}

impl TinyBuf {
    fn empty() -> Self {
        Self {
            data: [0; TINY],
            len: 0,
        }
    }

    fn from_slice(s: &[u8]) -> Option<Self> {
        if s.len() > TINY {
            return None;
        }
        let mut data = [0u8; TINY];
        data[..s.len()].copy_from_slice(s);
        Some(Self {
            data,
            len: s.len() as u8,
        })
    }

    fn eq(self, s: &[u8]) -> bool {
        self.as_slice() == s
    }

    fn as_slice(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }
}

struct LastGetSlot {
    /// Fat-apply epoch (`cache_epoch_base + point_tls_epoch`). 0 = never used.
    /// 1-key puts leave this still and bump [`Self::gen`] instead (RFC-0154 P1.5).
    epoch: u64,
    /// Per-encoded-key generation. A put of this key bumps the bucket; other
    /// zipf keys keep their gen and stay cached.
    gen: u64,
    cf: TinyBuf,
    key: TinyBuf,
    val: Option<Bytes>,
}

struct LastGetTable {
    slots: Box<[LastGetSlot]>,
    /// Last `LAST_RING` named stores. LAST_GET (`get_key` / `store_key`)
    /// does not touch this.
    ring: [LastGetSlot; LAST_RING],
    ring_i: u8,
}

impl LastGetTable {
    fn empty_slot() -> LastGetSlot {
        LastGetSlot {
            epoch: 0,
            gen: 0,
            cf: TinyBuf::empty(),
            key: TinyBuf::empty(),
            val: None,
        }
    }

    fn new() -> Self {
        Self {
            // Heap — TinyBuf slots overflow the thread stack if inline.
            slots: (0..LAST_N).map(|_| Self::empty_slot()).collect(),
            ring: std::array::from_fn(|_| Self::empty_slot()),
            ring_i: 0,
        }
    }

    fn hash(cf: &str, key: &[u8]) -> u64 {
        fx_bytes(fx_bytes(0, cf.as_bytes()), key)
    }

    fn get(&self, epoch: u64, gen: u64, cf: &str, key: &[u8]) -> Option<Option<Bytes>> {
        let cf_b = cf.as_bytes();
        let n = self.ring_i as usize;
        for k in 0..LAST_RING {
            let i = (n + LAST_RING - 1 - k) % LAST_RING;
            let s = &self.ring[i];
            if s.epoch == epoch && s.gen == gen && s.cf.eq(cf_b) && s.key.eq(key) {
                return Some(s.val.clone());
            }
        }
        let h = Self::hash(cf, key);
        for p in 0..LAST_PROBE {
            let s = &self.slots[last_slot(h, p)];
            if s.epoch != epoch || s.gen != gen {
                continue;
            }
            if s.cf.eq(cf_b) && s.key.eq(key) {
                return Some(s.val.clone());
            }
        }
        None
    }

    fn store(&mut self, epoch: u64, gen: u64, cf: &str, key: &[u8], val: Option<Bytes>) {
        let Some(cf_t) = TinyBuf::from_slice(cf.as_bytes()) else {
            return;
        };
        let Some(key_t) = TinyBuf::from_slice(key) else {
            return;
        };
        // Ring only — sequential raftlog 16-puts must not also walk the
        // hash table (p11h). Get idx-1 hits newest-first. Hash stays for
        // LAST_GET (`store_key`) and named misses that still hash-store.
        self.ring[self.ring_i as usize] = LastGetSlot {
            epoch,
            gen,
            cf: cf_t,
            key: key_t,
            val,
        };
        self.ring_i = (self.ring_i + 1) % LAST_RING as u8;
    }

    /// Default-CF `get()`: hash the user key only (no `default` prefix).
    fn get_key(&self, epoch: u64, gen: u64, key: &[u8]) -> Option<Option<Bytes>> {
        let h = fx_bytes(0, key);
        for p in 0..LAST_PROBE {
            let s = &self.slots[last_slot(h, p)];
            if s.epoch != epoch || s.gen != gen {
                continue;
            }
            if s.key.eq(key) {
                return Some(s.val.clone());
            }
        }
        None
    }

    fn store_key(&mut self, epoch: u64, gen: u64, key: &[u8], val: Option<Bytes>) {
        let Some(key_t) = TinyBuf::from_slice(key) else {
            return;
        };
        let h = fx_bytes(0, key);
        let mut free = None;
        for p in 0..LAST_PROBE {
            let i = last_slot(h, p);
            let s = &mut self.slots[i];
            if s.epoch == epoch && s.key.eq(key) {
                s.gen = gen;
                s.val = val;
                return;
            }
            if s.epoch != epoch && free.is_none() {
                free = Some(i);
            }
        }
        let i = free.unwrap_or_else(|| last_slot(h, LAST_PROBE - 1));
        self.slots[i] = LastGetSlot {
            epoch,
            gen,
            cf: TinyBuf::empty(),
            key: key_t,
            val,
        };
    }

    /// Named-CF get miss: fill the 4096-slot hash. `store` is ring-only
    /// so raftlog write-through stays 16-deep (p11h). lookup_100's 100
    /// repeating keys never fit the ring, so every named get re-entered
    /// the SST (guest v64 25M get_loop 0.94×). Hash-store on the get
    /// miss keeps the working set; writes stay ring-only.
    fn hash_store(&mut self, epoch: u64, gen: u64, cf: &str, key: &[u8], val: Option<Bytes>) {
        let Some(cf_t) = TinyBuf::from_slice(cf.as_bytes()) else {
            return;
        };
        let Some(key_t) = TinyBuf::from_slice(key) else {
            return;
        };
        let h = Self::hash(cf, key);
        let mut free = None;
        let cf_b = cf.as_bytes();
        for p in 0..LAST_PROBE {
            let i = last_slot(h, p);
            let s = &mut self.slots[i];
            if s.epoch == epoch && s.cf.eq(cf_b) && s.key.eq(key) {
                s.gen = gen;
                s.val = val;
                return;
            }
            if s.epoch != epoch && free.is_none() {
                free = Some(i);
            }
        }
        let i = free.unwrap_or_else(|| last_slot(h, LAST_PROBE - 1));
        self.slots[i] = LastGetSlot {
            epoch,
            gen,
            cf: cf_t,
            key: key_t,
            val,
        };
    }
}

/// Repeat puts of the same slice (kvrocks SET / YCSB payload) share one `Bytes`
/// so TLS write-through is a refcount, not a second memcpy (RFC-0154 P1.8).
fn intern_put_value(v: &[u8]) -> Bytes {
    thread_local! {
        static LAST: RefCell<Bytes> = const { RefCell::new(Bytes::new()) };
    }
    LAST.with(|slot| {
        let mut g = slot.borrow_mut();
        if g.len() == v.len() && !g.is_empty() && g.as_ref() == v {
            g.clone()
        } else {
            let b = Bytes::copy_from_slice(v);
            *g = b.clone();
            b
        }
    })
}

thread_local! {
    /// Default-CF last-get table shared by `get()` and `contains()` —
    /// the same query, so a warm from either call site serves both.
    static LAST_GET: RefCell<LastGetTable> = RefCell::new(LastGetTable::new());
    /// Named-CF last-get. Write-through on `write_cf_owned` so raftlog
    /// `get idx-1` hits without encode+lock (RFC-0062 P0.4; diag-6e I-cache).
    static LAST_CF: RefCell<LastGetTable> = RefCell::new(LastGetTable::new());
}

struct LastCountSlot {
    occupied: bool,
    cf: TinyBuf,
    start: TinyBuf,
    end: TinyBuf,
    limit: usize,
    n: usize,
}

struct LastCountTable {
    epoch: u64,
    slots: Box<[LastCountSlot]>,
}

impl LastCountTable {
    fn new() -> Self {
        Self {
            epoch: 0,
            slots: (0..LAST_N)
                .map(|_| LastCountSlot {
                    occupied: false,
                    cf: TinyBuf::empty(),
                    start: TinyBuf::empty(),
                    end: TinyBuf::empty(),
                    limit: 0,
                    n: 0,
                })
                .collect(),
        }
    }

    fn prepare(&mut self, epoch: u64) {
        if self.epoch != epoch {
            self.epoch = epoch;
            for s in &mut self.slots {
                s.occupied = false;
            }
        }
    }

    fn hash(cf: &str, start: &[u8], end: &[u8], limit: usize) -> u64 {
        fx_mix(
            fx_bytes(fx_bytes(fx_bytes(0, cf.as_bytes()), start), end),
            limit as u64,
        )
    }

    fn get(&self, epoch: u64, cf: &str, start: &[u8], end: &[u8], limit: usize) -> Option<usize> {
        if self.epoch != epoch {
            return None;
        }
        let h = Self::hash(cf, start, end, limit);
        let cf_b = cf.as_bytes();
        for p in 0..LAST_PROBE {
            let s = &self.slots[last_slot(h, p)];
            if !s.occupied {
                return None;
            }
            if s.limit == limit && s.cf.eq(cf_b) && s.start.eq(start) && s.end.eq(end) {
                return Some(s.n);
            }
        }
        None
    }

    fn store(&mut self, epoch: u64, cf: &str, start: &[u8], end: &[u8], limit: usize, n: usize) {
        self.prepare(epoch);
        let Some(cf_t) = TinyBuf::from_slice(cf.as_bytes()) else {
            return;
        };
        let Some(start_t) = TinyBuf::from_slice(start) else {
            return;
        };
        let Some(end_t) = TinyBuf::from_slice(end) else {
            return;
        };
        let h = Self::hash(cf, start, end, limit);
        let mut empty = None;
        for p in 0..LAST_PROBE {
            let i = last_slot(h, p);
            let s = &mut self.slots[i];
            if s.occupied
                && s.limit == limit
                && s.cf.eq(cf.as_bytes())
                && s.start.eq(start)
                && s.end.eq(end)
            {
                s.n = n;
                return;
            }
            if !s.occupied && empty.is_none() {
                empty = Some(i);
            }
        }
        let i = empty.unwrap_or_else(|| last_slot(h, LAST_PROBE - 1));
        self.slots[i] = LastCountSlot {
            occupied: true,
            cf: cf_t,
            start: start_t,
            end: end_t,
            limit,
            n,
        };
    }
}

/// First encoded key strictly greater than `enc`, if any.
fn encoded_succ(enc: &[u8]) -> Option<Vec<u8>> {
    let mut e = enc.to_vec();
    for i in (0..e.len()).rev() {
        if e[i] < 0xff {
            e[i] += 1;
            e.truncate(i + 1);
            return Some(e);
        }
    }
    None
}

/// Atomic write batch (one Pedra `apply_batch` = all-or-nothing).
#[derive(Debug, Default)]
pub struct WriteBatch {
    ops: Vec<(Option<String>, BatchOp)>,
}

impl WriteBatch {
    /// Empty batch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of staged ops.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Whether the batch is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Put into the default CF.
    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) {
        self.put_cf(
            &ColumnFamily {
                name: DEFAULT_CF.into(),
            },
            key,
            value,
        );
    }

    /// Put into a named CF.
    pub fn put_cf(&mut self, cf: &ColumnFamily, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) {
        self.ops.push((
            Some(cf.name.as_ref().to_string()),
            BatchOp::Put {
                key: Bytes::copy_from_slice(key.as_ref()),
                value: Bytes::copy_from_slice(value.as_ref()),
            },
        ));
    }

    /// Delete from the default CF.
    pub fn delete(&mut self, key: impl AsRef<[u8]>) {
        self.delete_cf(
            &ColumnFamily {
                name: DEFAULT_CF.into(),
            },
            key,
        );
    }

    /// Delete from a named CF.
    pub fn delete_cf(&mut self, cf: &ColumnFamily, key: impl AsRef<[u8]>) {
        self.ops.push((
            Some(cf.name.as_ref().to_string()),
            BatchOp::Delete {
                key: Bytes::copy_from_slice(key.as_ref()),
            },
        ));
    }

    /// Range-delete `[start, end)` in a named CF.
    pub fn delete_range_cf(
        &mut self,
        cf: &ColumnFamily,
        start: impl AsRef<[u8]>,
        end: impl AsRef<[u8]>,
    ) {
        self.ops.push((
            Some(cf.name.as_ref().to_string()),
            BatchOp::DeleteRange {
                start: Bytes::copy_from_slice(start.as_ref()),
                end: Bytes::copy_from_slice(end.as_ref()),
            },
        ));
    }
}

/// Iterator direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Ascending keys.
    Forward,
    /// Descending keys.
    Reverse,
}

/// Where an iterator starts (rust-rocksdb shape).
#[derive(Debug, Clone, Copy)]
pub enum IteratorMode<'a> {
    /// First key.
    Start,
    /// Last key.
    End,
    /// From `key` in `direction`.
    From(&'a [u8], Direction),
}

/// Page size (RFC-0032 P0.1). Forward refills; never materialises the whole CF.
/// 512 amortises the per-refill layer-stream setup (tombstone collect over
/// every overlapping SST + merge heap init) over 8× more rows; long scans
/// pay it twice per 1000 rows instead of 16 times.
const ITER_WINDOW: usize = 512;

/// `PEDRA_PAGE_DIAG=1`: one aggregate line every 2048 forward refills —
/// wall ns per `page_forward` call and rows per page. With SCANDIAG (core
/// setup+rows) and the criterion op time it splits the scan op into
/// `page_forward` (lock + setup + rows + compat glue) vs the harness
/// remainder, on the machine that matters (guest cores are ~4× slower per
/// row and only a guest-side wall counter can attribute that gap).
fn page_diag_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("PEDRA_PAGE_DIAG").is_some())
}

fn page_diag_note(t0: std::time::Instant, rows: usize) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static PAGES: AtomicU64 = AtomicU64::new(0);
    static ROWS: AtomicU64 = AtomicU64::new(0);
    static NS: AtomicU64 = AtomicU64::new(0);
    static LAST_PAGES: AtomicU64 = AtomicU64::new(0);
    static LAST_ROWS: AtomicU64 = AtomicU64::new(0);
    static LAST_NS: AtomicU64 = AtomicU64::new(0);
    NS.fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
    ROWS.fetch_add(rows as u64, Ordering::Relaxed);
    let pages = PAGES.fetch_add(1, Ordering::Relaxed) + 1;
    if pages % 2048 != 0 {
        return;
    }
    let d = pages - LAST_PAGES.swap(pages, Ordering::Relaxed);
    if d == 0 {
        return;
    }
    let rows_total = ROWS.load(Ordering::Relaxed);
    let ns_total = NS.load(Ordering::Relaxed);
    let d_rows = rows_total - LAST_ROWS.swap(rows_total, Ordering::Relaxed);
    let d_ns = ns_total - LAST_NS.swap(ns_total, Ordering::Relaxed);
    println!(
        "PAGEDIAG pages={} rows/page={:.1} page_ns/page={:.0}",
        pages,
        d_rows as f64 / d as f64,
        d_ns as f64 / d as f64,
    );
}

/// Windowed CF iterator (RFC-0032 P0.1). Same positioning semantics as v0.
pub struct DBIterator<E: PedraEnv = StdEnv> {
    /// Refill pages as zero-copy `Bytes` handles sliced from the cached
    /// blocks (`page_forward`/`page_last_n`): consuming a row is a refcount
    /// bump, not the two `to_vec` allocations per row the owned-Vec page
    /// paid on the scan path.
    items: Vec<(Bytes, Bytes)>,
    idx: usize,
    reverse: bool,
    inner: ConcurrentDb<E>,
    codec: KeyCodec,
    cf: String,
    seq: pedradb_core::SequenceNumber,
    cf_start: Bound<Vec<u8>>,
    cf_end: Bound<Vec<u8>>,
    /// Encoded last / first key of the current page — the refill resume
    /// points. Kept aside from `items` because consumed slots are moved
    /// out (zero-copy handoff), not just index-past.
    resume_fwd: Vec<u8>,
    resume_rev: Vec<u8>,
    exhausted: bool,
    /// Last refill error (fix C6 hardening): a failed window refill no
    /// longer vanishes — `status()` reports it instead of a silent truncation.
    err: Option<Error>,
}

impl<E: PedraEnv> Iterator for DBIterator<E> {
    type Item = Result<(Bytes, Bytes)>;
    fn next(&mut self) -> Option<Self::Item> {
        if !self.valid() {
            return None;
        }
        // Move the handle out instead of copying through `key()`/`value()`:
        // refill pages are `Bytes` slices, so the handoff is a pointer +
        // refcount — no allocation and no byte copy per entry.
        let item = std::mem::take(&mut self.items[self.idx]);
        DBIterator::advance(self);
        Some(Ok(item))
    }
}

impl<E: PedraEnv> DBIterator<E> {
    /// Whether positioned on a valid entry.
    #[must_use]
    pub fn valid(&self) -> bool {
        self.idx < self.items.len()
    }

    /// Advance (forward or backward per mode). Stepping past either end
    /// invalidates (reverse uses wrapping so index 0 → invalid, not clamped).
    pub fn next(&mut self) {
        self.advance();
    }

    fn advance(&mut self) {
        if !self.valid() {
            return;
        }
        if self.reverse {
            if self.idx == 0 {
                self.refill_reverse();
            } else {
                self.idx -= 1;
            }
        } else {
            self.idx += 1;
            if self.idx >= self.items.len() {
                self.refill_forward();
            }
        }
    }

    /// Current user key (empty when invalid).
    #[must_use]
    pub fn key(&self) -> &[u8] {
        self.items.get(self.idx).map(|(k, _)| &k[..]).unwrap_or(&[])
    }

    /// Current value (empty when invalid).
    #[must_use]
    pub fn value(&self) -> &[u8] {
        self.items.get(self.idx).map(|(_, v)| &v[..]).unwrap_or(&[])
    }

    /// Remaining entries from here to the CF bound (refills pages).
    pub fn collect_rest(&mut self) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut out = Vec::new();
        while self.valid() {
            let (k, v) = std::mem::take(&mut self.items[self.idx]);
            out.push((k.to_vec(), v.to_vec()));
            self.next();
        }
        out
    }

    /// Last refill error, if a window refill failed (fix C6 hardening).
    pub fn status(&self) -> Result<()> {
        match &self.err {
            Some(e) => Err(Error {
                msg: e.msg.clone(),
                kind: e.kind,
            }),
            None => Ok(()),
        }
    }

    fn invalidate(&mut self) {
        self.exhausted = true;
        self.idx = if self.reverse {
            usize::MAX
        } else {
            self.items.len()
        };
    }

    fn refill_forward(&mut self) {
        if self.exhausted || self.items.is_empty() {
            self.invalidate();
            return;
        }
        let start = Bound::Excluded(std::mem::take(&mut self.resume_fwd));
        match page_forward(
            &self.inner,
            &self.codec,
            &self.cf,
            self.seq,
            start,
            bound_as_ref(&self.cf_end),
            ITER_WINDOW,
        ) {
            Ok(page) if !page.is_empty() => {
                self.set_page(page);
                self.idx = 0;
            }
            Ok(_) => self.invalidate(),
            Err(e) => {
                self.err = Some(e);
                self.invalidate();
            }
        }
    }

    fn refill_reverse(&mut self) {
        if self.exhausted || self.items.is_empty() {
            self.invalidate();
            return;
        }
        let end = Bound::Excluded(std::mem::take(&mut self.resume_rev));
        match page_last_n(
            &self.inner,
            &self.codec,
            &self.cf,
            self.seq,
            bound_as_ref(&self.cf_start),
            end,
            ITER_WINDOW,
        ) {
            Ok(page) if !page.is_empty() => {
                self.idx = page.len() - 1;
                self.set_page(page);
            }
            Ok(_) => self.invalidate(),
            Err(e) => {
                self.err = Some(e);
                self.invalidate();
            }
        }
    }

    /// Install a fresh page and record its boundary resume keys.
    fn set_page(&mut self, page: Vec<(Bytes, Bytes)>) {
        let fwd = page
            .last()
            .map(|(k, _)| self.codec.encode_resume(&self.cf, k));
        let rev = page
            .first()
            .map(|(k, _)| self.codec.encode_resume(&self.cf, k));
        if let Some(k) = fwd {
            self.resume_fwd = k;
        }
        if let Some(k) = rev {
            self.resume_rev = k;
        }
        self.items = page;
    }
}

fn page_forward<E: PedraEnv>(
    inner: &ConcurrentDb<E>,
    codec: &KeyCodec,
    cf: &str,
    seq: pedradb_core::SequenceNumber,
    start: Bound<Vec<u8>>,
    end: Bound<&[u8]>,
    limit: usize,
) -> Result<Vec<(Bytes, Bytes)>> {
    if !page_diag_enabled() {
        return page_forward_inner(inner, codec, cf, seq, start, end, limit);
    }
    let t0 = std::time::Instant::now();
    let out = page_forward_inner(inner, codec, cf, seq, start, end, limit);
    page_diag_note(t0, out.as_ref().map_or(0, |p| p.len()));
    out
}

fn page_forward_inner<E: PedraEnv>(
    inner: &ConcurrentDb<E>,
    codec: &KeyCodec,
    cf: &str,
    seq: pedradb_core::SequenceNumber,
    start: Bound<Vec<u8>>,
    end: Bound<&[u8]>,
    limit: usize,
) -> Result<Vec<(Bytes, Bytes)>> {
    let s = bound_as_ref(&start);
    inner
        .with_read(|db| {
            let mut out = Vec::with_capacity(limit);
            for row in db.try_scan_window_at(seq, s, end)? {
                if !crate::iter_kernel::iter_window_keep(row.snapshot_live) {
                    continue;
                }
                out.push((codec.decode_bytes_owned(cf, row.key), row.value));
                if out.len() >= limit {
                    break;
                }
            }
            Ok::<Vec<(Bytes, Bytes)>, CoreError>(out)
        })
        .map_err(Error::from)
}

fn page_last_n<E: PedraEnv>(
    inner: &ConcurrentDb<E>,
    codec: &KeyCodec,
    cf: &str,
    seq: pedradb_core::SequenceNumber,
    start: Bound<&[u8]>,
    end: Bound<Vec<u8>>,
    n: usize,
) -> Result<Vec<(Bytes, Bytes)>> {
    let e = bound_as_ref(&end);
    inner
        .with_read(|db| {
            // Iterator borrows the Db — consume the ring window under the guard.
            let mut ring: VecDeque<(Bytes, Bytes)> = VecDeque::with_capacity(n.saturating_add(1));
            for row in db.try_scan_window_at(seq, start, e)? {
                if !crate::iter_kernel::iter_window_keep(row.snapshot_live) {
                    continue;
                }
                if ring.len() == n {
                    ring.pop_front();
                }
                ring.push_back((codec.decode_bytes_owned(cf, row.key), row.value));
            }
            Ok::<Vec<(Bytes, Bytes)>, CoreError>(ring.into_iter().collect())
        })
        .map_err(Error::from)
}

/// Read snapshot (sequence-pinned point + iterator reads).
pub struct Snapshot<'a, E: PedraEnv = StdEnv> {
    db: &'a DB<E>,
    snap: CoreSnapshot,
    /// GC pin (fix C5/C6): a live rust-rocksdb-shaped snapshot must stay
    /// readable; `auto_reclaim` GC honours the pin until Drop.
    pin: SnapshotPin,
}

impl<E: PedraEnv> Snapshot<'_, E> {
    /// Point read pinned at the snapshot sequence.
    ///
    /// # Errors
    /// Propagates Pedra errors (I/O, snapshot-too-old after version GC).
    pub fn get(&self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        self.db.get_at(self.snap, DEFAULT_CF, key)
    }

    /// Point read on a CF pinned at the snapshot sequence.
    ///
    /// # Errors
    /// Unknown CF or Pedra errors.
    pub fn get_cf(&self, cf: &ColumnFamily, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        self.db.get_at(self.snap, &cf.name, key)
    }

    /// Iterator pinned at the snapshot sequence.
    ///
    /// # Errors
    /// Unknown CF or snapshot-too-old.
    pub fn iterator(&self, mode: IteratorMode) -> Result<DBIterator<E>> {
        self.iterator_cf(
            &ColumnFamily {
                name: DEFAULT_CF.into(),
            },
            mode,
        )
    }

    /// CF iterator pinned at the snapshot sequence.
    ///
    /// # Errors
    /// Unknown CF or snapshot-too-old.
    pub fn iterator_cf(&self, cf: &ColumnFamily, mode: IteratorMode) -> Result<DBIterator<E>> {
        let names = self.db.cf_names();
        scan_cf_at(
            &self.db.inner,
            &self.db.codec,
            &cf.name,
            mode,
            self.snap.sequence(),
            &names,
            IterBounds::none(),
        )
    }
}

impl<E: PedraEnv> Drop for Snapshot<'_, E> {
    fn drop(&mut self) {
        self.db.inner.release_snapshot_pin(self.pin);
    }
}

fn cf_bounds(codec: &KeyCodec, cf: &str) -> (Bound<Vec<u8>>, Bound<Vec<u8>>) {
    if codec.default_raw && cf == DEFAULT_CF {
        (Bound::Unbounded, Bound::Unbounded)
    } else {
        let start = Bound::Included(codec.encode(cf, &[]));
        let mut succ = codec.encode(cf, &[]);
        *succ.last_mut().expect("prefix non-empty") = 1;
        (start, Bound::Excluded(succ))
    }
}

/// rust-rocksdb iterate bounds (`ReadOptions::lower` / `.upper`, user keys).
/// `upper` is exclusive, matching rocks. `None` leaves the side unbounded.
pub(crate) struct IterBounds<'a> {
    lower: Option<&'a [u8]>,
    upper: Option<&'a [u8]>,
}

impl IterBounds<'_> {
    pub(crate) fn none() -> Self {
        Self {
            lower: None,
            upper: None,
        }
    }
}

/// Tighten a start bound (`Unbounded`/`Included`) up to `k` when `k` sorts
/// after the current bound. Both are start-side, so only `Included` competes.
fn max_included(cur: Bound<Vec<u8>>, k: &[u8]) -> Bound<Vec<u8>> {
    match cur {
        Bound::Unbounded => Bound::Included(k.to_vec()),
        Bound::Included(c) if k > c.as_slice() => Bound::Included(k.to_vec()),
        b => b,
    }
}

/// rust-rocksdb `prefix_same_as_start`: clamp a `From(prefix)` scan to
/// `[prefix, next_prefix)`. An explicit tighter `iterate_upper_bound` wins.
fn clamp_prefix_same_as_start(ro: &mut ReadOptions, mode: IteratorMode<'_>) {
    if !ro.prefix_same_as_start {
        return;
    }
    let IteratorMode::From(p, _) = mode else {
        return;
    };
    let Some(end) = prefix_exclusive_end(p) else {
        return;
    };
    match &ro.upper {
        None => ro.upper = Some(end),
        Some(u) if end.as_slice() < u.as_slice() => ro.upper = Some(end),
        _ => {}
    }
}

/// Tighten an end bound (`Unbounded`/`Excluded`) down to `k`. Both are
/// end-side, so only `Excluded` competes.
fn min_excluded(cur: Bound<Vec<u8>>, k: &[u8]) -> Bound<Vec<u8>> {
    match cur {
        Bound::Unbounded => Bound::Excluded(k.to_vec()),
        Bound::Excluded(c) if k < c.as_slice() => Bound::Excluded(k.to_vec()),
        b => b,
    }
}

pub(crate) fn scan_cf_at<E: PedraEnv>(
    inner: &ConcurrentDb<E>,
    codec: &KeyCodec,
    cf: &str,
    mode: IteratorMode,
    seq: pedradb_core::SequenceNumber,
    known: &[Arc<str>],
    bounds: IterBounds<'_>,
) -> Result<DBIterator<E>> {
    if cf != DEFAULT_CF && !known.iter().any(|c| c.as_ref() == cf) {
        return Err(Error {
            msg: format!("column family not found: {cf}"),
            kind: ErrorKind::InvalidArgument,
        });
    }
    let (mut cf_start, mut cf_end) = cf_bounds(codec, cf);
    // Encode is order-preserving, so user-key bounds clamp the encoded
    // CF range directly; refills and the core scan both stop at `cf_end`.
    if let Some(lo) = bounds.lower {
        cf_start = max_included(cf_start, &codec.encode(cf, lo));
    }
    if let Some(up) = bounds.upper {
        cf_end = min_excluded(cf_end, &codec.encode(cf, up));
    }
    let (items, idx, reverse) = match mode {
        IteratorMode::Start | IteratorMode::From(_, Direction::Forward) => {
            let user_lo = match mode {
                IteratorMode::From(k, _) => {
                    let mut lo = Bound::Included(codec.encode(cf, k));
                    if let Some(l) = bounds.lower {
                        lo = max_included(lo, &codec.encode(cf, l));
                    }
                    lo
                }
                _ => cf_start.clone(),
            };
            let page = page_forward(
                inner,
                codec,
                cf,
                seq,
                user_lo,
                bound_as_ref(&cf_end),
                ITER_WINDOW,
            )?;
            (page, 0, false)
        }
        IteratorMode::End => {
            let page = page_last_n(
                inner,
                codec,
                cf,
                seq,
                bound_as_ref(&cf_start),
                cf_end.clone(),
                ITER_WINDOW,
            )?;
            let i = page.len().saturating_sub(1);
            (page, i, true)
        }
        IteratorMode::From(k, Direction::Reverse) => {
            let enc = codec.encode(cf, k);
            let hi = match encoded_succ(&enc) {
                Some(s) => match bounds.upper {
                    Some(up) => min_excluded(Bound::Excluded(s), &codec.encode(cf, up)),
                    None => Bound::Excluded(s),
                },
                None => cf_end.clone(),
            };
            let page = page_last_n(
                inner,
                codec,
                cf,
                seq,
                bound_as_ref(&cf_start),
                hi,
                ITER_WINDOW,
            )?;
            let i = page.len().saturating_sub(1);
            (page, i, true)
        }
    };
    let exhausted = items.is_empty();
    let resume_fwd = items
        .last()
        .map(|(k, _)| codec.encode_resume(cf, k))
        .unwrap_or_default();
    let resume_rev = items
        .first()
        .map(|(k, _)| codec.encode_resume(cf, k))
        .unwrap_or_default();
    Ok(DBIterator {
        items,
        idx,
        reverse,
        inner: inner.clone(),
        codec: codec.clone(),
        cf: cf.to_string(),
        seq,
        cf_start,
        cf_end,
        resume_fwd,
        resume_rev,
        exhausted,
        err: None,
    })
}

/// rust-rocksdb-shaped database on top of a Pedra `ConcurrentDb`.
///
/// Writes join the Rocks-style write group (one leader takes the write lock
/// per group: appends + a single fdatasync + apply); reads take RwLock read
/// guards; the host compact worker reuses the core staged flush pipeline.
pub struct DB<E: PedraEnv = IoUringEnv> {
    pub(crate) inner: ConcurrentDb<E>,
    /// CF registry. Read-locked per `cf_handle`/`check_cf`: `Arc<str>` names
    /// make both the handle handout and the membership check allocation-free
    /// on the read side (writes — `create_cf`/`drop_cf` — stay exclusive).
    pub(crate) cfs: RwLock<Vec<Arc<str>>>,
    pub(crate) codec: KeyCodec,
    /// Host compact worker (RFC-0037 P2.1). None when the caller injected Env
    /// (adversarial FailingEnv stays single-threaded / deterministic).
    compact_tx: Option<SyncSender<CompactCmd>>,
    compact_thread: Option<JoinHandle<()>>,
    /// Dedicated flush thread (bounded parked memory during sustained
    /// ingest). Spawned alongside the compact worker; `None` in the same
    /// injected-Env cases.
    flush_tx: Option<SyncSender<CompactCmd>>,
    flush_thread: Option<JoinHandle<()>>,
    compact_gate: Arc<Mutex<()>>,
    /// Last [`DB::resume`] outcome after a durability fence (RFC-0047 P1.1).
    /// `Arc`-shared with the host compact worker (P1.2 auto-resume writes
    /// here too).
    fence_recovery: Arc<Mutex<Option<pedradb_core::FenceRecovery>>>,
    /// RFC-0047 P1.2: worker auto-resumes Transient-class fences.
    auto_resume_transient: bool,
    /// TLS-cache epoch base unique per instance (fix C1/C1b).
    cache_epoch_base: u64,
    compaction_filter: Option<CompactionFilterFn>,
    merge_operator: Option<MergeOperatorFn>,
    /// Serializes `merge`/`merge_cf` RMW so concurrent operands on one key
    /// are not lost (issue #2). Rocks records operands; we combine under
    /// this lock then put.
    merge_gate: Mutex<()>,
}

impl DB<IoUringEnv> {
    /// Open (create if missing) with only the default CF.
    ///
    /// # Errors
    /// Pedra open errors (lock, manifest, I/O).
    pub fn open_default(path: impl AsRef<std::path::Path>) -> Result<Self> {
        // F192: rust-rocksdb's `open_default` creates the directory
        // (`opts.create_if_missing(true)`); `Options::new()` does not.
        let mut opts = Options::new();
        opts.create_if_missing(true);
        Self::open_cf(&opts, path, &[])
    }

    /// Open (create if missing) with explicit CFs (must include `default` only
    /// implicitly — `default` always exists).
    ///
    /// # Errors
    /// Pedra open errors.
    pub fn open(opts: &Options, path: impl AsRef<std::path::Path>) -> Result<Self> {
        Self::open_cf(opts, path, &[])
    }

    /// Open with named CFs registered.
    ///
    /// # Errors
    /// Pedra open errors; duplicate CF names.
    pub fn open_cf(
        opts: &Options,
        path: impl AsRef<std::path::Path>,
        cfs: &[&str],
    ) -> Result<Self> {
        let mut db = Self::open_cf_inner(
            opts,
            path,
            cfs,
            IoUringEnv::default(),
            false,
            |dir, core_opts, env| {
                ConcurrentDb::open_with_env_bounded(dir, core_opts, env).map_err(Error::from)
            },
        )?;
        let (tx, th) = spawn_compact_worker(
            db.inner.clone(),
            Arc::clone(&db.compact_gate),
            db.auto_resume_transient,
            Arc::clone(&db.fence_recovery),
            opts.background_error_listener.clone(),
        );
        if th.is_some() {
            db.inner.set_defer_auto_compact(true);
            db.compact_tx = tx;
            db.compact_thread = th;
            let (ftx, fth) = spawn_flush_worker(db.inner.clone());
            db.flush_tx = ftx;
            db.flush_thread = fth;
        }
        Ok(db)
    }

    /// rust-rocksdb `open_cf_descriptors` (SurrealDB versioned `default` CF).
    ///
    /// # Errors
    /// Pedra open errors.
    pub fn open_cf_descriptors(
        opts: &Options,
        path: impl AsRef<std::path::Path>,
        cfs: impl IntoIterator<Item = ColumnFamilyDescriptor>,
    ) -> Result<Self> {
        let descs: Vec<ColumnFamilyDescriptor> = cfs.into_iter().collect();
        let refs: Vec<&str> = descs
            .iter()
            .map(|d| d.name.as_str())
            .filter(|n| *n != DEFAULT_CF)
            .collect();
        let db = Self::open_cf(opts, path, &refs)?;
        for d in &descs {
            if d.options.write_buffer_size > 0 {
                db.inner
                    .set_cf_write_buffer(&d.name, d.options.write_buffer_size);
            }
        }
        Ok(db)
    }
}

impl DB<StdEnv> {
    /// RFC-0058 verified profile: `StdEnv` pinned (no io_uring ring),
    /// [`pedradb_core::OpenOptions::verified`] forced (sync, strongest WAL
    /// data class, fail-closed recovery) and the lone-commit-only group
    /// pin. Perf knobs of `opts` (memtable, blob threshold, reclaim) still
    /// apply. Same host compact worker as [`DB::open_cf`].
    ///
    /// # Errors
    /// Pedra open errors; duplicate CF names.
    pub fn open_verified(
        opts: &Options,
        path: impl AsRef<std::path::Path>,
        cfs: &[&str],
    ) -> Result<Self> {
        let mut db = Self::open_cf_inner(
            opts,
            path,
            cfs,
            StdEnv::default(),
            true,
            |dir, core_opts, env| {
                ConcurrentDb::open_with_env_bounded(dir, core_opts, env).map_err(Error::from)
            },
        )?;
        let (tx, th) = spawn_compact_worker(
            db.inner.clone(),
            Arc::clone(&db.compact_gate),
            db.auto_resume_transient,
            Arc::clone(&db.fence_recovery),
            opts.background_error_listener.clone(),
        );
        if th.is_some() {
            db.inner.set_defer_auto_compact(true);
            db.compact_tx = tx;
            db.compact_thread = th;
            let (ftx, fth) = spawn_flush_worker(db.inner.clone());
            db.flush_tx = ftx;
            db.flush_thread = fth;
        }
        Ok(db)
    }
}

impl<E: PedraEnv> DB<E> {
    /// Open with an explicit [`Env`] (adversarial `FailingEnv` campaigns).
    /// The payload pool stays unarmed on this path — Rc-based fault envs are
    /// not `Send`/`Sync`; eviction needs a shareable file source.
    ///
    /// # Errors
    /// Pedra open errors; duplicate CF names.
    pub fn open_cf_with_env(
        opts: &Options,
        path: impl AsRef<std::path::Path>,
        cfs: &[&str],
        env: E,
    ) -> Result<Self> {
        Self::open_cf_inner(opts, path, cfs, env, false, |dir, core_opts, env| {
            ConcurrentDb::open_with_env(dir, core_opts, env).map_err(Error::from)
        })
    }

    fn open_cf_inner<O>(
        opts: &Options,
        path: impl AsRef<std::path::Path>,
        cfs: &[&str],
        env: E,
        verified: bool,
        open: O,
    ) -> Result<Self>
    where
        O: FnOnce(std::path::PathBuf, pedradb_core::OpenOptions, E) -> Result<ConcurrentDb<E>>,
    {
        opts.refuse_g2()?;
        let dir = path.as_ref();
        if !dir.exists() {
            if !opts.create_if_missing {
                return Err(Error {
                    msg: format!("db path missing: {}", dir.display()),
                    kind: ErrorKind::InvalidArgument,
                });
            }
            std::fs::create_dir_all(dir).map_err(|e| Error {
                msg: format!("mkdir {}: {e}", dir.display()),
                kind: ErrorKind::Io,
            })?;
        }
        let mut names = vec![DEFAULT_CF.to_string()];
        for c in cfs {
            if *c == DEFAULT_CF {
                continue;
            }
            if names.iter().any(|n| n == c) {
                return Err(Error {
                    msg: format!("duplicate column family: {c}"),
                    kind: ErrorKind::InvalidArgument,
                });
            }
            names.push((*c).to_string());
        }
        // F185: freeze the codec against the persisted registry and
        // reconcile the CF set (see [`CFREG_FILE_NAME`]). `default_raw`
        // must never flip on reopen — it decides which physical keys the
        // default CF reads.
        for n in &names {
            validate_cf_name(n)?;
        }
        let supplied: Vec<String> = names.iter().skip(1).cloned().collect();
        let (default_raw, non_default) = match load_cf_registry(dir)? {
            None => (names.len() <= 1, supplied),
            Some((frozen, persisted)) => {
                for p in &persisted {
                    if !supplied.iter().any(|s| s == p) {
                        return Err(Error {
                            msg: format!(
                                "column family not opened: {p} \
                                 (existing families must all be listed at open)"
                            ),
                            kind: ErrorKind::InvalidArgument,
                        });
                    }
                }
                // F191: with the default CF stored raw (`frozen`), default
                // reads are unbounded — adding a named CF would leak its
                // `cf\0key` entries into default scans. Refuse the schema
                // change (fail-closed) instead of serving the leak.
                if frozen {
                    for s in &supplied {
                        if !persisted.iter().any(|p| p == s) {
                            return Err(Error {
                                msg: format!(
                                    "cannot add column family {s} to a default-only DB: \
                                     default-CF keys are stored raw; create the DB with \
                                     the full column family list"
                                ),
                                kind: ErrorKind::InvalidArgument,
                            });
                        }
                    }
                }
                let mut union = persisted;
                for s in supplied {
                    if !union.iter().any(|u| *u == s) {
                        union.push(s);
                    }
                }
                (frozen, union)
            }
        };
        store_cf_registry(dir, default_raw, &non_default)?;
        let mut names = vec![DEFAULT_CF.to_string()];
        names.extend(non_default);
        // RFC-0058 verified profile: the file composition is declared, not
        // caller-negotiated — sync + strongest data class + fail-closed
        // recovery are forced. Performance knobs (memtable/auto-flush, blob
        // threshold, reclaim) stay caller's.
        let mut core_opts = if verified {
            pedradb_core::OpenOptions::verified()
        } else {
            pedradb_core::OpenOptions {
                sync: opts.sync,
                wal_full_fsync: opts.wal_full_fsync,
                wal_recovery: match opts.wal_recovery {
                    WalRecoveryMode::PointInTime => pedradb_core::WalRecovery::PointInTime,
                    WalRecoveryMode::FailClosed
                    | WalRecoveryMode::AbsoluteConsistency
                    | WalRecoveryMode::TolerateCorruptedTailRecords => {
                        pedradb_core::WalRecovery::FailClosed
                    }
                    WalRecoveryMode::SkipAnyCorruptedRecord => {
                        pedradb_core::WalRecovery::FailClosed
                    }
                },
                ..pedradb_core::OpenOptions::default()
            }
        };
        core_opts.auto_flush_bytes = if opts.write_buffer_size == 0 {
            None
        } else {
            Some(opts.write_buffer_size)
        };
        // RFC-0042 v18 mapped the Rocks block-cache knob onto whole-file
        // SST residency. Rocks caches 4 KiB blocks; slipstream's default
        // 1 GiB knob then pinned 1 GiB of 64 MiB files on the 3.9 GiB
        // guest and v57 lookup_100 regressed. Cap whole-file residency at
        // the 256 MiB default; a smaller knob still shrinks it. The decoded
        // block cache stays separately capped below.
        core_opts.sst_payload_budget_bytes = Some(
            opts.block_cache_bytes
                .unwrap_or(DEFAULT_SST_PAYLOAD_BUDGET_BYTES)
                .min(DEFAULT_SST_PAYLOAD_BUDGET_BYTES),
        );
        if opts.enable_blob_files {
            core_opts.large_value_threshold = Some(opts.min_blob_size as usize);
        }
        let db = open(dir.to_path_buf(), core_opts, env)?;
        if opts.target_file_size_base > 0 {
            db.with_write(|d| d.set_compact_target_file_bytes(opts.target_file_size_base));
        }
        if verified {
            // Lone-commit-only: every commit a single-writer critical
            // section until the group-commit kernel (RFC-0057 P2.1).
            db.pin_verified();
        }
        if opts.enable_blob_files {
            if let Some(n) = opts.blob_file_size {
                db.set_vlog_rotate_bytes(Some(n));
            }
        }
        if opts.auto_reclaim {
            db.set_auto_reclaim(true);
        }
        if let Some(n) = opts.block_cache_bytes {
            // RFC-0160 P2.3: the caller's `NewLRUCache` / `set_block_cache`
            // sizes the 4 KiB decoded-block cache (Rocks block cache), not
            // whole-file SST residency (capped above at 256 MiB). The old
            // 32 MiB decoded cap left slipstream's 1 GiB knob inert and
            // get_hit tied with Rocks at 10M+.
            db.set_block_cache_budget_bytes(n.max(1));
        }
        // Rocks parity: rust-rocksdb drops superseded versions below the
        // oldest live snapshot (`Snapshot` pins / OCC begins). This bounds
        // parked-fold memory under overwrite-heavy loads (one core of pure
        // memmove + ~100x footprint growth otherwise). Core-only users keep
        // the F20 keep-everything default.
        db.set_fold_version_gc(true);
        db.set_physical_cfs(names.clone());
        // F185: frozen flag from the registry — not derived from `names`
        // (a reopen with a different supplied list must not flip the codec).
        let codec = KeyCodec { default_raw };
        let cache_epoch_base = CACHE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) << 32;
        let registry: Vec<Arc<str>> = names.iter().map(|n| Arc::from(n.as_str())).collect();
        Ok(Self {
            inner: db,
            cfs: RwLock::new(registry),
            codec,
            compact_tx: None,
            compact_thread: None,
            flush_tx: None,
            flush_thread: None,
            compact_gate: Arc::new(Mutex::new(())),
            fence_recovery: Arc::new(Mutex::new(None)),
            auto_resume_transient: opts.auto_resume_transient,
            cache_epoch_base,
            compaction_filter: opts.compaction_filter.clone(),
            merge_operator: opts.merge_operator.clone(),
            merge_gate: Mutex::new(()),
        })
    }

    fn notify_compact(&self) {
        if let Some(tx) = &self.compact_tx {
            let _ = tx.try_send(CompactCmd::Run);
        }
    }

    /// Handle for a registered CF.
    #[must_use]
    pub fn cf_handle(&self, name: &str) -> Option<ColumnFamily> {
        self.cfs
            .read()
            .iter()
            .find(|c| c.as_ref() == name)
            .map(|n| ColumnFamily {
                name: Arc::clone(n),
            })
    }

    pub(crate) fn cf_names(&self) -> Vec<Arc<str>> {
        self.cfs.read().clone()
    }

    fn check_cf(&self, cf: &str) -> Result<()> {
        if cf == DEFAULT_CF || self.cfs.read().iter().any(|c| c.as_ref() == cf) {
            Ok(())
        } else {
            Err(Error {
                msg: format!("column family not found: {cf}"),
                kind: ErrorKind::InvalidArgument,
            })
        }
    }

    /// Fat-apply epoch + per-key gen for TLS last-get (RFC-0154 P1.5).
    fn tls_point_ids(&self, cf: &str, key: &[u8]) -> (u64, u64) {
        let epoch = self.cache_epoch_base + self.inner.point_tls_epoch();
        let effective = cf_encode_effective(cf, self.codec.default_raw);
        let gen = if effective.is_empty() {
            self.inner.key_tls_gen(key)
        } else {
            self.inner.key_tls_gen_prefixed(effective.as_bytes(), key)
        };
        (epoch, gen)
    }

    /// Put into the default CF.
    ///
    /// # Errors
    /// WAL I/O or unknown CF.
    pub fn put(&self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        let key = key.as_ref();
        let value = value.as_ref();
        let interned = intern_put_value(value);
        self.codec
            .encode_with(DEFAULT_CF, key, |enc| {
                self.inner.put(enc, interned.as_ref())
            })
            .map_err(Error::from)?;
        // Blob SET never GETs in the timed window; copying 16 KiB into TLS
        // was pure tax (RFC-0149 P2.1). Small YCSB/SET values still warm,
        // sharing the interned Bytes (RFC-0154 P1.8).
        if interned.len() <= 1024 {
            let (epoch, gen) = self.tls_point_ids(DEFAULT_CF, key);
            LAST_GET.with(|t| t.borrow_mut().store_key(epoch, gen, key, Some(interned)));
        }
        Ok(())
    }

    /// Put into a named CF.
    ///
    /// # Errors
    /// WAL I/O or unknown CF.
    pub fn put_cf(
        &self,
        cf: &ColumnFamily,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<()> {
        self.check_cf(&cf.name)?;
        let key = key.as_ref();
        let value = value.as_ref();
        let interned = intern_put_value(value);
        self.codec
            .encode_with(&cf.name, key, |enc| self.inner.put(enc, interned.as_ref()))
            .map_err(Error::from)?;
        if interned.len() <= 1024 {
            let (epoch, gen) = self.tls_point_ids(&cf.name, key);
            LAST_CF.with(|t| {
                t.borrow_mut()
                    .store(epoch, gen, &cf.name, key, Some(interned))
            });
        }
        Ok(())
    }

    /// Get from the default CF.
    ///
    /// # Errors
    /// Pedra read errors.
    pub fn get(&self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        // RFC-0041 YCSB-C: default-CF get hashes the user key only (no
        // `default` prefix / CF compare). Same bytes as `get_named`.
        let key = key.as_ref();
        let (epoch, gen) = self.tls_point_ids(DEFAULT_CF, key);
        if let Some(hit) = LAST_GET.with(|slot| slot.borrow().get_key(epoch, gen, key)) {
            return Ok(hit.map(|b| b.to_vec()));
        }
        let got = self
            .codec
            .encode_with(DEFAULT_CF, key, |enc| self.inner.get(enc));
        LAST_GET.with(|slot| slot.borrow_mut().store_key(epoch, gen, key, got.clone()));
        Ok(got.map(|b| b.to_vec()))
    }

    /// Point lookup without copying the value to `Vec` (RFC-0044 P1.3 GET).
    ///
    /// # Errors
    /// Pedra read errors.
    pub fn contains(&self, key: impl AsRef<[u8]>) -> Result<bool> {
        let key = key.as_ref();
        let (epoch, gen) = self.tls_point_ids(DEFAULT_CF, key);
        if let Some(hit) = LAST_GET.with(|slot| slot.borrow().get_key(epoch, gen, key)) {
            return Ok(hit.is_some());
        }
        let got = self
            .codec
            .encode_with(DEFAULT_CF, key, |enc| self.inner.get(enc));
        LAST_GET.with(|slot| slot.borrow_mut().store_key(epoch, gen, key, got.clone()));
        Ok(got.is_some())
    }

    /// Get from a named CF.
    ///
    /// The handle's name was validated when the handle was created, so the
    /// per-get path skips the registry check (`get_named` keeps it — it takes
    /// an arbitrary string). A stale handle used after `drop_cf` reads the
    /// dropped prefix as empty rather than erroring; rocks makes the same
    /// use-after-drop the caller's problem.
    ///
    /// # Errors
    /// Pedra read errors.
    pub fn get_cf(&self, cf: &ColumnFamily, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        self.get_cached(&cf.name, key)
    }

    /// Point get by CF name (no handle alloc). Same bytes as [`Self::get_cf`].
    ///
    /// # Errors
    /// Unknown CF or Pedra read errors.
    pub fn get_named(&self, cf: &str, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        if cf != DEFAULT_CF {
            self.check_cf(cf)?;
        }
        self.get_cached(cf, key)
    }

    /// TLS-warmed point get on a CF name already known valid.
    fn get_cached(&self, cf: &str, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        let key = key.as_ref();
        // RFC-0041 YCSB-C: zipf (θ=0.99, 4096 keys) concentrates on a hot
        // set. Direct-mapped last-N skips CF-prefix encode + point-cache
        // mutex. Bytes stay shared with the point cache; we copy into Vec
        // only for the rust-rocksdb return type. Epoch bumps on publish.
        let (epoch, gen) = self.tls_point_ids(cf, key);
        if let Some(hit) = LAST_CF.with(|slot| slot.borrow().get(epoch, gen, cf, key)) {
            return Ok(hit.map(|b| b.to_vec()));
        }
        let got = self.codec.encode_with(cf, key, |enc| self.inner.get(enc));
        LAST_CF.with(|slot| slot.borrow_mut().store(epoch, gen, cf, key, got.clone()));
        Ok(got.map(|b| b.to_vec()))
    }

    /// Test helper: named get is a LAST_CF hit (no encode / inner get).
    #[cfg(test)]
    fn last_cf_is_hot(&self, cf: &str, key: &[u8]) -> bool {
        let (epoch, gen) = self.tls_point_ids(cf, key);
        LAST_CF.with(|slot| slot.borrow().get(epoch, gen, cf, key).is_some())
    }

    /// Test helper: default-CF last-get is a TLS hit.
    #[cfg(test)]
    fn last_get_is_hot(&self, key: &[u8]) -> bool {
        let (epoch, gen) = self.tls_point_ids(DEFAULT_CF, key);
        LAST_GET.with(|slot| slot.borrow().get_key(epoch, gen, key).is_some())
    }

    fn get_at(
        &self,
        snap: CoreSnapshot,
        cf: &str,
        key: impl AsRef<[u8]>,
    ) -> Result<Option<Vec<u8>>> {
        self.check_cf(cf)?;
        let encoded = self.codec.encode(cf, key.as_ref());
        self.inner
            .with_read(|db| db.get_at(snap, &encoded).map(|v| v.map(|b| b.to_vec())))
            .map_err(Error::from)
    }

    /// Delete from the default CF.
    ///
    /// # Errors
    /// WAL I/O or unknown CF.
    pub fn delete(&self, key: impl AsRef<[u8]>) -> Result<()> {
        self.delete_cf(
            &ColumnFamily {
                name: DEFAULT_CF.into(),
            },
            key,
        )
    }

    /// Delete from a named CF.
    ///
    /// # Errors
    /// WAL I/O or unknown CF.
    pub fn delete_cf(&self, cf: &ColumnFamily, key: impl AsRef<[u8]>) -> Result<()> {
        self.check_cf(&cf.name)?;
        let encoded = self.codec.encode(&cf.name, key.as_ref());
        self.inner.delete(encoded).map_err(Error::from)
    }

    /// Range-delete `[start, end)` in a named CF.
    ///
    /// # Errors
    /// WAL I/O or unknown CF.
    pub fn delete_range_cf(
        &self,
        cf: &ColumnFamily,
        start: impl AsRef<[u8]>,
        end: impl AsRef<[u8]>,
    ) -> Result<()> {
        self.check_cf(&cf.name)?;
        let lo = self.codec.encode(&cf.name, start.as_ref());
        let hi = self.codec.encode(&cf.name, end.as_ref());
        self.inner.delete_range(lo, hi).map_err(Error::from)
    }

    /// Apply a `WriteBatch` atomically (one Pedra batch = one WAL record group).
    ///
    /// # Errors
    /// WAL I/O; nothing partially applied on error.
    pub fn write(&self, batch: &WriteBatch) -> Result<()> {
        if self.try_write_latched(batch)? {
            return Ok(());
        }
        thread_local! {
            static KEY_POOL: std::cell::RefCell<bytes::BytesMut> =
                std::cell::RefCell::new(bytes::BytesMut::with_capacity(8 * 1024));
        }
        let r = KEY_POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            // Outstanding Bytes from the last batch may still sit in the
            // memtable; reserve will allocate a fresh unique buffer then.
            let mut ops = Vec::with_capacity(batch.ops.len());
            for (cf, op) in &batch.ops {
                let name = cf.as_deref().unwrap_or(DEFAULT_CF);
                self.check_cf(name)?;
                let encoded = match op {
                    BatchOp::Put { key, value } => BatchOp::Put {
                        key: self.codec.encode_pooled(name, key, &mut pool),
                        value: value.clone(),
                    },
                    BatchOp::Delete { key } => BatchOp::Delete {
                        key: self.codec.encode_pooled(name, key, &mut pool),
                    },
                    BatchOp::DeleteRange { start, end } => BatchOp::DeleteRange {
                        start: self.codec.encode_pooled(name, start, &mut pool),
                        end: self.codec.encode_pooled(name, end, &mut pool),
                    },
                };
                ops.push(encoded);
            }
            self.inner
                .apply_batch_vec(ops)
                .map(|_| ())
                .map_err(Error::from)
        });
        r
    }

    /// Slipstream hydrate is `WriteBatch` + `write_opt`, not `write_cf_owned`.
    /// After the data family latches, skip `BatchOp` / WriteGroup for the
    /// prefix run; the meta cursor still ladders.
    fn try_write_latched(&self, batch: &WriteBatch) -> Result<bool> {
        let Some((Some(first_cf), BatchOp::Put { .. })) = batch.ops.first() else {
            return Ok(false);
        };
        if !self.inner.family_is_latched_async(first_cf) {
            return Ok(false);
        }
        let family = first_cf.as_str();
        let mut n = 0usize;
        for (cf, op) in &batch.ops {
            match (cf.as_deref(), op) {
                (Some(cf), BatchOp::Put { .. }) if cf == family => n += 1,
                _ => break,
            }
        }
        if n == 0 {
            return Ok(false);
        }
        thread_local! {
            static KEY_POOL: std::cell::RefCell<bytes::BytesMut> =
                std::cell::RefCell::new(bytes::BytesMut::with_capacity(8 * 1024));
        }
        KEY_POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            self.check_cf(family)?;
            let mut pfx = Vec::with_capacity(16);
            self.codec.fill_run_prefix(family, &mut pfx);
            let mut keys = Vec::with_capacity(n);
            let mut vals = Vec::with_capacity(n);
            for (_, op) in batch.ops.iter().take(n) {
                if let BatchOp::Put { key, value } = op {
                    keys.push(self.codec.encode_run(&pfx, key.as_ref(), &mut pool));
                    vals.push(value.clone());
                }
            }
            let mut tail = Vec::with_capacity(batch.ops.len().saturating_sub(n));
            let mut last_ok: Option<&str> = None;
            let mut pfx2 = Vec::with_capacity(16);
            for (cf, op) in batch.ops.iter().skip(n) {
                let name = cf.as_deref().unwrap_or(DEFAULT_CF);
                if last_ok != Some(name) {
                    self.check_cf(name)?;
                    self.codec.fill_run_prefix(name, &mut pfx2);
                    last_ok = Some(name);
                }
                tail.push(match op {
                    BatchOp::Put { key, value } => BatchOp::Put {
                        key: self.codec.encode_run(&pfx2, key.as_ref(), &mut pool),
                        value: value.clone(),
                    },
                    BatchOp::Delete { key } => BatchOp::Delete {
                        key: self.codec.encode_run(&pfx2, key.as_ref(), &mut pool),
                    },
                    BatchOp::DeleteRange { start, end } => BatchOp::DeleteRange {
                        start: self.codec.encode_run(&pfx2, start.as_ref(), &mut pool),
                        end: self.codec.encode_run(&pfx2, end.as_ref(), &mut pool),
                    },
                });
            }
            self.inner
                .apply_latched_bulk(family, keys, vals, tail)
                .map(|_| ())
                .map_err(Error::from)
        })?;
        Ok(true)
    }

    /// Consume a [`WriteBatch`] so values move into the WAL encode (RFC-0041:
    /// `write(&batch)` cloned every 1 KiB payload; apply/raftlog is 16–64 ops).
    ///
    /// # Errors
    /// Unknown CF or WAL I/O.
    pub fn write_owned(&self, batch: WriteBatch) -> Result<()> {
        thread_local! {
            static KEY_POOL: std::cell::RefCell<bytes::BytesMut> =
                std::cell::RefCell::new(bytes::BytesMut::with_capacity(8 * 1024));
        }
        let r = KEY_POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            let mut ops = Vec::with_capacity(batch.ops.len());
            for (cf, op) in batch.ops {
                let name = cf.as_deref().unwrap_or(DEFAULT_CF);
                self.check_cf(name)?;
                let encoded = match op {
                    BatchOp::Put { key, value } => BatchOp::Put {
                        key: self.codec.encode_pooled(name, key.as_ref(), &mut pool),
                        value,
                    },
                    BatchOp::Delete { key } => BatchOp::Delete {
                        key: self.codec.encode_pooled(name, key.as_ref(), &mut pool),
                    },
                    BatchOp::DeleteRange { start, end } => BatchOp::DeleteRange {
                        start: self.codec.encode_pooled(name, start.as_ref(), &mut pool),
                        end: self.codec.encode_pooled(name, end.as_ref(), &mut pool),
                    },
                };
                ops.push(encoded);
            }
            self.inner
                .apply_batch_vec(ops)
                .map(|_| ())
                .map_err(Error::from)
        });
        r
    }

    /// One atomic multi-CF write from raw slices (RFC-0041 apply/raftlog):
    /// no `WriteBatch` handle/`String` per op and no extra key `Bytes` copy.
    ///
    /// `puts` are `(cf, key, value)`; `deletes` are `(cf, key)`.
    ///
    /// # Errors
    /// Unknown CF or WAL I/O.
    pub fn write_cf_slices(
        &self,
        puts: &[(&str, &[u8], &[u8])],
        deletes: &[(&str, &[u8])],
    ) -> Result<()> {
        thread_local! {
            static KEY_POOL: std::cell::RefCell<bytes::BytesMut> =
                std::cell::RefCell::new(bytes::BytesMut::with_capacity(8 * 1024));
        }
        let r = KEY_POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            let mut ops = Vec::with_capacity(puts.len() + deletes.len());
            let mut last_ok: Option<&str> = None;
            for (cf, k, v) in puts {
                if last_ok != Some(*cf) {
                    self.check_cf(cf)?;
                    last_ok = Some(*cf);
                }
                ops.push(BatchOp::Put {
                    key: self.codec.encode_pooled(cf, k, &mut pool),
                    value: Bytes::copy_from_slice(v),
                });
            }
            for (cf, k) in deletes {
                if last_ok != Some(*cf) {
                    self.check_cf(cf)?;
                    last_ok = Some(*cf);
                }
                ops.push(BatchOp::Delete {
                    key: self.codec.encode_pooled(cf, k, &mut pool),
                });
            }
            if ops.is_empty() {
                return Ok(());
            }
            self.inner
                .apply_batch_vec(ops)
                .map(|_| ())
                .map_err(Error::from)
        });
        r
    }

    fn cf_bucket(cf: &str) -> usize {
        match cf {
            "default" => 0,
            "lock" => 1,
            "write" => 2,
            "raftlog" => 3,
            _ => 4,
        }
    }

    fn already_single_cf_puts(puts: &[(&str, Vec<u8>, Vec<u8>)]) -> bool {
        match puts.split_first() {
            None | Some((_, [])) => true,
            Some((first, rest)) => rest.iter().all(|p| p.0 == first.0),
        }
    }

    fn already_single_cf_deletes(deletes: &[(&str, Vec<u8>)]) -> bool {
        match deletes.split_first() {
            None | Some((_, [])) => true,
            Some((first, rest)) => rest.iter().all(|p| p.0 == first.0),
        }
    }

    fn group_cf_puts(puts: Vec<(&str, Vec<u8>, Vec<u8>)>) -> Vec<(&str, Vec<u8>, Vec<u8>)> {
        let mut b: [Vec<(&str, Vec<u8>, Vec<u8>)>; 5] =
            [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for p in puts {
            let i = Self::cf_bucket(p.0);
            b[i].push(p);
        }
        let mut out = Vec::new();
        for g in b {
            out.extend(g);
        }
        out
    }

    fn group_cf_deletes(deletes: Vec<(&str, Vec<u8>)>) -> Vec<(&str, Vec<u8>)> {
        let mut b: [Vec<(&str, Vec<u8>)>; 5] =
            [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for p in deletes {
            let i = Self::cf_bucket(p.0);
            b[i].push(p);
        }
        let mut out = Vec::new();
        for g in b {
            out.extend(g);
        }
        out
    }

    /// Like [`Self::write_cf_slices`] but values (and user keys) move into
    /// `Bytes` — no extra 1 KiB payload copy per apply/raftlog op (RFC-0041).
    ///
    /// # Errors
    /// Unknown CF or WAL I/O.
    pub fn write_cf_owned(
        &self,
        mut puts: Vec<(&str, Vec<u8>, Vec<u8>)>,
        mut deletes: Vec<(&str, Vec<u8>)>,
    ) -> Result<()> {
        // Apply prewrite interleaves lock+default 32×; grouping makes
        // `fill_run_prefix` once per family (RFC-0149 P1.1). Distinct keys
        // — seq order across CFs is not user-visible after one publish.
        // One-pass CF buckets — `sort_by` swapped 100 B payloads O(n log n)
        // on apply (RFC-0149 P2.1). Encode order is still grouped by family.
        if puts.len() > 1 && !Self::already_single_cf_puts(&puts) {
            puts = Self::group_cf_puts(puts);
        }
        if deletes.len() > 1 && !Self::already_single_cf_deletes(&deletes) {
            deletes = Self::group_cf_deletes(deletes);
        }
        // Raftlog reads idx-1 of a 16-append (LAST_RING). Fat apply/lock
        // batches never read-your-writes in the same op — skip the warm Vec.
        let need_warm = puts.len() + deletes.len() <= LAST_RING
            && deletes.is_empty()
            && puts.iter().all(|(cf, _, _)| *cf == "raftlog");
        // RFC-0159 P1.5: latched first-CF run skips BatchOp / WriteGroup.
        // Hydrate is 1024 data + 1 meta; only `data` latches.
        if deletes.is_empty() && !puts.is_empty() {
            let family = puts[0].0;
            if self.inner.family_is_latched_async(family) {
                let n = puts.iter().take_while(|p| p.0 == family).count();
                return self.write_latched_cf_owned(family, n, puts, need_warm);
            }
        }
        thread_local! {
            static KEY_POOL: std::cell::RefCell<bytes::BytesMut> =
                std::cell::RefCell::new(bytes::BytesMut::with_capacity(8 * 1024));
        }
        let mut warm: Vec<(&str, Vec<u8>, Option<Bytes>)> = if need_warm {
            Vec::with_capacity(puts.len())
        } else {
            Vec::new()
        };
        let r = KEY_POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            let mut ops = Vec::with_capacity(puts.len() + deletes.len());
            let mut last_ok: Option<&str> = None;
            let mut pfx = Vec::with_capacity(16);
            let mut prev_val: Option<Bytes> = None;
            for (cf, k, v) in puts {
                if last_ok != Some(cf) {
                    self.check_cf(cf)?;
                    self.codec.fill_run_prefix(cf, &mut pfx);
                    last_ok = Some(cf);
                }
                // RFC-0062 P1.1: raftlog 16× same yval. Share the Bytes so
                // WAL v2 intern fires (prepare also shares by content).
                let val = match prev_val.as_ref() {
                    Some(p) if p.as_ref() == v.as_slice() => p.clone(),
                    _ => Bytes::from(v),
                };
                prev_val = Some(val.clone());
                ops.push(BatchOp::Put {
                    key: self.codec.encode_run(&pfx, k.as_ref(), &mut pool),
                    value: val.clone(),
                });
                if need_warm {
                    warm.push((cf, k, Some(val)));
                }
            }
            for (cf, k) in deletes {
                if last_ok != Some(cf) {
                    self.check_cf(cf)?;
                    self.codec.fill_run_prefix(cf, &mut pfx);
                    last_ok = Some(cf);
                }
                ops.push(BatchOp::Delete {
                    key: self.codec.encode_run(&pfx, k.as_ref(), &mut pool),
                });
            }
            if ops.is_empty() {
                return Ok(());
            }
            self.inner
                .apply_batch_vec(ops)
                .map(|_| ())
                .map_err(Error::from)
        });
        if r.is_ok() && !warm.is_empty() {
            // Raftlog reads idx-1 of a 16-append (LAST_RING). Fat apply/lock
            // batches never read-your-writes in the same op.
            let skip = warm.len().saturating_sub(LAST_RING);
            LAST_CF.with(|t| {
                let mut t = t.borrow_mut();
                for (cf, k, v) in warm.into_iter().skip(skip) {
                    let (epoch, gen) = self.tls_point_ids(cf, &k);
                    t.store(epoch, gen, cf, &k, v);
                }
            });
        }
        r
    }

    /// Latched first-CF run: intern the value once, skip `BatchOp` for the
    /// span, ladder the remainder (hydrate meta cursor).
    fn write_latched_cf_owned(
        &self,
        family: &str,
        n: usize,
        puts: Vec<(&str, Vec<u8>, Vec<u8>)>,
        need_warm: bool,
    ) -> Result<()> {
        thread_local! {
            static KEY_POOL: std::cell::RefCell<bytes::BytesMut> =
                std::cell::RefCell::new(bytes::BytesMut::with_capacity(8 * 1024));
        }
        let mut warm: Vec<(&str, Vec<u8>, Option<Bytes>)> = if need_warm {
            Vec::with_capacity(n)
        } else {
            Vec::new()
        };
        let r = KEY_POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            self.check_cf(family)?;
            let mut pfx = Vec::with_capacity(16);
            self.codec.fill_run_prefix(family, &mut pfx);
            let mut keys = Vec::with_capacity(n);
            let mut vals = Vec::with_capacity(n);
            let mut prev_val: Option<Bytes> = None;
            let mut rest: Vec<(&str, Vec<u8>, Vec<u8>)> = Vec::new();
            for (i, (cf, k, v)) in puts.into_iter().enumerate() {
                if i < n {
                    let val = match prev_val.as_ref() {
                        Some(p) if p.as_ref() == v.as_slice() => p.clone(),
                        _ => Bytes::from(v),
                    };
                    prev_val = Some(val.clone());
                    keys.push(self.codec.encode_run(&pfx, k.as_ref(), &mut pool));
                    if need_warm {
                        warm.push((cf, k, Some(val.clone())));
                    }
                    vals.push(val);
                } else {
                    rest.push((cf, k, v));
                }
            }
            let mut tail = Vec::with_capacity(rest.len());
            let mut last_ok: Option<&str> = None;
            let mut pfx2 = Vec::with_capacity(16);
            for (cf, k, v) in rest {
                if last_ok != Some(cf) {
                    self.check_cf(cf)?;
                    self.codec.fill_run_prefix(cf, &mut pfx2);
                    last_ok = Some(cf);
                }
                tail.push(BatchOp::Put {
                    key: self.codec.encode_run(&pfx2, k.as_ref(), &mut pool),
                    value: Bytes::from(v),
                });
            }
            self.inner
                .apply_latched_bulk(family, keys, vals, tail)
                .map(|_| ())
                .map_err(Error::from)
        });
        if r.is_ok() && !warm.is_empty() {
            let skip = warm.len().saturating_sub(LAST_RING);
            LAST_CF.with(|t| {
                let mut t = t.borrow_mut();
                for (cf, k, v) in warm.into_iter().skip(skip) {
                    let (epoch, gen) = self.tls_point_ids(cf, &k);
                    t.store(epoch, gen, cf, &k, v);
                }
            });
        }
        r
    }

    /// N puts of the same payload: one `Bytes` allocation, N refcount clones
    /// (RFC-0044 P1.1 pipeline).
    ///
    /// # Errors
    /// Unknown CF or WAL I/O.
    pub fn put_batch_same(&self, cf: &str, keys: &[Vec<u8>], v: &[u8]) -> Result<()> {
        self.check_cf(cf)?;
        let val = Bytes::copy_from_slice(v);
        thread_local! {
            static KEY_POOL: std::cell::RefCell<bytes::BytesMut> =
                std::cell::RefCell::new(bytes::BytesMut::with_capacity(8 * 1024));
        }
        KEY_POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            let mut ops = Vec::with_capacity(keys.len());
            for k in keys {
                ops.push(BatchOp::Put {
                    key: self.codec.encode_owned(cf, k.as_slice(), &mut pool),
                    value: val.clone(),
                });
            }
            self.inner
                .apply_batch_vec(ops)
                .map(|_| ())
                .map_err(Error::from)
        })
    }

    /// Sequence-pinned snapshot (fix C5/C6: registers a GC pin, released on
    /// Drop — a live snapshot stays readable under `auto_reclaim`, matching
    /// rust-rocksdb where a snapshot is valid until dropped).
    #[must_use]
    pub fn snapshot(&self) -> Snapshot<'_, E> {
        let snap = self.inner.snapshot();
        let pin = self.inner.pin_snapshot();
        Snapshot {
            db: self,
            snap,
            pin,
        }
    }

    /// rust-rocksdb `OptimisticTransactionDB::transaction` shape (RFC-0043 P2.4).
    /// Pedra [`pedradb_core::OccTransaction`]: snapshot isolation + write-set
    /// conflict at commit. Durability follows [`Options::sync`] (drop-in
    /// default false, RFC-0054). Per-txn `WriteOptions.sync` is accepted
    /// and ignored (SurrealDB sets `sync=false` on the txn).
    #[must_use]
    pub fn transaction(&self) -> Transaction<'_, E> {
        Transaction::new(self)
    }

    /// Same as [`Self::transaction`]; options are accepted for API shape.
    #[must_use]
    pub fn transaction_opt(
        &self,
        writeopts: &WriteOptions,
        otxn_opts: &OptimisticTransactionOptions,
    ) -> Transaction<'_, E> {
        let _ = (writeopts, otxn_opts);
        self.transaction()
    }

    /// Iterator over the default CF at the latest sequence.
    ///
    /// # Errors
    /// Pedra scan errors.
    pub fn iterator(&self, mode: IteratorMode) -> Result<DBIterator<E>> {
        self.iterator_cf(
            &ColumnFamily {
                name: DEFAULT_CF.into(),
            },
            mode,
        )
    }

    /// Iterator over a named CF at the latest sequence.
    ///
    /// # Errors
    /// Unknown CF or Pedra scan errors.
    pub fn iterator_cf(&self, cf: &ColumnFamily, mode: IteratorMode) -> Result<DBIterator<E>> {
        let seq = self.inner.visible_sequence();
        let names = self.cf_names();
        scan_cf_at(
            &self.inner,
            &self.codec,
            &cf.name,
            mode,
            seq,
            &names,
            IterBounds::none(),
        )
    }

    /// Last user key in `cf` that starts with `prefix` (RFC-0033).
    ///
    /// Same visibility as `get` (newest live version, no tombstones). Does not
    /// walk the prefix. WAL / fencing / accept-set unchanged.
    ///
    /// # Errors
    /// Unknown CF or Pedra read errors.
    pub fn last_key_with_prefix(
        &self,
        cf: &ColumnFamily,
        prefix: impl AsRef<[u8]>,
    ) -> Result<Option<Vec<u8>>> {
        self.last_key_named(&cf.name, prefix)
    }

    /// [`Self::last_key_with_prefix`] by CF name (no handle alloc).
    ///
    /// # Errors
    /// Unknown CF or Pedra read errors.
    pub fn last_key_named(&self, cf: &str, prefix: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        self.check_cf(cf)?;
        let encoded = self.codec.encode(cf, prefix.as_ref());
        self.inner
            .with_read(|db| {
                let seq = db.visible_sequence();
                db.last_under_user_prefix(seq, &encoded)
                    .map(|k| k.map(|k| self.codec.decode(cf, &k).to_vec()))
            })
            .map_err(Error::from)
    }

    /// Latest key under `prefix` in `last_cf`, then point-get that user key in
    /// `get_cf` (RFC-0035 P1.1). One mutex — same visibility as the two calls.
    ///
    /// # Errors
    /// Unknown CF or Pedra read errors.
    pub fn last_prefix_then_get(
        &self,
        last_cf: &ColumnFamily,
        prefix: impl AsRef<[u8]>,
        get_cf: &ColumnFamily,
    ) -> Result<Option<Vec<u8>>> {
        self.check_cf(&last_cf.name)?;
        self.check_cf(&get_cf.name)?;
        let t_enc0 = Instant::now();
        self.codec
            .encode_with(&last_cf.name, prefix.as_ref(), |enc| {
                let ns_enc0 = u64::try_from(t_enc0.elapsed().as_nanos()).unwrap_or(u64::MAX);
                self.inner.with_read(|db| {
                    let seq = db.visible_sequence();
                    let t_last = Instant::now();
                    let Some(k) = db.last_under_user_prefix(seq, enc)? else {
                        return Ok(None);
                    };
                    let ns_last = u64::try_from(t_last.elapsed().as_nanos()).unwrap_or(u64::MAX);
                    let user = self.codec.decode(&last_cf.name, &k);
                    let t_enc1 = Instant::now();
                    self.codec.encode_with(&get_cf.name, user, |gk| {
                        let ns_enc = ns_enc0.saturating_add(
                            u64::try_from(t_enc1.elapsed().as_nanos()).unwrap_or(u64::MAX),
                        );
                        let t_get = Instant::now();
                        let got = db.get(gk);
                        let ns_get = u64::try_from(t_get.elapsed().as_nanos()).unwrap_or(u64::MAX);
                        let t_copy = Instant::now();
                        let out = got.map(|b| b.to_vec());
                        let ns_copy =
                            u64::try_from(t_copy.elapsed().as_nanos()).unwrap_or(u64::MAX);
                        db.record_mvcc_split(ns_enc, ns_last, ns_get, ns_copy);
                        Ok(out)
                    })
                })
            })
    }

    /// Count live keys in `[start, end)` in `cf`, stopping at `limit` (RFC-0033).
    ///
    /// Key-only projection: same visibility as a forward iterator, no value
    /// resolve. Used by deps_scan; does not change iterator value semantics.
    ///
    /// # Errors
    /// Unknown CF or Pedra scan errors.
    pub fn count_cf(
        &self,
        cf: &ColumnFamily,
        start: impl AsRef<[u8]>,
        end: impl AsRef<[u8]>,
        limit: usize,
    ) -> Result<usize> {
        self.count_named(&cf.name, start, end, limit)
    }

    /// [`Self::count_cf`] by CF name (no handle alloc; RFC-0041 `deps_scan`).
    ///
    /// # Errors
    /// Unknown CF or Pedra scan errors.
    pub fn count_named(
        &self,
        cf: &str,
        start: impl AsRef<[u8]>,
        end: impl AsRef<[u8]>,
        limit: usize,
    ) -> Result<usize> {
        let start = start.as_ref();
        let end = end.as_ref();
        // RFC-0041 `deps_scan`: zipf windows concentrate on a hot set.
        // Last-N skips CF-prefix encode + count-cache mutex. Epoch bumps
        // on publish so a put cannot leave a stale count.
        thread_local! {
            static LAST: RefCell<LastCountTable> = RefCell::new(LastCountTable::new());
        }
        let epoch = self.cache_epoch_base + self.inner.read_cache_epoch();
        if let Some(n) = LAST.with(|slot| slot.borrow().get(epoch, cf, start, end, limit)) {
            return Ok(n);
        }
        if cf != DEFAULT_CF {
            self.check_cf(cf)?;
        }
        let n = self.codec.encode_with(cf, start, |lo| {
            self.codec.encode_with(cf, end, |hi| {
                self.inner
                    .count_in_range(Bound::Included(lo), Bound::Excluded(hi), Some(limit))
                    .map_err(Error::from)
            })
        })?;
        LAST.with(|slot| slot.borrow_mut().store(epoch, cf, start, end, limit, n));
        Ok(n)
    }

    /// Zero latest/scan probe counters (RFC-0035).
    pub fn reset_read_probe(&self) {
        self.inner.with_read(|db| db.reset_read_probe());
    }

    /// Version-GC watermark; advances when reclaim GC drops versions
    /// (see [`pedradb_core::Db::earliest_readable_sequence`]).
    #[must_use]
    pub fn earliest_readable_sequence(&self) -> pedradb_core::SequenceNumber {
        self.inner.earliest_readable_sequence()
    }

    /// Snapshot latest/scan counters + LSM shape (RFC-0035).
    #[must_use]
    pub fn read_probe(&self) -> pedradb_core::ReadProbeSnap {
        self.inner.with_read(|db| db.read_probe())
    }

    /// Write-group diagnostics (RFC-0040 P1.2): submits / queued / groups / ops.
    #[must_use]
    pub fn write_group_stats(&self) -> (u64, u64, u64, u64) {
        self.inner.write_group_stats()
    }

    /// Whether the verified group policy is pinned (RFC-0058:
    /// [`DB::open_verified`] lone-commit-only).
    #[must_use]
    pub fn is_verified(&self) -> bool {
        self.inner.is_verified()
    }

    /// Kernel [`pedradb_core::DbStats`] (mem/SST counters). RFC-0054 probe.
    #[must_use]
    pub fn stats(&self) -> pedradb_core::DbStats {
        self.inner.stats()
    }

    /// Write-phase timers when `PEDRA_WRITE_PHASE_STATS=1` at open.
    #[must_use]
    pub fn write_phase_stats(&self) -> Option<std::sync::Arc<pedradb_core::WritePhaseStats>> {
        self.inner.write_phase_stats()
    }

    /// Fold the memtable tail (no SST). See [`pedradb_core::Db::fold_active_tail`].
    pub fn fold_mem_tail(&self) -> usize {
        self.inner.fold_mem_tail()
    }

    /// Toggle default WAL barrier. Drop-in default is false (RFC-0054).
    pub fn set_write_sync(&self, sync: bool) {
        self.inner.set_default_write_sync(sync);
    }

    /// Whether puts `fdatasync` before Ok.
    #[must_use]
    pub fn write_sync(&self) -> bool {
        self.inner.default_write_sync()
    }

    /// Flush memtable to SST (staged pipeline; SST I/O off the write lock).
    ///
    /// # Errors
    /// Pedra flush errors (I/O).
    pub fn flush(&self) -> Result<()> {
        // Serialize with the host L0 worker: compact deletes retired files
        // and must not race an in-flight L0 install (ENOENT on put/flush).
        let _gate = self.compact_gate.lock();
        let r = self.inner.flush().map_err(Error::from);
        drop(_gate);
        self.notify_compact();
        r
    }

    /// Manual compaction (whole merge). Runs a compaction filter first if set.
    ///
    /// # Errors
    /// Pedra compaction / write errors.
    pub fn compact(&self) -> Result<()> {
        self.apply_compaction_filter()?;
        let _gate = self.compact_gate.lock();
        self.inner.compact().map_err(Error::from)
    }

    /// Compact after applying `filter` once (RFC-0043 P2.7). Same decisions
    /// as [`Options::set_compaction_filter`]: Keep / Remove / Change.
    ///
    /// # Errors
    /// Pedra compaction / write errors.
    pub fn compact_with_filter<F>(&self, mut filter: F) -> Result<()>
    where
        F: FnMut(u32, &[u8], &[u8]) -> CompactionDecision,
    {
        let names = self.cf_names();
        for name in names {
            let cf = ColumnFamily { name: name.clone() };
            let mut it = self.iterator_cf(&cf, IteratorMode::Start)?;
            let mut items = Vec::new();
            while it.valid() {
                items.push((it.key().to_vec(), it.value().to_vec()));
                it.next();
            }
            for (k, v) in items {
                match filter(0, &k, &v) {
                    CompactionDecision::Keep => {}
                    CompactionDecision::Remove => self.delete_cf(&cf, k)?,
                    CompactionDecision::Change(nv) => self.put_cf(&cf, k, nv)?,
                }
            }
        }
        let _gate = self.compact_gate.lock();
        self.inner.compact().map_err(Error::from)
    }

    fn apply_compaction_filter(&self) -> Result<()> {
        let Some(filter) = &self.compaction_filter else {
            return Ok(());
        };
        let names = self.cf_names();
        for name in names {
            let cf = ColumnFamily { name: name.clone() };
            let mut it = self.iterator_cf(&cf, IteratorMode::Start)?;
            let mut items = Vec::new();
            while it.valid() {
                items.push((it.key().to_vec(), it.value().to_vec()));
                it.next();
            }
            for (k, v) in items {
                let decision = {
                    let mut f = filter.lock();
                    f(0, &k, &v)
                };
                match decision {
                    CompactionDecision::Keep => {}
                    CompactionDecision::Remove => self.delete_cf(&cf, k)?,
                    CompactionDecision::Change(nv) => self.put_cf(&cf, k, nv)?,
                }
            }
        }
        Ok(())
    }

    /// rust-rocksdb raw iterator (SurrealDB scan / count). An attached
    /// `ReadOptions::set_snapshot` pins the reads at that sequence (F180);
    /// without one it reads the latest visible sequence.
    #[must_use]
    pub fn raw_iterator_opt(&self, ro: ReadOptions) -> DBRawIteratorWithThreadMode<'_, Self, E> {
        let seq = ro.snap.unwrap_or_else(|| self.inner.visible_sequence());
        DBRawIteratorWithThreadMode::open(self, seq, &ro)
    }

    /// rust-rocksdb property. Unknown names → `Ok(None)`.
    ///
    /// Mapped from Pedra [`pedradb_core::DbStats`] (RFC-0050 P0.6): estimate-num-keys,
    /// SST bytes, memtable bytes, `num-files-at-levelN`. Not a Rocks ticker dump.
    pub fn property_int_value(&self, name: impl AsRef<str>) -> Result<Option<u64>> {
        Ok(map_property_int(&self.inner, name.as_ref()))
    }

    /// rust-rocksdb `ingest_external_file` (default CF).
    ///
    /// Loads Pedra SSTs produced by [`SstFileWriter`]. Keys are written through
    /// the WAL (G1) then flushed.
    pub fn ingest_external_file<P: AsRef<std::path::Path>>(&self, paths: Vec<P>) -> Result<()> {
        self.ingest_external_file_opts(&IngestExternalFileOptions::default(), paths)
    }

    /// Ingest with options.
    pub fn ingest_external_file_opts<P: AsRef<std::path::Path>>(
        &self,
        opts: &IngestExternalFileOptions,
        paths: Vec<P>,
    ) -> Result<()> {
        let cf = ColumnFamily {
            name: DEFAULT_CF.into(),
        };
        self.ingest_external_file_cf_opts(&cf, opts, paths)
    }

    /// rust-rocksdb `ingest_external_file_cf`.
    pub fn ingest_external_file_cf<P: AsRef<std::path::Path>>(
        &self,
        cf: &ColumnFamily,
        paths: Vec<P>,
    ) -> Result<()> {
        self.ingest_external_file_cf_opts(cf, &IngestExternalFileOptions::default(), paths)
    }

    /// Ingest into a CF with options.
    pub fn ingest_external_file_cf_opts<P: AsRef<std::path::Path>>(
        &self,
        cf: &ColumnFamily,
        opts: &IngestExternalFileOptions,
        paths: Vec<P>,
    ) -> Result<()> {
        self.check_cf(&cf.name)?;
        for p in paths {
            let path = p.as_ref();
            let table = crate::api::open_writer_sst(path)?;
            let mut batch = WriteBatch::new();
            for (ikey, val) in table.iter_internal() {
                match ikey.kind {
                    pedradb_core::ValueType::Value => {
                        batch.put_cf(cf, ikey.user_key.as_ref(), val.as_ref());
                    }
                    pedradb_core::ValueType::Deletion => {
                        batch.delete_cf(cf, ikey.user_key.as_ref());
                    }
                    pedradb_core::ValueType::RangeDeletion => {
                        batch.delete_range_cf(cf, ikey.user_key.as_ref(), val.as_ref());
                    }
                }
            }
            self.write(&batch)?;
            if opts.move_files {
                let _ = std::fs::remove_file(path);
            }
        }
        self.flush()
    }

    /// rust-rocksdb `delete_file_in_range`: keys in `[from, to)` become
    /// invisible. Implemented as `delete_range` + flush + compact (tombstones,
    /// never a silent SST unlink).
    pub fn delete_file_in_range<K: AsRef<[u8]>>(&self, from: K, to: K) -> Result<()> {
        let cf = ColumnFamily {
            name: DEFAULT_CF.into(),
        };
        self.delete_file_in_range_cf(&cf, from, to)
    }

    /// rust-rocksdb plural alias.
    pub fn delete_files_in_range<K: AsRef<[u8]>>(&self, from: K, to: K) -> Result<()> {
        self.delete_file_in_range(from, to)
    }

    /// CF variant.
    pub fn delete_file_in_range_cf<K: AsRef<[u8]>>(
        &self,
        cf: &ColumnFamily,
        from: K,
        to: K,
    ) -> Result<()> {
        self.delete_range_cf(cf, from, to)?;
        self.flush()?;
        self.compact()
    }

    /// CF plural alias.
    pub fn delete_files_in_range_cf<K: AsRef<[u8]>>(
        &self,
        cf: &ColumnFamily,
        from: K,
        to: K,
    ) -> Result<()> {
        self.delete_file_in_range_cf(cf, from, to)
    }

    /// rust-rocksdb `flush_opt` (wait flag ignored: flush is synchronous).
    pub fn flush_opt(&self, _opts: &FlushOptions) -> Result<()> {
        self.flush()
    }

    /// RFC-0047 P0.2: what the last open discarded under
    /// [`WalRecoveryMode::PointInTime`] — Rocks-shaped availability with a
    /// typed, honest report (`None` = clean open). Repeated corruption that
    /// trips the CORRUPTLOG escalation limit refuses the open instead.
    #[must_use]
    pub fn last_recovery_report(&self) -> Option<pedradb_core::RecoveryReport> {
        self.inner.last_recovery_report()
    }

    /// rust-rocksdb `DB::resume`: recover from a background durability
    /// fence (fsync failure class) via close+replay+reopen (RFC-0047 P1.1).
    /// `Ok(())` also when nothing was fenced (defensive resume, like
    /// Rocks). The typed outcome — uncertain sequence range and whether the
    /// replay proved writes lost — is on [`Self::last_fence_recovery`]:
    /// never a silent "as if nothing happened".
    ///
    /// # Errors
    /// Reopen I/O or a still-in-flight commit — the DB is then unusable;
    /// drop it.
    pub fn resume(&self) -> Result<()> {
        compat_resume(&self.inner, &self.fence_recovery)
    }

    /// RFC-0047 P1.2: one auto-resume tick — exactly what the host compact
    /// worker runs when [`Options::auto_resume_transient`] is on. Resumes
    /// only a `Transient`-class fence (ENOSPC-like); every other class (and
    /// a healthy DB) is `Ok(false)` = stays manual. Hosts driving their own
    /// tick (no compat worker) can call this directly.
    ///
    /// # Errors
    /// Reopen I/O — same contract as [`Self::resume`].
    pub fn try_auto_resume(&self) -> Result<bool> {
        if !self.inner.is_durability_fenced() {
            return Ok(false);
        }
        let transient = self
            .inner
            .fence_report()
            .is_some_and(|r| r.class == pedradb_core::FenceClass::Transient);
        if !transient {
            return Ok(false);
        }
        compat_resume(&self.inner, &self.fence_recovery)?;
        Ok(true)
    }

    /// Outcome of the last successful resume after a durability fence
    /// (manual [`Self::resume`] or P1.2 auto-resume): which sequences were
    /// in flight and whether the reopen proved them lost.
    #[must_use]
    pub fn last_fence_recovery(&self) -> Option<pedradb_core::FenceRecovery> {
        self.fence_recovery.lock().clone()
    }

    /// rust-rocksdb `flush_wal`. With `sync: true` the WAL already
    /// `fdatasync`s before every Ok (G1), so this is a cheap barrier re-run.
    /// With `Options::set_sync(false)` writes are async — `flush_wal(true)`
    /// is the durability barrier (F193: it used to be a hard no-op).
    /// `false` matches pedra's WAL shape (appends go straight to the fd; no
    /// userspace buffer to flush).
    ///
    /// # Errors
    /// WAL fsync I/O.
    pub fn flush_wal(&self, sync: bool) -> Result<()> {
        if sync {
            self.inner.sync().map_err(Error::from)
        } else {
            Ok(())
        }
    }

    /// rust-rocksdb `wait_for_compact` (no-op: compact worker is host-side).
    pub fn wait_for_compact(&self, _opts: &WaitForCompactOptions) -> Result<()> {
        Ok(())
    }

    /// rust-rocksdb `cancel_all_background_work`.
    pub fn cancel_all_background_work(&self, _wait: bool) {}

    /// rust-rocksdb `compact_range`.
    pub fn compact_range<S: AsRef<[u8]>, E2: AsRef<[u8]>>(
        &self,
        start: Option<S>,
        end: Option<E2>,
    ) {
        self.compact_range_opt(start, end, &CompactOptions::default());
    }

    /// rust-rocksdb `compact_range_opt`.
    pub fn compact_range_opt<S: AsRef<[u8]>, E2: AsRef<[u8]>>(
        &self,
        _start: Option<S>,
        _end: Option<E2>,
        _opts: &CompactOptions,
    ) {
        let _ = self.compact();
    }

    /// rust-rocksdb `compact_range_cf`.
    pub fn compact_range_cf<S: AsRef<[u8]>, E2: AsRef<[u8]>>(
        &self,
        cf: &ColumnFamily,
        _start: Option<S>,
        _end: Option<E2>,
    ) {
        let _gate = self.compact_gate.lock();
        let _ = self.inner.compact_cf(&cf.name);
    }

    /// rust-rocksdb `compact_range_cf_opt`.
    pub fn compact_range_cf_opt<S: AsRef<[u8]>, E2: AsRef<[u8]>>(
        &self,
        cf: &ColumnFamily,
        start: Option<S>,
        end: Option<E2>,
        _opts: &CompactOptions,
    ) {
        self.compact_range_cf(cf, start, end);
    }

    /// rust-rocksdb `create_cf`.
    pub fn create_cf(&self, name: impl AsRef<str>, _opts: &Options) -> Result<()> {
        let name = name.as_ref();
        validate_cf_name(name)?;
        if name == DEFAULT_CF {
            return Ok(());
        }
        if self.codec.default_raw {
            return Err(Error::invalid(format!(
                "cannot add column family {name} to a default-only DB: \
                 default-CF keys are stored raw; create the DB with the full column family list"
            )));
        }
        {
            let mut cfs = self.cfs.write();
            if cfs.iter().any(|c| c.as_ref() == name) {
                return Ok(());
            }
            cfs.push(Arc::from(name));
            let non_default: Vec<String> = cfs
                .iter()
                .filter(|c| c.as_ref() != DEFAULT_CF)
                .map(|c| c.to_string())
                .collect();
            store_cf_registry(&self.inner.path(), false, &non_default)?;
            self.inner
                .set_physical_cfs(cfs.iter().map(|c| c.to_string()).collect());
        }
        Ok(())
    }

    /// rust-rocksdb `drop_cf`: range-delete the CF prefix, compact, unregisters.
    pub fn drop_cf(&self, name: impl AsRef<str>) -> Result<()> {
        let name = name.as_ref();
        if name == DEFAULT_CF {
            return Err(Error::invalid("cannot drop default column family"));
        }
        self.check_cf(name)?;
        let start = self.codec.encode(name, &[]);
        let mut end = start.clone();
        *end.last_mut().expect("prefix") = 1;
        self.inner.delete_range(start, end).map_err(Error::from)?;
        self.flush()?;
        self.compact()?;
        let mut cfs = self.cfs.write();
        cfs.retain(|c| c.as_ref() != name);
        let non_default: Vec<String> = cfs
            .iter()
            .filter(|c| c.as_ref() != DEFAULT_CF)
            .map(|c| c.to_string())
            .collect();
        self.inner
            .set_physical_cfs(cfs.iter().map(|c| c.to_string()).collect());
        drop(cfs);
        store_cf_registry(&self.inner.path(), self.codec.default_raw, &non_default)
    }

    /// rust-rocksdb `list_cf`.
    pub fn list_cf<P: AsRef<std::path::Path>>(_opts: &Options, path: P) -> Result<Vec<String>> {
        match load_cf_registry(path.as_ref())? {
            None => Ok(vec![DEFAULT_CF.to_string()]),
            Some((_raw, names)) => {
                let mut v = vec![DEFAULT_CF.to_string()];
                v.extend(names);
                Ok(v)
            }
        }
    }

    /// rust-rocksdb `destroy`.
    pub fn destroy<P: AsRef<std::path::Path>>(_opts: &Options, path: P) -> Result<()> {
        match std::fs::remove_dir_all(path.as_ref()) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error {
                msg: format!("destroy: {e}"),
                kind: ErrorKind::Io,
            }),
        }
    }

    /// rust-rocksdb `repair` — open-and-close (Pedra recover is the repair).
    pub fn repair<P: AsRef<std::path::Path>>(opts: &Options, path: P) -> Result<()> {
        let _db = DB::<StdEnv>::open_cf_with_env(opts, path, &[], StdEnv)?;
        Ok(())
    }

    /// rust-rocksdb `path`.
    #[must_use]
    pub fn path(&self) -> std::path::PathBuf {
        self.inner.path()
    }

    /// rust-rocksdb `property_value`.
    pub fn property_value(&self, name: impl AsRef<str>) -> Result<Option<String>> {
        Ok(self
            .property_int_value(name.as_ref())?
            .map(|n| n.to_string()))
    }

    /// rust-rocksdb `property_int_value_cf` (shared LSM — same stats).
    pub fn property_int_value_cf(
        &self,
        _cf: &ColumnFamily,
        name: impl AsRef<str>,
    ) -> Result<Option<u64>> {
        self.property_int_value(name)
    }

    /// rust-rocksdb `get_opt`.
    pub fn get_opt(&self, key: impl AsRef<[u8]>, ro: &ReadOptions) -> Result<Option<Vec<u8>>> {
        ro.refuse_checksums_off()?;
        let seq = ro.snap.unwrap_or_else(|| self.inner.visible_sequence());
        self.get_at(CoreSnapshot::at(seq), DEFAULT_CF, key)
    }

    /// rust-rocksdb `get_cf_opt`.
    pub fn get_cf_opt(
        &self,
        cf: &ColumnFamily,
        key: impl AsRef<[u8]>,
        ro: &ReadOptions,
    ) -> Result<Option<Vec<u8>>> {
        ro.refuse_checksums_off()?;
        let seq = ro.snap.unwrap_or_else(|| self.inner.visible_sequence());
        self.get_at(CoreSnapshot::at(seq), &cf.name, key)
    }

    /// rust-rocksdb `get_pinned`.
    pub fn get_pinned(&self, key: impl AsRef<[u8]>) -> Result<Option<DBPinnableSlice<'_>>> {
        Ok(self.get(key)?.map(DBPinnableSlice::from_vec))
    }

    /// rust-rocksdb `get_pinned_cf`.
    pub fn get_pinned_cf(
        &self,
        cf: &ColumnFamily,
        key: impl AsRef<[u8]>,
    ) -> Result<Option<DBPinnableSlice<'_>>> {
        Ok(self.get_cf(cf, key)?.map(DBPinnableSlice::from_vec))
    }

    /// rust-rocksdb `multi_get`.
    ///
    /// One snapshot and one `ConcurrentDb` read lock for the whole batch
    /// (RFC-0160 P2.1). A loop of [`Self::get`] took one lock + CF-encode
    /// envelope per key — 1M lookup_100 multi_get sat at 1.08× while
    /// get_loop (same 100 keys, 100 locks) was 1.36×.
    pub fn multi_get<K, I>(&self, keys: I) -> Vec<Result<Option<Vec<u8>>>>
    where
        K: AsRef<[u8]>,
        I: IntoIterator<Item = K>,
    {
        let keys: Vec<K> = keys.into_iter().collect();
        if keys.is_empty() {
            return Vec::new();
        }
        let encoded: Vec<Vec<u8>> = keys
            .iter()
            .map(|k| {
                self.codec
                    .encode_with(DEFAULT_CF, k.as_ref(), |enc| enc.to_vec())
            })
            .collect();
        self.inner
            .multi_get(&encoded)
            .into_iter()
            .map(|v| Ok(v.map(|b| b.to_vec())))
            .collect()
    }

    /// rust-rocksdb `multi_get_cf`.
    ///
    /// Same one-lock batch as [`Self::multi_get`] (RFC-0160 P2.1). Answers
    /// the same bytes as 100 [`Self::get_cf`]s, including CRC fail-closed
    /// (a corrupt block panics the process on this path, matching `get`).
    pub fn multi_get_cf<'a, K, I>(&self, keys: I) -> Vec<Result<Option<Vec<u8>>>>
    where
        K: AsRef<[u8]>,
        I: IntoIterator<Item = (&'a ColumnFamily, K)>,
    {
        let pairs: Vec<(&'a ColumnFamily, K)> = keys.into_iter().collect();
        if pairs.is_empty() {
            return Vec::new();
        }
        let encoded: Vec<Vec<u8>> = pairs
            .iter()
            .map(|(cf, k)| {
                self.codec
                    .encode_with(&cf.name, k.as_ref(), |enc| enc.to_vec())
            })
            .collect();
        self.inner
            .multi_get(&encoded)
            .into_iter()
            .map(|v| Ok(v.map(|b| b.to_vec())))
            .collect()
    }

    /// rust-rocksdb `key_may_exist`.
    #[must_use]
    pub fn key_may_exist(&self, key: impl AsRef<[u8]>) -> bool {
        self.get(key).ok().flatten().is_some()
    }

    /// rust-rocksdb `key_may_exist_cf`.
    #[must_use]
    pub fn key_may_exist_cf(&self, cf: &ColumnFamily, key: impl AsRef<[u8]>) -> bool {
        self.get_cf(cf, key).ok().flatten().is_some()
    }

    /// rust-rocksdb `put_opt`.
    pub fn put_opt(
        &self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
        wo: &WriteOptions,
    ) -> Result<()> {
        let prev = self.inner.default_write_sync();
        self.inner.set_default_write_sync(wo.sync);
        let r = self.put(key, value);
        self.inner.set_default_write_sync(prev);
        r
    }

    /// rust-rocksdb `delete_opt`.
    pub fn delete_opt(&self, key: impl AsRef<[u8]>, wo: &WriteOptions) -> Result<()> {
        let prev = self.inner.default_write_sync();
        self.inner.set_default_write_sync(wo.sync);
        let r = self.delete(key);
        self.inner.set_default_write_sync(prev);
        r
    }

    /// rust-rocksdb `write_opt`.
    pub fn write_opt(&self, batch: &WriteBatch, wo: &WriteOptions) -> Result<()> {
        let prev = self.inner.default_write_sync();
        self.inner.set_default_write_sync(wo.sync);
        let r = self.write(batch);
        self.inner.set_default_write_sync(prev);
        r
    }

    /// rust-rocksdb `write_without_wal` — Pedra still WAL-appends; sync is off.
    pub fn write_without_wal(&self, batch: WriteBatch) -> Result<()> {
        let mut wo = WriteOptions::default();
        wo.set_sync(false);
        self.write_opt(&batch, &wo)
    }

    /// rust-rocksdb `merge`.
    ///
    /// OCC read-modify-write with retry so concurrent `merge`s on one key
    /// do not drop operands (a bare get+put lost the other writer — issue #2).
    pub fn merge(&self, key: impl AsRef<[u8]>, operand: impl AsRef<[u8]>) -> Result<()> {
        self.merge_on(DEFAULT_CF, key.as_ref(), operand.as_ref())
    }

    /// rust-rocksdb `merge_cf`.
    pub fn merge_cf(
        &self,
        cf: &ColumnFamily,
        key: impl AsRef<[u8]>,
        operand: impl AsRef<[u8]>,
    ) -> Result<()> {
        self.merge_on(&cf.name, key.as_ref(), operand.as_ref())
    }

    fn merge_on(&self, cf: &str, key: &[u8], operand: &[u8]) -> Result<()> {
        let op = self
            .merge_operator
            .as_ref()
            .ok_or_else(|| Error::invalid("merge operator not set"))?
            .clone();
        // Occupies the same slot as a Rocks merge operand: get+combine+put
        // under a lock so two threads cannot both observe the same existing
        // value. OCC group-commit treats simultaneous members as one
        // snapshot and would still last-write-wins.
        let _g = self.merge_gate.lock();
        self.codec.encode_with(cf, key, |enc| {
            let existing = self.inner.get(enc);
            let operands = MergeOperands::one(operand.to_vec());
            match op(key, existing.as_deref(), &operands) {
                Some(v) => self.inner.put(enc, v).map_err(Error::from),
                None => self.inner.delete(enc).map_err(Error::from),
            }
        })
    }

    /// rust-rocksdb `flush_cf`.
    pub fn flush_cf(&self, cf: &ColumnFamily) -> Result<()> {
        let _gate = self.compact_gate.lock();
        let r = self.inner.flush_cf(cf.name()).map_err(Error::from);
        drop(_gate);
        self.notify_compact();
        r
    }

    /// rust-rocksdb `flush_cf_opt`.
    pub fn flush_cf_opt(&self, cf: &ColumnFamily, _opts: &FlushOptions) -> Result<()> {
        self.flush_cf(cf)
    }

    /// rust-rocksdb `live_files`.
    pub fn live_files(&self) -> Result<Vec<LiveFile>> {
        let rows = self.inner.live_sst_meta();
        Ok(rows
            .into_iter()
            .map(|r| LiveFile {
                column_family_name: if r.cf.is_empty() {
                    DEFAULT_CF.to_string()
                } else {
                    r.cf
                },
                name: r.name,
                size: r.size as usize,
                level: r.level as i32,
                start_key: r.start_key,
                end_key: r.end_key,
                num_entries: r.num_entries,
            })
            .collect())
    }

    /// rust-rocksdb `raw_iterator`.
    #[must_use]
    pub fn raw_iterator(&self) -> DBRawIteratorWithThreadMode<'_, Self, E> {
        self.raw_iterator_opt(ReadOptions::default())
    }

    /// rust-rocksdb `raw_iterator_cf`.
    #[must_use]
    pub fn raw_iterator_cf(&self, cf: &ColumnFamily) -> DBRawIteratorWithThreadMode<'_, Self, E> {
        self.raw_iterator_cf_opt(cf, ReadOptions::default())
    }

    /// rust-rocksdb `raw_iterator_cf_opt`.
    #[must_use]
    pub fn raw_iterator_cf_opt(
        &self,
        cf: &ColumnFamily,
        ro: ReadOptions,
    ) -> DBRawIteratorWithThreadMode<'_, Self, E> {
        let seq = ro.snap.unwrap_or_else(|| self.inner.visible_sequence());
        DBRawIteratorWithThreadMode::open_cf(self, &cf.name, seq, &ro)
    }

    /// rust-rocksdb `iterator_opt`. Honours snapshot (F180), iterate bounds,
    /// `prefix_same_as_start`, and refuses `set_verify_checksums(false)`
    /// (RFC-0062 P1.6).
    pub fn iterator_opt(
        &self,
        mode: IteratorMode<'_>,
        mut ro: ReadOptions,
    ) -> Result<DBIterator<E>> {
        ro.refuse_checksums_off()?;
        clamp_prefix_same_as_start(&mut ro, mode);
        let seq = ro.snap.unwrap_or_else(|| self.inner.visible_sequence());
        let names = self.cf_names();
        let bounds = IterBounds {
            lower: ro.lower.as_deref(),
            upper: ro.upper.as_deref(),
        };
        scan_cf_at(
            &self.inner,
            &self.codec,
            DEFAULT_CF,
            mode,
            seq,
            &names,
            bounds,
        )
    }

    /// rust-rocksdb `iterator_cf_opt`. Iterate bounds are honoured: both are
    /// clamped into the encoded CF range, so the core scan stops at the
    /// upper bound instead of over-reading past it.
    pub fn iterator_cf_opt(
        &self,
        cf: &ColumnFamily,
        mode: IteratorMode<'_>,
        mut ro: ReadOptions,
    ) -> Result<DBIterator<E>> {
        ro.refuse_checksums_off()?;
        clamp_prefix_same_as_start(&mut ro, mode);
        let seq = ro.snap.unwrap_or_else(|| self.inner.visible_sequence());
        let names = self.cf_names();
        let bounds = IterBounds {
            lower: ro.lower.as_deref(),
            upper: ro.upper.as_deref(),
        };
        scan_cf_at(
            &self.inner,
            &self.codec,
            &cf.name,
            mode,
            seq,
            &names,
            bounds,
        )
    }

    /// rust-rocksdb `prefix_iterator`.
    ///
    /// Stops at the exclusive next prefix (`prefix_exclusive_end`). A seek
    /// to `prefix` with no upper bound leaked sibling keys (issue #4).
    pub fn prefix_iterator(&self, prefix: impl AsRef<[u8]>) -> Result<DBIterator<E>> {
        let p = prefix.as_ref();
        let mut ro = ReadOptions::default();
        ro.set_prefix_same_as_start(true);
        self.iterator_opt(IteratorMode::From(p, Direction::Forward), ro)
    }

    /// rust-rocksdb `prefix_iterator_cf`.
    pub fn prefix_iterator_cf(
        &self,
        cf: &ColumnFamily,
        prefix: impl AsRef<[u8]>,
    ) -> Result<DBIterator<E>> {
        let p = prefix.as_ref();
        let mut ro = ReadOptions::default();
        ro.set_prefix_same_as_start(true);
        self.iterator_cf_opt(cf, IteratorMode::From(p, Direction::Forward), ro)
    }

    /// rust-rocksdb `full_iterator`.
    pub fn full_iterator(&self, mode: IteratorMode<'_>) -> Result<DBIterator<E>> {
        self.iterator(mode)
    }

    /// rust-rocksdb `delete_range` on default CF.
    pub fn delete_range<K: AsRef<[u8]>>(&self, from: K, to: K) -> Result<()> {
        let cf = ColumnFamily {
            name: DEFAULT_CF.into(),
        };
        self.delete_range_cf(&cf, from, to)
    }
}

impl<E: PedraEnv> Drop for DB<E> {
    fn drop(&mut self) {
        // Flush first: the compact worker's shutdown path drains every
        // parked mem itself, without a racing materializer.
        if let Some(tx) = self.flush_tx.take() {
            let _ = tx.send(CompactCmd::Shutdown);
        }
        if let Some(h) = self.flush_thread.take() {
            let _ = h.join();
        }
        if let Some(tx) = self.compact_tx.take() {
            let _ = tx.send(CompactCmd::Shutdown);
        }
        if let Some(h) = self.compact_thread.take() {
            let _ = h.join();
        }
    }
}

/// Shared resume path (manual [`DB::resume`] and the P1.2 auto tick).
fn compat_resume<E: PedraEnv>(
    inner: &ConcurrentDb<E>,
    sink: &Mutex<Option<pedradb_core::FenceRecovery>>,
) -> Result<()> {
    match inner.recover_from_fence() {
        Ok(None) => Ok(()),
        Ok(Some(rec)) => {
            *sink.lock() = Some(rec);
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

fn spawn_compact_worker<E>(
    inner: ConcurrentDb<E>,
    gate: Arc<Mutex<()>>,
    auto_resume_transient: bool,
    fence_sink: Arc<Mutex<Option<pedradb_core::FenceRecovery>>>,
    background_error_listener: Option<BackgroundErrorListener>,
) -> (Option<SyncSender<CompactCmd>>, Option<JoinHandle<()>>)
where
    E: PedraEnv + Send + Sync + 'static,
    E::File: Send + Sync + 'static,
{
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = thread::Builder::new()
        .name("pedra-compat-compact".into())
        .spawn(move || {
            let poll = Duration::from_millis(5);
            // wake2: 2 ms idle + adaptive wait still left L0=21–24 at
            // scan (rewrite of ~20 files cannot finish in MVCC). Park
            // during writes (no lz4); fold pairwise into one BTree so
            // scan/count merge mem, not 20 L0s. Materialize+compact only
            // after a long idle so MVCC/scan do not pay SST I/O (host
            // tests wait 5–10 s). Skip fold while apply_mc4 is multi
            // (incrfold apply 1.25 → 0.67).
            let persist_idle = Duration::from_millis(200);
            let fold_multi_hold = Duration::from_millis(2);
            let mut wait = poll;
            // RFC-0047 P2.1: fire on_background_error once per fence.
            let mut fence_notified = false;
            loop {
                match rx.recv_timeout(wait) {
                    Ok(CompactCmd::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                        while inner.materialize_bulk_once() {}
                        while inner.park_imm_once() {}
                        while inner.fold_parked_once_off_lock() {}
                        while inner.materialize_parked_once() {}
                        while inner.drain_imm_once() {}
                        break;
                    }
                    Ok(CompactCmd::Run) | Err(RecvTimeoutError::Timeout) => {
                        while inner.materialize_bulk_once() {}
                        compact_diag(&inner);
                        let fenced = inner.is_durability_fenced();
                        if fenced && !fence_notified {
                            if let (Some(listener), Some(report)) =
                                (background_error_listener.as_ref(), inner.fence_report())
                            {
                                listener(BackgroundError::from_fence(&report));
                            }
                            fence_notified = true;
                        } else if !fenced {
                            fence_notified = false;
                        }
                        // RFC-0047 P1.2: auto-resume a Transient-class fence
                        // (ENOSPC-like); other classes stay manual.
                        if auto_resume_transient && fenced {
                            let transient = inner
                                .fence_report()
                                .is_some_and(|r| r.class == pedradb_core::FenceClass::Transient);
                            if transient {
                                let _ = compat_resume(&inner, &fence_sink);
                            }
                        }
                        // RFC-0062 P1.1: G1 `lone_commit` drops the write
                        // lock for `fdatasync`. Fold/stage in that window
                        // steals the lock from apply. 1c still counts as
                        // `writes_active() == 1`, so that predicate is not
                        // enough — skip while a commit is inflight.
                        if inner.with_read(|db| db.commit_inflight() > 0) {
                            wait = poll;
                            continue;
                        }
                        if !inner.recently_multi(fold_multi_hold) {
                            let _ = inner.try_stage_if_full();
                        }
                        while inner.park_imm_once() {}
                        // Fold deep-clones the pair (~3 tables live mid-merge).
                        // "Not multi-writer" still fired during single-writer
                        // bulk ingest, exactly when materialization lagged and
                        // two 256 MiB parked tables piled up — a >1.5 GiB
                        // transient that OOMed a 4 GiB host at 25M entries.
                        // Fold only once writers have been idle; the idle
                        // branch below then materializes what is left.
                        // A flush-debt-throttled writer looks idle to
                        // `writes_idle_for` while it sleeps — never fold
                        // (deep-clone) the full parked tables it waits on.
                        let may_fold = inner.writes_idle_for(persist_idle)
                            && inner.parked_unflushed_bytes()
                                < inner.flush_debt_cap().unwrap_or(usize::MAX);
                        if may_fold && inner.parked_unflushed_count() >= 2 {
                            let _ = inner.fold_parked_once_off_lock();
                        }
                        // RFC-0039 P2.2: if L0 is at/above the trigger, drain
                        // now — do not wait for the 200 ms write-idle window
                        // (that was the scan-vs-apply race).
                        let l0 = inner.with_read(|db| db.level_file_count(0));
                        if l0 >= pedradb_core::L0_COMPACTION_TRIGGER {
                            while compat_compact_once(&inner, &gate) {}
                            wait = poll;
                        } else if inner.writes_idle_for(persist_idle) {
                            while inner.materialize_parked_once() {}
                            let _ = inner.persist_unsynced_l0s_off_lock();
                            let _ = inner.rotate_wal_if_writers_idle();
                            while compat_compact_once(&inner, &gate) {}
                            wait = poll;
                        } else {
                            wait = poll;
                        }
                    }
                }
            }
        })
        .ok();
    (Some(tx), handle)
}

/// Dedicated flush thread (Rocks-shaped: a memtable flush never queues
/// behind compaction). Owns the parked-memory bound: under sustained
/// ingest (bulk load, bench hydrate) a commit is in flight at nearly
/// every compact-worker tick, so parked mems — and the fold union, which
/// absorbs every new table while the count stays flat — used to grow
/// with everything written since the last 200 ms idle window and OOM
/// the host (observed at 25M entries on a 4 GiB box). Above the write
/// buffer this thread materializes mid-burst, paying the lz4/SST write
/// Rocks pays on every memtable flush; short bursts (gate shapes) park
/// <= 1 partial mem and stay under it. 1x is the honest bound: a parked
/// mem holds ~3x its KV bytes in memory, so parking even one extra
/// full memtable overshoots the write buffer the host configured —
/// at 3x, a 620 MB 2M-entry hydrate (one full 256 MiB mem + a 108 MB
/// tail) never reached 768 MiB, sat parked until settle, and the drain
/// OOMed the same 4 GiB box (guest CHV, SETTLE_RSS 1.9 GB -> kill). The drain is budgeted per
/// tick — an unbounded loop monopolizes the thread because the producer
/// refills the queue as fast as it drains. Parked tables are immutable
/// and `materialize_parked_once` serializes on the flush lock, so this
/// races the compact worker safely; its brief write-lock sections cannot
/// corrupt an inflight commit, they only insert a sub-ms delay ahead of
/// its re-acquire.
fn spawn_flush_worker<E>(
    inner: ConcurrentDb<E>,
) -> (Option<SyncSender<CompactCmd>>, Option<JoinHandle<()>>)
where
    E: PedraEnv + Send + Sync + 'static,
    E::File: Send + Sync + 'static,
{
    let (tx, rx) = mpsc::sync_channel(1);
    // Flush backpressure is armed: with this worker draining parked mems,
    // submits may block on flush debt (WriteGroup::await_flush_debt).
    inner.set_flush_worker_attached(true);
    let tick_secs = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mat_count = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let tick_secs_w = std::sync::Arc::clone(&tick_secs);
    let mat_count_w = std::sync::Arc::clone(&mat_count);
    let handle = thread::Builder::new()
        .name("pedra-compat-flush".into())
        .spawn(move || {
            let poll = Duration::from_millis(5);
            loop {
                match rx.recv_timeout(poll) {
                    Ok(CompactCmd::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                    Ok(CompactCmd::Run) | Err(RecvTimeoutError::Timeout) => {
                        let t0 = std::time::Instant::now();
                        let before = inner.parked_unflushed_count();
                        flush_worker_tick(&inner);
                        if inner.parked_unflushed_count() < before {
                            mat_count_w.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        tick_secs_w
                            .store(t0.elapsed().as_secs(), std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
        })
        .ok();
    if let Ok(mut s) = FLUSH_DIAG_STATE.lock() {
        *s = Some((tick_secs, mat_count));
    }
    (Some(tx), handle)
}

/// Shared flush-worker diag state (last-tick seconds, materialize count).
static FLUSH_DIAG_STATE: std::sync::Mutex<
    Option<(
        std::sync::Arc<std::sync::atomic::AtomicU64>,
        std::sync::Arc<std::sync::atomic::AtomicU64>,
    )>,
> = std::sync::Mutex::new(None);

/// `PEDRA_FLUSH_DIAG=1`: at most one stderr line per second with the
/// memory-layer breakdown (parked/active/imm/retired/sst/rss), so
/// sustained-ingest growth is attributable in situ (25M slipstream
/// hydrate OOM forensics). Zero cost when the env is unset.
fn flush_worker_diag<E: PedraEnv>(inner: &ConcurrentDb<E>) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST_NS: AtomicU64 = AtomicU64::new(0);
    if std::env::var_os("PEDRA_FLUSH_DIAG").is_none() {
        return;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let last = LAST_NS.load(Ordering::Relaxed);
    if now.saturating_sub(last) < 1_000_000_000
        || LAST_NS
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
    {
        return;
    }
    let rss_kb = std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| {
            s.split_whitespace()
                .nth(1)
                .and_then(|f| f.parse::<u64>().ok())
        })
        .map_or(0, |pages| pages.saturating_mul(4096) / 1024);
    // v19 forensics: payload-pool occupancy (must sit at/below the budget)
    // and per-table decoded-entries caches (unbounded per table — the
    // other table-sized layer a growing RSS floor can come from).
    let (pool_n, pool_b, ent_e) = inner.with_read(|db| {
        (
            db.sst_payload_pool().tracked_tables(),
            db.sst_payload_pool().resident_bytes(),
            db.sst_cached_entries(),
        )
    });
    let (tick_s, mat_n) = FLUSH_DIAG_STATE
        .lock()
        .ok()
        .and_then(|s| {
            s.as_ref()
                .map(|(t, m)| (t.load(Ordering::Relaxed), m.load(Ordering::Relaxed)))
        })
        .map_or((0, 0), |v| v);
    eprintln!(
        "FLUSHDIAG parked_n={} parked_b={} active_b={} imm={} retired_b={} sst_n={} rss_kb={} tick_s={} mat_n={} pool_n={} pool_b={} ent_e={}",
        inner.parked_unflushed_count(),
        inner.parked_unflushed_bytes(),
        inner.active_mem_usage(),
        inner.has_imm(),
        inner.retired_mem_bytes(),
        inner.sst_count(),
        rss_kb,
        tick_s,
        mat_n,
        pool_n,
        pool_b,
        ent_e,
    );
}

/// `PEDRA_FLUSH_DIAG=1` also heartbeats the compact worker (level counts),
/// so flush starvation vs compaction activity is attributable in situ.
fn compact_diag<E: PedraEnv>(inner: &ConcurrentDb<E>) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST_NS: AtomicU64 = AtomicU64::new(0);
    if std::env::var_os("PEDRA_FLUSH_DIAG").is_none() {
        return;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let last = LAST_NS.load(Ordering::Relaxed);
    if now.saturating_sub(last) < 1_000_000_000
        || LAST_NS
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
    {
        return;
    }
    eprintln!(
        "COMPACTDIAG l0={} l1={} parked_n={} pool_n={} pool_b={} ent_e={}",
        inner.with_read(|db| db.level_file_count(0)),
        inner.with_read(|db| db.level_file_count(1)),
        inner.parked_unflushed_count(),
        inner.with_read(|db| db.sst_payload_pool().tracked_tables()),
        inner.with_read(|db| db.sst_payload_pool().resident_bytes()),
        inner.with_read(|db| db.sst_cached_entries()),
    );
}

/// One flush-worker tick: park every staged imm, then enforce the
/// parked-memory bound. Split out so the policy is unit-testable
/// without thread or fsync timing.
fn flush_worker_tick<E: PedraEnv>(inner: &ConcurrentDb<E>) {
    while inner.materialize_bulk_once() {}
    while inner.park_imm_once() {}
    flush_worker_diag(inner);
    let bound = inner
        .with_read(|db| db.auto_flush_threshold())
        .map_or(0, |t| t);
    if bound > 0 && inner.parked_unflushed_bytes() >= bound {
        let mut budget = 2usize;
        while budget > 0
            && inner.materialize_parked_once()
            && inner.parked_unflushed_bytes() >= bound / 2
        {
            budget -= 1;
        }
    }
}

/// Cap on L0 inputs per host-worker merge job. An unbounded job merges every
/// L0 of the family at once — at the 256 MiB bench buffer that is ≥1 GiB of
/// inputs per job, a multi-second merge that monopolizes the disk while
/// ingest keeps parking tables and RSS spikes (+1.7 GB/s observed at 25M
/// slipstream, v15). Two inputs per job bound merge memory and time; the
/// trigger loop still drains L0 to zero, just in bounded slices.
const COMPACT_MAX_L0_INPUTS: usize = 2;

/// One L0→L1 job. I/O runs without the write lock (G5: failed write is not installed).
fn compat_compact_once<E: PedraEnv>(inner: &ConcurrentDb<E>, gate: &Mutex<()>) -> bool {
    // Only invoked when writers are idle — drain every leftover L0 so a
    // mid-loop compact that hits L0=0 cannot leave a sub-trigger remnant.
    let l0 = inner.with_read(|db| db.level_file_count(0));
    if l0 == 0 {
        return false;
    }
    let _gate = gate.lock();
    let job = inner.with_write(|db| {
        if db.level_file_count(0) == 0 {
            return None;
        }
        // Mirror core `maybe_auto_compact`: honor `auto_reclaim` with
        // pin-aware GC (Rocks-shaped retention); default keeps history.
        let opts = if db.auto_reclaim() {
            let oldest = db
                .oldest_pinned_sequence()
                .unwrap_or_else(|| db.last_sequence());
            CoreCompactOptions {
                gc: pedradb_core::merge::CompactGcOptions::for_oldest_snapshot(oldest),
                max_input_files: Some(COMPACT_MAX_L0_INPUTS),
            }
        } else {
            CoreCompactOptions {
                max_input_files: Some(COMPACT_MAX_L0_INPUTS),
                ..CoreCompactOptions::default()
            }
        };
        db.prepare_l0_compact(opts).ok().flatten()
    });
    let Some(job) = job else {
        return false;
    };
    let tables = match job.write() {
        Ok(t) => t,
        Err(_) => return false,
    };
    if !inner.install_prepared_l0_off_lock(job, tables) {
        return false;
    }
    inner.with_read(|db| db.level_file_count(0)) > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g1_opts() -> Options {
        let mut o = Options::new();
        o.create_if_missing(true);
        o.set_sync(true);
        o.set_wal_full_fsync(true);
        o
    }

    /// RFC-0044 P2.2 probe: deps_raftlog shape through the exact bench path
    /// (`write_cf_owned`, async lone-writer). Prints WritePhaseStats so the
    /// per-batch gap vs Rocks has numbers. Run with:
    /// `cargo test -p rocksdb-compat --lib --release --ignored raftlog_phase -- --nocapture`
    #[test]
    #[ignore]
    fn raftlog_phase_probe() {
        let want_stats = std::env::var("RAFTLOG_PROBE_STATS")
            .map(|v| v != "0")
            .unwrap_or(true);
        if want_stats {
            std::env::set_var("PEDRA_WRITE_PHASE_STATS", "1");
        }
        let d = tmp("raftlog_probe");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        if std::env::var("RAFTLOG_PROBE_NOFLUSH").is_ok() {
            // Discriminator: no auto-flush ⇒ no parked memtables ⇒ no folds.
            opts.set_write_buffer_size(0);
        }
        let db = DB::open_cf(&opts, &d, &["raftlog"]).unwrap();
        db.inner.set_default_write_sync(false);
        // 3.2M sequential keys = 100x the official battery leg; small enough
        // that cache/invalidation structures stay battery-scale.
        let per_batch: usize = std::env::var("RAFTLOG_PROBE_BATCH")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(16);
        let batches = std::env::var("RAFTLOG_PROBE_N")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(200_000);
        let mut idx = 0u64;
        let val = vec![b'r'; 100];
        let t0 = std::time::Instant::now();
        for _ in 0..batches {
            let mut puts = Vec::with_capacity(per_batch);
            for _ in 0..per_batch {
                idx += 1;
                puts.push((
                    "raftlog",
                    format!("raftlog/{idx:08}").into_bytes(),
                    val.clone(),
                ));
            }
            db.write_cf_owned(puts, Vec::new()).unwrap();
        }
        let wall = t0.elapsed();
        let commits = batches;
        let ops = commits as usize * per_batch;
        if let Some(st) = db.inner.write_phase_stats() {
            let rd = |v: &std::sync::atomic::AtomicU64| {
                v.load(std::sync::atomic::Ordering::Relaxed) as f64 / commits as f64 / 1000.0
            };
            println!(
                "  prepare={:.2}µs wal={:.2}µs mem={:.2}µs publish={:.2}µs flush_chk={:.2}µs lock_wait={:.2}µs",
                rd(&st.prepare_ns),
                rd(&st.wal_ns),
                rd(&st.mem_ns),
                rd(&st.publish_ns),
                rd(&st.flush_check_ns),
                rd(&st.lock_wait_ns),
            );
        }
        println!(
            "raftlog probe: {commits} batches x {per_batch} ops ({ops} ops), wall {wall:?} ({:.2} µs/batch, {:.3} µs/op)",
            wall.as_secs_f64() * 1e6 / commits as f64,
            wall.as_secs_f64() * 1e6 / ops as f64
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// RFC-0036 addendum cost probe: per-commit price of the strong barrier
    /// class (`F_FULLFSYNC` on Darwin) vs the default `fdatasync` class, in
    /// the two product shapes — single put and raftlog batch (16 puts / one
    /// WAL barrier per commit). Cost is the barrier itself, so box noise is
    /// second-order; still not an official battery.
    #[test]
    #[ignore]
    fn wal_full_fsync_cost_probe() {
        let n: usize = std::env::var("WAL_FF_N")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(200);
        let val = vec![b'r'; 100];
        let mut idx = 0u64;
        for full in [false, true] {
            for per_batch in [1usize, 16] {
                let d = tmp("wal_ff_cost");
                let mut opts = Options::new();
                opts.create_if_missing(true);
                opts.set_wal_full_fsync(full);
                opts.set_write_buffer_size(256 * 1024 * 1024);
                let db = DB::open_cf(&opts, &d, &["raftlog"]).unwrap();
                // Warm-up: first barrier pays open/extent costs.
                for _ in 0..8 {
                    idx += 1;
                    let _ = db.put_cf(
                        &db.cf_handle("raftlog").expect("raftlog cf"),
                        format!("warm/{idx:08}").as_bytes(),
                        val.as_slice(),
                    );
                }
                let mut walls: Vec<u128> = Vec::with_capacity(n);
                let t_all = std::time::Instant::now();
                for _ in 0..n {
                    let mut wb: Vec<(&str, Vec<u8>, Vec<u8>)> = Vec::with_capacity(per_batch);
                    for _ in 0..per_batch {
                        idx += 1;
                        wb.push((
                            "raftlog",
                            format!("raftlog/{idx:08}").into_bytes(),
                            val.clone(),
                        ));
                    }
                    let t = std::time::Instant::now();
                    db.write_cf_owned(wb, Vec::new()).unwrap();
                    walls.push(t.elapsed().as_nanos());
                }
                let total_ns = t_all.elapsed().as_nanos();
                walls.sort_unstable();
                let p50 = walls[(walls.len() - 1) / 2] as f64 / 1e6;
                // Integer-first arithmetic: a float division path in this
                // test binary miscompiles to inf/0 (u128→f64 expressions);
                // the integer rates cross-check against p50.
                let mean = (total_ns / n as u128) as f64 / 1e6;
                let cps = n as u128 * 1_000_000_000 / total_ns.max(1);
                println!(
                    "wal_ff full={full} batch={per_batch:>2}: p50={:.3}ms mean={:.3}ms commits/s={} puts/s={}",
                    p50,
                    mean,
                    cps,
                    cps * per_batch as u128,
                );
                let _ = std::fs::remove_dir_all(&d);
            }
        }
    }

    // Tail probe: per-batch wall + WritePhaseStats deltas for the slowest
    // batches — attributes the once-per-leg multi-ms stall to a phase (a
    // large residual means the stall is outside the measured phases).
    #[test]
    #[ignore]
    fn raftlog_tail_probe() {
        std::env::set_var("PEDRA_WRITE_PHASE_STATS", "1");
        let d = tmp("raftlog_tail");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        // Match the bench engine config: no auto-flush inside the window.
        opts.set_write_buffer_size(256 * 1024 * 1024);
        let db = DB::open_cf(&opts, &d, &["raftlog"]).unwrap();
        db.inner.set_default_write_sync(false);
        let per_batch: usize = std::env::var("RAFTLOG_TAIL_BATCH")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(16);
        let batches: usize = std::env::var("RAFTLOG_TAIL_N")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2000);
        let stats = db
            .inner
            .write_phase_stats()
            .expect("PEDRA_WRITE_PHASE_STATS=1");
        let rd = |v: &std::sync::atomic::AtomicU64| v.load(std::sync::atomic::Ordering::Relaxed);
        let mut idx = 0u64;
        let val = vec![b'r'; 100];
        // Discriminator: batch construction cost alone (format! + clone x16),
        // paid identically by both engines in the bench loop.
        {
            let mut builds: Vec<u128> = Vec::with_capacity(batches);
            let mut sink: usize = 0;
            let mut j = 0u64;
            for _ in 0..batches {
                let t = std::time::Instant::now();
                let mut puts = Vec::with_capacity(per_batch);
                for _ in 0..per_batch {
                    j += 1;
                    puts.push((
                        "raftlog",
                        format!("raftlog/{j:08}").into_bytes(),
                        val.clone(),
                    ));
                }
                let dt = t.elapsed().as_nanos();
                sink += puts.len();
                builds.push(dt);
            }
            builds.sort_unstable();
            let q = |p: f64| builds[((builds.len() as f64 - 1.0) * p) as usize] as f64 / 1000.0;
            println!(
                "raftlog build-only: {batches} x {per_batch}: p50={:.2}µs p95={:.2}µs (sink={})",
                q(0.50),
                q(0.95),
                sink
            );
        }
        // (batch, wall_ns, prepare, wal, mem, publish, flush_chk, lock_wait)
        let mut recs: Vec<(usize, u128, u64, u64, u64, u64, u64, u64)> =
            Vec::with_capacity(batches);
        for i in 0..batches {
            let mut puts = Vec::with_capacity(per_batch);
            for _ in 0..per_batch {
                idx += 1;
                puts.push((
                    "raftlog",
                    format!("raftlog/{idx:08}").into_bytes(),
                    val.clone(),
                ));
            }
            let before = (
                rd(&stats.prepare_ns),
                rd(&stats.wal_ns),
                rd(&stats.mem_ns),
                rd(&stats.publish_ns),
                rd(&stats.flush_check_ns),
                rd(&stats.lock_wait_ns),
            );
            let t0 = std::time::Instant::now();
            db.write_cf_owned(puts, Vec::new()).unwrap();
            let wall = t0.elapsed().as_nanos();
            let after = (
                rd(&stats.prepare_ns),
                rd(&stats.wal_ns),
                rd(&stats.mem_ns),
                rd(&stats.publish_ns),
                rd(&stats.flush_check_ns),
                rd(&stats.lock_wait_ns),
            );
            recs.push((
                i,
                wall,
                after.0 - before.0,
                after.1 - before.1,
                after.2 - before.2,
                after.3 - before.3,
                after.4 - before.4,
                after.5 - before.5,
            ));
        }
        let mut walls: Vec<u128> = recs.iter().map(|r| r.1).collect();
        walls.sort_unstable();
        let q = |p: f64| walls[((walls.len() as f64 - 1.0) * p) as usize];
        println!(
            "raftlog tail probe: {batches} x {per_batch}: wall p50={:.1}µs p95={:.1}µs p99={:.1}µs max={:.1}µs",
            q(0.50) as f64 / 1000.0,
            q(0.95) as f64 / 1000.0,
            q(0.99) as f64 / 1000.0,
            *walls.last().unwrap() as f64 / 1000.0
        );
        println!("  idx   wall_ms  prepare  wal_ms   mem  publish flsh_chk lock_wt  residual_ms");
        let mut slow = recs.clone();
        slow.sort_unstable_by_key(|r| std::cmp::Reverse(r.1));
        for r in slow.iter().take(10) {
            let phases = r.2 + r.3 + r.4 + r.5 + r.6 + r.7;
            let residual = (r.1.saturating_sub(phases as u128)) as f64 / 1e6;
            println!(
                "  {:>4}  {:>7.2}  {:>6.2}µs {:>6.2}  {:>6.2}µs {:>5.2}µs {:>5.2}µs {:>6.2}µs {:>9.2}",
                r.0,
                r.1 as f64 / 1e6,
                r.2 as f64 / 1000.0,
                r.3 as f64 / 1e6,
                r.4 as f64 / 1000.0,
                r.5 as f64 / 1000.0,
                r.6 as f64 / 1000.0,
                r.7 as f64 / 1000.0,
                residual
            );
        }
        // Per-phase p50 over all batches (attribution of the steady state).
        {
            let n = recs.len();
            let pct = |mut v: Vec<u64>| {
                v.sort_unstable();
                v[((n as f64 - 1.0) * 0.5) as usize] as f64 / 1000.0
            };
            println!(
                "  phases p50: prepare={:.2}µs wal={:.2}µs mem={:.2}µs publish={:.2}µs flsh_chk={:.2}µs lock_wait={:.2}µs",
                pct(recs.iter().map(|r| r.2).collect()),
                pct(recs.iter().map(|r| r.3).collect()),
                pct(recs.iter().map(|r| r.4).collect()),
                pct(recs.iter().map(|r| r.5).collect()),
                pct(recs.iter().map(|r| r.6).collect()),
                pct(recs.iter().map(|r| r.7).collect()),
            );
        }
        // Bench-shape replica: construction + write + per-op Instant pair +
        // every-8th get, all inside the timing — isolates what the official
        // loop adds over the write-only probe above (same process, same DB).
        {
            let mut walls: Vec<u128> = Vec::with_capacity(batches);
            let mut idx2 = idx;
            let cf = db.cf_handle("raftlog").expect("raftlog cf");
            let t_all = std::time::Instant::now();
            for op in 0..batches {
                let t = std::time::Instant::now();
                let mut wb: Vec<(&str, Vec<u8>, Vec<u8>)> = Vec::with_capacity(16);
                for _ in 0..per_batch {
                    idx2 += 1;
                    wb.push((
                        "raftlog",
                        format!("raftlog/{idx2:08}").into_bytes(),
                        val.clone(),
                    ));
                }
                db.write_cf_owned(wb, Vec::new()).unwrap();
                if op % 8 == 0 && idx2 > 1 {
                    let _ = db.get_cf(&cf, format!("raftlog/{:08}", idx2 - 1).as_bytes());
                }
                walls.push(t.elapsed().as_nanos());
            }
            let total = t_all.elapsed();
            walls.sort_unstable();
            let q = |p: f64| walls[((walls.len() as f64 - 1.0) * p) as usize] as f64 / 1000.0;
            println!(
                "raftlog bench-shape: {batches} x {per_batch}: p50={:.2}µs p95={:.2}µs mean={:.2}µs (leg wall {:.4}s)",
                q(0.50),
                q(0.95),
                total.as_nanos() as f64 / batches as f64 / 1000.0,
                total.as_secs_f64()
            );
        }
        // Discriminators for the replica delta over the probe loop (same
        // `write_cf_owned` call, +3.5µs unexplained): (a) same loop with NO
        // get → get interference; (b) with-get again at a larger DB →
        // state growth vs loop shape.
        for (label, with_get) in [("bench-shape-noget", false), ("bench-shape+get2", true)] {
            let mut walls: Vec<u128> = Vec::with_capacity(batches);
            let mut idx2 = idx;
            let cf = db.cf_handle("raftlog").expect("raftlog cf");
            let t_all = std::time::Instant::now();
            for op in 0..batches {
                let t = std::time::Instant::now();
                let mut wb: Vec<(&str, Vec<u8>, Vec<u8>)> = Vec::with_capacity(16);
                for _ in 0..per_batch {
                    idx2 += 1;
                    wb.push((
                        "raftlog",
                        format!("raftlog/{idx2:08}").into_bytes(),
                        val.clone(),
                    ));
                }
                db.write_cf_owned(wb, Vec::new()).unwrap();
                if with_get && op % 8 == 0 && idx2 > 1 {
                    let _ = db.get_cf(&cf, format!("raftlog/{:08}", idx2 - 1).as_bytes());
                }
                walls.push(t.elapsed().as_nanos());
            }
            let total = t_all.elapsed();
            walls.sort_unstable();
            let q = |p: f64| walls[((walls.len() as f64 - 1.0) * p) as usize] as f64 / 1000.0;
            println!(
                "raftlog {label}: {batches} x {per_batch}: p50={:.2}µs p95={:.2}µs mean={:.2}µs (leg wall {:.4}s)",
                q(0.50),
                q(0.95),
                total.as_nanos() as f64 / batches as f64 / 1000.0,
                total.as_secs_f64()
            );
        }
        // Probe-loop repeat at the larger DB: identical body to the first
        // probe pass (build outside timing). Separates loop-shape from
        // DB-growth-over-time.
        {
            let mut walls: Vec<u128> = Vec::with_capacity(batches);
            let mut idx2 = idx;
            for _ in 0..batches {
                let mut puts = Vec::with_capacity(per_batch);
                for _ in 0..per_batch {
                    idx2 += 1;
                    puts.push((
                        "raftlog",
                        format!("raftlog/{idx2:08}").into_bytes(),
                        val.clone(),
                    ));
                }
                let t0 = std::time::Instant::now();
                db.write_cf_owned(puts, Vec::new()).unwrap();
                walls.push(t0.elapsed().as_nanos());
            }
            walls.sort_unstable();
            let q = |p: f64| walls[((walls.len() as f64 - 1.0) * p) as usize] as f64 / 1000.0;
            println!(
                "raftlog probe2: {batches} x {per_batch}: p50={:.2}µs p95={:.2}µs",
                q(0.50),
                q(0.95)
            );
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    fn tmp(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("rdbcompat-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// `decode_bytes` must agree with `decode` on every key shape the scan
    /// page can see: raw-default and prefixed CFs, empty keys, keys equal to
    /// or shorter than the `cf\0` prefix (old behavior decoded those to
    /// empty — the Bytes path must not panic or diverge).
    #[test]
    fn key_codec_decode_bytes_matches_decode() {
        for (cf, raw) in [
            ("default", true),
            ("default", false),
            ("data", true),
            ("data", false),
        ] {
            let codec = KeyCodec { default_raw: raw };
            for key in [&b""[..], b"d", b"data", b"data\0", b"data\0k", b"\0k", b"k"] {
                let owned = Bytes::copy_from_slice(key);
                assert_eq!(
                    codec.decode_bytes_owned(cf, owned).as_ref(),
                    codec.decode(cf, key),
                    "cf={cf} raw={raw} key={key:?}"
                );
            }
        }
    }

    #[test]
    fn default_options_match_rust_rocksdb_factory() {
        let o = Options::new();
        assert!(!o.sync, "WriteOptions.sync=false");
        assert_eq!(o.write_buffer_size, 64 * 1024 * 1024, "Rocks C++ 0x4000000");
        assert!(!o.create_if_missing);
        assert!(!o.enable_blob_files);
        assert!(
            o.wal_full_fsync,
            "upstream Rocks CMake HAVE_FULLFSYNC on Darwin"
        );
        assert_eq!(o.wal_recovery, WalRecoveryMode::PointInTime);
    }

    /// RFC-0062 P0.3 + P1.6: G2 setters never Ok with CRC/paranoid/skip-any off.
    #[test]
    fn g2_setters_are_not_supported() {
        let d = tmp("g2-verify");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        let db = DB::open(&opts, &d).unwrap();
        db.put(b"k", b"v").unwrap();
        let mut ro = ReadOptions::default();
        ro.set_verify_checksums(false);
        let err = db.get_opt(b"k", &ro).expect_err("checksums-off must fail");
        assert_eq!(err.kind(), ErrorKind::NotSupported);
        let err = db
            .iterator_opt(IteratorMode::Start, ro)
            .err()
            .expect("iterator checksums-off");
        assert_eq!(err.kind(), ErrorKind::NotSupported);
        assert_eq!(db.get(b"k").unwrap().as_deref(), Some(&b"v"[..]));
        drop(db);
        let _ = std::fs::remove_dir_all(&d);

        let d = tmp("g2-paranoid");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        opts.set_paranoid_checks(false);
        let err = DB::open(&opts, &d).err().expect("paranoid-off");
        assert_eq!(err.kind(), ErrorKind::NotSupported);
        let _ = std::fs::remove_dir_all(&d);

        let d = tmp("g2-skipany");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        opts.set_wal_recovery_mode(WalRecoveryMode::SkipAnyCorruptedRecord);
        let err = DB::open(&opts, &d).err().expect("skip-any");
        assert_eq!(err.kind(), ErrorKind::NotSupported);
        let _ = std::fs::remove_dir_all(&d);

        let d = tmp("g2-nochecksum");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        let mut bb = BlockBasedOptions::default();
        bb.set_checksum_type(ChecksumType::NoChecksum);
        opts.set_block_based_table_factory(&bb);
        let err = DB::open(&opts, &d).err().expect("NoChecksum");
        assert_eq!(err.kind(), ErrorKind::NotSupported);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Iterate bounds (`ReadOptions::lower`/`upper`) must be honoured by
    /// `iterator_cf_opt` — they used to be stored and silently dropped,
    /// forcing callers into a manual `starts_with` break past the prefix.
    #[test]
    fn iterator_cf_opt_honours_iterate_bounds() {
        let d = tmp("iter-bounds");
        let db = DB::open_cf(&Options::new(), &d, &["data"]).unwrap();
        let cf = db.cf_handle("data").unwrap();
        for i in 0..10u8 {
            db.put_cf(&cf, format!("k{i:02}").as_bytes(), [i]).unwrap();
        }
        let mut ro = ReadOptions::default();
        ro.set_iterate_lower_bound(b"k03".to_vec());
        ro.set_iterate_upper_bound(b"k07".to_vec());
        let walk = |mode, ro: &ReadOptions| -> Vec<String> {
            db.iterator_cf_opt(&cf, mode, ro.clone())
                .unwrap()
                .map(|r| String::from_utf8(r.unwrap().0.to_vec()).unwrap())
                .collect()
        };
        // Forward: seek below the lower bound clamps up to it; the scan
        // stops at the exclusive upper bound without a caller-side break.
        assert_eq!(
            walk(IteratorMode::From(b"k00", Direction::Forward), &ro),
            ["k03", "k04", "k05", "k06"]
        );
        assert_eq!(walk(IteratorMode::Start, &ro), ["k03", "k04", "k05", "k06"]);
        // Reverse from above the upper bound: last key inside the range,
        // walking down to the inclusive lower bound.
        assert_eq!(
            walk(IteratorMode::From(b"k09", Direction::Reverse), &ro),
            ["k06", "k05", "k04", "k03"]
        );
        // Bound outside the CF prefix: everything in `data` stays visible.
        let mut wide = ReadOptions::default();
        wide.set_iterate_upper_bound(b"zzz".to_vec());
        assert_eq!(walk(IteratorMode::Start, &wide).len(), 10);
        drop(db);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Default-CF (raw-key encoding) bounds plus a multi-window walk
    /// (10 x ITER_WINDOW keys) so refills resume through the clamped
    /// range repeatedly.
    #[test]
    fn iterator_opt_bounds_and_multiwindow_refill() {
        let d = tmp("iter-bounds-win");
        let db = DB::open(&Options::new(), &d).unwrap();
        for i in 0..10 * ITER_WINDOW {
            db.put(format!("k{i:04}").as_bytes(), [1]).unwrap();
        }
        let mut ro = ReadOptions::default();
        let lo = format!("k{:04}", 3 * ITER_WINDOW);
        let hi = format!("k{:04}", 7 * ITER_WINDOW);
        ro.set_iterate_lower_bound(lo.clone().into_bytes());
        ro.set_iterate_upper_bound(hi.into_bytes());
        let got: Vec<String> = db
            .iterator_opt(IteratorMode::Start, ro)
            .unwrap()
            .map(|r| String::from_utf8(r.unwrap().0.to_vec()).unwrap())
            .collect();
        assert_eq!(got.len(), 4 * ITER_WINDOW, "bounded window count");
        assert_eq!(got.first().map(String::as_str), Some(lo.as_str()));
        let want_last = format!("k{:04}", 7 * ITER_WINDOW - 1);
        assert_eq!(got.last().map(String::as_str), Some(want_last.as_str()));
        // Distinct and strictly ascending across window refills.
        assert!(got.windows(2).all(|w| w[0] < w[1]));
        drop(db);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// RFC-0062 P1.5: rust-rocksdb Checkpoint + BackupEngine names.
    #[test]
    fn checkpoint_and_backup_engine_roundtrip() {
        use crate::backup::{BackupEngine, BackupEngineOptions, RestoreOptions};
        use crate::{Checkpoint, Env};

        let d = tmp("ckpt-live");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        opts.set_sync(true);
        let db = DB::open(&opts, &d).unwrap();
        db.put(b"k", b"v").unwrap();

        let ckpt = tmp("ckpt-dest");
        Checkpoint::new(&db)
            .unwrap()
            .create_checkpoint(&ckpt)
            .unwrap();
        let opened = DB::open(&opts, &ckpt).unwrap();
        assert_eq!(opened.get(b"k").unwrap().as_deref(), Some(&b"v"[..]));
        drop(opened);

        // Named-CF checkpoint must copy CFREG or default keys go missing.
        let d_cf = tmp("ckpt-cf");
        let db_cf = DB::open_cf(&opts, &d_cf, &["raft"]).unwrap();
        db_cf.put(b"k", b"v").unwrap();
        let ckpt_cf = tmp("ckpt-cf-dest");
        Checkpoint::new(&db_cf)
            .unwrap()
            .create_checkpoint(&ckpt_cf)
            .unwrap();
        let opened_cf = DB::open_cf(&opts, &ckpt_cf, &["raft"]).unwrap();
        assert_eq!(
            opened_cf.get(b"k").unwrap().as_deref(),
            Some(&b"v"[..]),
            "checkpoint of a CF db must restore default-CF keys"
        );
        drop(opened_cf);
        drop(db_cf);
        let _ = std::fs::remove_dir_all(&d_cf);
        let _ = std::fs::remove_dir_all(&ckpt_cf);

        let backup_root = tmp("backup-root");
        let env = Env::new().unwrap();
        let backup_opts = BackupEngineOptions::new(&backup_root).unwrap();
        let mut engine = BackupEngine::open(&backup_opts, &env).unwrap();
        engine.create_new_backup_flush(&db, true).unwrap();
        let info = engine.get_backup_info();
        assert_eq!(info.len(), 1);
        engine.verify_backup(info[0].backup_id).unwrap();

        drop(db);
        let restore = tmp("backup-restore");
        let ropts = RestoreOptions::default();
        engine
            .restore_from_latest_backup(&restore, &restore, &ropts)
            .unwrap();
        let restored = DB::open(&opts, &restore).unwrap();
        assert_eq!(restored.get(b"k").unwrap().as_deref(), Some(&b"v"[..]));
        drop(restored);
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&ckpt);
        let _ = std::fs::remove_dir_all(&backup_root);
        let _ = std::fs::remove_dir_all(&restore);
    }

    /// iterator_opt must pin the snapshot (F180 class on the non-raw path).
    #[test]
    fn iterator_opt_honours_snapshot() {
        let d = tmp("iter-opt-snap");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        let db = DB::open(&opts, &d).unwrap();
        db.put(b"a", b"1").unwrap();
        let snap = db.snapshot();
        db.put(b"b", b"2").unwrap();
        let mut ro = ReadOptions::default();
        ro.set_snapshot(&SnapshotWithThreadMode::<DB>::at(snap.snap.sequence()));
        let it = db.iterator_opt(IteratorMode::Start, ro).unwrap();
        let mut keys = Vec::new();
        let mut it = it;
        while it.valid() {
            keys.push(it.key().to_vec());
            it.next();
        }
        assert_eq!(keys, vec![b"a".to_vec()], "post-snapshot put must not leak");
        drop(snap);
        drop(db);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// RFC-0151 P1: windowed iterator must not emit a key `visible_at` hid.
    /// AS-IS `iter_window_keep` would keep a deleted / range-covered version.
    #[test]
    fn iter_window_keep_on_live_hidden_is_not_ok() {
        use crate::iter_kernel::{iter_window_keep, iter_window_keep_as_is};
        use std::ops::Bound;
        let d = tmp("iter-window-hidden");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        let db = DB::open(&opts, &d).unwrap();
        db.put(b"a", b"1").unwrap();
        db.put(b"b", b"2").unwrap();
        db.put(b"c", b"3").unwrap();
        db.delete(b"b").unwrap();
        let seq = db.inner.with_read(|core| core.visible_sequence());
        let rows: Vec<_> = db
            .inner
            .with_read(|core| {
                core.try_scan_window_at(seq, Bound::Unbounded, Bound::Unbounded)
                    .map(|it| it.collect::<Vec<_>>())
            })
            .unwrap();
        assert!(
            rows.iter().any(|r| !r.snapshot_live),
            "deleted b must arrive as a window candidate with snapshot_live=false"
        );
        let kept: Vec<_> = rows
            .iter()
            .filter(|r| iter_window_keep(r.snapshot_live))
            .collect();
        let leaked: Vec<_> = rows
            .iter()
            .filter(|r| iter_window_keep_as_is(r.snapshot_live))
            .collect();
        assert!(
            leaked.len() > kept.len(),
            "AS-IS keep would emit the hidden row"
        );
        let mut it = db.iterator(IteratorMode::Start).unwrap();
        let keys: Vec<Vec<u8>> = it.collect_rest().into_iter().map(|(k, _)| k).collect();
        assert_eq!(
            keys,
            vec![b"a".to_vec(), b"c".to_vec()],
            "shipped window keep must not scan deleted b"
        );
        drop(db);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// RFC-0058 P1.1: `open_verified` pins `StdEnv` (the type is the
    /// no-ring assertion), forces the profile options and the lone-only
    /// pin; writes never queue (single-writer critical sections).
    #[test]
    fn open_verified_pins_lone_profile() {
        let d = tmp("verified");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        let db: DB<StdEnv> = DB::open_verified(&opts, &d, &[]).unwrap();
        assert!(db.is_verified(), "verified constructor must pin");
        for i in 0..8u8 {
            db.put([b'k', i], [b'v', i]).unwrap();
        }
        let (submits, queued, batches, _ops) = db.write_group_stats();
        assert_eq!(submits, 8);
        assert_eq!(queued, 0, "verified compat must never merge writers");
        assert_eq!(batches, submits);
        assert_eq!(db.get(&[b'k', 3]).unwrap().as_deref(), Some(&[b'v', 3][..]));
        drop(db);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn compat_error_kinds_map_core() {
        // RFC-0047 P0.1: a drop-in host programs policy on the kind, not
        // on parsing Display strings.
        use super::{Error, ErrorKind};
        let fenced: Error = CoreError::DurabilityFenced.into();
        assert_eq!(fenced.kind(), ErrorKind::Fenced);
        let crc: Error = CoreError::Crc {
            offset: 42,
            expected: 1,
            found: 2,
        }
        .into();
        assert_eq!(crc.kind(), ErrorKind::Corruption);
        let escalated: Error = CoreError::CorruptionEscalated {
            events: 3,
            limit: 3,
        }
        .into();
        assert_eq!(escalated.kind(), ErrorKind::CorruptionEscalated);
        let conflict: Error = CoreError::TransactionConflict.into();
        assert_eq!(conflict.kind(), ErrorKind::TransactionConflict);
        let stall: Error = CoreError::WriteStall {
            l0_files: 99,
            limit: 4,
        }
        .into();
        assert_eq!(stall.kind(), ErrorKind::WriteStall);
        // The message survives for logs (rust-rocksdb-shaped opaque Error).
        assert!(fenced.to_string().contains("fenced"));
    }

    #[test]
    fn compat_default_recovers_point_in_time_and_reports() {
        // RFC-0047 P0.2: the compat face defaults to the Rocks-shaped
        // recovery profile (kPointInTimeRecovery) — a corrupted WAL suffix
        // is discarded, the prefix is served, and the discard is reported.
        // Never silently skipped (G2 kernel floor intact underneath).
        use pedradb_core::wal::recover_choose::{apply_recover_choice, RecoverChoice};

        let dir = tmp("pit-default");
        let opts = g1_opts();
        {
            let db = DB::open(&opts, &dir).unwrap();
            for i in 0..8 {
                db.put(format!("k{i:02}").as_bytes(), &[7u8; 120]).unwrap();
            }
        }
        let wal = dir.join(pedradb_core::WAL_FILE_NAME);
        let mut bytes = std::fs::read(&wal).unwrap();
        assert!(apply_recover_choice(
            &mut bytes,
            RecoverChoice::FlipCrc { index: 3 }
        ));
        std::fs::write(&wal, &bytes).unwrap();

        let db = DB::open(&opts, &dir).unwrap();
        assert_eq!(db.get(b"k02").unwrap().as_deref(), Some(&[7u8; 120][..]));
        assert_eq!(
            db.get(b"k03").unwrap(),
            None,
            "suffix after the flip is discarded"
        );
        let report = db
            .last_recovery_report()
            .expect("compat default must report");
        assert_eq!(report.kind, "crc");
        assert!(report.discarded_bytes > 0);
        assert_eq!(report.corrupt_offset, report.good_through_offset);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn auto_reclaim_default_matches_rocks_profile() {
        // RFC-0047 P0.3: the drop-in default is the Rocks storage profile —
        // unpinned obsolete versions are GCed on auto-compact (disk ≈ live
        // set + pins). Same overwrite workload A/B: default vs F20 opt-out
        // must diverge exactly on retained history.
        assert!(
            Options::default().auto_reclaim,
            "drop-in default must be the Rocks storage profile"
        );
        fn dir_size(dir: &std::path::Path) -> u64 {
            let mut total = 0;
            for entry in std::fs::read_dir(dir).unwrap() {
                let p = entry.unwrap().path();
                if p.is_dir() {
                    total += dir_size(&p);
                } else {
                    total += std::fs::metadata(&p).unwrap().len();
                }
            }
            total
        }
        let run = |tag: &str, reclaim: Option<bool>| -> u64 {
            let dir = tmp(tag);
            let mut opts = Options::new();
            opts.create_if_missing(true);
            // Small buffer so flush+auto-compact run inside the workload.
            opts.write_buffer_size = 64 * 1024;
            if let Some(v) = reclaim {
                opts.auto_reclaim = v;
            }
            // Inline auto-compact path (no worker): deterministic — the
            // worker path with reclaim is covered by
            // `auto_reclaim_worker_gcs_versions`. Explicit per-round flush
            // drives L0 past the compaction trigger (on this path staging
            // is the host's job, RFC-0037 P2.1).
            let db = DB::open_cf_with_env(&opts, &dir, &[], pedradb_core::StdEnv).unwrap();
            // Deterministic incompressible values (xorshift): F20 retention
            // keeps every round's bytes; reclaim keeps only the live set.
            let mut seed = 0x5EED_0047_u64;
            let mut value = vec![0u8; 1024];
            for _round in 0..20u64 {
                for byte in value.iter_mut() {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    *byte = seed as u8;
                }
                for i in 0..10 {
                    // Same 10 keys every round (overwrites): retention, not
                    // live-set growth, is what must diverge.
                    db.put(format!("k{i:02}").as_bytes(), &value).unwrap();
                }
                db.flush().unwrap();
            }
            drop(db);
            dir_size(&dir)
        };
        let live_set_bytes = 10 * 1024;
        let written_bytes = 20 * live_set_bytes;
        let default_size = run("reclaim-default", None);
        let f20_size = run("reclaim-f20", Some(false));
        assert!(
            default_size * 3 < f20_size,
            "default (reclaim) {default_size}B must be far below F20 {f20_size}B"
        );
        assert!(
            default_size < written_bytes / 2,
            "default retention must bound disk near the live set ({default_size}B for {live_set_bytes}B live)"
        );
    }

    #[test]
    fn dropin_default_sync_matches_rocks() {
        // RFC-0054: drop-in WAL class is Rocks factory (async).
        // `set_sync(true)` on Darwin is upstream C++ Rocks: F_FULLFSYNC.
        assert!(
            !Options::default().sync,
            "drop-in default must match WriteOptions.sync=false"
        );
        assert!(
            Options::default().wal_full_fsync,
            "Darwin Sync() in CMake Rocks is F_FULLFSYNC"
        );
        let mut on = Options::new();
        on.set_sync(true);
        assert!(on.sync);
        assert!(on.wal_full_fsync);
    }

    #[test]
    fn background_error_listener_maps_fence_report() {
        // RFC-0047 P2.1: the on_background_error payload is typed (kind +
        // severity class), default off, builder-installed.
        let report = pedradb_core::FenceReport {
            io_error: "injected ENOSPC".into(),
            class: pedradb_core::FenceClass::Transient,
            uncertain_from: 7,
            uncertain_through: 9,
        };
        let bg = BackgroundError::from_fence(&report);
        assert_eq!(bg.kind, ErrorKind::Fenced);
        assert_eq!(bg.class, FenceClass::Transient);
        assert_eq!(bg.message, "injected ENOSPC");
        assert!(Options::default().background_error_listener.is_none());
        let fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sink = Arc::clone(&fired);
        let mut opts = Options::new();
        opts.set_background_error_listener(Arc::new(move |_| {
            sink.store(true, std::sync::atomic::Ordering::Release);
        }));
        (opts.background_error_listener.as_ref().unwrap())(bg);
        assert!(fired.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn auto_reclaim_worker_gcs_versions() {
        // RFC-0044 P2.2: with `auto_reclaim`, the host compact worker must
        // use pin-aware GC (the deferred auto-compact path), not the
        // history-preserving default merge. Without it the GC watermark
        // never advances and hot-key version piles survive compaction.
        let dir = tmp("autoreclaim");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        // Small buffer → several flushes → L0 → worker compacts when idle.
        opts.write_buffer_size = 64 * 1024;
        opts.auto_reclaim = true;
        let db = DB::open_cf(&opts, &dir, &[]).unwrap();
        let before = db.earliest_readable_sequence();
        let n = 2000;
        for i in 0..n {
            db.put(b"hot", vec![b'v'; 100])
                .and_then(|_| db.put(b"hot2", format!("{i:08}").as_bytes()))
                .unwrap();
        }
        let mut advanced = false;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if db.earliest_readable_sequence() > before {
                advanced = true;
                break;
            }
        }
        assert!(
            advanced,
            "auto_reclaim worker must advance the GC watermark (before={before})"
        );
        // Latest version still readable after GC.
        assert!(db.get(b"hot").unwrap().is_some());
        assert_eq!(
            db.get(b"hot2").unwrap().as_deref(),
            Some(format!("{:08}", n - 1).as_bytes())
        );
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn blob_files_spill_large_value_and_read_back() {
        let dir = tmp("blob-spill");
        let mut opts = g1_opts();
        opts.set_enable_blob_files(true);
        opts.set_min_blob_size(4096);
        let db = DB::open(&opts, &dir).unwrap();
        let small = vec![b's'; 100];
        let big = vec![b'B'; 16 * 1024];
        db.put(b"small", &small).unwrap();
        db.put(b"big", &big).unwrap();
        assert_eq!(db.get(b"small").unwrap().as_deref(), Some(small.as_slice()));
        assert_eq!(db.get(b"big").unwrap().as_deref(), Some(big.as_slice()));
        assert!(
            dir.join("VALUES.vlog").exists(),
            "16KiB put must create vlog"
        );
        drop(db);
        let db = DB::open(&opts, &dir).unwrap();
        assert_eq!(
            db.get(b"big").unwrap().as_deref(),
            Some(big.as_slice()),
            "G1 large put must fsync vlog before Ok"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn contains_matches_get_without_copying() {
        let dir = tmp("contains");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        let db = DB::open(&opts, &dir).unwrap();
        db.put(b"k", b"v").unwrap();
        assert!(db.contains(b"k").unwrap());
        assert!(!db.contains(b"missing").unwrap());
        assert_eq!(db.get(b"k").unwrap().as_deref(), Some(&b"v"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn put_batch_same_interns_and_reads_back() {
        let dir = tmp("batch-same");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        let db = DB::open(&opts, &dir).unwrap();
        let v = vec![b'x'; 128];
        let keys: Vec<Vec<u8>> = (0..32).map(|i| format!("k/{i:06}").into_bytes()).collect();
        db.put_batch_same("default", &keys, &v).unwrap();
        assert_eq!(db.get(b"k/000000").unwrap().as_deref(), Some(v.as_slice()));
        assert_eq!(db.get(b"k/000031").unwrap().as_deref(), Some(v.as_slice()));
        db.flush().unwrap();
        drop(db);
        let db = DB::open(&opts, &dir).unwrap();
        assert_eq!(db.get(b"k/000015").unwrap().as_deref(), Some(v.as_slice()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_batch_pooled_cf_keys_survive_later_writes() {
        let dir = tmp("pool-keys");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        let db = DB::open_cf(&opts, &dir, &["default", "write", "lock"]).unwrap();
        let lock = db.cf_handle("lock").unwrap();
        let write = db.cf_handle("write").unwrap();
        let mut first = WriteBatch::new();
        first.put_cf(&lock, b"keep", b"l1");
        first.put_cf(&write, b"keep", b"w1");
        db.write(&first).unwrap();
        for i in 0..256u32 {
            let mut wb = WriteBatch::new();
            wb.put_cf(&lock, i.to_be_bytes(), b"lx");
            wb.put_cf(&write, i.to_be_bytes(), b"wx");
            db.write(&wb).unwrap();
        }
        assert_eq!(
            db.get_cf(&lock, b"keep").unwrap().as_deref(),
            Some(&b"l1"[..])
        );
        assert_eq!(
            db.get_cf(&write, b"keep").unwrap().as_deref(),
            Some(&b"w1"[..])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_worker_flush_writes_sst_and_keeps_keys() {
        let dir = tmp("worker-flush");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        opts.set_write_buffer_size(256 * 1024);
        let db = DB::open(&opts, &dir).unwrap();
        let payload = vec![b'x'; 2048];
        for i in 0..3000u32 {
            db.put(i.to_be_bytes(), &payload).unwrap();
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if db.read_probe().sst_count >= 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            db.read_probe().sst_count >= 1,
            "host worker must write L0, probe={:?}",
            db.read_probe()
        );
        for i in [0u32, 1500, 2999] {
            assert_eq!(
                db.get(i.to_be_bytes()).unwrap().as_deref(),
                Some(payload.as_slice()),
                "acked key {i}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Bounded park policy: under sustained ingest a commit is in flight at
    /// nearly every compact-worker tick, so parked mems used to grow with
    /// everything written since the last 200 ms idle window — the
    /// 25M-hydrate OOM. [`flush_worker_tick`] must drain by a fixed budget
    /// (never monopolizing the thread) and leave every acked key readable.
    /// Deterministic: drives the tick directly with the workers disabled,
    /// so host fsync throughput cannot mask the policy.
    #[test]
    fn sustained_ingest_bounds_parked_bytes() {
        let dir = tmp("parked-bound");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        opts.set_write_buffer_size(8 * 1024);
        // `open_cf_with_env` spawns no workers — the test drives the tick.
        let db = DB::open_cf_with_env(&opts, &dir, &[], IoUringEnv::default()).unwrap();
        db.inner.set_defer_auto_compact(true);
        let payload = vec![b'x'; 1024];
        // 6 tables x ~8 KiB = 48 KiB parked, well over the 8 KiB bound.
        for i in 0..48u32 {
            db.put(i.to_be_bytes(), &payload).unwrap();
            if i % 8 == 7 {
                let _ = db.inner.with_write(|d| d.stage_flush_imm());
                while db.inner.park_imm_once() {}
            }
        }
        let before = db.inner.parked_unflushed_count();
        assert!(before >= 5, "expected a parked pile, got {before}");
        let bound = 8 * 1024;
        assert!(
            db.inner.parked_unflushed_bytes() >= bound,
            "parked bytes {}B must exceed the bound",
            db.inner.parked_unflushed_bytes()
        );
        flush_worker_tick(&db.inner);
        let after = db.inner.parked_unflushed_count();
        assert_eq!(
            before.saturating_sub(2),
            after,
            "one tick must materialize exactly the budget of 2 tables"
        );
        // Below the bound the pile is left alone (the park optimization
        // for short bursts) — but it must never sit above the bound.
        for _ in 0..10 {
            if db.inner.parked_unflushed_bytes() < bound {
                break;
            }
            flush_worker_tick(&db.inner);
        }
        assert!(
            db.inner.parked_unflushed_bytes() < bound,
            "repeated ticks must bring parked bytes under the bound, got {}B",
            db.inner.parked_unflushed_bytes()
        );
        assert!(
            db.read_probe().sst_count >= 1,
            "materialized mems must be L0 files, probe={:?}",
            db.read_probe()
        );
        for i in [0u32, 23, 47] {
            assert_eq!(
                db.get(i.to_be_bytes()).unwrap().as_deref(),
                Some(payload.as_slice()),
                "key {i}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The default open path must run the flush thread alongside the
    /// compact worker (bounded parked memory is not best-effort).
    #[test]
    fn open_spawns_flush_worker() {
        let dir = tmp("flush-worker-spawned");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        let db = DB::open(&opts, &dir).unwrap();
        assert!(db.compact_thread.is_some(), "compact worker must spawn");
        assert!(db.flush_thread.is_some(), "flush worker must spawn");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_worker_compacts_l0_without_hiding_keys() {
        let dir = tmp("worker-l0");
        let db = DB::open_default(&dir).unwrap();
        // 4 MiB auto-flush is huge for this test — flush explicitly.
        for i in 0..8u8 {
            db.put([b'k', i], [b'v', i]).unwrap();
            db.flush().unwrap();
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            let p = db.read_probe();
            if p.l0_files == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            db.read_probe().l0_files,
            0,
            "host worker should drain every L0, got {}",
            db.read_probe().l0_files
        );
        for i in 0..8u8 {
            assert_eq!(db.get(&[b'k', i]).unwrap().as_deref(), Some(&[b'v', i][..]));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0039 P2.2: `DB::open` spawns `pedra-compat-compact`. When L0
    /// reaches `L0_COMPACTION_TRIGGER`, the worker must drain on the 5 ms
    /// poll — not wait for the 200 ms write-idle window. Writers stay busy
    /// so `writes_idle_for(200ms)` cannot fire. Does not call
    /// `drain_l0_below_trigger`.
    #[test]
    fn host_worker_drains_l0_at_trigger_without_idle() {
        let dir = tmp("worker-l0-poll");
        let db = DB::open_default(&dir).unwrap();
        let trigger = L0_COMPACTION_TRIGGER;
        let persist_idle = std::time::Duration::from_millis(200);

        let mut saw_at_trigger = false;
        let mut max_l0 = 0usize;
        // Two rounds so a compact-in-flight of the first TRIGGER files
        // cannot hide the second round from `num-files-at-level0`.
        for i in 0..(L0_COMPACTION_TRIGGER * 2) {
            db.put([b'k', i as u8], [b'v', i as u8]).unwrap();
            db.flush().unwrap();
            let l0 = db.read_probe().l0_files;
            max_l0 = max_l0.max(l0);
            if l0 >= trigger {
                saw_at_trigger = true;
            }
        }
        assert!(
            saw_at_trigger,
            "explicit flush must land L0 files at the trigger (max L0={max_l0})"
        );

        let t0 = std::time::Instant::now();
        let deadline = std::time::Duration::from_millis(150);
        let mut n = 0u32;
        let mut drained = false;
        loop {
            n = n.wrapping_add(1);
            db.put(n.to_be_bytes(), n.to_be_bytes()).unwrap();
            assert!(
                !db.inner.writes_idle_for(persist_idle),
                "writers must stay busy so the 200 ms idle path cannot fire"
            );
            let l0 = db.read_probe().l0_files;
            if l0 < trigger {
                drained = true;
                break;
            }
            if t0.elapsed() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let elapsed = t0.elapsed();
        let l0 = db.read_probe().l0_files;
        assert!(
            drained,
            "worker must drain L0 below trigger on the 5 ms poll while writers are busy (not 200 ms idle); elapsed={elapsed:?} L0={l0} max_l0={max_l0}"
        );
        assert!(
            elapsed < persist_idle,
            "drain took {elapsed:?} (>= {persist_idle:?} idle window)"
        );
        for i in 0..(L0_COMPACTION_TRIGGER * 2) {
            assert_eq!(
                db.get(&[b'k', i as u8]).unwrap().as_deref(),
                Some(&[b'v', i as u8][..]),
                "acked key {i} must survive the drain"
            );
        }
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn basic_put_get_delete_reopen() {
        let dir = tmp("basic");
        {
            let db = DB::open_default(&dir).unwrap();
            db.put(b"k1", b"v1").unwrap();
            db.put(b"k2", b"v2").unwrap();
            assert_eq!(db.get(b"k1").unwrap().as_deref(), Some(&b"v1"[..]));
            db.delete(b"k1").unwrap();
            assert_eq!(db.get(b"k1").unwrap(), None);
            db.flush().unwrap();
        }
        let db = DB::open_default(&dir).unwrap();
        assert_eq!(db.get(b"k1").unwrap(), None);
        assert_eq!(db.get(b"k2").unwrap().as_deref(), Some(&b"v2"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Issue #1: `raw_iterator_cf` must walk the named CF, not default.
    #[test]
    fn raw_iterator_cf_walks_named_column_family() {
        let dir = tmp("raw-iter-cf");
        let db = DB::open_cf(&Options::new(), &dir, &["lock"]).unwrap();
        let lock = db.cf_handle("lock").unwrap();
        db.put(b"default-key", b"d").unwrap();
        db.put_cf(&lock, b"lock-key", b"l").unwrap();

        let via_cf: Vec<_> = db
            .iterator_cf(&lock, IteratorMode::Start)
            .unwrap()
            .map(|r| r.unwrap().0.to_vec())
            .collect();
        assert_eq!(via_cf, vec![b"lock-key".to_vec()]);

        let mut raw = db.raw_iterator_cf(&lock);
        raw.seek_to_first();
        assert_eq!(raw.key(), Some(b"lock-key".as_ref()));
        assert_eq!(raw.value(), Some(b"l".as_ref()));
        raw.next();
        assert!(!raw.valid(), "lock CF has one key");

        let mut def = db.raw_iterator();
        def.seek_to_first();
        assert_eq!(def.key(), Some(b"default-key".as_ref()));
        def.next();
        assert!(!def.valid());

        // reopen/seek must stay on the CF (not fall back to default).
        let mut raw = db.raw_iterator_cf(&lock);
        raw.seek(b"lock-key");
        assert_eq!(raw.key(), Some(b"lock-key".as_ref()));
        raw.seek(b"default-key");
        assert!(
            !raw.valid() || raw.key() != Some(b"default-key".as_ref()),
            "seek on lock CF must not surface default-CF keys"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_batch_atomic_and_cfs() {
        let dir = tmp("batch");
        let db = DB::open_cf(&Options::new(), &dir, &["cf_a", "cf_b"]).unwrap();
        let a = db.cf_handle("cf_a").unwrap();
        let b = db.cf_handle("cf_b").unwrap();
        assert!(db.cf_handle("nope").is_none());

        let mut wb = WriteBatch::new();
        wb.put(b"dk", b"dv");
        wb.put_cf(&a, b"ak", b"av");
        wb.put_cf(&b, b"bk", b"bv");
        wb.delete(b"tmp");
        db.write(&wb).unwrap();

        assert_eq!(db.get(b"dk").unwrap().as_deref(), Some(&b"dv"[..]));
        assert_eq!(db.get_cf(&a, b"ak").unwrap().as_deref(), Some(&b"av"[..]));
        assert_eq!(db.get_cf(&b, b"bk").unwrap().as_deref(), Some(&b"bv"[..]));

        // Atomicity: a failing batch applies nothing (unknown CF short-circuits).
        let ghost = ColumnFamily {
            name: "ghost".into(),
        };
        let mut wb = WriteBatch::new();
        wb.put(b"staged", b"x");
        wb.put_cf(&ghost, b"gk", b"gv");
        assert!(db.write(&wb).is_err());
        assert_eq!(db.get(b"staged").unwrap(), None);

        // Range delete scoped to one CF.
        db.delete_range_cf(&a, b"a", b"az").unwrap();
        assert_eq!(db.get_cf(&a, b"ak").unwrap(), None);
        assert_eq!(db.get_cf(&b, b"bk").unwrap().as_deref(), Some(&b"bv"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn iterator_modes_and_snapshot_isolation() {
        let dir = tmp("iter");
        let db = DB::open_cf(&Options::new(), &dir, &["cf"]).unwrap();
        let cf = db.cf_handle("cf").unwrap();
        for i in 0..6u8 {
            db.put_cf(&cf, [b'k', i], [i]).unwrap();
        }
        let mut it = db.iterator_cf(&cf, IteratorMode::Start).unwrap();
        assert_eq!(it.key(), &[b'k', 0]);
        it.next();
        assert_eq!(it.key(), &[b'k', 1]);

        let mut it = db
            .iterator_cf(&cf, IteratorMode::From(&[b'k', 3], Direction::Forward))
            .unwrap();
        assert_eq!(it.key(), &[b'k', 3]);
        it.next();
        assert_eq!(it.key(), &[b'k', 4]);

        let mut it = db
            .iterator_cf(&cf, IteratorMode::From(&[b'k', 3], Direction::Reverse))
            .unwrap();
        assert_eq!(it.key(), &[b'k', 3]);
        it.next();
        assert_eq!(it.key(), &[b'k', 2]);

        let mut it = db.iterator_cf(&cf, IteratorMode::End).unwrap();
        assert_eq!(it.key(), &[b'k', 5]);
        for expect in [4u8, 3, 2, 1, 0] {
            it.next();
            assert_eq!(it.key(), &[b'k', expect]);
        }
        it.next();
        assert!(!it.valid());

        // Snapshot isolation: pinned view does not see later writes.
        let snap = db.snapshot();
        db.put_cf(&cf, b"k9", b"new").unwrap();
        assert_eq!(snap.get_cf(&cf, b"k9").unwrap(), None);
        assert_eq!(db.get_cf(&cf, b"k9").unwrap().as_deref(), Some(&b"new"[..]));
        let it = snap.iterator_cf(&cf, IteratorMode::End).unwrap();
        assert_eq!(it.key(), &[b'k', 5]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cf_prefix_isolation() {
        let dir = tmp("cfiso");
        let db = DB::open_cf(&Options::new(), &dir, &["raft"]).unwrap();
        let raft = db.cf_handle("raft").unwrap();
        db.put_cf(&raft, b"log-1", b"r1").unwrap();
        db.put(b"log-1", b"d1").unwrap();
        assert_eq!(
            db.get_cf(&raft, b"log-1").unwrap().as_deref(),
            Some(&b"r1"[..])
        );
        assert_eq!(db.get(b"log-1").unwrap().as_deref(), Some(&b"d1"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn iterator_window_forward_matches_model() {
        let dir = tmp("iterwin");
        let db = DB::open_default(&dir).unwrap();
        let n = 200usize;
        for i in 0..n {
            db.put(format!("k{i:04}").as_bytes(), [i as u8]).unwrap();
        }
        let mid = format!("k{:04}", 150);
        let mut it = db
            .iterator(IteratorMode::From(mid.as_bytes(), Direction::Forward))
            .unwrap();
        let got = it.collect_rest();
        assert_eq!(got.len(), n - 150, "got {}", got.len());
        assert_eq!(got[0].0, b"k0150");
        assert_eq!(got.last().unwrap().0, b"k0199");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn latest_prefix_does_not_see_other_users() {
        let dir = tmp("latestp");
        let db = DB::open_default(&dir).unwrap();
        for u in 0..80u8 {
            for ver in 1..=3u8 {
                let mut k = format!("u/{u:03}").into_bytes();
                k.extend_from_slice(&u64::from(ver).to_be_bytes());
                db.put(&k, [ver]).unwrap();
            }
        }
        let prefix = format!("u/{:03}", 40).into_bytes();
        let mut it = db
            .iterator(IteratorMode::From(prefix.as_slice(), Direction::Forward))
            .unwrap();
        let mut last = None;
        while it.valid() && it.key().starts_with(&prefix) {
            last = Some(it.key().to_vec());
            it.next();
        }
        let last = last.expect("user 40 has versions");
        assert!(last.starts_with(&prefix));
        assert_eq!(&last[prefix.len()..], &3u64.to_be_bytes());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_key_with_prefix_and_count_cf() {
        let dir = tmp("lastpref");
        let db = DB::open_cf(&Options::new(), &dir, &["write"]).unwrap();
        let cf = db.cf_handle("write").unwrap();
        for u in 0..8u8 {
            for ver in 1..=3u8 {
                let mut k = format!("u/{u:02}").into_bytes();
                k.extend_from_slice(&u64::from(ver).to_be_bytes());
                db.put_cf(&cf, &k, [ver]).unwrap();
            }
        }
        let prefix = b"u/03".as_slice();
        let last = db
            .last_key_with_prefix(&cf, prefix)
            .unwrap()
            .expect("user 03");
        assert!(last.starts_with(prefix), "{last:?}");
        assert_eq!(&last[prefix.len()..], &3u64.to_be_bytes());
        db.delete_cf(&cf, &last).unwrap();
        let prev = db
            .last_key_with_prefix(&cf, prefix)
            .unwrap()
            .expect("older");
        assert_eq!(&prev[prefix.len()..], &2u64.to_be_bytes());
        let n = db.count_cf(&cf, b"u/00", b"u/05", 25).unwrap();
        assert_eq!(n, 14); // 5 users × 3 vers − 1 delete
        let def = db.cf_handle(DEFAULT_CF).unwrap();
        db.put_cf(&def, &prev, b"val").unwrap();
        let got = db
            .last_prefix_then_get(&cf, prefix, &def)
            .unwrap()
            .expect("combined");
        assert_eq!(got, b"val");
        // RFC-0041: name-based read APIs match handle APIs (no handle alloc).
        assert_eq!(
            db.last_key_named("write", prefix).unwrap(),
            db.last_key_with_prefix(&cf, prefix).unwrap()
        );
        assert_eq!(
            db.count_named("write", b"u/00", b"u/05", 25).unwrap(),
            db.count_cf(&cf, b"u/00", b"u/05", 25).unwrap()
        );
        assert_eq!(
            db.get_named(DEFAULT_CF, &prev).unwrap().as_deref(),
            Some(b"val".as_ref())
        );
        assert_eq!(got, b"val");
        let probe = db.read_probe();
        assert_eq!(probe.mvcc_split_ops, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_owned_moves_values_and_is_durable() {
        let dir = tmp("writeown");
        let db = DB::open_cf(&g1_opts(), &dir, &["write"]).unwrap();
        let cf = db.cf_handle("write").unwrap();
        let mut wb = WriteBatch::new();
        wb.put_cf(&cf, b"k1", b"payload-one");
        wb.put_cf(&cf, b"k2", vec![0xcd; 1024]);
        db.write_owned(wb).unwrap();
        assert_eq!(
            db.get_named("write", b"k1").unwrap().as_deref(),
            Some(b"payload-one".as_ref())
        );
        let big = db.get_named("write", b"k2").unwrap().expect("k2");
        assert_eq!(big.len(), 1024);
        assert!(big.iter().all(|&b| b == 0xcd));
        drop(db);
        let db = DB::open_cf(&g1_opts(), &dir, &["write"]).unwrap();
        assert_eq!(
            db.get_named("write", b"k1").unwrap().as_deref(),
            Some(b"payload-one".as_ref())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn count_named_tls_hits_then_invalidates_on_put() {
        let dir = tmp("counttls");
        let db = DB::open_cf(&Options::new(), &dir, &["write"]).unwrap();
        let cf = db.cf_handle("write").unwrap();
        for i in 0..8u8 {
            db.put_cf(&cf, [b'k', i], [b'v', i]).unwrap();
        }
        let a = db.count_named("write", b"k", b"z", 25).unwrap();
        let b = db.count_named("write", b"k", b"z", 25).unwrap();
        assert_eq!(a, 8);
        assert_eq!(b, 8, "zipf-style repeat must return the same count");
        db.put_cf(&cf, b"ky", b"new").unwrap();
        let c = db.count_named("write", b"k", b"z", 25).unwrap();
        assert_eq!(c, 9, "TLS last-count must miss after a published put");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn count_named_tls_last_n_keeps_two_windows() {
        let dir = tmp("counttlsn");
        let db = DB::open_cf(&Options::new(), &dir, &["write"]).unwrap();
        let cf = db.cf_handle("write").unwrap();
        for i in 0..8u8 {
            db.put_cf(&cf, [b'k', i], [b'v', i]).unwrap();
        }
        let a = db.count_named("write", b"k", b"kd", 25).unwrap();
        let b = db.count_named("write", b"kd", b"z", 25).unwrap();
        assert_eq!(a, db.count_named("write", b"k", b"kd", 25).unwrap());
        assert_eq!(b, db.count_named("write", b"kd", b"z", 25).unwrap());
        assert_eq!(a + b, 8);
        db.put_cf(&cf, b"ky", b"new").unwrap();
        let a2 = db.count_named("write", b"k", b"kd", 25).unwrap();
        let b2 = db.count_named("write", b"kd", b"z", 25).unwrap();
        assert_eq!(a2, a, "window below the new key stays");
        assert_eq!(b2, b + 1, "epoch bump must recompute the covering window");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_default_tls_prefixed_cf_hits_then_invalidates() {
        // Official YCSB-C opens with DEPS_CFS, so `default` is prefixed.
        let dir = tmp("getdefpx");
        let db = DB::open_cf(&Options::new(), &dir, &["write"]).unwrap();
        db.put(b"hot", b"v1").unwrap();
        assert_eq!(db.get(b"hot").unwrap().as_deref(), Some(b"v1".as_ref()));
        assert_eq!(
            db.get(b"hot").unwrap().as_deref(),
            Some(b"v1".as_ref()),
            "prefixed default get() must still last-N hit"
        );
        db.put(b"hot", b"v2").unwrap();
        assert_eq!(
            db.get(b"hot").unwrap().as_deref(),
            Some(b"v2".as_ref()),
            "prefixed default last-N must miss after put"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_default_tls_hits_then_invalidates_on_put() {
        let dir = tmp("getdeftls");
        let db = DB::open_default(&dir).unwrap();
        db.put(b"hot", b"v1").unwrap();
        let a = db.get(b"hot").unwrap();
        let b = db.get(b"hot").unwrap();
        assert_eq!(a.as_deref(), Some(b"v1".as_ref()));
        assert_eq!(b, a, "YCSB-C get() must hit the key-only last-N");
        db.put(b"hot", b"v2").unwrap();
        let c = db.get(b"hot").unwrap();
        assert_eq!(
            c.as_deref(),
            Some(b"v2".as_ref()),
            "default last-N must miss after a published put"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_default_tls_keeps_zipf_working_set() {
        let dir = tmp("getdefws");
        let db = DB::open_default(&dir).unwrap();
        for i in 0..200u16 {
            let k = format!("ycsb/{i:06}").into_bytes();
            db.put(&k, [b'v', (i & 0xff) as u8]).unwrap();
        }
        for i in 0..200u16 {
            let k = format!("ycsb/{i:06}").into_bytes();
            assert_eq!(
                db.get(&k).unwrap().as_deref(),
                Some([b'v', (i & 0xff) as u8].as_ref()),
                "fill {i}"
            );
        }
        for i in 0..200u16 {
            let k = format!("ycsb/{i:06}").into_bytes();
            assert_eq!(
                db.get(&k).unwrap().as_deref(),
                Some([b'v', (i & 0xff) as u8].as_ref()),
                "key-only last-N must still answer ycsb/{i:06}"
            );
        }
        db.put(b"other", b"x").unwrap();
        let k0 = format!("ycsb/{:06}", 0).into_bytes();
        assert_eq!(
            db.get(&k0).unwrap().as_deref(),
            Some([b'v', 0].as_ref()),
            "epoch bump must not serve a stale default last-N value"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_named_tls_hits_then_invalidates_on_put() {
        let dir = tmp("gettls");
        let db = DB::open_default(&dir).unwrap();
        db.put(b"hot", b"v1").unwrap();
        let a = db.get_named(DEFAULT_CF, b"hot").unwrap();
        let b = db.get_named(DEFAULT_CF, b"hot").unwrap();
        assert_eq!(a.as_deref(), Some(b"v1".as_ref()));
        assert_eq!(b, a, "zipf-style repeat must return the same bytes");
        db.put(b"hot", b"v2").unwrap();
        let c = db.get_named(DEFAULT_CF, b"hot").unwrap();
        assert_eq!(
            c.as_deref(),
            Some(b"v2".as_ref()),
            "TLS last-get must miss after a published put"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_named_tls_last_n_keeps_two_hot_keys() {
        let dir = tmp("gettlsn");
        let db = DB::open_default(&dir).unwrap();
        db.put(b"hot", b"v1").unwrap();
        db.put(b"hot2", b"v2").unwrap();
        assert_eq!(
            db.get_named(DEFAULT_CF, b"hot").unwrap().as_deref(),
            Some(b"v1".as_ref())
        );
        assert_eq!(
            db.get_named(DEFAULT_CF, b"hot2").unwrap().as_deref(),
            Some(b"v2".as_ref())
        );
        assert_eq!(
            db.get_named(DEFAULT_CF, b"hot").unwrap().as_deref(),
            Some(b"v1".as_ref()),
            "last-N must still hold the first key after a second fill"
        );
        db.put(b"hot", b"v3").unwrap();
        assert_eq!(
            db.get_named(DEFAULT_CF, b"hot").unwrap().as_deref(),
            Some(b"v3".as_ref()),
            "put of hot must publish the new value"
        );
        assert_eq!(
            db.get_named(DEFAULT_CF, b"hot2").unwrap().as_deref(),
            Some(b"v2".as_ref())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_named_tls_direct_map_keeps_zipf_hot_set() {
        let dir = tmp("gettlsdm");
        let db = DB::open_default(&dir).unwrap();
        for i in 0..64u8 {
            db.put([b'k', i], [b'v', i]).unwrap();
        }
        for i in 0..64u8 {
            assert_eq!(
                db.get_named(DEFAULT_CF, [b'k', i]).unwrap().as_deref(),
                Some([b'v', i].as_ref()),
                "fill key {i}"
            );
        }
        for i in 0..64u8 {
            assert_eq!(
                db.get_named(DEFAULT_CF, [b'k', i]).unwrap().as_deref(),
                Some([b'v', i].as_ref()),
                "direct-map last-N must still answer key {i}"
            );
        }
        db.put(b"other", b"x").unwrap();
        assert_eq!(
            db.get_named(DEFAULT_CF, [b'k', 0]).unwrap().as_deref(),
            Some([b'v', 0].as_ref()),
            "1c put of another key must not serve a stale last-N value"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_get_survives_put_of_other_key() {
        let dir = tmp("gettls-other");
        let db = DB::open_default(&dir).unwrap();
        db.put(b"hot", b"v1").unwrap();
        db.put(b"hot2", b"v2").unwrap();
        assert!(db.last_get_is_hot(b"hot"));
        assert!(db.last_get_is_hot(b"hot2"));
        db.put(b"other", b"x").unwrap();
        assert!(
            db.last_get_is_hot(b"hot"),
            "1c put of another key must not wipe TLS of zipf neighbors"
        );
        assert!(db.last_get_is_hot(b"hot2"));
        assert_eq!(db.get(b"hot").unwrap().as_deref(), Some(b"v1".as_ref()));
        db.put(b"hot", b"v3").unwrap();
        assert_eq!(db.get(b"hot").unwrap().as_deref(), Some(b"v3".as_ref()));
        assert_eq!(db.get(b"hot2").unwrap().as_deref(), Some(b"v2".as_ref()));
        assert!(
            db.last_get_is_hot(b"hot2"),
            "overwrite of hot must leave hot2 TLS-hot"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn intern_put_value_shares_repeat_payload() {
        let a = intern_put_value(b"yyyy");
        let b = intern_put_value(b"yyyy");
        assert_eq!(
            a.as_ptr(),
            b.as_ptr(),
            "repeat SET payload must share Bytes"
        );
        let c = intern_put_value(b"zzzz");
        assert_ne!(a.as_ptr(), c.as_ptr());
        let dir = tmp("intern-put");
        let db = DB::open_default(&dir).unwrap();
        db.put(b"k1", b"yyyy").unwrap();
        db.put(b"k2", b"yyyy").unwrap();
        assert!(db.last_get_is_hot(b"k2"));
        assert_eq!(db.get(b"k1").unwrap().as_deref(), Some(b"yyyy".as_ref()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_get_other_thread_sees_put() {
        let dir = tmp("gettls-thr");
        let db = std::sync::Arc::new(DB::open_default(&dir).unwrap());
        db.put(b"k", b"v1").unwrap();
        let db_r = std::sync::Arc::clone(&db);
        let h = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match db_r.get(b"k").unwrap().as_deref() {
                    Some(b"v1") => {
                        assert!(
                            Instant::now() < deadline,
                            "reader never observed the published put"
                        );
                    }
                    Some(b"v2") => return,
                    other => panic!("unexpected last-get value {other:?}"),
                }
            }
        });
        db.put(b"k", b"v2").unwrap();
        h.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_cf_owned_grouped_multi_cf_is_atomic() {
        let dir = tmp("cfowned-group");
        let db = DB::open_cf(&g1_opts(), &dir, &["lock", "default"]).unwrap();
        // Interleaved lock/default — encode groups by CF; both must land.
        let puts = vec![
            ("lock", b"k1".to_vec(), b"L1".to_vec()),
            ("default", b"k1".to_vec(), b"D1".to_vec()),
            ("lock", b"k2".to_vec(), b"L2".to_vec()),
            ("default", b"k2".to_vec(), b"D2".to_vec()),
        ];
        db.write_cf_owned(puts, vec![]).unwrap();
        assert_eq!(
            db.get_named("lock", b"k1").unwrap().as_deref(),
            Some(b"L1".as_ref())
        );
        assert_eq!(
            db.get_named("default", b"k1").unwrap().as_deref(),
            Some(b"D1".as_ref())
        );
        assert_eq!(
            db.get_named("lock", b"k2").unwrap().as_deref(),
            Some(b"L2".as_ref())
        );
        assert_eq!(
            db.get_named("default", b"k2").unwrap().as_deref(),
            Some(b"D2".as_ref())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_cf_owned_moves_values_and_is_durable() {
        let dir = tmp("cfowned");
        let db = DB::open_cf(&g1_opts(), &dir, &["raftlog"]).unwrap();
        db.write_cf_owned(
            vec![("raftlog", b"raftlog/00000001".to_vec(), vec![0xab; 1024])],
            vec![],
        )
        .unwrap();
        let got = db
            .get_named("raftlog", b"raftlog/00000001")
            .unwrap()
            .expect("owned put");
        assert_eq!(got.len(), 1024);
        assert!(got.iter().all(|&b| b == 0xab));
        drop(db);
        let db = DB::open_cf(&g1_opts(), &dir, &["raftlog"]).unwrap();
        let got = db
            .get_named("raftlog", b"raftlog/00000001")
            .unwrap()
            .expect("replay");
        assert_eq!(got.len(), 1024);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_cf_owned_warms_named_get_tls() {
        let dir = tmp("cfowned-tls");
        let db = DB::open_cf(&g1_opts(), &dir, &["raftlog"]).unwrap();
        let mut puts = Vec::new();
        for i in 1u8..=16 {
            puts.push((
                "raftlog",
                format!("raftlog/{i:08}").into_bytes(),
                vec![i; 100],
            ));
        }
        let syncs_before = db.inner.wal_sync_count();
        db.write_cf_owned(puts, vec![]).unwrap();
        assert_eq!(
            db.inner.wal_sync_count().saturating_sub(syncs_before),
            1,
            "G1 raftlog batch is one WAL barrier, not 16"
        );
        let got = db
            .get_named("raftlog", b"raftlog/00000016")
            .unwrap()
            .expect("last of batch");
        assert_eq!(got.len(), 100);
        assert_eq!(got[0], 16);
        // Bench reads idx-1 after a 16-append (every 8th op).
        let prev = db
            .get_named("raftlog", b"raftlog/00000015")
            .unwrap()
            .expect("idx-1 of batch");
        assert_eq!(prev[0], 15);
        assert!(
            db.last_cf_is_hot("raftlog", b"raftlog/00000015"),
            "idx-1 must be a TLS hit (LAST_RING holds the 16-key batch)"
        );
        for i in 1u8..=16 {
            assert!(
                db.last_cf_is_hot("raftlog", format!("raftlog/{i:08}").as_bytes()),
                "key {i} of the 16-append batch must stay TLS-hot"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0159 P1.5: hydrate-shaped `write_cf_owned` latches and reads back.
    #[test]
    fn write_cf_owned_latched_hydrate_roundtrip() {
        let dir = tmp("cfowned-bulk");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        opts.set_sync(false);
        let db = DB::open_cf(&opts, &dir, &["data", "meta"]).unwrap();
        let val = vec![b'v'; 64];
        let mut i = 0u32;
        for _ in 0..12u32 {
            let end = i + 32;
            let mut puts = Vec::with_capacity(33);
            for j in i..end {
                puts.push((
                    "data",
                    format!("route.svc-{j:06}").into_bytes(),
                    val.clone(),
                ));
            }
            puts.push(("meta", b"cursor".to_vec(), i.to_le_bytes().to_vec()));
            db.write_cf_owned(puts, Vec::new()).unwrap();
            i = end;
        }
        assert!(
            db.inner.family_is_latched_async("data"),
            "12 hydrate batches must latch data"
        );
        let last = format!("route.svc-{:06}", i - 1).into_bytes();
        assert_eq!(
            db.get_named("data", &last).unwrap().as_deref(),
            Some(val.as_slice())
        );
        assert_eq!(
            db.get_named("meta", b"cursor").unwrap().as_deref(),
            Some((i - 32).to_le_bytes().as_ref())
        );
        db.flush().unwrap();
        drop(db);
        let db = DB::open_cf(&opts, &dir, &["data", "meta"]).unwrap();
        assert_eq!(
            db.get_named("data", &last).unwrap().as_deref(),
            Some(val.as_slice())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Slipstream path: `WriteBatch` + `write_opt` after latch, then get.
    #[test]
    fn write_opt_latched_hydrate_roundtrip() {
        let dir = tmp("writeopt-bulk");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        opts.set_sync(false);
        let db = DB::open_cf(&opts, &dir, &["data", "meta"]).unwrap();
        let data = db.cf_handle("data").unwrap();
        let meta = db.cf_handle("meta").unwrap();
        let val = vec![b'v'; 64];
        let mut wo = WriteOptions::default();
        wo.set_sync(false);
        let mut i = 0u32;
        for _ in 0..12u32 {
            let end = i + 32;
            let mut wb = WriteBatch::default();
            for j in i..end {
                wb.put_cf(&data, format!("route.svc-{j:06}").as_bytes(), &val);
            }
            wb.put_cf(&meta, b"cursor", i.to_le_bytes());
            db.write_opt(&wb, &wo).unwrap();
            i = end;
        }
        assert!(db.inner.family_is_latched_async("data"));
        let last = format!("route.svc-{:06}", i - 1);
        assert_eq!(
            db.get_named("data", last.as_bytes()).unwrap().as_deref(),
            Some(val.as_slice())
        );
        db.flush().unwrap();
        assert_eq!(
            db.get_named("data", last.as_bytes()).unwrap().as_deref(),
            Some(val.as_slice())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0062 P1.1: 16 identical raftlog payloads stay readable (WAL v2 intern).
    #[test]
    fn write_cf_owned_sixteen_same_payload_roundtrip() {
        let dir = tmp("cfowned-intern");
        let db = DB::open_cf(&g1_opts(), &dir, &["raftlog"]).unwrap();
        let val = vec![b'r'; 100];
        let puts: Vec<_> = (1..=16)
            .map(|i| {
                (
                    "raftlog",
                    format!("raftlog/{i:08}").into_bytes(),
                    val.clone(),
                )
            })
            .collect();
        db.write_cf_owned(puts, vec![]).unwrap();
        for i in 1..=16 {
            let got = db
                .get_named("raftlog", format!("raftlog/{i:08}").as_bytes())
                .unwrap()
                .expect("interned put");
            assert_eq!(got, val);
        }
        drop(db);
        let db = DB::open_cf(&g1_opts(), &dir, &["raftlog"]).unwrap();
        assert_eq!(
            db.get_named("raftlog", b"raftlog/00000016")
                .unwrap()
                .as_deref(),
            Some(val.as_slice())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_cf_slices_is_durable() {
        let dir = tmp("cfslices");
        let db = DB::open_cf(&g1_opts(), &dir, &["raftlog"]).unwrap();
        db.write_cf_slices(
            &[(
                "raftlog",
                b"raftlog/00000001".as_slice(),
                b"entry".as_slice(),
            )],
            &[],
        )
        .unwrap();
        assert_eq!(
            db.get_named("raftlog", b"raftlog/00000001")
                .unwrap()
                .as_deref(),
            Some(b"entry".as_ref())
        );
        drop(db);
        let db = DB::open_cf(&g1_opts(), &dir, &["raftlog"]).unwrap();
        assert_eq!(
            db.get_named("raftlog", b"raftlog/00000001")
                .unwrap()
                .as_deref(),
            Some(b"entry".as_ref())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_get_table_lazy_epoch_invalidation() {
        let mut t = LastGetTable::new();
        t.store_key(1, 1, b"k1", Some(Bytes::from_static(b"v1")));
        assert_eq!(
            t.get_key(1, 1, b"k1"),
            Some(Some(Bytes::from_static(b"v1")))
        );
        // Epoch bump: the stale entry must not answer for the new epoch.
        assert_eq!(t.get_key(2, 1, b"k1"), None);
        // Re-store under the new epoch answers without any clear-all pass.
        t.store_key(2, 1, b"k1", Some(Bytes::from_static(b"v2")));
        assert_eq!(
            t.get_key(2, 1, b"k1"),
            Some(Some(Bytes::from_static(b"v2")))
        );
        // An entry stored under an old epoch coexists but never leaks.
        t.store_key(1, 1, b"k2", Some(Bytes::from_static(b"old")));
        assert_eq!(t.get_key(2, 1, b"k2"), None);
        assert_eq!(
            t.get_key(1, 1, b"k2"),
            Some(Some(Bytes::from_static(b"old")))
        );
        // Per-key gen bump: same epoch, new gen misses; other gen stays.
        t.store_key(2, 2, b"k1", Some(Bytes::from_static(b"v3")));
        assert_eq!(t.get_key(2, 1, b"k1"), None);
        assert_eq!(
            t.get_key(2, 2, b"k1"),
            Some(Some(Bytes::from_static(b"v3")))
        );
    }

    #[test]
    fn last_get_table_keeps_uniform_working_set() {
        // kvrocks-shaped uniform hot set (1024 × `k/NNNNNN`) must stay
        // ≥95% cached (RFC-0044 P1.3): 2048 slots, 4-probe, stale-preferred
        // eviction. Deterministic: fixed key strings, fixed hash.
        let mut t = LastGetTable::new();
        let keys: Vec<Vec<u8>> = (0..1024)
            .map(|i| format!("k/{i:06}").into_bytes())
            .collect();
        for k in &keys {
            t.store_key(7, 1, k, Some(Bytes::from_static(b"v")));
        }
        let hits = keys.iter().filter(|k| t.get_key(7, 1, k).is_some()).count();
        assert!(hits >= 972, "uniform hot set hit rate {hits}/1024 < 95%");
    }

    #[test]
    fn last_cf_hash_keeps_lookup_100() {
        // Slipstream lookup_100: 100 named-CF keys, same set every iter.
        // Ring-only store (16) cannot hold them; hash_store must.
        let mut t = LastGetTable::new();
        let keys: Vec<Vec<u8>> = (0..100)
            .map(|i| format!("route.svc-{:06}.{:08}", i / 4, i % 4).into_bytes())
            .collect();
        for k in &keys {
            t.store(3, 1, "data", k, Some(Bytes::from_static(b"v")));
        }
        let ring_hits = keys
            .iter()
            .filter(|k| t.get(3, 1, "data", k).is_some())
            .count();
        assert!(
            ring_hits < 100,
            "ring-only must not hold the whole lookup_100 set (got {ring_hits})"
        );
        for k in &keys {
            t.hash_store(3, 1, "data", k, Some(Bytes::from_static(b"v")));
        }
        let hits = keys
            .iter()
            .filter(|k| t.get(3, 1, "data", k).is_some())
            .count();
        assert_eq!(hits, 100, "hash_store must keep all 100 lookup keys");
    }

    #[test]
    fn last_cf_ring_holds_long_slipstream_keys() {
        // TINY was 64; ~60–80 B slipstream keys silently failed
        // TinyBuf::from_slice and LAST_CF never warmed (Mac get_hit sample
        // was 100 % inner.get).
        let mut t = LastGetTable::new();
        let k: Vec<u8> = (0..80).map(|i| b'a' + (i % 26)).collect();
        t.store(1, 1, "data", &k, Some(Bytes::from_static(b"v")));
        assert_eq!(
            t.get(1, 1, "data", &k),
            Some(Some(Bytes::from_static(b"v"))),
            "80-byte named-CF keys must fit LAST_CF"
        );
    }

    #[test]
    fn fold_gc_keeps_pinned_snapshot_and_bounds_versions() {
        // Compat opens with fold version GC on (rust-rocksdb snapshot-list
        // semantics). A pinned Snapshot must read its exact version across
        // folds; with no snapshot open, superseded versions collapse.
        let d = tmp("fold_gc_compat");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        let db = DB::open(&opts, &d).unwrap();
        for i in 0..200u32 {
            db.put(b"hot", format!("v{i}").as_bytes()).unwrap();
        }
        let snap = db.snapshot();
        for i in 200..400u32 {
            db.put(b"hot", format!("v{i}").as_bytes()).unwrap();
        }
        // Drain the compat worker's fold ticks synchronously (write lock).
        for _ in 0..64 {
            if db.inner.fold_parked_once_off_lock() {
                // folded — keep draining until the worker parks
            } else {
                break;
            }
        }
        assert!(
            db.inner.fold_gc_enabled(),
            "compat opens with fold version GC on"
        );
        assert_eq!(snap.get(b"hot").unwrap().as_deref(), Some(&b"v199"[..]));
        assert_eq!(db.get(b"hot").unwrap().as_deref(), Some(&b"v399"[..]));
        drop(snap);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn tls_get_cache_never_answers_across_instances() {
        // Two DB instances share one thread's TLS tables. Distinct
        // `cache_epoch_base` (fix C1/C1b) keeps instance A's entries from
        // answering for instance B even before any write bumps an epoch.
        let d1 = tmp("tls_cross_a");
        let d2 = tmp("tls_cross_b");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        let a = DB::open(&opts, &d1).unwrap();
        let b = DB::open(&opts, &d2).unwrap();
        a.put(b"shared", b"from-a").unwrap();
        // Fill A's TLS entry (default-CF get + contains share the table).
        assert_eq!(a.get(b"shared").unwrap().as_deref(), Some(&b"from-a"[..]));
        assert!(a.contains(b"shared").unwrap());
        assert_eq!(b.get(b"shared").unwrap(), None);
        assert!(!b.contains(b"shared").unwrap());
        b.put(b"shared", b"from-b").unwrap();
        assert_eq!(b.get(b"shared").unwrap().as_deref(), Some(&b"from-b"[..]));
        assert_eq!(a.get(b"shared").unwrap().as_deref(), Some(&b"from-a"[..]));
        let _ = std::fs::remove_dir_all(&d1);
        let _ = std::fs::remove_dir_all(&d2);
    }

    #[test]
    fn sst_writer_ingest_roundtrip() {
        let dir = tmp("ingest");
        let sst = dir.join("ext.sst");
        let mut w = SstFileWriter::create(&Options::new());
        w.open(&sst).unwrap();
        w.put(b"ik", b"iv").unwrap();
        w.put(b"jk", b"jv").unwrap();
        w.finish().unwrap();
        let db = DB::open_default(&dir).unwrap();
        db.ingest_external_file(vec![&sst]).unwrap();
        assert_eq!(db.get(b"ik").unwrap().as_deref(), Some(&b"iv"[..]));
        assert_eq!(db.get(b"jk").unwrap().as_deref(), Some(&b"jv"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_file_in_range_tombstones_keys() {
        let dir = tmp("dfr");
        let db = DB::open_default(&dir).unwrap();
        db.put(b"a", b"1").unwrap();
        db.put(b"b", b"2").unwrap();
        db.put(b"c", b"3").unwrap();
        db.delete_file_in_range(b"a", b"c").unwrap();
        assert!(db.get(b"a").unwrap().is_none());
        assert!(db.get(b"b").unwrap().is_none());
        assert_eq!(db.get(b"c").unwrap().as_deref(), Some(&b"3"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wbwi_read_your_writes() {
        let dir = tmp("wbwi");
        let db = DB::open_default(&dir).unwrap();
        db.put(b"k", b"old").unwrap();
        let mut b = WriteBatchWithIndex::new();
        b.put(b"k", b"new");
        b.put(b"n", b"x");
        assert_eq!(
            b.get_from_batch_and_db(&db, b"k").unwrap().as_deref(),
            Some(&b"new"[..])
        );
        assert_eq!(
            b.get_from_batch_and_db(&db, b"n").unwrap().as_deref(),
            Some(&b"x"[..])
        );
        db.write(b.get_write_batch()).unwrap();
        assert_eq!(db.get(b"k").unwrap().as_deref(), Some(&b"new"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compaction_filter_removes_keys() {
        let dir = tmp("cfilt");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        opts.set_compaction_filter("drop-x", |_lvl, key, _val| {
            if key == b"x" {
                CompactionDecision::Remove
            } else {
                CompactionDecision::Keep
            }
        });
        let db = DB::open(&opts, &dir).unwrap();
        db.put(b"x", b"1").unwrap();
        db.put(b"y", b"2").unwrap();
        db.flush().unwrap();
        db.compact().unwrap();
        assert!(db.get(b"x").unwrap().is_none());
        assert_eq!(db.get(b"y").unwrap().as_deref(), Some(&b"2"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_with_filter_drops_prefix() {
        let dir = tmp("cfilt-oneshot");
        let db = DB::open_default(&dir).unwrap();
        db.put(b"keep/a", b"1").unwrap();
        db.put(b"drop/a", b"2").unwrap();
        db.flush().unwrap();
        db.compact_with_filter(|_lvl, key, _val| {
            if key.starts_with(b"drop/") {
                CompactionDecision::Remove
            } else {
                CompactionDecision::Keep
            }
        })
        .unwrap();
        assert!(db.get(b"drop/a").unwrap().is_none());
        assert_eq!(db.get(b"keep/a").unwrap().as_deref(), Some(&b"1"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0065 P0.1: flush with lock+default keys yields one SST per family.
    #[test]
    fn flush_emits_one_sst_per_cf() {
        let dir = tmp("cf-split-flush");
        let db = DB::open_cf(&Options::new(), &dir, &["lock", "write"]).unwrap();
        let lock = db.cf_handle("lock").unwrap();
        let write = db.cf_handle("write").unwrap();
        db.put(b"dk", b"dv").unwrap();
        db.put_cf(&lock, b"lk", b"lv").unwrap();
        db.put_cf(&write, b"wk", b"wv").unwrap();
        db.flush().unwrap();
        let files = db.live_files().unwrap();
        let mut cfs: Vec<_> = files
            .iter()
            .map(|f| f.column_family_name.as_str())
            .collect();
        cfs.sort_unstable();
        cfs.dedup();
        assert!(
            cfs.contains(&"lock") && cfs.contains(&"default") && cfs.contains(&"write"),
            "expected SST per CF, live={files:?}"
        );
        drop(db);
        let db = DB::open_cf(&Options::new(), &dir, &["lock", "write"]).unwrap();
        let lock = db.cf_handle("lock").unwrap();
        let write = db.cf_handle("write").unwrap();
        assert_eq!(db.get(b"dk").unwrap().as_deref(), Some(&b"dv"[..]));
        assert_eq!(
            db.get_cf(&lock, b"lk").unwrap().as_deref(),
            Some(&b"lv"[..])
        );
        assert_eq!(
            db.get_cf(&write, b"wk").unwrap().as_deref(),
            Some(&b"wv"[..])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0065 P1.3: flush_cf(lock) does not emit a default SST.
    #[test]
    fn flush_cf_lock_leaves_default_in_mem() {
        let dir = tmp("cf-flush-lock");
        let db = DB::open_cf(&g1_opts(), &dir, &["lock"]).unwrap();
        let lock = db.cf_handle("lock").unwrap();
        db.put(b"dk", b"dv").unwrap();
        db.put_cf(&lock, b"lk", b"lv").unwrap();
        let def_before = db
            .live_files()
            .unwrap()
            .into_iter()
            .filter(|f| f.column_family_name == "default")
            .count();
        db.flush_cf(&lock).unwrap();
        let def_after = db
            .live_files()
            .unwrap()
            .into_iter()
            .filter(|f| f.column_family_name == "default")
            .count();
        assert_eq!(def_before, def_after);
        assert!(db
            .live_files()
            .unwrap()
            .iter()
            .any(|f| f.column_family_name == "lock"));
        assert_eq!(db.get(b"dk").unwrap().as_deref(), Some(&b"dv"[..]));
        assert_eq!(
            db.get_cf(&lock, b"lk").unwrap().as_deref(),
            Some(&b"lv"[..])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0065 P0.2: compact_range_cf(lock) does not rewrite default SSTs.
    #[test]
    fn compact_range_cf_lock_leaves_default() {
        let dir = tmp("cf-compact-lock");
        let db = DB::open_cf(&Options::new(), &dir, &["lock"]).unwrap();
        let lock = db.cf_handle("lock").unwrap();
        db.put(b"d0", b"0").unwrap();
        db.put_cf(&lock, b"l0", b"0").unwrap();
        db.flush().unwrap();
        db.put(b"d1", b"1").unwrap();
        db.put_cf(&lock, b"l1", b"1").unwrap();
        db.flush().unwrap();
        let default_before: Vec<_> = db
            .live_files()
            .unwrap()
            .into_iter()
            .filter(|f| f.column_family_name == "default")
            .map(|f| (f.name, f.size, f.level, f.num_entries))
            .collect();
        assert!(
            default_before.len() >= 2,
            "need ≥2 default SSTs, got {default_before:?}"
        );
        db.compact_range_cf(&lock, None::<&[u8]>, None::<&[u8]>);
        let default_after: Vec<_> = db
            .live_files()
            .unwrap()
            .into_iter()
            .filter(|f| f.column_family_name == "default")
            .map(|f| (f.name, f.size, f.level, f.num_entries))
            .collect();
        assert_eq!(default_before, default_after);
        assert_eq!(db.get(b"d0").unwrap().as_deref(), Some(&b"0"[..]));
        assert_eq!(db.get_cf(&lock, b"l0").unwrap().as_deref(), Some(&b"0"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keycodec_encode_decode_roundtrip_uses_cf_kernel() {
        for default_raw in [false, true] {
            let codec = KeyCodec { default_raw };
            for (cf, key) in [
                (DEFAULT_CF, b"k".as_slice()),
                ("lock", b"lk".as_slice()),
                ("write", &[0u8, 1, 255][..]),
            ] {
                let enc = codec.encode(cf, key);
                assert_eq!(codec.decode(cf, &enc), key);
                assert!(
                    key_in_cf_family(&enc, cf),
                    "cf={cf} default_raw={default_raw}"
                );
                assert_eq!(enc, encode_cf_key(cf, key, default_raw));
            }
        }
        assert!(
            !key_in_cf_family(&encode_cf_key("lock", b"k", false), "default"),
            "named CF must not leak into default"
        );
    }

    #[test]
    fn merge_operator_rmw() {
        let dir = tmp("merge");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        opts.set_merge_operator_associative("concat", |_k, exist, ops| {
            let mut out = exist.unwrap_or(&[]).to_vec();
            for o in ops.iter() {
                out.extend_from_slice(o);
            }
            Some(out)
        });
        let db = DB::open(&opts, &dir).unwrap();
        db.put(b"k", b"a").unwrap();
        db.merge(b"k", b"b").unwrap();
        assert_eq!(db.get(b"k").unwrap().as_deref(), Some(&b"ab"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_list_drop_cf() {
        let dir = tmp("cflife");
        let opts = g1_opts();
        let db = DB::open_cf(&opts, &dir, &["cf1"]).unwrap();
        db.create_cf("cf2", &Options::new()).unwrap();
        assert!(db.cf_handle("cf2").is_some());
        db.put_cf(&db.cf_handle("cf2").unwrap(), b"k", b"v")
            .unwrap();
        db.drop_cf("cf2").unwrap();
        assert!(db.cf_handle("cf2").is_none());
        let listed = DB::<StdEnv>::list_cf(&opts, &dir).unwrap();
        assert!(listed.contains(&"cf1".to_string()));
        assert!(!listed.contains(&"cf2".to_string()));
        // DestroyDB contract: the instance must be closed first — its
        // compaction thread would race `remove_dir_all` (ENOTEMPTY).
        drop(db);
        DB::<StdEnv>::destroy(&opts, &dir).unwrap();
    }

    #[test]
    fn cfreg_crc_mismatch_fails_closed() {
        let dir = tmp("cfreg-crc");
        let opts = g1_opts();
        {
            let db = DB::open_cf(&opts, &dir, &["write"]).unwrap();
            db.put(b"k", b"v").unwrap();
            drop(db);
        }
        let path = dir.join("CFREG");
        let mut raw = std::fs::read(&path).unwrap();
        let text = std::str::from_utf8(&raw).unwrap();
        assert!(
            text.lines().any(|l| l.starts_with("c:")),
            "new CFREG must carry a crc line: {text:?}"
        );
        let last = raw.len() - 2;
        raw[last] = if raw[last] == b'0' { b'1' } else { b'0' };
        std::fs::write(&path, &raw).unwrap();
        let err = match DB::open_cf(&opts, &dir, &["write"]) {
            Err(e) => e,
            Ok(_) => panic!("poison CFREG crc must refuse open"),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("crc") || msg.contains("CFREG"),
            "open must fail-closed on CFREG crc, got {msg}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0090 P2.1: production `store_cf_registry` writes `CFREG`; XOR
    /// only the stored CRC u32 and rewrite as `c:` 8 hex (prefix intact).
    /// Open is crc mismatch. AS-IS would load the registry. ASCII XOR of
    /// a hex digit (`cfreg_crc_mismatch_fails_closed`) is not this tooth
    /// unless it pins `crc_match_ok`.
    #[test]
    fn crc_mismatch_on_live_cfreg_is_not_ok() {
        assert!(!pedradb_core::wal::crc::crc_match_ok(1, 2));
        assert!(
            pedradb_core::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any CFREG crc would match"
        );
        let dir = tmp("cfreg-0090");
        let opts = g1_opts();
        {
            let db = DB::open_cf(&opts, &dir, &["write"]).unwrap();
            db.put(b"k", b"v").unwrap();
            drop(db);
        }
        let path = dir.join("CFREG");
        let raw = std::fs::read(&path).unwrap();
        let text = std::str::from_utf8(&raw).unwrap();
        let trimmed = text.trim_end_matches('\n');
        let (head, last) = trimmed.rsplit_once('\n').expect("CFREG crc line");
        let hex = last.strip_prefix("c:").expect("c: trailer");
        let crc = u32::from_str_radix(hex, 16).expect("CFREG crc hex");
        std::fs::write(&path, format!("{head}\nc:{:08x}\n", crc ^ 0xffff_ffff)).unwrap();
        match DB::open_cf(&opts, &dir, &["write"]) {
            Ok(_) => {
                let _ = std::fs::remove_dir_all(&dir);
                panic!("AS-IS hole: opened CFREG after CRC-hex lie");
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                let msg = e.to_string();
                assert!(
                    msg.to_ascii_lowercase().contains("crc mismatch"),
                    "must fail on crc_match_ok, not a pointer parse; got {msg}"
                );
            }
        }
    }

    #[test]
    fn cfreg_legacy_without_crc_still_opens() {
        let dir = tmp("cfreg-leg");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("CFREG"), b"COMPATCF1\nP\nwrite\n").unwrap();
        let opts = g1_opts();
        let db = DB::open_cf(&opts, &dir, &["write"]).unwrap();
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn property_int_value_maps_estimate_num_keys() {
        let dir = tmp("props");
        let db = DB::open_default(&dir).unwrap();
        db.put(b"a", b"1").unwrap();
        db.put(b"b", b"2").unwrap();
        let n = db
            .property_int_value(properties::ESTIMATE_NUM_KEYS)
            .unwrap()
            .expect("estimate-num-keys mapped");
        assert!(n >= 2, "estimate-num-keys={n}");
        let sst = db
            .property_int_value(properties::LIVE_SST_FILES_SIZE)
            .unwrap();
        assert!(sst.is_some());
        let l0 = db
            .property_int_value("rocksdb.num-files-at-level0")
            .unwrap();
        assert!(l0.is_some());
        assert!(db
            .property_int_value("rocksdb.no-such-property")
            .unwrap()
            .is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multi_get_cf_matches_get_cf() {
        let dir = tmp("mget-cf");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        opts.set_sync(false);
        let db = DB::open_cf(&opts, &dir, &["data"]).unwrap();
        let cf = db.cf_handle("data").expect("data cf");
        let keys: Vec<Vec<u8>> = (0..32u32)
            .map(|i| format!("k{i:04}").into_bytes())
            .collect();
        for (i, k) in keys.iter().enumerate() {
            db.put_cf(&cf, k, format!("v{i}").as_bytes()).unwrap();
        }
        db.flush().unwrap();
        let refs: Vec<_> = keys.iter().map(|k| (&cf, k.as_slice())).collect();
        let batched = db.multi_get_cf(refs);
        for (i, k) in keys.iter().enumerate() {
            let want = db.get_cf(&cf, k).unwrap();
            let got = batched[i].as_ref().unwrap().clone();
            assert_eq!(got, want, "multi_get_cf key {i}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multi_get_matches_get_default_cf() {
        let dir = tmp("mget-def");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        opts.set_sync(false);
        let db = DB::open(&opts, &dir).unwrap();
        db.put(b"a", b"1").unwrap();
        db.put(b"b", b"2").unwrap();
        db.put(b"c", b"3").unwrap();
        let batched = db.multi_get([
            "a".as_bytes(),
            "b".as_bytes(),
            "missing".as_bytes(),
            "c".as_bytes(),
        ]);
        assert_eq!(batched[0].as_ref().unwrap().as_deref(), Some(&b"1"[..]));
        assert_eq!(batched[1].as_ref().unwrap().as_deref(), Some(&b"2"[..]));
        assert_eq!(batched[2].as_ref().unwrap().as_deref(), None);
        assert_eq!(batched[3].as_ref().unwrap().as_deref(), Some(&b"3"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn optimize_for_point_lookup_sizes_cache() {
        let mut o = Options::new();
        o.optimize_for_point_lookup(8);
        assert_eq!(o.block_cache_bytes, Some(8 * 1024 * 1024));
        o.set_block_cache(&Cache::new_lru_cache(4096));
        assert_eq!(o.block_cache_bytes, Some(4096));
        let mut bb = BlockBasedOptions::default();
        bb.set_block_cache(&Cache::new_lru_cache(12345));
        let mut o2 = Options::new();
        o2.set_block_based_table_factory(&bb);
        assert_eq!(o2.block_cache_bytes, Some(12345));
    }

    #[test]
    fn block_cache_usage_is_bytes_not_hits() {
        let dir = tmp("bc-usage");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        opts.set_block_cache(&Cache::new_lru_cache(1024 * 1024));
        {
            let db = DB::open(&opts, &dir).unwrap();
            db.put(b"k", vec![b'v'; 64]).unwrap();
            db.flush().unwrap();
        }
        let db = DB::open(&opts, &dir).unwrap();
        // RFC-0160 P2.3: a byte-budgeted cache is filled by point gets
        // (decoded 4 KiB blocks). Scan still loads blocks too.
        let mut n = 0usize;
        for item in db
            .iterator_opt(IteratorMode::Start, ReadOptions::default())
            .unwrap()
        {
            n += item.unwrap().1.len();
        }
        assert!(n > 0, "scan must see the flushed row");
        let usage = db
            .property_int_value(properties::BLOCK_CACHE_USAGE)
            .unwrap()
            .unwrap_or(0);
        assert!(usage > 16, "occupancy must be payload bytes, got {usage}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
