//! `rocksdb-compat` — rust-rocksdb-shaped API subset implemented on
//! **pedradb-core** [`ConcurrentDb`](pedradb_core::ConcurrentDb).
//!
//! Goal: let small rust-rocksdb-dependent programs swap
//! `rocksdb = { package = "rocksdb-compat", path = ... }` and run, then feed the
//! same workload through Pedra's adversarial suite (FailingEnv crash campaigns).
//!
//! **Not drop-in TiKV** (see `docs/rocksdb-compat.md`): no ingest, compaction
//! filters, properties, statistics, Titan, or `delete_files_in_range`. Column
//! families are emulated by key prefix (`cf_name \x00 key`; `default` is raw).
//! Cross-CF key collisions from embedded `\x00` in keys are a documented
//! constraint of the emulation, not of Pedra itself.
//!
//! Coverage: `open_default` / `open_cf` / `open_cf_descriptors`, `put`/`get`/`delete` (± CF),
//! `delete_range_cf`, atomic `write(WriteBatch)`, point + iterator reads on
//! `snapshot()`, `OptimisticTransactionDB` / `Transaction` (OCC via Pedra
//! `OccTransaction`; rust-rocksdb shape for SurrealDB `kv-rocksdb`),
//! `raw_iterator_opt` / `ReadOptions` / `property_int_value` / `flush_opt`,
//! `flush`, `compact`. Options tunables SurrealDB sets at open are accepted
//! no-ops. UDT timestamps remain a documented gap.

#![forbid(unsafe_code)]

mod txn;
pub use txn::{OptimisticTransactionDB, OptimisticTransactionOptions, Transaction, WriteOptions};

use bytes::Bytes;
use parking_lot::Mutex;
mod shape;
pub use shape::{
    properties, BottommostLevelCompaction, ColumnFamilyDescriptor, CompactOptions,
    DBCompactionStyle, DBCompressionType, DBRawIteratorWithThreadMode, FlushOptions, LogLevel,
    ReadOptions, SliceTransform, SnapshotWithThreadMode, UniversalCompactOptions,
    UniversalCompactionStopStyle, WaitForCompactOptions,
};

use pedradb_core::{
    BatchOp, CompactOptions as CoreCompactOptions, ConcurrentDb, CoreError, Env,
    Snapshot as CoreSnapshot, SnapshotPin, StdEnv,
};
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
    /// Anything else.
    Other,
}

/// Compatibility error surface (rust-rocksdb exposes one opaque `Error`).
#[derive(Debug, Clone)]
pub struct Error {
    msg: String,
    kind: ErrorKind,
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

/// Open options (builder subset). `create_if_missing` mirrors rust-rocksdb;
/// Pedra always requires the directory to be creatable.
pub struct Options {
    /// Whether to create the database directory when absent.
    pub create_if_missing: bool,
    /// Memtable flush threshold (Pedra `auto_flush_bytes`, default 4 MiB).
    /// `0` disables auto-flush (manual [`DB::flush`] only).
    ///
    /// Isolated apply (2000× pre+com): 4 MiB + drain **2251** qps vs 64 MiB
    /// drain **1228** (one 64 MiB SST write at the end). 64 MiB matched Rocks
    /// `write_buffer_size` and lost apply (RFC-0041).
    pub write_buffer_size: usize,
    /// Pedra WAL `fdatasync` before Ok (G1). Default `true` (product).
    /// `false` is Rocks-shaped async WAL — bench-only same-class column.
    pub sync: bool,
    /// Version GC on auto-compact (Pedra `auto_reclaim`): drops versions
    /// older than the oldest open snapshot pin, like RocksDB compaction
    /// dropping unpinned obsolete versions. Default **`true`** (RFC-0047
    /// P0.3): the drop-in ships the Rocks storage profile — disk ≈ live
    /// set + pins. `false` is the Pedra kernel default (RFC-0009 F20: keep
    /// all versions, PITR grátis) as an explicit opt-out for hosts that
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
}

/// WAL recovery mode at open (rust-rocksdb `WalRecoveryMode` subset).
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
}

impl fmt::Debug for Options {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Options")
            .field("create_if_missing", &self.create_if_missing)
            .field("write_buffer_size", &self.write_buffer_size)
            .field("sync", &self.sync)
            .field("auto_reclaim", &self.auto_reclaim)
            .field("auto_resume_transient", &self.auto_resume_transient)
            .field("wal_recovery", &self.wal_recovery)
            .field(
                "background_error_listener",
                &self.background_error_listener.is_some(),
            )
            .finish()
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            create_if_missing: false,
            write_buffer_size: 4 * 1024 * 1024,
            sync: true,
            auto_reclaim: true,
            auto_resume_transient: true,
            background_error_listener: None,
            wal_recovery: WalRecoveryMode::PointInTime,
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

    /// WAL `fdatasync` before Ok. Default `true` (G1). `false` = Rocks async.
    pub fn set_sync(&mut self, v: bool) -> &mut Self {
        self.sync = v;
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
    pub fn set_target_file_size_base(&mut self, _n: u64) {}
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
    pub fn set_enable_blob_files(&mut self, _v: bool) {}
    pub fn set_min_blob_size(&mut self, _n: u64) {}
    pub fn set_blob_file_size(&mut self, _n: u64) {}
    pub fn set_enable_blob_gc(&mut self, _v: bool) {}
    pub fn set_blob_gc_age_cutoff(&mut self, _n: f64) {}
    pub fn set_blob_compression_type(&mut self, _c: DBCompressionType) {}
    pub fn set_universal_compaction_options(&mut self, _o: &UniversalCompactOptions) {}
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

/// Column family handle (name-keyed emulation).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ColumnFamily {
    name: String,
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
    if !raw.starts_with(CFREG_MAGIC) {
        return bad("bad magic");
    }
    let mut lines = raw[CFREG_MAGIC.len()..].split(|&b| b == b'\n');
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

/// Persist the registry atomically (tmp + rename + dir fsync). Written
/// BEFORE the core DB opens so a crash mid-open never leaves a CF'd DB
/// without its registry.
///
/// # Errors
/// I/O.
fn store_cf_registry(dir: &std::path::Path, default_raw: bool, non_default: &[String]) -> Result<()> {
    use std::io::Write as _;
    let path = cfreg_path(dir);
    let tmp = dir.join(format!("{CFREG_FILE_NAME}.tmp"));
    let mut buf = CFREG_MAGIC.to_vec();
    buf.extend_from_slice(if default_raw { b"R\n" } else { b"P\n" });
    for n in non_default {
        buf.extend_from_slice(n.as_bytes());
        buf.push(b'\n');
    }
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
struct KeyCodec {
    default_raw: bool,
}

impl KeyCodec {
    fn encode(&self, cf: &str, key: &[u8]) -> Vec<u8> {
        self.encode_with(cf, key, <[u8]>::to_vec)
    }

    /// Append `cf\\0key` onto `pool` and freeze a shared `Bytes` (one backing
    /// alloc per `write()` instead of one malloc per op).
    fn encode_pooled(&self, cf: &str, key: &[u8], pool: &mut bytes::BytesMut) -> Bytes {
        let effective = if cf == DEFAULT_CF && self.default_raw {
            ""
        } else {
            cf
        };
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

    /// Default-CF raw: copy user key; otherwise `cf\\0key` via the pool.
    fn encode_owned(&self, cf: &str, key: &[u8], pool: &mut bytes::BytesMut) -> Bytes {
        if cf == DEFAULT_CF && self.default_raw {
            Bytes::copy_from_slice(key)
        } else {
            self.encode_pooled(cf, key, pool)
        }
    }

    /// Encode into a stack buffer when the key fits (RFC-0035 P1.2).
    pub(crate) fn encode_with<R>(&self, cf: &str, key: &[u8], f: impl FnOnce(&[u8]) -> R) -> R {
        const STACK: usize = 192;
        let effective = if cf == DEFAULT_CF && self.default_raw {
            ""
        } else {
            cf
        };
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
        let effective = if cf == DEFAULT_CF && self.default_raw {
            ""
        } else {
            cf
        };
        if effective.is_empty() {
            return encoded;
        }
        encoded.get(effective.len() + 1..).unwrap_or(&[])
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
/// mutex (~the 39 ns C still needs for 2.0). 2-probe; epoch drops every
/// slot on publish.
const LAST_N: usize = 2048;
const LAST_PROBE: usize = 8;
const TINY: usize = 64;

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
    /// Epoch the entry was stored under; 0 = never used. A published write
    /// bumps the shared epoch, so any slot whose epoch differs from the
    /// reader's is stale — lazy invalidation instead of a clear-all walk.
    epoch: u64,
    cf: TinyBuf,
    key: TinyBuf,
    val: Option<Bytes>,
}

struct LastGetTable {
    slots: Box<[LastGetSlot]>,
}

impl LastGetTable {
    fn new() -> Self {
        Self {
            // Heap — TinyBuf slots overflow the thread stack if inline.
            slots: (0..LAST_N)
                .map(|_| LastGetSlot {
                    epoch: 0,
                    cf: TinyBuf::empty(),
                    key: TinyBuf::empty(),
                    val: None,
                })
                .collect(),
        }
    }

    fn hash(cf: &str, key: &[u8]) -> u64 {
        fx_bytes(fx_bytes(0, cf.as_bytes()), key)
    }

    fn get(&self, epoch: u64, cf: &str, key: &[u8]) -> Option<Option<Bytes>> {
        let h = Self::hash(cf, key);
        let cf_b = cf.as_bytes();
        for p in 0..LAST_PROBE {
            let s = &self.slots[last_slot(h, p)];
            // Stale (or never-used) slots do not terminate the probe: the
            // live entry for this key may sit deeper.
            if s.epoch != epoch {
                continue;
            }
            if s.cf.eq(cf_b) && s.key.eq(key) {
                return Some(s.val.clone());
            }
        }
        None
    }

    fn store(&mut self, epoch: u64, cf: &str, key: &[u8], val: Option<Bytes>) {
        let Some(cf_t) = TinyBuf::from_slice(cf.as_bytes()) else {
            return;
        };
        let Some(key_t) = TinyBuf::from_slice(key) else {
            return;
        };
        let h = Self::hash(cf, key);
        let mut free = None;
        for p in 0..LAST_PROBE {
            let i = last_slot(h, p);
            let s = &mut self.slots[i];
            if s.epoch == epoch && s.cf.eq(cf.as_bytes()) && s.key.eq(key) {
                s.val = val;
                return;
            }
            // Prefer a stale/empty slot over evicting a live entry.
            if s.epoch != epoch && free.is_none() {
                free = Some(i);
            }
        }
        let i = free.unwrap_or_else(|| last_slot(h, LAST_PROBE - 1));
        self.slots[i] = LastGetSlot {
            epoch,
            cf: cf_t,
            key: key_t,
            val,
        };
    }

    /// Default-CF `get()`: hash the user key only (no `default` prefix).
    fn get_key(&self, epoch: u64, key: &[u8]) -> Option<Option<Bytes>> {
        let h = fx_bytes(0, key);
        for p in 0..LAST_PROBE {
            let s = &self.slots[last_slot(h, p)];
            if s.epoch != epoch {
                continue;
            }
            if s.key.eq(key) {
                return Some(s.val.clone());
            }
        }
        None
    }

    fn store_key(&mut self, epoch: u64, key: &[u8], val: Option<Bytes>) {
        let Some(key_t) = TinyBuf::from_slice(key) else {
            return;
        };
        let h = fx_bytes(0, key);
        let mut free = None;
        for p in 0..LAST_PROBE {
            let i = last_slot(h, p);
            let s = &mut self.slots[i];
            if s.epoch == epoch && s.key.eq(key) {
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
            cf: TinyBuf::empty(),
            key: key_t,
            val,
        };
    }
}


thread_local! {
    /// Default-CF last-get table shared by `get()` and `contains()` —
    /// the same query, so a warm from either call site serves both.
    static LAST_GET: RefCell<LastGetTable> = RefCell::new(LastGetTable::new());
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
            Some(cf.name.clone()),
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
            Some(cf.name.clone()),
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
            Some(cf.name.clone()),
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
const ITER_WINDOW: usize = 64;

/// Windowed CF iterator (RFC-0032 P0.1). Same positioning semantics as v0.
pub struct DBIterator<E: Env = StdEnv> {
    items: Vec<(Vec<u8>, Vec<u8>)>,
    idx: usize,
    reverse: bool,
    inner: ConcurrentDb<E>,
    codec: KeyCodec,
    cf: String,
    seq: pedradb_core::SequenceNumber,
    cf_start: Bound<Vec<u8>>,
    cf_end: Bound<Vec<u8>>,
    exhausted: bool,
    /// Last refill error (fix C6 hardening): a failed window refill no
    /// longer vanishes — `status()` reports it instead of a silent truncation.
    err: Option<Error>,
}

impl<E: Env> DBIterator<E> {
    /// Whether positioned on a valid entry.
    #[must_use]
    pub fn valid(&self) -> bool {
        self.idx < self.items.len()
    }

    /// Advance (forward or backward per mode). Stepping past either end
    /// invalidates (reverse uses wrapping so index 0 → invalid, not clamped).
    pub fn next(&mut self) {
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
        self.items
            .get(self.idx)
            .map(|(k, _)| k.as_slice())
            .unwrap_or(&[])
    }

    /// Current value (empty when invalid).
    #[must_use]
    pub fn value(&self) -> &[u8] {
        self.items
            .get(self.idx)
            .map(|(_, v)| v.as_slice())
            .unwrap_or(&[])
    }

    /// Remaining entries from here to the CF bound (refills pages).
    pub fn collect_rest(&mut self) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut out = Vec::new();
        while self.valid() {
            out.push((self.key().to_vec(), self.value().to_vec()));
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
        let last = self.items[self.items.len() - 1].0.clone();
        let start = Bound::Excluded(self.codec.encode(&self.cf, &last));
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
                self.items = page;
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
        let first = self.items[0].0.clone();
        let end = Bound::Excluded(self.codec.encode(&self.cf, &first));
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
                self.items = page;
            }
            Ok(_) => self.invalidate(),
            Err(e) => {
                self.err = Some(e);
                self.invalidate();
            }
        }
    }
}

fn page_forward<E: Env>(
    inner: &ConcurrentDb<E>,
    codec: &KeyCodec,
    cf: &str,
    seq: pedradb_core::SequenceNumber,
    start: Bound<Vec<u8>>,
    end: Bound<&[u8]>,
    limit: usize,
) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let s = bound_as_ref(&start);
    inner
        .with_read(|db| db.range_at_limited(seq, s, end, Some(limit)))
        .map_err(Error::from)
        .map(|rows| {
            rows.into_iter()
                .map(|(k, v)| (codec.decode(cf, &k).to_vec(), v.to_vec()))
                .collect()
        })
}

fn page_last_n<E: Env>(
    inner: &ConcurrentDb<E>,
    codec: &KeyCodec,
    cf: &str,
    seq: pedradb_core::SequenceNumber,
    start: Bound<&[u8]>,
    end: Bound<Vec<u8>>,
    n: usize,
) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let e = bound_as_ref(&end);
    inner
        .with_read(|db| {
            // Iterator borrows the Db — consume the ring window under the guard.
            let mut ring: VecDeque<(Vec<u8>, Vec<u8>)> =
                VecDeque::with_capacity(n.saturating_add(1));
            for pair in db.try_scan_at(seq, start, e, None)? {
                if ring.len() == n {
                    ring.pop_front();
                }
                ring.push_back((codec.decode(cf, &pair.key).to_vec(), pair.value.to_vec()));
            }
            Ok::<Vec<(Vec<u8>, Vec<u8>)>, CoreError>(ring.into_iter().collect())
        })
        .map_err(Error::from)
}

/// Read snapshot (sequence-pinned point + iterator reads).
pub struct Snapshot<'a, E: Env = StdEnv> {
    db: &'a DB<E>,
    snap: CoreSnapshot,
    /// GC pin (fix C5/C6): a live rust-rocksdb-shaped snapshot must stay
    /// readable; `auto_reclaim` GC honours the pin until Drop.
    pin: SnapshotPin,
}

impl<E: Env> Snapshot<'_, E> {
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
        scan_cf_at(
            &self.db.inner,
            &self.db.codec,
            &cf.name,
            mode,
            self.snap.sequence(),
            &self.db.cfs,
        )
    }
}

impl<E: Env> Drop for Snapshot<'_, E> {
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

pub(crate) fn scan_cf_at<E: Env>(
    inner: &ConcurrentDb<E>,
    codec: &KeyCodec,
    cf: &str,
    mode: IteratorMode,
    seq: pedradb_core::SequenceNumber,
    known: &[String],
) -> Result<DBIterator<E>> {
    if cf != DEFAULT_CF && !known.iter().any(|c| c == cf) {
        return Err(Error {
            msg: format!("column family not found: {cf}"),
            kind: ErrorKind::InvalidArgument,
        });
    }
    let (cf_start, cf_end) = cf_bounds(codec, cf);
    let (items, idx, reverse) = match mode {
        IteratorMode::Start | IteratorMode::From(_, Direction::Forward) => {
            let user_lo = match mode {
                IteratorMode::From(k, _) => Bound::Included(codec.encode(cf, k)),
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
                Some(s) => Bound::Excluded(s),
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
        exhausted,
        err: None,
    })
}

/// rust-rocksdb-shaped database on top of a Pedra `ConcurrentDb`.
///
/// Writes join the Rocks-style write group (one leader takes the write lock
/// per group: appends + a single fdatasync + apply); reads take RwLock read
/// guards; the host compact worker reuses the core staged flush pipeline.
pub struct DB<E: Env = StdEnv> {
    pub(crate) inner: ConcurrentDb<E>,
    pub(crate) cfs: Vec<String>,
    pub(crate) codec: KeyCodec,
    /// Host compact worker (RFC-0037 P2.1). None when the caller injected Env
    /// (adversarial FailingEnv stays single-threaded / deterministic).
    compact_tx: Option<SyncSender<CompactCmd>>,
    compact_thread: Option<JoinHandle<()>>,
    compact_gate: Arc<Mutex<()>>,
    /// Last [`DB::resume`] outcome after a durability fence (RFC-0047 P1.1).
    /// `Arc`-shared with the host compact worker (P1.2 auto-resume writes
    /// here too).
    fence_recovery: Arc<Mutex<Option<pedradb_core::FenceRecovery>>>,
    /// RFC-0047 P1.2: worker auto-resumes Transient-class fences.
    auto_resume_transient: bool,
    /// TLS-cache epoch base unique per instance (fix C1/C1b).
    cache_epoch_base: u64,
}

impl DB<StdEnv> {
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
        let mut db = Self::open_cf_with_env(opts, path, cfs, StdEnv)?;
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
        let names: Vec<String> = cfs.into_iter().map(|d| d.name).collect();
        let refs: Vec<&str> = names
            .iter()
            .map(String::as_str)
            .filter(|n| *n != DEFAULT_CF)
            .collect();
        Self::open_cf(opts, path, &refs)
    }
}

impl<E: Env> DB<E> {
    /// Open with an explicit [`Env`] (adversarial `FailingEnv` campaigns).
    ///
    /// # Errors
    /// Pedra open errors; duplicate CF names.
    pub fn open_cf_with_env(
        opts: &Options,
        path: impl AsRef<std::path::Path>,
        cfs: &[&str],
        env: E,
    ) -> Result<Self> {
        Self::open_cf_inner(opts, path, cfs, env)
    }

    fn open_cf_inner(
        opts: &Options,
        path: impl AsRef<std::path::Path>,
        cfs: &[&str],
        env: E,
    ) -> Result<Self> {
        let dir = path.as_ref();
        if !dir.exists() {
            if !opts.create_if_missing {
                return Err(Error {
                    msg: format!("db path missing: {}", dir.display()),
                    kind: ErrorKind::InvalidArgument,
                });
            }
            std::fs::create_dir_all(dir)
                .map_err(|e| Error {
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
        let mut core_opts = pedradb_core::OpenOptions::default();
        core_opts.sync = opts.sync;
        core_opts.wal_recovery = match opts.wal_recovery {
            WalRecoveryMode::PointInTime => pedradb_core::WalRecovery::PointInTime,
            WalRecoveryMode::FailClosed => pedradb_core::WalRecovery::FailClosed,
        };
        core_opts.auto_flush_bytes = if opts.write_buffer_size == 0 {
            None
        } else {
            Some(opts.write_buffer_size)
        };
        let db = ConcurrentDb::open_with_env(dir, core_opts, env)?;
        if opts.auto_reclaim {
            db.set_auto_reclaim(true);
        }
        // Rocks parity: rust-rocksdb drops superseded versions below the
        // oldest live snapshot (`Snapshot` pins / OCC begins). This bounds
        // parked-fold memory under overwrite-heavy loads (one core of pure
        // memmove + ~100x footprint growth otherwise). Core-only users keep
        // the F20 keep-everything default.
        db.set_fold_version_gc(true);
        // F185: frozen flag from the registry — not derived from `names`
        // (a reopen with a different supplied list must not flip the codec).
        let codec = KeyCodec { default_raw };
        let cache_epoch_base = CACHE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) << 32;
        Ok(Self {
            inner: db,
            cfs: names,
            codec,
            compact_tx: None,
            compact_thread: None,
            compact_gate: Arc::new(Mutex::new(())),
            fence_recovery: Arc::new(Mutex::new(None)),
            auto_resume_transient: opts.auto_resume_transient,
            cache_epoch_base,
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
            .iter()
            .find(|c| c.as_str() == name)
            .map(|n| ColumnFamily { name: n.clone() })
    }

    fn check_cf(&self, cf: &str) -> Result<()> {
        if cf == DEFAULT_CF || self.cfs.iter().any(|c| c == cf) {
            Ok(())
        } else {
            Err(Error {
                msg: format!("column family not found: {cf}"),
                kind: ErrorKind::InvalidArgument,
            })
        }
    }

    /// Put into the default CF.
    ///
    /// # Errors
    /// WAL I/O or unknown CF.
    pub fn put(&self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        let key = key.as_ref();
        let value = value.as_ref();
        self.codec
            .encode_with(DEFAULT_CF, key, |enc| self.inner.put(enc, value))
            .map_err(Error::from)
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
        self.codec
            .encode_with(&cf.name, key, |enc| self.inner.put(enc, value))
            .map_err(Error::from)
    }

    /// Get from the default CF.
    ///
    /// # Errors
    /// Pedra read errors.
    pub fn get(&self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        // RFC-0041 YCSB-C: default-CF get hashes the user key only (no
        // `default` prefix / CF compare). Same bytes as `get_named`.
        let key = key.as_ref();
        let epoch = self.cache_epoch_base + self.inner.read_cache_epoch();
        if let Some(hit) = LAST_GET.with(|slot| slot.borrow().get_key(epoch, key)) {
            return Ok(hit.map(|b| b.to_vec()));
        }
        let got = self
            .codec
            .encode_with(DEFAULT_CF, key, |enc| self.inner.get(enc));
        LAST_GET.with(|slot| slot.borrow_mut().store_key(epoch, key, got.clone()));
        Ok(got.map(|b| b.to_vec()))
    }

    /// Point lookup without copying the value to `Vec` (RFC-0044 P1.3 GET).
    ///
    /// # Errors
    /// Pedra read errors.
    pub fn contains(&self, key: impl AsRef<[u8]>) -> Result<bool> {
        let key = key.as_ref();
        let epoch = self.cache_epoch_base + self.inner.read_cache_epoch();
        if let Some(hit) = LAST_GET.with(|slot| slot.borrow().get_key(epoch, key)) {
            return Ok(hit.is_some());
        }
        let got = self
            .codec
            .encode_with(DEFAULT_CF, key, |enc| self.inner.get(enc));
        LAST_GET.with(|slot| slot.borrow_mut().store_key(epoch, key, got.clone()));
        Ok(got.is_some())
    }

    /// Get from a named CF.
    ///
    /// # Errors
    /// Unknown CF or Pedra read errors.
    pub fn get_cf(&self, cf: &ColumnFamily, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        self.get_named(&cf.name, key)
    }

    /// Point get by CF name (no handle alloc). Same bytes as [`Self::get_cf`].
    ///
    /// # Errors
    /// Unknown CF or Pedra read errors.
    pub fn get_named(&self, cf: &str, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        let key = key.as_ref();
        // RFC-0041 YCSB-C: zipf (θ=0.99, 4096 keys) concentrates on a hot
        // set. Direct-mapped last-N skips CF-prefix encode + point-cache
        // mutex. Bytes stay shared with the point cache; we copy into Vec
        // only for the rust-rocksdb return type. Epoch bumps on publish.
        thread_local! {
            static LAST: RefCell<LastGetTable> = RefCell::new(LastGetTable::new());
        }
        let epoch = self.cache_epoch_base + self.inner.read_cache_epoch();
        if let Some(hit) = LAST.with(|slot| slot.borrow().get(epoch, cf, key)) {
            return Ok(hit.map(|b| b.to_vec()));
        }
        if cf != DEFAULT_CF {
            self.check_cf(cf)?;
        }
        let got = self.codec.encode_with(cf, key, |enc| self.inner.get(enc));
        LAST.with(|slot| slot.borrow_mut().store(epoch, cf, key, got.clone()));
        Ok(got.map(|b| b.to_vec()))
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
            self.inner.apply_batch_vec(ops).map(|_| ()).map_err(Error::from)
        });
        r
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
            self.inner.apply_batch_vec(ops).map(|_| ()).map_err(Error::from)
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
            self.inner.apply_batch_vec(ops).map(|_| ()).map_err(Error::from)
        });
        r
    }

    /// Like [`Self::write_cf_slices`] but values (and user keys) move into
    /// `Bytes` — no extra 1 KiB payload copy per apply/raftlog op (RFC-0041).
    ///
    /// # Errors
    /// Unknown CF or WAL I/O.
    pub fn write_cf_owned(
        &self,
        puts: Vec<(&str, Vec<u8>, Vec<u8>)>,
        deletes: Vec<(&str, Vec<u8>)>,
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
                if last_ok != Some(cf) {
                    self.check_cf(cf)?;
                    last_ok = Some(cf);
                }
                ops.push(BatchOp::Put {
                    key: self.codec.encode_pooled(cf, k.as_ref(), &mut pool),
                    value: Bytes::from(v),
                });
            }
            for (cf, k) in deletes {
                if last_ok != Some(cf) {
                    self.check_cf(cf)?;
                    last_ok = Some(cf);
                }
                ops.push(BatchOp::Delete {
                    key: self.codec.encode_pooled(cf, k.as_ref(), &mut pool),
                });
            }
            if ops.is_empty() {
                return Ok(());
            }
            self.inner.apply_batch_vec(ops).map(|_| ()).map_err(Error::from)
        });
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
            self.inner.apply_batch_vec(ops).map(|_| ()).map_err(Error::from)
        })
    }

    /// Sequence-pinned snapshot (fix C5/C6: registers a GC pin, released on
    /// Drop — a live snapshot stays readable under `auto_reclaim`, matching
    /// rust-rocksdb where a snapshot is valid until dropped).
    #[must_use]
    pub fn snapshot(&self) -> Snapshot<'_, E> {
        let snap = self.inner.snapshot();
        let pin = self.inner.pin_snapshot();
        Snapshot { db: self, snap, pin }
    }

    /// rust-rocksdb `OptimisticTransactionDB::transaction` shape (RFC-0043 P2.4).
    /// Pedra [`pedradb_core::OccTransaction`]: snapshot isolation + write-set
    /// conflict at commit. Always `fdatasync`s before Ok (G1); `WriteOptions.sync`
    /// is accepted and ignored (SurrealDB sets `sync=false` on the txn).
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
        scan_cf_at(&self.inner, &self.codec, &cf.name, mode, seq, &self.cfs)
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

    /// Toggle default WAL `fdatasync` (G1). Product default is `true`.
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

    /// Manual compaction (whole merge).
    ///
    /// # Errors
    /// Pedra compaction errors.
    pub fn compact(&self) -> Result<()> {
        let _gate = self.compact_gate.lock();
        self.inner.compact().map_err(Error::from)
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
    pub fn property_int_value(&self, _name: impl AsRef<str>) -> Result<Option<u64>> {
        Ok(None)
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

    /// rust-rocksdb `compact_range_opt` — whole merge (bounds ignored).
    pub fn compact_range_opt<S: AsRef<[u8]>, E2: AsRef<[u8]>>(
        &self,
        _start: Option<S>,
        _end: Option<E2>,
        _opts: &CompactOptions,
    ) {
        let _ = self.compact();
    }
}

impl<E: Env> Drop for DB<E> {
    fn drop(&mut self) {
        if let Some(tx) = self.compact_tx.take() {
            let _ = tx.send(CompactCmd::Shutdown);
        }
        if let Some(h) = self.compact_thread.take() {
            let _ = h.join();
        }
    }
}

/// Shared resume path (manual [`DB::resume`] and the P1.2 auto tick).
fn compat_resume<E: Env>(
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

fn spawn_compact_worker(
    inner: ConcurrentDb<StdEnv>,
    gate: Arc<Mutex<()>>,
    auto_resume_transient: bool,
    fence_sink: Arc<Mutex<Option<pedradb_core::FenceRecovery>>>,
    background_error_listener: Option<BackgroundErrorListener>,
) -> (Option<SyncSender<CompactCmd>>, Option<JoinHandle<()>>) {
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
                        while inner.park_imm_once() {}
                        while inner.fold_parked_once_off_lock() {}
                        while inner.materialize_parked_once() {}
                        while inner.drain_imm_once() {}
                        break;
                    }
                    Ok(CompactCmd::Run) | Err(RecvTimeoutError::Timeout) => {
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
                        if !inner.recently_multi(fold_multi_hold) {
                            let _ = inner.try_stage_if_full();
                        }
                        while inner.park_imm_once() {}
                        let may_fold =
                            inner.writes_active() <= 1 && !inner.recently_multi(fold_multi_hold);
                        if may_fold && inner.parked_unflushed_count() >= 2 {
                            let _ = inner.fold_parked_once_off_lock();
                        }
                        if inner.writes_idle_for(persist_idle) {
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

/// One L0→L1 job. I/O runs without the write lock (G5: failed write is not installed).
fn compat_compact_once<E: Env>(inner: &ConcurrentDb<E>, gate: &Mutex<()>) -> bool {
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
            }
        } else {
            CoreCompactOptions::default()
        };
        db.prepare_l0_compact(opts).ok().flatten()
    });
    let Some(job) = job else {
        return false;
    };
    let table = match job.write() {
        Ok(t) => t,
        Err(_) => return false,
    };
    if !inner.install_prepared_l0_off_lock(job, table) {
        return false;
    }
    inner.with_read(|db| db.level_file_count(0)) > 0
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // (batch, wall_ns, prepare, wal, mem, publish, flush_chk, lock_wait)
        let mut recs: Vec<(usize, u128, u64, u64, u64, u64, u64, u64)> =
            Vec::with_capacity(batches);
        for i in 0..batches {
            let mut puts = Vec::with_capacity(per_batch);
            for _ in 0..per_batch {
                idx += 1;
                puts.push(("raftlog", format!("raftlog/{idx:08}").into_bytes(), val.clone()));
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

    #[test]
    fn default_write_buffer_is_4_mib() {
        assert_eq!(Options::new().write_buffer_size, 4 * 1024 * 1024);
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
        let mut opts = Options::new();
        opts.create_if_missing(true);
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
        assert_eq!(db.get(b"k03").unwrap(), None, "suffix after the flip is discarded");
        let report = db.last_recovery_report().expect("compat default must report");
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
            let db =
                DB::open_cf_with_env(&opts, &dir, &[], pedradb_core::StdEnv).unwrap();
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
        let db = DB::open_cf(&Options::new(), &dir, &["write"]).unwrap();
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
        let db = DB::open_cf(&Options::new(), &dir, &["write"]).unwrap();
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
            "epoch bump must drop every last-N slot"
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
            "epoch bump must not serve a stale last-N value"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_cf_owned_moves_values_and_is_durable() {
        let dir = tmp("cfowned");
        let db = DB::open_cf(&Options::new(), &dir, &["raftlog"]).unwrap();
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
        let db = DB::open_cf(&Options::new(), &dir, &["raftlog"]).unwrap();
        let got = db
            .get_named("raftlog", b"raftlog/00000001")
            .unwrap()
            .expect("replay");
        assert_eq!(got.len(), 1024);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_cf_slices_is_durable() {
        let dir = tmp("cfslices");
        let db = DB::open_cf(&Options::new(), &dir, &["raftlog"]).unwrap();
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
        let db = DB::open_cf(&Options::new(), &dir, &["raftlog"]).unwrap();
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
        t.store_key(1, b"k1", Some(Bytes::from_static(b"v1")));
        assert_eq!(
            t.get_key(1, b"k1"),
            Some(Some(Bytes::from_static(b"v1")))
        );
        // Epoch bump: the stale entry must not answer for the new epoch.
        assert_eq!(t.get_key(2, b"k1"), None);
        // Re-store under the new epoch answers without any clear-all pass.
        t.store_key(2, b"k1", Some(Bytes::from_static(b"v2")));
        assert_eq!(
            t.get_key(2, b"k1"),
            Some(Some(Bytes::from_static(b"v2")))
        );
        // An entry stored under an old epoch coexists but never leaks.
        t.store_key(1, b"k2", Some(Bytes::from_static(b"old")));
        assert_eq!(t.get_key(2, b"k2"), None);
        assert_eq!(
            t.get_key(1, b"k2"),
            Some(Some(Bytes::from_static(b"old")))
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
            t.store_key(7, k, Some(Bytes::from_static(b"v")));
        }
        let hits = keys.iter().filter(|k| t.get_key(7, k).is_some()).count();
        assert!(hits >= 972, "uniform hot set hit rate {hits}/1024 < 95%");
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
        let mut folded = 0;
        for _ in 0..64 {
            if db.inner.fold_parked_once_off_lock() {
                folded += 1;
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
}
