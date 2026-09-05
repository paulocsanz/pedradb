//! Single-process database handle: WAL + MemTable + SSTs, recover on open.
//!
//! Auto-commit put/delete/get (P0.3), multi-key [`Transaction`](crate::tx::Transaction)
//! (P0.4), and MemTable → SST flush (P1.1).
//!
//! # Durability (P0.5 / RFC-0015)
//!
//! With default [`OpenOptions::sync`] = `true`:
//!
//! - Successful [`Db::put`], [`Db::delete`], and [`Transaction::commit`](crate::tx::Transaction::commit)
//!   return only after the WAL record is appended and **`fdatasync` completes**
//!   (RFC-0001 O1 / RFC-0036; same class as Rocks/TiKV `WriteOptions.sync`).
//! - After that `Ok`, a **process crash** (kill -9) must not lose the write if the OS
//!   and disk honor `fdatasync`. Reopen replays complete WAL records into the MemTable and
//!   loads SST files.
//! - A crash **during** append may leave a truncated trailing record; recovery **skips**
//!   it (no partial TX visible). Multi-key commit is one WAL record → all-or-nothing.
//! - Uncommitted transactions leave no WAL record (drop/abort = no durability side effect).
//! - [`OpenOptions::sync`] = `false` is for bulk load/benches only: process crash may
//!   still retain OS-buffered data; **power loss can lose recent acks** (JetStream/Jepsen lesson).
//! - **`Err` after a required WAL sync does not mean “record absent on disk”** (uncertain):
//!   append may have succeeded while `sync_data` failed. The open handle is then
//!   **durability-fenced** ([`CoreError::DurabilityFenced`]) — further writes refuse until
//!   `close` + `open` (recover rebuilds mem from WAL).
//! - When `sync=true`, **`Env::sync_dir` failures** on flush SST publish, MANIFEST/`CURRENT`
//!   install, and checkpoint are **propagated** (not discarded).
//!
//! # Flush (P1.1)
//!
//! [`Db::flush`] writes the MemTable to a new `.sst`, fsyncs it, clears the MemTable,
//! and truncates the WAL (data now lives on SST). `get` merges MemTable ∪ SSTs
//! (newest layer first).
//!
//! # Range, compaction & GC (RFC-0009 P1)
//!
//! [`Db::range`] scans user keys across MemTable ∪ SSTs at a snapshot.
//! [`Db::compact`] / [`Db::compact_with`] merge SSTs (tmp → rename); optional
//! version GC via [`CompactOptions`]. [`OpenOptions::auto_compact_sst_count`]
//! triggers compact after flush when SST count is high.
//!
//! # MANIFEST & exclusive open (RFC-0009 P2)
//!
//! Live SST inventory is written to `MANIFEST-*` + `CURRENT` after each flush/compact.
//! [`OpenOptions::exclusive`] (default true) takes a PID `LOCK` file so a second
//! process cannot open the same directory.
//!
//! # Ops surfaces (RFC-0014)
//!
//! - [`Db::range_limited`] — bounded scans (pagination).
//! - [`Db::create_checkpoint`] — point-in-time file-set copy (RocksDB Checkpoint class).
//! - [`Db::stats`] / [`Db::verify_checksums`] — observability and integrity.
//! - SST v3 embeds a Bloom filter; get prunes by bounds + filter.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;

use crate::batch::{WriteOp, WriteRecord};
use crate::cache::{AnswerCache, BlockCache, KeyGenMap, PointCache, TableCache};
use crate::change_feed::{ChangeEntry, ChangeKind, ChangeLog};
use crate::changelog_kernel::{changelog_needs_sst_rebuild, changelog_should_store};
use crate::env::{AdviseKind, Env, EnvFile, StdEnv};
use crate::error::{CoreError, Result};
use crate::host::Host;
use crate::key::{InternalKey, SequenceNumber, ValueType, MAX_SEQUENCE_NUMBER};
use crate::lock::DirLock;
use crate::manifest::{self, VersionSet};
use crate::memtable::{Lookup, MemTable};
use crate::merge::{range_deleted, range_tombstone_covers, StreamingVisibleIter, VisibleKv};
use crate::sst::{
    put_tls_point_seek_scratch, take_tls_point_seek_scratch, write_l0_sst, write_l0_sst_for_family,
    write_sst_bulk_arrays, write_sst_entries_on, SstTable,
};
use crate::tx::Transaction;
use crate::vlog::{self, ValueLog, VlogRewriteStats, VLOG_FILE_NAME};
use crate::wal::Wal;
use parking_lot::{Mutex, RwLock};
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::OnceLock;

/// Max LSM level we promote into (L0 = flush target, L1+ = compacted).
pub const MAX_LSM_LEVEL: u32 = 3;
/// When L0 file count reaches this, auto-compact merges L0 → L1 (subset compact).
pub const L0_COMPACTION_TRIGGER: usize = 4;

/// Hard cap on horizon `(seq, time)` samples (RFC-0046 P0.1). The time-based
/// keep rule bounds the *span* (2× the window) but not the *count* — one
/// sample per 32 publishes would grow unbounded on a long window under
/// sustained writes. At the cap the cutoff granularity is `window / cap`
/// (24 h / 4096 ≈ 21 s of writes) — noise against a 24 h horizon.
pub const HORIZON_SAMPLE_RING_CAP: usize = 4096;

/// Default WAL file name inside the DB directory.
pub const WAL_FILE_NAME: &str = "CURRENT.log";

/// F188: marker byte prepended to inline values that would otherwise sniff
/// as a vlog pointer (`VLG1…`/`VLG3…`) or that already start with the marker,
/// so [`Db::resolve_stored_value`] strips exactly one byte unconditionally.
const INLINE_ESCAPE: u8 = 0x01;

/// Block-cache id tag for value-resolved slots (see `Db::scan_at_raw`).
/// Resolving a stored value is not idempotent (`INLINE_ESCAPE` is stripped),
/// so resolved and raw forms of one block never share a slot.
const RESOLVED_BLOCK_TAG: u64 = 0x7265_736f_6c76_3d21;

/// F188: stored form of an inline (non-spilled) value. Escaped iff the raw
/// value could be misread as a vlog pointer, or already starts with the
/// marker (so reader/writer stay inverse for every input).
fn escape_inline_value(value: Bytes) -> Bytes {
    if value.is_empty() {
        return value;
    }
    if value[0] == INLINE_ESCAPE || vlog::decode_vlog_ptr(&value).is_some() {
        let mut escaped = Vec::with_capacity(value.len() + 1);
        escaped.push(INLINE_ESCAPE);
        escaped.extend_from_slice(&value);
        return Bytes::from(escaped);
    }
    value
}

/// WAL recovery policy at open (RFC-0047 P0.2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WalRecovery {
    /// Product default: a mid-WAL integrity failure (CRC, orphan) fails the
    /// open — never silent-wrong (G2). Journaled for CORRUPTLOG escalation.
    #[default]
    FailClosed,
    /// Rocks-shaped `kPointInTimeRecovery`: recover every complete record
    /// before the damage, report the discarded suffix via
    /// [`Db::last_recovery_report`], and keep serving. The event is still
    /// journaled; escalation (RFC-0038) refuses the open **in this mode too**.
    PointInTime,
}

/// What a [`WalRecovery::PointInTime`] open discarded (RFC-0047 P0.2).
/// Reported, never silently skipped. Record count is not included: the
/// damaged region cannot be parsed honestly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    /// Journal kind of the event (`"crc"`, `"truncated_head"`).
    pub kind: &'static str,
    /// Stream offset where the failing record began.
    pub corrupt_offset: u64,
    /// Last known-good append offset (the recovered prefix ends here).
    pub good_through_offset: u64,
    /// `file_len - good_through_offset`: bytes after the recoverable point.
    pub discarded_bytes: u64,
}

/// What a durability fence caught in flight (RFC-0047 P1.1). Sequences in
/// `uncertain_from..=uncertain_through` were assigned (and possibly WAL
/// buffered) but never confirmed durable — after resume they may or may
/// not be there (G5: the report never pretends to know).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FenceReport {
    /// The WAL write/sync I/O error that tripped the fence.
    pub io_error: String,
    /// Typed retryability class (RFC-0047 P1.2): hosts program auto-resume
    /// on this, never on parsing strings.
    pub class: FenceClass,
    /// First sequence not confirmed durable at fence time.
    pub uncertain_from: SequenceNumber,
    /// Last assigned sequence at fence time (inclusive). Empty in-flight
    /// range when `uncertain_from > uncertain_through`.
    pub uncertain_through: SequenceNumber,
}

/// Retryability class of a durability fence (RFC-0047 P1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FenceClass {
    /// Heals on its own (ENOSPC freed, EINTR): auto-resume is reasonable.
    Transient,
    /// Will not heal by retrying (dead disk, EACCES): manual resume only.
    Persistent,
    /// Non-I/O kernel error (vlog promote class): no classification — manual.
    Unknown,
}

impl FenceClass {
    /// Classify a WAL write/sync I/O error.
    #[must_use]
    pub fn of_io(kind: std::io::ErrorKind) -> Self {
        match kind {
            std::io::ErrorKind::StorageFull | std::io::ErrorKind::Interrupted => Self::Transient,
            _ => Self::Persistent,
        }
    }

    /// Classify a kernel error (vlog promote class fences): I/O errors by
    /// kind, everything else `Unknown` (manual resume only).
    #[must_use]
    pub fn of_core(err: &CoreError) -> Self {
        match err {
            CoreError::Io(e) => Self::of_io(e.kind()),
            _ => Self::Unknown,
        }
    }
}

/// Typed outcome of [`Db::recover_from_fence`] (close+replay+reopen,
/// RFC-0047 P1.1): the fence's uncertain range plus where the durable
/// replay actually landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FenceRecovery {
    /// The fence being recovered from (first fence wins).
    pub fence: FenceReport,
    /// Last sequence the replayed durable state reached.
    pub replayed_through: SequenceNumber,
    /// Some in-flight writes were not durable after reopen.
    pub lost_writes: bool,
}

/// Options for [`Db::open`].
#[derive(Debug, Clone, Copy)]
pub struct OpenOptions {
    /// When true (default), each successful `put`/`delete`/`commit` `fdatasync`s
    /// the WAL before returning (RFC-0001 O1 / RFC-0036). Overridable per write
    /// via [`WriteOptions`].
    pub sync: bool,
    /// When true, every WAL barrier on this DB uses the platform's **strongest
    /// data class** — on Darwin `fcntl(F_FULLFSYNC)` via
    /// [`EnvFile::sync_data_strong`](crate::env::EnvFile::sync_data_strong),
    /// which is the class a CMake build of RocksDB uses for
    /// `WriteOptions.sync`. Default **true** (RFC-0036 addendum v2): the
    /// `sync=true` contract means durable-Ok, and on Darwin only
    /// `F_FULLFSYNC` delivers it — SST/MANIFEST publishes already pay this
    /// class via `sync_all`, so the acked WAL (the durability boundary) must
    /// not be weaker than the derived artifacts. `false` restores the
    /// `fdatasync`/`fsync` weak class (the `librocksdb-sys` crate build
    /// class; ~120× faster per commit on Apple hardware — dev opt-out). On
    /// Linux the two classes are the same barrier — the flag is a no-op
    /// there.
    pub wal_full_fsync: bool,
    /// WAL recovery mode at open (RFC-0047). Default [`WalRecovery::FailClosed`];
    /// [`WalRecovery::PointInTime`] is the Rocks-shaped drop-in profile.
    pub wal_recovery: WalRecovery,
    /// When MemTable approximate size reaches this many bytes, flush to SST.
    /// `None` or `0` disables auto-flush (manual [`Db::flush`] only).
    pub auto_flush_bytes: Option<usize>,
    /// After a flush, if SST count is ≥ this, run [`Db::compact`].
    /// `None` or `0` disables auto-compact.
    pub auto_compact_sst_count: Option<usize>,
    /// After a flush, if total on-disk SST bytes is ≥ this, run [`Db::compact`].
    /// `None` or `0` disables size-based auto-compact (RFC-0014).
    pub auto_compact_sst_bytes: Option<u64>,
    /// When true (default), acquire exclusive `LOCK` in the DB directory.
    pub exclusive: bool,
    /// When `Some(n)`, values with length ≥ `n` are stored in a separate value
    /// log (`VALUES.vlog`); SST/mem keep a compact pointer (RFC-0014 P2.2 WiscKey-shaped).
    /// `None` (**default**) = always inline — production-safe. Enabling the threshold is
    /// opt-in; under update-heavy large-value workloads call [`Db::compact_vlog`]
    /// (RFC-0016 P0.1) or the log only grows.
    pub large_value_threshold: Option<usize>,
    /// MVCC history retention (RFC-0046). Default [`HistoryOptions::default`]:
    /// a 24 h horizon — superseded versions older than the horizon are
    /// archived to a capped local history tier and then GCed (pin-aware).
    /// `HistoryHorizon::All` (F20) is the explicit opt-out.
    pub history: HistoryOptions,
    /// Byte budget for resident SST file bodies (RFC-0042 v18 payload pool).
    /// `None` (default) = every payload stays resident (legacy behavior).
    /// `Some(n)` takes effect only on a bounded open
    /// ([`Db::open_with_env_bounded`]): the freshest `n` bytes of payloads
    /// stay in RAM, the rest are served from file block-by-block,
    /// CRC-verified. The bounded-open default budget is
    /// [`DEFAULT_SST_PAYLOAD_BUDGET_BYTES`] (256 MiB).
    pub sst_payload_budget_bytes: Option<u64>,
}

/// Default bounded-open SST payload budget (RFC-0042 v18): 256 MiB. RocksDB's
/// defaults never hold a whole LSM in RAM; this is the matching bar for the
/// drop-in surface (compat maps the caller's cache knob onto it).
pub const DEFAULT_SST_PAYLOAD_BUDGET_BYTES: u64 = 256 * 1024 * 1024;

/// Async bulk installs between MANIFEST persists (RFC-0159 P1.2). 1 = every
/// chunk (v51). 4 was v50 **under** the write lock (regressed 0.91×); off
/// lock it is 90→23 persists at 25M / 64 MiB.
const BULK_MANIFEST_EVERY: u8 = 4;

/// RFC-0160 P0.5: latched BulkRun always has a chunk cap so 100M hydrate
/// cannot accumulate the open tail unboundedly. Used when the caller left
/// `auto_flush_bytes` / per-CF buffers unset (`None` used to mean "never
/// park" and SIGKILL'd the 3.9 GiB guest). One parked + one encoding +
/// this tail is the RAM bound.
pub(crate) const DEFAULT_BULK_CHUNK_BYTES: usize = 64 * 1024 * 1024;

/// Default read-handle cache size for bounded opens
/// ([`crate::env::FileHandleCache`]): covers the post-settle file count of
/// the 25M slipstream shape (~85 SSTs) with fd headroom. RocksDB holds the
/// equivalent per-DB file cache; `open()`-per-block was 41% of the 6M
/// `get_hit` profile.
pub const DEFAULT_SST_FILE_CACHE_ENTRIES: usize = 256;

/// `PEDRA_SST_FILE_CACHE` — read-handle cache size override (bench A/B
/// knob; `0` disables handle reuse). Unset or unparsable →
/// [`DEFAULT_SST_FILE_CACHE_ENTRIES`].
fn sst_file_cache_entries_from_env() -> usize {
    std::env::var("PEDRA_SST_FILE_CACHE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SST_FILE_CACHE_ENTRIES)
}

/// `PEDRA_SST_PAYLOAD_BUDGET` — resident SST-body budget in bytes (bench
/// A/B). Unset or unparsable → [`DEFAULT_SST_PAYLOAD_BUDGET_BYTES`].
/// 10M hydrate is 2.4 GiB; the 256 MiB default leaves get_hit pread-tied
/// with Rocks (v56 10M 13.049 vs 13.045 µs). 1 GiB holds ~16 of ~38 files.
fn sst_payload_budget_from_env() -> u64 {
    std::env::var("PEDRA_SST_PAYLOAD_BUDGET")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SST_PAYLOAD_BUDGET_BYTES)
}

/// MVCC history horizon (RFC-0046 P0.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryHorizon {
    /// F20: keep every version forever (the pre-0046 product default).
    /// Explicit opt-in — disk grows O(total writes).
    All,
    /// Keep versions published within the window. The newest (live) version
    /// of a key is kept regardless of age; superseded versions older than
    /// the window are archived (P0.2) then GCed pin-aware. Reads/pins below
    /// the GC watermark fail closed with [`CoreError::SnapshotTooOld`].
    Window(Duration),
}

impl Default for HistoryHorizon {
    /// RFC-0046 P0.1 decision: **24 h**. Matches the RFC strawman; the
    /// horizon's job is bounding the SSD tier (live set + one day of
    /// history), while PITR beyond the window goes through the archive
    /// (restore-time, like the market). Cassandra's 10-day `gc_grace`
    /// exists for multi-node tombstone replay windows — not this engine's
    /// constraint.
    fn default() -> Self {
        Self::Window(Duration::from_secs(24 * 60 * 60))
    }
}

/// Retention/history options (RFC-0046 P0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryOptions {
    /// Horizon (default [`HistoryHorizon::default`] = 24 h window).
    pub horizon: HistoryHorizon,
    /// Cap of the local history tier in bytes (default 1 GiB). When the cap
    /// overflows, the oldest archive segments are dropped and the GC
    /// watermark advances (older snaps become [`CoreError::SnapshotTooOld`])
    /// — never a silent destroy; an open pin holds its segment.
    /// `0` = unbounded (no cap enforcement).
    pub cap_bytes: u64,
}

impl Default for HistoryOptions {
    fn default() -> Self {
        Self {
            horizon: HistoryHorizon::default(),
            cap_bytes: 1 << 30,
        }
    }
}

/// Lightweight observability snapshot (RocksDB `GetProperty`-class).
///
/// Not `Copy`: includes optional last auto-compact error text (RFC-0015 P2.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DbStats {
    /// Highest committed sequence.
    pub last_sequence: SequenceNumber,
    /// Approximate MemTable memory usage in bytes.
    pub mem_approx_bytes: usize,
    /// Number of internal versions in the MemTable.
    pub mem_entries: usize,
    /// Number of live SST files.
    pub sst_count: usize,
    /// Sum of internal versions across all SSTs (not distinct live keys).
    pub sst_entries: usize,
    /// Sum of SST file sizes on disk (bytes).
    pub sst_bytes: u64,
    /// Whether WAL file exists and its length (0 if missing).
    pub wal_bytes: u64,
    /// Highest LSM level that currently holds at least one SST (0 if only L0 / empty).
    pub max_level: u32,
    /// Table-cache hits (second+ open of the same SST path).
    pub table_cache_hits: u64,
    /// Table-cache misses (Env open performed).
    pub table_cache_misses: u64,
    /// Block-cache hits.
    pub block_cache_hits: u64,
    /// Block-cache misses.
    pub block_cache_misses: u64,
    /// Block-cache occupancy in bytes (Rocks `block-cache-usage`).
    pub block_cache_bytes: u64,
    /// Times auto-compact failed after a successful flush (flush still returned `Ok`).
    pub auto_compact_failures: u64,
    /// Most recent auto-compact error after flush (empty if never failed).
    pub last_auto_compact_error: String,
    /// WAL `sync_data` calls (group commit amortizes this under concurrent writers).
    pub wal_sync_count: u64,
    /// On-disk size of `VALUES.vlog` (0 if absent).
    pub vlog_bytes: u64,
    /// Estimated live payload bytes referenced by mem/imm/SST (sum of record lengths).
    pub vlog_live_bytes: u64,
    /// Distinct live vlog records referenced by mem/imm/SST.
    pub vlog_live_records: u64,
    /// User-value bytes accepted by put/apply (logical ingest).
    pub bytes_ingested: u64,
    /// Bytes written to the WAL (encoded record payloads, approximate).
    pub bytes_written_wal: u64,
    /// Bytes written to SST files (flush + compact + vlog GC rewrite).
    pub bytes_written_sst: u64,
    /// Successful SST-level compact operations.
    pub compact_count: u64,
    /// Successful value-log GC rewrites ([`Db::compact_vlog`]).
    pub vlog_gc_count: u64,
    /// Number of `NNNNNN.blob` generations on disk (RFC-0029).
    pub blob_files: u32,
    /// Scan windows that issued a vlog prefetch (RFC-0029 P0.3).
    pub scan_prefetch_hits: u64,
    /// Version-GC watermark (MANIFEST v4; snaps below this are too old).
    pub earliest_readable_seq: SequenceNumber,
    /// Open [`SnapshotPin`] count (process-local; not durable).
    pub snapshot_pin_count: usize,
    /// Whether auto-compact uses pin-aware reclaim (session setter).
    pub auto_reclaim: bool,
    /// Writes refused by L0 / mem write stall (open-items §2.3).
    pub write_stall_count: u64,
    /// Soft L0 pressure drains (not refusals).
    pub write_pressure_count: u64,
    /// Configured L0 stall limit (`0` = disabled).
    pub write_stall_l0: u64,
    /// Configured mem stall limit in bytes (`0` = disabled).
    pub write_stall_mem_bytes: u64,
    /// Configured L0 soft-pressure threshold (`0` = disabled).
    pub write_pressure_l0: u64,
    /// Current L0 SST file count (admission / stall observability).
    pub l0_files: u64,
    /// Durable-commit interval between CHANGELOG cache stores (RFC-0031). `0` = never
    /// on the commit path (flush / close / checkpoint still persist).
    pub changelog_interval: u64,
    /// Successful CHANGELOG cache stores since open (RFC-0031 observability).
    pub changelog_store_count: u64,
}

/// RFC-0035 P0: snapshot of latest/scan counters + LSM shape (no thread).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadProbeSnap {
    /// `last_under_user_prefix` calls.
    pub latest_ops: u64,
    /// Newest-mem live hit (SST not probed).
    pub latest_mem_hit: u64,
    /// Fell through to full [`Db::last_under_prefix`].
    pub latest_sst_fallback: u64,
    /// SST files considered on fallback (sum; divide by fallback for mean).
    pub latest_sst_probed: u64,
    /// `scan_at_raw` calls.
    pub scan_ops: u64,
    /// SST files offered to the merge (sum).
    pub scan_sst_probed: u64,
    /// Live SST files now.
    pub sst_count: usize,
    /// L0 file count now.
    pub l0_files: usize,
    /// L1 file count now.
    pub level1_files: usize,
    /// Active memtable internal entries.
    pub mem_entries: usize,
    /// Block-cache hits since last reset.
    pub block_cache_hits: u64,
    /// Block-cache misses since last reset.
    pub block_cache_misses: u64,
    /// SST blocks actually decompressed on this thread since last reset.
    pub blocks_decoded: u64,
    /// Probes served without a CRC re-run (verified-residency marks).
    pub blocks_crc_skipped: u64,
    /// `lookup` answered from a mem layer (no SST probe).
    pub get_mem_hit: u64,
    /// `lookup` had to probe SSTs.
    pub get_sst_fallback: u64,
    /// `get` resolved an inline value (not a vlog pointer).
    pub get_inline: u64,
    /// `get` resolved a vlog pointer.
    pub get_vlog: u64,
    /// `last_prefix_then_get` ops that recorded an intra-lock split (RFC-0035 P1.2).
    pub mvcc_split_ops: u64,
    /// Sum of encode nanos across those ops.
    pub mvcc_ns_encode: u64,
    /// Sum of `last_under_user_prefix` nanos.
    pub mvcc_ns_last: u64,
    /// Sum of point-get nanos.
    pub mvcc_ns_get: u64,
    /// Sum of 1 KB `to_vec` nanos.
    pub mvcc_ns_copy: u64,
}

/// Per-blob GC stats for operator / auto-pick (RFC-0029 P1.1).
#[derive(Debug, Clone)]
pub struct BlobGcCandidate {
    /// Blob generation number (`0` = `VALUES.vlog`).
    pub file_num: u32,
    /// On-disk file size in bytes.
    pub bytes: u64,
    /// Sum of live record lengths still referenced by mem/imm/SST.
    pub live_bytes: u64,
    /// Count of live vlog records in this file.
    pub live_records: u64,
    /// `1.0 - live_bytes/bytes` (0 if empty file).
    pub dead_ratio: f64,
    /// True when this is the active append generation (auto GC skips).
    pub is_active: bool,
}

impl DbStats {
    /// Live payload / on-disk vlog. `1.0` if there is no vlog file.
    ///
    /// RFC-0026 P0.1: one number an operator can alert on (`≪ 1` ⇒ garbage).
    #[must_use]
    pub fn vlog_live_ratio(&self) -> f64 {
        if self.vlog_bytes == 0 {
            return 1.0;
        }
        #[allow(clippy::cast_precision_loss)]
        {
            (self.vlog_live_bytes as f64 / self.vlog_bytes as f64).clamp(0.0, 1.0)
        }
    }

    /// One-line vlog observability (CLI / usage).
    #[must_use]
    pub fn vlog_line(&self) -> String {
        format!(
            "vlog file={}B live={}B records={} ratio={:.3} gc={} blobs={} prefetch={} sst_written={}B",
            self.vlog_bytes,
            self.vlog_live_bytes,
            self.vlog_live_records,
            self.vlog_live_ratio(),
            self.vlog_gc_count,
            self.blob_files,
            self.scan_prefetch_hits,
            self.bytes_written_sst
        )
    }

    /// One-line version-GC / pin observability.
    #[must_use]
    pub fn gc_line(&self) -> String {
        format!(
            "earliest_readable={} pins={} auto_reclaim={} compact={} auto_compact_fail={} l0={} write_stall={} pressure={} (l0_limit={} mem_limit={} pressure_l0={})",
            self.earliest_readable_seq,
            self.snapshot_pin_count,
            self.auto_reclaim,
            self.compact_count,
            self.auto_compact_failures,
            self.l0_files,
            self.write_stall_count,
            self.write_pressure_count,
            self.write_stall_l0,
            self.write_stall_mem_bytes,
            self.write_pressure_l0
        )
    }
}

/// Metadata written next to a checkpoint (ops / restore tooling).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointMeta {
    /// Sequence frozen at checkpoint time.
    pub last_sequence: SequenceNumber,
    /// Number of SST files copied.
    pub sst_count: usize,
    /// Version-GC watermark at checkpoint time (MANIFEST v4 / open-items §2.1).
    pub earliest_readable_seq: SequenceNumber,
}

/// File name for checkpoint metadata under the checkpoint directory.
pub const CHECKPOINT_META_FILE: &str = "CHECKPOINT";

/// RFC-0045 P0.1: per-phase write-path timings, opt-in via
/// `PEDRA_WRITE_PHASE_STATS=1` (bench/diagnostics only). Atomics keep the
/// hot path lock-free; when the env is unset the `Db` holds `None` and the
/// only cost is one branch per commit. Not a product feature.
#[derive(Debug, Default)]
pub struct WritePhaseStats {
    /// `commit_async_ops` invocations timed.
    pub commits: AtomicU64,
    /// `prepare_write_ops` (seq alloc, large-value spill, WriteOp build).
    pub prepare_ns: AtomicU64,
    /// WAL encode + append (`encode_write_op_batches` + pending write()).
    pub wal_ns: AtomicU64,
    /// Dirty-point note + `apply_ops_owned` (BTree insert).
    pub mem_ns: AtomicU64,
    /// `publish_sequence` (CAS + cache invalidation).
    pub publish_ns: AtomicU64,
    /// `maybe_auto_flush_best_effort`.
    pub flush_check_ns: AtomicU64,
    /// `ConcurrentDb` bypass: time blocked acquiring the Db write lock.
    pub lock_wait_ns: AtomicU64,
}

/// Per-write durability / batching knobs (RFC-0009 P0.1).
#[derive(Debug, Clone, Copy, Default)]
pub struct WriteOptions {
    /// If set, overrides [`OpenOptions::sync`] for this write.
    /// `false` = WAL write without fsync (group with later [`Db::sync`]).
    pub sync: Option<bool>,
}

impl WriteOptions {
    /// Use database default sync policy.
    #[must_use]
    pub fn default_sync() -> Self {
        Self::default()
    }

    /// Force fsync after this write.
    #[must_use]
    pub fn sync() -> Self {
        Self { sync: Some(true) }
    }

    /// Skip fsync (caller must [`Db::sync`] for durability).
    #[must_use]
    pub fn no_sync() -> Self {
        Self { sync: Some(false) }
    }
}

/// One operation in an ordered external apply batch (P2.3 — no OCC).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchOp {
    /// Put `key → value`.
    Put {
        /// User key bytes.
        key: Bytes,
        /// Value bytes.
        value: Bytes,
    },
    /// Delete `key`.
    Delete {
        /// User key bytes.
        key: Bytes,
    },
    /// Range-delete `[start, end)` (end exclusive).
    DeleteRange {
        /// Inclusive start user key.
        start: Bytes,
        /// Exclusive end user key.
        end: Bytes,
    },
}

impl BatchOp {
    /// Put helper.
    #[must_use]
    pub fn put(key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Self {
        Self::Put {
            key: Bytes::copy_from_slice(key.as_ref()),
            value: intern_bytes(value.as_ref()),
        }
    }

    /// Delete helper.
    #[must_use]
    pub fn delete(key: impl AsRef<[u8]>) -> Self {
        Self::Delete {
            key: Bytes::copy_from_slice(key.as_ref()),
        }
    }

    /// Range-delete helper (`[start, end)`).
    #[must_use]
    pub fn delete_range(start: impl AsRef<[u8]>, end: impl AsRef<[u8]>) -> Self {
        Self::DeleteRange {
            start: Bytes::copy_from_slice(start.as_ref()),
            end: Bytes::copy_from_slice(end.as_ref()),
        }
    }
}

/// Read snapshot: sequence number visible to get/range (P2.3).
///
/// Cheap copy of a sequence. **Does not** register with the DB — version GC
/// via [`Db::compact_reclaim`] will not preserve this unless you also hold a
/// [`SnapshotPin`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Snapshot {
    /// Highest committed sequence included in this snapshot.
    seq: SequenceNumber,
}

impl Snapshot {
    /// Pin reads at `seq` (inclusive).
    #[must_use]
    pub fn at(seq: SequenceNumber) -> Self {
        Self { seq }
    }

    /// Sequence this snapshot pins.
    #[must_use]
    pub fn sequence(self) -> SequenceNumber {
        self.seq
    }
}

/// Registered read pin that blocks snapshot-safe version GC below its sequence.
///
/// Create with [`Db::pin_snapshot`]; release with [`Db::release_snapshot_pin`]
/// (or drop without release only if you accept blocked reclaim until process end).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SnapshotPin {
    id: u64,
    seq: SequenceNumber,
}

impl SnapshotPin {
    /// Sequence this pin protects.
    #[must_use]
    pub fn sequence(self) -> SequenceNumber {
        self.seq
    }

    /// Read snapshot view of this pin.
    #[must_use]
    pub fn snapshot(self) -> Snapshot {
        Snapshot::at(self.seq)
    }
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            sync: true,
            wal_full_fsync: true,
            wal_recovery: WalRecovery::FailClosed,
            // 4 MiB default encourages SST creation under load without manual flush.
            auto_flush_bytes: Some(4 * 1024 * 1024),
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
            history: HistoryOptions::default(),
            sst_payload_budget_bytes: None,
        }
    }
}

/// `PEDRA_CHANGELOG_INTERVAL` (RFC-0031). Unset → `0` (never on the commit
/// path; flush/close still persist). The cache is rebuilt from WAL (RFC-0019).
fn changelog_interval_from_env() -> u64 {
    std::env::var("PEDRA_CHANGELOG_INTERVAL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// F1 fail-stop for read APIs whose shape cannot express an error
/// (`get`/`multi_get`/`scan` iterator/`changes_after`). Swallowing a value-log
/// resolve failure would serve corruption as a miss (or as an empty value) —
/// indistinguishable from deleted data. Error-shaped twins (`get_at`,
/// `scan_at`, `changes`) propagate instead.
///
/// # Panics
/// Always — corruption on a read path must be loud, never silent.
fn fail_stop_corrupt_value(context: &str, e: &CoreError) -> ! {
    panic!(
        "pedradb: corrupt value log while resolving {context}: {e}; \
         refusing to serve a silent miss — use get_at/scan_at/verify_checksums \
         for an error-shaped read"
    )
}

/// F1 fail-stop sibling of [`fail_stop_corrupt_value`] for the point path's
/// SST block faults. The pre-seek path swallowed `decode_block` errors with
/// `unwrap_or_default()`, serving a CRC-broken block as a silent miss —
/// indistinguishable from deleted data.
///
/// # Panics
/// Always — corruption on a read path must be loud, never silent.
fn fail_stop_corrupt_block(path: &Path, e: &CoreError) -> ! {
    panic!(
        "pedradb: corrupt SST block in {} on point seek: {e}; \
         refusing to serve a silent miss — use get_at/verify_checksums \
         for an error-shaped read",
        path.display()
    )
}

/// What a range scan yields (RFC-0019 P1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScanProjection {
    /// Full key + resolved value (default).
    #[default]
    Full,
    /// User keys only; `VisibleKv::value` is empty (no vlog resolve).
    KeyOnly,
}

/// Files snapshotted for an off-lock leveled rewrite (RFC-0037 P1.2; the job
/// may mix L0 inputs with the L1 slice they overlap, or one L`n` file with
/// its L`n+1` overlaps — a pushdown).
///
/// Inputs stay in the live inventory until [`Db::install_prepared_l0_compact`].
/// [`Self::write`] does not need the `Db` write lock.
pub struct PreparedL0Compact<E: Env> {
    inputs: Vec<SstTable>,
    /// Level the outputs land at (1 for L0→L1, `n+1` for a pushdown).
    to_level: u32,
    file_num: u64,
    gc: crate::merge::CompactGcOptions,
    dir: PathBuf,
    env: E,
    sync: bool,
    split_target: u64,
    /// Hard cap on emitted chunks; `file_num`'s reserved range is exactly
    /// this wide, so a split past the cap would hand out an unreserved
    /// number.
    chunk_budget: usize,
    /// Payload-pool kit for the emitted chunks: each chunk is opened with
    /// its whole body resident, so it must be registered (evictable) the
    /// moment it exists.
    kit: Option<crate::cache::PayloadKit>,
    /// Type-erased parallel merge executor (see [`Db::set_parallel_merge`]).
    parallel: Option<Arc<dyn ParallelMerge>>,
}

/// Type-erased key-space-parallel merge executor. `E` cannot be shared
/// across threads generically (`Env: Clone` only), and the compat host
/// drives compaction through unbounded generic code — so the host open
/// path (where `E: Send + Sync + 'static` holds) installs one concrete
/// implementation behind this seam, and prepared jobs inherit it.
pub(crate) trait ParallelMerge: Send + Sync {
    /// Merge `tables` into chunked SSTs (see [`write_merged_tables`]),
    /// in parallel across key-space spans when the inputs are large.
    ///
    /// # Errors
    /// SST encode / I/O.
    fn merge(
        &self,
        first_file_num: u64,
        tables: &[SstTable],
        dir: &Path,
        gc: crate::merge::CompactGcOptions,
        do_sync_dir: bool,
        split_target: u64,
        chunk_budget: usize,
        kit: Option<crate::cache::PayloadKit>,
    ) -> Result<Vec<SstTable>>;

    /// Write N pairwise key-disjoint jobs concurrently — one thread per
    /// job, each job merged sequentially (across-job parallelism; the
    /// within-job [`Self::merge`] spans are a separate, opt-in dimension).
    /// Returns outputs in job order, CF-attached exactly like
    /// [`PreparedL0Compact::write`]. Each job's reserved file-number range
    /// is its own (`build_prepared` burned disjoint ranges in prepare
    /// order), so concurrent writers cannot collide. Nothing is installed;
    /// the caller installs sequentially and may drop everything on error.
    ///
    /// # Errors
    /// SST encode / I/O of any job (first error wins; siblings' output
    /// files are orphaned tmp/rename artifacts the caller's failure path
    /// already tolerates — nothing references them).
    fn merge_jobs(&self, jobs: Vec<ParallelJobSpec>) -> Result<Vec<Vec<SstTable>>>;
}

/// E-free payload of one prepared job for [`ParallelMerge::merge_jobs`] —
/// the env lives in the executor, so the trait stays object-safe.
pub(crate) struct ParallelJobSpec {
    pub(crate) inputs: Vec<SstTable>,
    pub(crate) file_num: u64,
    pub(crate) cf: String,
    pub(crate) gc: crate::merge::CompactGcOptions,
    pub(crate) dir: PathBuf,
    pub(crate) sync: bool,
    pub(crate) split_target: u64,
    pub(crate) chunk_budget: usize,
    pub(crate) kit: Option<crate::cache::PayloadKit>,
}

/// Concrete [`ParallelMerge`] for any thread-shareable env.
pub(crate) struct ParallelMergeEnv<E> {
    env: E,
}

impl<E> ParallelMergeEnv<E> {
    pub(crate) fn new(env: E) -> Self {
        Self { env }
    }
}

impl<E> ParallelMerge for ParallelMergeEnv<E>
where
    E: Env + Send + Sync + 'static,
{
    fn merge(
        &self,
        first_file_num: u64,
        tables: &[SstTable],
        dir: &Path,
        gc: crate::merge::CompactGcOptions,
        do_sync_dir: bool,
        split_target: u64,
        chunk_budget: usize,
        kit: Option<crate::cache::PayloadKit>,
    ) -> Result<Vec<SstTable>> {
        write_merged_tables_parallel(
            &self.env,
            dir,
            first_file_num,
            tables,
            gc,
            do_sync_dir,
            split_target,
            chunk_budget,
            kit.as_ref(),
        )
    }

    fn merge_jobs(&self, jobs: Vec<ParallelJobSpec>) -> Result<Vec<Vec<SstTable>>> {
        if jobs.len() <= 1 {
            return jobs.into_iter().map(|j| self.write_spec(&j)).collect();
        }
        let results: Vec<Result<Vec<SstTable>>> = std::thread::scope(|scope| {
            let handles: Vec<_> = jobs
                .into_iter()
                .map(|spec| scope.spawn(move || self.write_spec(&spec)))
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or_else(|_| {
                        Err(CoreError::Internal(
                            "parallel compaction job panicked".into(),
                        ))
                    })
                })
                .collect()
        });
        results.into_iter().collect()
    }
}

impl<E: Env> ParallelMergeEnv<E> {
    /// One job, sequentially — the per-thread body of `merge_jobs` and the
    /// single-job fallback. Mirrors `PreparedL0Compact::write_merged_with_cf`
    /// (same `write_merged_tables` call shape + CF attachment).
    fn write_spec(&self, spec: &ParallelJobSpec) -> Result<Vec<SstTable>> {
        write_merged_tables(
            &self.env,
            &spec.dir,
            spec.file_num,
            &spec.inputs,
            spec.gc,
            spec.sync,
            spec.split_target,
            spec.chunk_budget,
            spec.kit.as_ref(),
        )
        .map(|ts| {
            ts.into_iter()
                .map(|t| t.with_cf(spec.cf.clone()))
                .collect::<Vec<_>>()
        })
    }
}

/// `PEDRA_PARALLEL_MERGE=1`: within-job key-space span merges (installed
/// seam only). Guest run #17 measured this neutral on the guest and the
/// local 6M A/B was net-negative — default off.
fn parallel_merge_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("PEDRA_PARALLEL_MERGE").is_some_and(|v| v == "1"))
}

/// `PEDRA_PARALLEL_JOBS`: max concurrently-written disjoint compaction
/// jobs in `Db::compact_leveled`, clamped 1..=8. Default 1 (off).
fn parallel_jobs_from_env() -> usize {
    static K: OnceLock<usize> = OnceLock::new();
    *K.get_or_init(|| {
        std::env::var("PEDRA_PARALLEL_JOBS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1)
            .clamp(1, 8)
    })
}

impl<E: Env> PreparedL0Compact<E> {
    /// Paths of L0 files this job will replace.
    #[must_use]
    pub fn input_paths(&self) -> Vec<PathBuf> {
        self.inputs.iter().map(|t| t.path().to_path_buf()).collect()
    }

    /// Merge inputs into one or more SSTs split at the compaction target
    /// file size (streaming when `gc` is default). Large jobs run as
    /// parallel key-space spans through the [`ParallelMerge`] seam when the
    /// host installed one ([`Db::set_parallel_merge`]); otherwise sequential.
    ///
    /// # Errors
    /// SST encode / I/O. On error the live L0 inventory is unchanged.
    pub fn write(&self) -> Result<Vec<SstTable>> {
        // PEDRA_LEVEL_DIAG: per-job merge+encode wall time — splits a slow
        // settle/ingest between fd barriers, memtable flush, and compaction.
        let t0 = std::env::var_os("PEDRA_LEVEL_DIAG").map(|_| Instant::now());
        let out = self.write_merged_with_cf();
        if let Some(t0) = t0 {
            let n_out = out.as_ref().map(Vec::len).unwrap_or(0);
            println!(
                "COMPDUR ms={} inputs={} outputs={}",
                t0.elapsed().as_millis(),
                self.inputs.len(),
                n_out
            );
        }
        out
    }

    fn write_merged_with_cf(&self) -> Result<Vec<SstTable>> {
        let cf = self
            .inputs
            .first()
            .map(|t| t.cf().to_string())
            .unwrap_or_default();
        // The seam is installed for either parallel dimension; only the
        // within-job spans opt in here (run #17 no-go keeps them off by
        // default). Across-job batching goes through `merge_jobs` instead.
        let merged = match &self.parallel {
            Some(pm) if parallel_merge_enabled() => pm.merge(
                self.file_num,
                &self.inputs,
                &self.dir,
                self.gc,
                self.sync,
                self.split_target,
                self.chunk_budget,
                self.kit.clone(),
            ),
            _ => write_merged_tables(
                &self.env,
                &self.dir,
                self.file_num,
                &self.inputs,
                self.gc,
                self.sync,
                self.split_target,
                self.chunk_budget,
                self.kit.as_ref(),
            ),
        };
        merged.map(|ts| {
            ts.into_iter()
                .map(|t| t.with_cf(cf.clone()))
                .collect::<Vec<_>>()
        })
    }

    /// E-free copy of this job for [`ParallelMerge::merge_jobs`] (the
    /// executor owns the env; inputs are cheap `Arc` clones).
    fn job_spec(&self) -> ParallelJobSpec {
        ParallelJobSpec {
            inputs: self.inputs.clone(),
            file_num: self.file_num,
            cf: self
                .inputs
                .first()
                .map(|t| t.cf().to_string())
                .unwrap_or_default(),
            gc: self.gc,
            dir: self.dir.clone(),
            sync: self.sync,
            split_target: self.split_target,
            chunk_budget: self.chunk_budget,
            kit: self.kit.clone(),
        }
    }
}

/// Live SST inventory row (RFC-0065 P0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SstLiveMeta {
    /// File name (`NNNNNN.sst`).
    pub name: String,
    /// LSM level (0 = L0).
    pub level: u32,
    /// Column-family name; empty = mixed / prefix-era file.
    pub cf: String,
    /// On-disk size in bytes.
    pub size: u64,
    /// Smallest user key (empty if unknown).
    pub start_key: Vec<u8>,
    /// Largest user key (empty if unknown).
    pub end_key: Vec<u8>,
    /// Internal-key count.
    pub num_entries: u64,
}

/// Options for [`Db::compact_with`] (RFC-0009 compaction / version GC).
#[derive(Debug, Clone, Copy, Default)]
pub struct CompactOptions {
    /// Version GC policy applied while rewriting SSTs.
    pub gc: crate::merge::CompactGcOptions,
    /// Cap on input L0 files per prepared job (`None` = all, single-writer
    /// default). The compat host worker bounds this so one L0→L1 merge
    /// holds a bounded input/output set instead of every parked L0 at once
    /// (25M slipstream OOM: 5 × 256 MiB inputs spiked RSS +1.7 GB/s).
    pub max_input_files: Option<usize>,
}

impl CompactOptions {
    /// Compact keeping only the newest version of each user key.
    #[must_use]
    pub fn latest_only() -> Self {
        Self {
            gc: crate::merge::CompactGcOptions::latest_only(),
            max_input_files: None,
        }
    }
}

/// Embedded database: one process, one directory.
///
/// Generic over [`Env`] so tests can inject disk faults (see `pedradb-sim::FailingEnv`).
/// Production default is [`StdEnv`].
///
/// # LSM levels
///
/// Flushes land in **L0**. Compaction merges a **subset** of files (typically all
/// of level N plus overlapping level N+1) into level N+1 — not only a whole-merge
/// of every SST into one file. Levels are persisted in MANIFEST v2.
///
/// # Concurrency
///
/// [`Db`] itself is single-threaded (`&mut` for writes). Use [`ConcurrentDb`] for
/// multi-thread access with a coarse mutex/rwlock.
/// RFC-0046 P1.2 remote mirror destination: any `Env` + a
/// [`crate::history::RemoteTier`] root inside it.
struct RemoteHistory<E: Env> {
    env: E,
    tier: crate::history::RemoteTier,
}

/// RFC-0046 P2.8: bounded LRU of remote segments already fetched and
/// CRC-verified this open. Object names are content-addressed
/// (len + crc32c + FNV-1a 64, P2.7), so one verified decode is stable
/// for the name — cache hits skip both the object fetch and the record
/// walk. The cache trusts the name: object bytes replaced under an
/// existing name (operator-level corruption) are not re-detected after
/// that segment's first verified read. Entries larger than the budget
/// are never cached; a budget cut evicts immediately (LRU order).
#[derive(Default)]
struct SegmentCache {
    budget: u64,
    used: u64,
    clock: u64,
    map: std::collections::HashMap<String, SegmentCacheEntry>,
    order: std::collections::BTreeSet<(u64, String)>,
}

struct SegmentCacheEntry {
    records: std::sync::Arc<Vec<crate::history::HistoryRecord>>,
    cost: u64,
    at: u64,
}

impl SegmentCache {
    fn get(&mut self, name: &str) -> Option<std::sync::Arc<Vec<crate::history::HistoryRecord>>> {
        let entry = self.map.get_mut(name)?;
        self.clock += 1;
        let now = self.clock;
        self.order.remove(&(entry.at, name.to_string()));
        entry.at = now;
        self.order.insert((now, name.to_string()));
        Some(std::sync::Arc::clone(&entry.records))
    }

    fn insert(
        &mut self,
        name: &str,
        records: std::sync::Arc<Vec<crate::history::HistoryRecord>>,
        cost: u64,
    ) {
        self.remove(name);
        if cost > self.budget {
            return; // oversize segments never cache
        }
        while self.used + cost > self.budget {
            if !self.evict_lru() {
                break;
            }
        }
        self.clock += 1;
        let at = self.clock;
        self.order.insert((at, name.to_string()));
        self.used += cost;
        self.map
            .insert(name.to_string(), SegmentCacheEntry { records, cost, at });
    }

    fn evict_lru(&mut self) -> bool {
        let Some((at, name)) = self.order.iter().next().cloned() else {
            return false;
        };
        self.order.remove(&(at, name.clone()));
        if let Some(entry) = self.map.remove(&name) {
            self.used = self.used.saturating_sub(entry.cost);
        }
        true
    }

    fn remove(&mut self, name: &str) -> bool {
        if let Some(entry) = self.map.remove(name) {
            self.order.remove(&(entry.at, name.to_string()));
            self.used = self.used.saturating_sub(entry.cost);
            true
        } else {
            false
        }
    }

    fn set_budget(&mut self, budget: u64) {
        self.budget = budget;
        while self.used > self.budget {
            if !self.evict_lru() {
                break;
            }
        }
    }
}

/// WAL-encoded op not yet in the memtable (RFC-0045 P2.1). Occupied only
/// while a [`crate::concurrent::ConcurrentDb`] leader has dropped the write
/// lock for `fdatasync`. OCC [`Db::key_has_write_after`] consults this so a
/// later group cannot miss a sequenced-but-unapplied write.
struct UnappliedOp {
    seq: SequenceNumber,
    kind: ValueType,
    key: Bytes,
    /// Range-tombstone end; empty for point puts/deletes.
    end: Bytes,
}

/// One level's tables grouped for point lookup: newest-first order, plus
/// the same tables sorted by `lo` key when the run is provably pairwise
/// disjoint. Disjoint runs bisect to the single candidate table instead of
/// walking every table's bounds + bloom.
struct SstRun {
    level: u32,
    tables_newest_first: Vec<usize>,
    disjoint_by_lo: Option<Vec<usize>>,
    sorted_by_lo: Option<Vec<usize>>,
    packed_lo: Option<DisjointLos>,
    packed_hi: Option<DisjointLos>,
    disjoint_los: Option<DisjointLos>,
    /// Any table in the run carries a range tombstone. Bulk hydrate never
    /// does; skipping the per-get collect over ~400 files was the remaining
    /// O(n) on `probe_miss` after bounds+bloom.
    any_range_tombstones: bool,
}

#[derive(Clone)]
struct DisjointLos {
    bytes: Vec<u8>,
    ends: Vec<u32>,
}

impl DisjointLos {
    fn from_tables(ssts: &[SstTable], by_lo: &[usize]) -> Self {
        let mut bytes = Vec::new();
        let mut ends = Vec::with_capacity(by_lo.len());
        for &i in by_lo {
            let lo = ssts[i]
                .smallest_user_key()
                .expect("disjoint run tables are bounded");
            bytes.extend_from_slice(lo);
            ends.push(u32::try_from(bytes.len()).expect("concatenated lo keys fit u32"));
        }
        Self { bytes, ends }
    }

    fn lo(&self, i: usize) -> &[u8] {
        let start = if i == 0 { 0 } else { self.ends[i - 1] as usize };
        &self.bytes[start..self.ends[i] as usize]
    }

    fn partition_point_gt(&self, key: &[u8]) -> usize {
        let n = self.ends.len();
        let mut lo = 0usize;
        let mut hi = n;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.lo(mid) <= key {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }
}

impl SstRun {
    /// Tables sorted by `lo`, or `None` unless every table is bounded and
    /// the run is strictly disjoint (`hi[i] < lo[i+1]`). Overlaps, duplicate
    /// bounds, or unbounded tables keep the linear newest-first walk.
    fn sorted_by_lo(ssts: &[SstTable], tables_newest_first: &[usize]) -> Option<Vec<usize>> {
        if tables_newest_first.is_empty() {
            return None;
        }
        for &i in tables_newest_first {
            if ssts[i].smallest_user_key().is_none() || ssts[i].largest_user_key().is_none() {
                return None;
            }
        }
        let mut by_lo: Vec<usize> = tables_newest_first.to_vec();
        by_lo.sort_by(|&a, &b| {
            ssts[a]
                .smallest_user_key()
                .unwrap()
                .cmp(ssts[b].smallest_user_key().unwrap())
        });
        Some(by_lo)
    }

    fn pairwise_disjoint(ssts: &[SstTable], by_lo: &[usize]) -> bool {
        by_lo.len() >= 2
            && by_lo.windows(2).all(|pair| {
                ssts[pair[0]].largest_user_key().unwrap()
                    < ssts[pair[1]].smallest_user_key().unwrap()
            })
    }
}

/// Lazy concatenation of one disjoint level's tables in `lo` order. The
/// scan window visits at most one table at a time (strict disjointness is
/// proven when the run's `disjoint_by_lo` order is built), so the merge
/// sees ONE
/// stream per level instead of one per file — heap width at 25 M drops
/// from ~#SSTs to ~#levels + L0 + memtables, cutting sift levels per row.
struct LevelRunStream<'a, E: Env> {
    db: &'a Db<E>,
    files_by_lo: &'a [usize],
    next_file: usize,
    start: Bound<Bytes>,
    end: Bound<Bytes>,
    snapshot: SequenceNumber,
    resolve_values: bool,
    // Concrete iter (not a boxed LayerStream): one dyn call per row from the
    // merge, the inner per-row walk stays a static, inlinable call.
    current: Option<crate::sst::SstRangeIter<'a>>,
}

/// Bound copies for the per-file `iter_user_range` calls (the iter owns its
/// own copies; this just re-derives the borrowed view for each call).
fn bound_slice(b: &Bound<Bytes>) -> Bound<&[u8]> {
    match b {
        Bound::Included(k) => Bound::Included(&k[..]),
        Bound::Excluded(k) => Bound::Excluded(&k[..]),
        Bound::Unbounded => Bound::Unbounded,
    }
}

/// First lo-sorted disjoint file that can overlap `[start, end)`.
/// Files before this have `hi` strictly before `start`, so a prefix in
/// the middle of a bulk run must not walk them (O(log n) vs O(#SST)).
fn first_overlapping_disjoint_file(
    ssts: &[SstTable],
    files_by_lo: &[usize],
    start: Bound<&[u8]>,
) -> usize {
    match start {
        Bound::Unbounded => 0,
        Bound::Included(s) => {
            files_by_lo.partition_point(|&i| ssts[i].largest_user_key().is_some_and(|hi| hi < s))
        }
        Bound::Excluded(s) => {
            files_by_lo.partition_point(|&i| ssts[i].largest_user_key().is_some_and(|hi| hi <= s))
        }
    }
}

impl<'a, E: Env> LevelRunStream<'a, E> {
    fn new(
        db: &'a Db<E>,
        files_by_lo: &'a [usize],
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        snapshot: SequenceNumber,
        resolve_values: bool,
    ) -> Self {
        let own = |b: Bound<&[u8]>| match b {
            Bound::Included(k) => Bound::Included(Bytes::copy_from_slice(k)),
            Bound::Excluded(k) => Bound::Excluded(Bytes::copy_from_slice(k)),
            Bound::Unbounded => Bound::Unbounded,
        };
        let next_file = first_overlapping_disjoint_file(&db.ssts, files_by_lo, start);
        Self {
            db,
            files_by_lo,
            next_file,
            start: own(start),
            end: own(end),
            snapshot,
            resolve_values,
            current: None,
        }
    }
}

impl<'a, E: Env> Iterator for LevelRunStream<'a, E> {
    type Item = (InternalKey, Bytes);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(cur) = self.current.as_mut() {
                if let Some(row) = cur.next() {
                    return Some(row);
                }
                self.current = None;
            }
            while self.next_file < self.files_by_lo.len() {
                let fi = self.files_by_lo[self.next_file];
                self.next_file += 1;
                let table = &self.db.ssts[fi];
                let (start, end) = (bound_slice(&self.start), bound_slice(&self.end));
                if !table.overlaps_user_range(start, end) {
                    continue;
                }
                self.db.scan_sst_probed.fetch_add(1, Ordering::Relaxed);
                let _ = self.db.env.advise(table.path(), 0, 0, AdviseKind::WillNeed);
                let id = crate::cache::path_id(table.path())
                    ^ if self.resolve_values {
                        RESOLVED_BLOCK_TAG
                    } else {
                        0
                    };
                let db = self.db;
                let resolve = self.resolve_values;
                let load = Box::new(move |bi| {
                    Some(crate::sst::scan_block_get_or_insert(id, bi, || {
                        let mut entries = match table.decode_block(bi) {
                            Ok(entries) => entries,
                            // F1: fail loudly on a corrupt block — same
                            // contract as the per-file stream path.
                            Err(e) => fail_stop_corrupt_block(table.path(), &e),
                        };
                        if resolve {
                            db.prefetch_resolve_stream(&mut entries);
                        }
                        entries
                    }))
                });
                self.current = Some(table.iter_user_range(
                    start,
                    end,
                    self.snapshot,
                    self.resolve_values,
                    load,
                ));
                break;
            }
            // Nothing drained and no file pulled a stream: run exhausted.
            self.current.as_ref()?;
        }
    }
}

/// Visible page from one or more internally-disjoint levels (L0 first =
/// newest). Same user key across levels keeps the highest sequence.
fn merge_disjoint_level_page<E: Env>(
    db: &Db<E>,
    runs: &[&[usize]],
    start: Bound<&[u8]>,
    end: Bound<&[u8]>,
    snapshot: SequenceNumber,
    limit: usize,
) -> Vec<(Bytes, Bytes)> {
    if runs.len() == 1 {
        let mut stream = LevelRunStream::new(db, runs[0], start, end, snapshot, true);
        let mut out = Vec::with_capacity(limit);
        while out.len() < limit {
            let Some((ikey, value)) = stream.next() else {
                break;
            };
            if ikey.sequence > snapshot || ikey.kind != ValueType::Value {
                continue;
            }
            out.push((ikey.user_key, value));
        }
        return out;
    }
    let mut streams: Vec<LevelRunStream<'_, E>> = runs
        .iter()
        .map(|by_lo| LevelRunStream::new(db, by_lo, start, end, snapshot, true))
        .collect();
    let mut heads: Vec<Option<(InternalKey, Bytes)>> =
        streams.iter_mut().map(Iterator::next).collect();
    let mut out = Vec::with_capacity(limit);
    while out.len() < limit {
        let mut best: Option<usize> = None;
        for (i, h) in heads.iter().enumerate() {
            let Some((ik, _)) = h else {
                continue;
            };
            match best {
                None => best = Some(i),
                Some(b) => {
                    let bk = &heads[b].as_ref().expect("best index has a head").0;
                    match ik.user_key.as_ref().cmp(bk.user_key.as_ref()) {
                        std::cmp::Ordering::Less => best = Some(i),
                        std::cmp::Ordering::Equal if ik.sequence > bk.sequence => best = Some(i),
                        _ => {}
                    }
                }
            }
        }
        let Some(i) = best else {
            break;
        };
        let (ikey, value) = heads[i].take().expect("best head");
        for (j, h) in heads.iter_mut().enumerate() {
            if j == i {
                continue;
            }
            if h.as_ref().is_some_and(|(k, _)| k.user_key == ikey.user_key) {
                *h = streams[j].next();
            }
        }
        loop {
            match streams[i].next() {
                Some((k, _)) if k.user_key == ikey.user_key => continue,
                other => {
                    heads[i] = other;
                    break;
                }
            }
        }
        if ikey.sequence <= snapshot && ikey.kind == ValueType::Value {
            out.push((ikey.user_key, value));
        }
    }
    out
}

/// [`Db`] itself is single-threaded (`&mut` for writes). Use [`ConcurrentDb`] for
/// multi-thread access with a coarse mutex/rwlock.
pub struct Db<E: Env = StdEnv> {
    dir: PathBuf,
    env: E,
    /// Shared so ConcurrentDb can `fdatasync` without the Db write lock
    /// (RFC-0041 P1.1). Rotate waits for [`Self::commit_inflight`] == 0.
    wal: Arc<Mutex<Wal<E::File>>>,
    /// Group-commit appends in flight (not yet applied). Blocks WAL rotate.
    /// `Arc` so `ConcurrentDb` can observe it lock-free: the RFC-0062 lone
    /// path holds the write lock through `fdatasync`, so a counter read
    /// behind the `Db` RwLock is blind exactly while a commit is in flight.
    commit_inflight: Arc<AtomicUsize>,
    /// Sequenced WAL ops waiting for the off-lock `fdatasync` to finish
    /// before memtable apply (RFC-0045 P2.1).
    unapplied: Vec<UnappliedOp>,
    /// Active memtable (new writes).
    mem: MemTable,
    /// Immutable memtable being flushed (Rocks dual-memtable / pipeline).
    /// Reads consult `mem` then `imm` then SSTs. Writers only mutate `mem`.
    imm: Option<MemTable>,
    /// Clone of the table taken by [`Self::prepare_flush_imm`] so readers still
    /// see acked keys while SST I/O runs off the write lock.
    flush_read_pin: Option<MemTable>,
    /// Flushed mems with **no L0 SST yet**. WAL still covers them (G1);
    /// rotate is blocked until the host materializes files. Park is a move
    /// (no BTree clone) so apply does not pay lz4 mid-burst (RFC-0041).
    /// `Arc` so pairwise fold can snapshot two tables without cloning the
    /// BTree under the read lock (parkfold MVCC max 16 ms was that clone).
    parked_unflushed: Vec<Arc<MemTable>>,
    /// F174: pair an in-flight fold cloned; the swap only proceeds while
    /// these are still the two oldest (a concurrent materialize may have
    /// installed the front as an L0 and popped it during the off-lock build).
    fold_pair_expected: Option<(Arc<MemTable>, Arc<MemTable>)>,
    /// Flushed pins waiting to be folded (cheap push on the write path).
    retired_pending: Vec<MemTable>,
    /// Single BTree of flushed versions (built off-lock when writers idle).
    retired_fold: MemTable,
    /// How many L0 files the retired cache covers (pending + fold).
    retired_l0s: usize,
    /// Cached [`Self::sst_indices_newest_first`] (L0 newest → L1+).
    sst_order_newest: Vec<usize>,
    /// Per-level lookup runs built alongside [`Self::sst_order_newest`]
    /// (see [`SstRun`]). `last_under_user_prefix_sst` keeps walking the
    /// flat order above.
    sst_runs: Vec<SstRun>,
    /// Inclusive envelope of every SST's user-key bounds. A point miss
    /// outside it cannot be in any file (probe_miss keys of the form
    /// `route.svc-9…` sit past `svc-099999` at 100M).
    sst_user_lo: Option<Bytes>,
    sst_user_hi: Option<Bytes>,
    sst_envelope: Arc<RwLock<Vec<(Bytes, Bytes)>>>,
    settled_sst_only: Arc<AtomicBool>,
    /// Immutable tables, oldest → newest within inventory order.
    ssts: Vec<SstTable>,
    /// LSM level for each entry in [`Self::ssts`] (parallel array; 0 = L0).
    sst_levels: Vec<u32>,
    /// Registered CF names (RFC-0065). Empty = kernel / no split: every SST
    /// is one family. Compat sets this from `open_cf` / CFREG.
    physical_cfs: Vec<String>,
    /// L0 files written without `fdatasync`. Must be synced before WAL rotate
    /// or any MANIFEST publish (RFC-0041). Crash before that is recovered
    /// from WAL; `gc_orphan_ssts` drops the unsynced files.
    unsynced_ssts: Vec<PathBuf>,
    /// Next SST file number (`000001.sst`, …).
    next_file_num: u64,
    /// Last written MANIFEST file number (0 = none yet).
    manifest_file_num: u64,
    /// Monotonic MANIFEST snapshot epoch (F187). Unlike
    /// [`Self::manifest_file_num`] (which undos roll back), it never
    /// decreases for the process lifetime, so it can order off-lock
    /// [`ManifestPersist::write`] calls.
    manifest_epoch: u64,
    /// Highest epoch actually written to disk (F187). A stale off-lock
    /// persist no-ops instead of regressing `CURRENT`.
    manifest_write_gate: Arc<Mutex<u64>>,
    /// MANIFEST flag: open `VALUES.vlog.new` (SST pointers already remapped).
    vlog_use_new: bool,
    /// Next sequence to assign (1-based; 0 means “no writes yet”).
    next_seq: SequenceNumber,
    /// Highest sequence default reads may observe. Assigned (`next_seq-1`)
    /// may be ahead while a ConcurrentDb leader has encoded WAL but not yet
    /// applied+published (RFC-0045 P2.1: apply after durable fsync; G1: Ok
    /// and `get` wait for publish after fd).
    published_seq: Arc<AtomicU64>,
    sync: bool,
    auto_flush_bytes: Option<usize>,
    /// Per-CF auto-flush caps (RFC-0065 P1.1). Empty = use [`Self::auto_flush_bytes`]
    /// for the whole table (legacy). When non-empty, each registered family
    /// flushes independently.
    cf_write_buffer: std::collections::BTreeMap<String, usize>,
    auto_compact_sst_count: Option<usize>,
    auto_compact_sst_bytes: Option<u64>,
    /// Reuses decoded SST handles (verify / reopen path).
    table_cache: TableCache,
    /// Decompressed block cache (hit stats for read path).
    block_cache: BlockCache,
    /// Bounded SST payload residency (RFC-0042 v18). Budget `None` = legacy
    /// (every payload resident). Armed only by a bounded open.
    sst_payload_pool: Arc<crate::cache::SstPayloadPool>,
    /// File source for evicted-payload reloads; `None` on a legacy open
    /// (the pool then never evicts — decode never fails for lack of a source).
    sst_source: Option<Arc<dyn crate::env::SstFileSource>>,
    /// Read-handle cache behind [`Self::sst_source`] (bounded opens):
    /// evicted-table block reads reuse open handles instead of paying
    /// `open()` per 4 KiB block. Empty (capacity 0) on legacy opens.
    sst_file_cache: Arc<crate::env::FileHandleCache>,
    /// Latest-snapshot point answers; per-key inval on write (RFC-0035 / 0041).
    /// `Arc` so [`crate::concurrent::ConcurrentDb`] answers a hit without the
    /// Db read lock (YCSB C hit path).
    point_cache: Arc<PointCache>,
    /// User keys applied since last publish (point-cache drop, not a gen bump).
    dirty_points: Mutex<Vec<Bytes>>,
    /// Fat apply / range-delete: publish must gen-bump, not per-key inval.
    point_cache_reset: AtomicBool,
    /// Latest `last_under_user_prefix` answers; cleared on write.
    last_prefix_cache: AnswerCache<Option<Bytes>>,
    /// Latest `count_in_range` answers; range-aware invalidation
    /// (`CountCache`) so writes outside a window keep it hot.
    /// `Arc` so ConcurrentDb can hit without the Db read lock (`deps_scan`).
    count_cache: Arc<crate::cache::CountCache>,
    /// Bumped in [`Self::invalidate_read_answers`]. Compat TLS last-count
    /// (`deps_scan` zipf) checks this without encoding or locking the cache.
    read_cache_epoch: Arc<AtomicU64>,
    /// Fat-apply epoch for compat TLS last-get. 1-key puts leave this still
    /// and bump [`Self::key_gen`] instead (RFC-0154 P1.5).
    point_tls_epoch: Arc<AtomicU64>,
    /// Per-encoded-key generation for 1-key TLS invalidation.
    key_gen: Arc<KeyGenMap>,
    /// RFC-0045 P0.1 phase timings (`PEDRA_WRITE_PHASE_STATS=1`).
    phase_stats: Option<Arc<WritePhaseStats>>,
    /// RFC-0047 P0.2: set when a [`WalRecovery::PointInTime`] open discarded
    /// a damaged WAL suffix (reported, never silently skipped).
    last_recovery: Option<RecoveryReport>,
    /// Exclusive directory lock (released via Env on close/drop when possible).
    dir_lock: Option<DirLock>,
    /// Set when append succeeded but required WAL `sync_all` failed (RFC-0015 H1).
    durability_fenced: bool,
    /// Live OCC transaction snapshot lower bounds (F201): `ConcurrentDb`
    /// shares its registry here so `compact_reclaim` / auto-compact GC
    /// floors cannot pass an open transaction's snapshot. `None` on a bare
    /// `Db` (OCC lives on `ConcurrentDb`; `tx::Transaction` is single-writer
    /// `&mut self` and cannot overlap a reclaim).
    occ_floor_registry: Option<Arc<Mutex<std::collections::BTreeMap<u64, SequenceNumber>>>>,
    /// First fence wins (RFC-0047 P1.1): I/O error + uncertain range.
    fence_report: Option<FenceReport>,
    /// Options this Db opened with (RFC-0047 P1.1: resume reopens with them).
    open_opts: OpenOptions,
    /// Auto-compact failures after successful flush (RFC-0015 M4 / P2.2).
    auto_compact_failures: u64,
    /// Last auto-compact error message (cleared only on successful auto-compact).
    last_auto_compact_error: Option<String>,
    /// Whole-levels rewrite chunk target (logical bytes); see
    /// [`REWRITE_CHUNK_TARGET_BYTES`].
    rewrite_chunk_target_bytes: u64,
    /// Large-value threshold (bytes); `None` = inline only.
    large_value_threshold: Option<usize>,
    /// Append-only value log when large values / existing vlog file present.
    vlog: Option<Mutex<ValueLog<E::File>>>,
    /// Rotate the active blob after this many bytes (`None` = single `VALUES.vlog`).
    vlog_rotate_bytes: Option<u64>,
    /// Active blob generation (`0` = `VALUES.vlog`).
    blob_active: u32,
    /// Prefetch window for scan vlog resolves (`0` = one-by-one).
    scan_prefetch: usize,
    /// Windows of prefetch issued (observability).
    prefetch_hits: AtomicU64,
    /// RFC-0035 latest/scan counters.
    latest_ops: AtomicU64,
    latest_mem_hit: AtomicU64,
    latest_sst_fallback: AtomicU64,
    latest_sst_probed: AtomicU64,
    scan_ops: AtomicU64,
    scan_sst_probed: AtomicU64,
    get_mem_hit: AtomicU64,
    get_sst_fallback: AtomicU64,
    lookup_sst_considered: AtomicU64,
    get_inline: AtomicU64,
    get_vlog: AtomicU64,
    mvcc_split_ops: AtomicU64,
    mvcc_ns_encode: AtomicU64,
    mvcc_ns_last: AtomicU64,
    mvcc_ns_get: AtomicU64,
    mvcc_ns_copy: AtomicU64,
    /// When set, best-effort [`Self::compact_blob_auto`] after flush / latest_only
    /// compact (RFC-0026 residual: no bg thread — runs on write path).
    auto_blob_gc_min_ratio: Option<f64>,
    /// When true, auto-compact uses snapshot-safe reclaim GC (open-items §2.1)
    /// instead of history-preserving merge. Off by default (F20).
    auto_reclaim: bool,
    /// When true, [`Self::finish_flush_pipeline`] does not compact. A host
    /// worker (compat/store, not this crate) drains L0 via
    /// [`Self::prepare_l0_compact`] (RFC-0037 P2.1). Default false.
    defer_auto_compact: bool,
    /// Sorted-ingest latch (RFC-0159): every committed batch is classified
    /// per family; a latched family's qualifying flush spans install
    /// directly at `MAX_LSM_LEVEL` instead of L0 (written once, never
    /// pushdown-rewritten). Pure decision state — see `bulk_ingest`.
    bulk_latch: crate::bulk_ingest::BulkLatch,
    /// `PEDRA_BULK` read once at open (per-batch env lookups would tax the
    /// commit path; the knob is static for a process lifetime).
    pub(crate) bulk_route_enabled: bool,
    /// Uninstalled sorted tails for latched families (RFC-0159 P0.3).
    bulk_runs: HashMap<String, crate::bulk_run::BulkRun>,
    /// Full chunks waiting for off-lock SST materialize (writer parks,
    /// host worker encodes). Lookup still sees them.
    parked_bulk: VecDeque<(String, Arc<crate::bulk_run::BulkRun>)>,
    /// Chunk the worker is encoding off-lock (not in `parked_bulk`).
    bulk_encoding: Option<(String, Arc<crate::bulk_run::BulkRun>)>,
    /// Bulk SST installs since the last MANIFEST persist (RFC-0159 P1.2).
    /// Async hydrate persists every [`BULK_MANIFEST_EVERY`] chunks off the
    /// write lock; v50 batched under the lock and regressed 0.98→0.91×.
    bulk_manifest_debt: u8,
    /// When `Some(n)`, refuse writes if L0 SST count ≥ n (open-items §2.3).
    write_stall_l0: Option<usize>,
    /// When `Some(n)`, refuse writes if active mem ≈ ≥ n bytes (open-items §2.3 c).
    write_stall_mem_bytes: Option<usize>,
    /// When `Some(n)`, one flush+compact when L0 ≥ n before admit (open-items §2.3 b).
    write_pressure_l0: Option<usize>,
    /// When true with a stall limit: one flush+compact attempt before refusing.
    write_stall_drain: bool,
    /// Count of writes refused by L0 / mem stall.
    write_stall_count: u64,
    /// Count of soft pressure drains (not errors).
    write_pressure_count: u64,
    /// Open [`SnapshotPin`]s: pin id → sequence (open-items §2.1).
    snapshot_pins: std::collections::BTreeMap<u64, SequenceNumber>,
    /// Next pin id (monotonic; never reused for this process open).
    next_snapshot_pin_id: u64,
    /// Version-GC watermark: snapshots with `seq < earliest_readable_seq` are
    /// [`CoreError::SnapshotTooOld`] (open-items §2.1 (c)). `0` = no floor.
    earliest_readable_seq: SequenceNumber,
    /// RFC-0046 retention options as opened (horizon + archive cap).
    history: HistoryOptions,
    /// Sampled `(published seq, unix_ms)` pairs for the horizon cutoff
    /// (RFC-0046 P0.1). Empty while the horizon is `All`.
    seq_times: Mutex<std::collections::VecDeque<(SequenceNumber, u64)>>,
    /// Publishes since the last seq/time sample (amortizes the clock read).
    seq_time_counter: AtomicU64,
    /// RFC-0046 P0.2 local history tier (archive-before-GC). Opened at open;
    /// the manifest floor feeds `earliest_readable_seq` across reopens.
    history_tier: Option<crate::history::HistoryTier>,
    /// RFC-0046 P1.2 remote history mirror (opt-in via
    /// [`Self::set_remote_history`]). Uploads run inline on the auto-compact
    /// path the host opted into; a failing tier pauses GC (backpressure —
    /// never drop what has not uploaded).
    remote_history: Option<RemoteHistory<E>>,
    /// Local segment ids verified present at the remote tier (P1.2). Lost on
    /// reopen — the first upload step repairs it idempotently.
    uploaded_history_segs: std::collections::HashSet<u64>,
    /// RFC-0046 P2.2: max segment bytes one upload step may ship (`None` =
    /// unlimited). Leftover segments stay pending; the cap holds them.
    upload_bandwidth: Option<u64>,
    /// RFC-0046 P2.8 read cache for remote segments (verified-decoded,
    /// LRU, byte-bounded by [`Self::set_remote_read_cache`]).
    remote_read_cache: Mutex<SegmentCache>,
    /// Unix-ms of the last archive pass this open (P2.2 age metric;
    /// in-memory — `None` after reopen until the next pass).
    last_archive_millis: Option<u64>,
    /// RFC-0046 P0.5: state of the last horizon-driven full reclaim —
    /// (GC floor it ran at, total SST bytes right after). The next full
    /// rewrite waits until the floor advanced AND SST bytes at least
    /// doubled (classic size-ratio trigger: at most one rewrite per
    /// dead-weight doubling). In-memory; a reopen may pay one extra
    /// rewrite (self-limiting, correct either way).
    last_horizon_reclaim: Option<(SequenceNumber, u64)>,
    /// Count of successful WAL `sync_all` (observability / group-commit tests).
    wal_sync_count: AtomicU64,
    /// Logical user-value bytes ingested.
    bytes_ingested: u64,
    /// Approximate WAL payload bytes written.
    bytes_written_wal: u64,
    /// SST file bytes written (flush/compact/vlog GC).
    bytes_written_sst: u64,
    /// SST compact success count.
    compact_count: u64,
    /// Value-log GC success count.
    vlog_gc_count: u64,
    /// Durable post-commit change log (RFC-0019 P0.3).
    change_log: ChangeLog,
    /// Persist CHANGELOG at most every N durable commits (RFC-0031). `0` = never
    /// on the commit path.
    changelog_interval: u64,
    /// Lazy-feed rebuild budget in entries: explicit flush / checkpoint /
    /// close only materializes MemTable ∪ SSTs into the CHANGELOG cache
    /// while the live entry count stays within it. Above it the cache stays
    /// stale and the feed is rebuilt on demand (RFC-0019: on-disk CHANGELOG
    /// is a cache).
    changelog_rebuild_budget_entries: u64,
    /// Merged compaction output splits into multiple SSTs at this many
    /// bytes (the SST writer buffers one output file in memory — see
    /// [`crate::compact_kernel::COMPACT_TARGET_FILE_BYTES`]). Rocks
    /// `target_file_size_base` role; operator-tunable.
    compact_target_file_bytes: u64,
    /// Size target of L1 for the leveled scheduler ([`crate::leveling`]);
    /// L`n+1` targets multiply by the fanout. Rocks `max_bytes_for_level_base`
    /// role. Independent of [`Self::compact_target_file_bytes`] so small-file
    /// tests do not trip level pushdowns.
    l1_target_bytes: u64,
    /// Key-space-parallel merge executor for prepared leveled jobs, shared
    /// with every job [`Self::build_prepared`] snapshots. Installed by the
    /// bounded host open path ([`ConcurrentDb::open_with_env_bounded`]) where
    /// `E: Send + Sync + 'static` holds; `None` = sequential merges.
    parallel_merge: Option<Arc<dyn ParallelMerge>>,
    /// Max concurrently-written pairwise-disjoint pushdown jobs in
    /// [`Self::compact_leveled`] (across-job parallelism; each job still
    /// merges sequentially). 1 = off. Set from `PEDRA_PARALLEL_JOBS` on the
    /// host open path; needs [`Self::parallel_merge`] installed (thread-
    /// shareable env).
    parallel_jobs: usize,
    /// Durable commits since the last CHANGELOG store.
    commits_since_changelog: u64,
    /// Successful CHANGELOG stores since open.
    changelog_store_count: u64,
}

impl Db<StdEnv> {
    /// Open or create a database at `path` on **POSIX** [`StdEnv`].
    ///
    /// Engine tests and DST (`FailingEnv`) use this. **Production** processes
    /// (CLI, store, compat, HTTP) open via `pedradb_io_uring::open` —
    /// Linux `io_uring`, POSIX fallback elsewhere.
    ///
    /// Loads `*.sst` files, then recovers the WAL into the MemTable.
    ///
    /// # Errors
    /// I/O failures, corrupt logical records, or CRC errors on non-truncated data.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, OpenOptions::default())
    }

    /// Open with explicit options on POSIX [`StdEnv`] (see [`Self::open`]).
    ///
    /// # Errors
    /// Same as [`Self::open`].
    pub fn open_with(path: impl AsRef<Path>, opts: OpenOptions) -> Result<Self> {
        Self::open_with_env(path, opts, StdEnv)
    }
}

// RFC-0162 P0.2: per-chunk bulk-hydrate timing for diagnostic runs
// (`PEDRA_BULK_STAGE_TIMING=1`). Zero cost when the env is unset; the
// official gate runs never set it.
static BSTAGE_IDX: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static BSTAGE_EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

fn bulk_stage_timing_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("PEDRA_BULK_STAGE_TIMING").is_ok_and(|v| !v.is_empty() && v != "0")
    })
}

fn bulk_stage_line(
    idx: u64,
    epoch_ms: u128,
    t_ms: u128,
    entries: usize,
    bytes: usize,
    caller: &str,
    sync: bool,
) -> String {
    format!(
        "BSTAGE idx={idx} epoch_ms={epoch_ms} t_ms={t_ms} entries={entries} bytes={bytes} caller={caller} sync={sync}"
    )
}

impl<E: Env> Db<E> {
    /// Open using a [`Host`]'s filesystem seam (`host.env()`).
    ///
    /// Clock/RNG on the host are unused by the kernel (leases / election live in
    /// layers); this is the one-call DST plug: `DetHost` + `FailingEnv`.
    ///
    /// # Errors
    /// Same as [`Self::open_with_env`].
    pub fn open_with_host(
        path: impl AsRef<Path>,
        opts: OpenOptions,
        host: &impl Host<Env = E>,
    ) -> Result<Self> {
        Self::open_with_env(path, opts, host.env().clone())
    }

    /// Open with an explicit [`Env`] (fault injection, in-memory, …).
    ///
    /// Payloads of recovered SSTs stay fully resident (legacy behavior); use
    /// [`Self::open_with_env_bounded`] to bound residency.
    ///
    /// # Errors
    /// I/O failures, corrupt logical records, CRC errors, [`CoreError::AlreadyOpen`],
    /// or corrupt MANIFEST.
    pub fn open_with_env(path: impl AsRef<Path>, opts: OpenOptions, env: E) -> Result<Self> {
        Self::open_with_env_sourced(
            path,
            opts,
            env,
            None,
            Arc::new(crate::env::FileHandleCache::new(0)),
        )
    }

    /// Open with an explicit [`Env`] **and the SST payload pool armed**
    /// (RFC-0042 v18). Resident file bodies are held to
    /// `opts.sst_payload_budget_bytes` (default
    /// [`DEFAULT_SST_PAYLOAD_BUDGET_BYTES`]); eviction happens during
    /// recovery, so reopening a multi-GiB store does not transiently hold
    /// the whole dataset in RAM. Requires `E: Send + Sync + 'static` because
    /// evicted tables re-read their file through a shared
    /// [`SstFileSource`](crate::env::SstFileSource) built from the env.
    ///
    /// # Errors
    /// Same as [`Self::open_with_env`].
    pub fn open_with_env_bounded(
        path: impl AsRef<Path>,
        mut opts: OpenOptions,
        env: E,
    ) -> Result<Self>
    where
        E: Env + Send + Sync + 'static,
        E::File: Send + 'static,
    {
        if opts.sst_payload_budget_bytes.is_none() {
            opts.sst_payload_budget_bytes = Some(sst_payload_budget_from_env());
        }
        let file_cache = Arc::new(crate::env::FileHandleCache::new(
            sst_file_cache_entries_from_env(),
        ));
        let source: Arc<dyn crate::env::SstFileSource> = Arc::new(
            crate::env::CachedEnvSource::new(<E as Clone>::clone(&env), Arc::clone(&file_cache)),
        );
        // Parallel merge executor for thread-shareable envs. Two opt-in
        // dimensions share the seam: within-job key-space spans
        // (`PEDRA_PARALLEL_MERGE=1` — guest run #17 neutral, local 6M A/B
        // net-negative: settle 7.9 vs 5.9 s, apply ~3× slower; stays off)
        // and across-job disjoint-batch compaction (`PEDRA_PARALLEL_JOBS`,
        // default 1 = off). Either being on needs the concrete executor
        // installed; the historical no-seam path (tests, generic envs)
        // merges sequentially either way.
        let jobs_k = parallel_jobs_from_env();
        let seam: Option<Arc<dyn ParallelMerge>> = if parallel_merge_enabled() || jobs_k > 1 {
            Some(Arc::new(ParallelMergeEnv::new(<E as Clone>::clone(&env))))
        } else {
            None
        };
        let mut db = Self::open_with_env_sourced(path, opts, env, Some(source), file_cache)?;
        if let Some(pm) = seam {
            db.set_parallel_merge(pm);
        }
        db.parallel_jobs = jobs_k;
        Ok(db)
    }

    /// Shared open path; `source = Some` arms the payload pool before
    /// recovery so reopen never materializes the whole dataset in RAM.
    ///
    /// # Errors
    /// Same as [`Self::open_with_env`].
    #[allow(clippy::too_many_lines)] // recover WAL + CHANGELOG + vlog in one open path
    fn open_with_env_sourced(
        path: impl AsRef<Path>,
        opts: OpenOptions,
        env: E,
        source: Option<Arc<dyn crate::env::SstFileSource>>,
        sst_file_cache: Arc<crate::env::FileHandleCache>,
    ) -> Result<Self> {
        crate::buggify_hooks::inject_checked(crate::buggify_hooks::sites::AFTER_OPEN_LOCK)?;
        let dir = path.as_ref().to_path_buf();
        env.create_dir_all(&dir)?;

        let lock = if opts.exclusive {
            Some(DirLock::acquire(&env, &dir)?)
        } else {
            None
        };

        manifest::cleanup_tmp_files(&env, &dir)?;

        let table_cache = TableCache::new(64);
        let block_cache = BlockCache::new(8192);
        let sst_payload_pool = Arc::new(crate::cache::SstPayloadPool::with_budget(
            opts.sst_payload_budget_bytes,
        ));
        if let Some(src) = &source {
            sst_payload_pool.arm();
            table_cache.set_payload_kit(crate::cache::PayloadKit {
                source: Arc::clone(src),
                pool: Arc::clone(&sst_payload_pool),
            });
        }
        // Official YCSB records=4096 (zipfian). 2048 FIFO + sequential load
        // evicted the hot low IDs; C then started cold (parkfold2 C 1.6×).
        let point_cache = Arc::new(PointCache::new(8192));
        let last_prefix_cache = AnswerCache::new(8192);
        let count_cache = Arc::new(crate::cache::CountCache::new(8192));
        let (
            ssts,
            sst_levels,
            next_file_num,
            manifest_file_num,
            vlog_use_new,
            mut max_seq,
            earliest_readable_seq,
        ) = recover_ssts(&env, &dir, opts.sync, &table_cache)?;

        let wal_path = dir.join(WAL_FILE_NAME);
        let mut mem = MemTable::new();
        let mut change_log = ChangeLog::load_on(&env, &dir)?;
        // RFC-0047 P0.2: set when a PointInTime open discards a WAL suffix.
        let mut point_in_time_report: Option<RecoveryReport> = None;

        if env.exists(&wal_path) {
            // Tiny WAL + Truncated(0): failed first append after rotate (or crash
            // before any complete record). Tolerate empty so SSTs still load (F6).
            // Large WAL + Truncated(0): bitrot of the first record — fail-stop (F4),
            // journaled for escalation (RFC-0038 D: repeated events refuse open).
            // RFC-0047 P0.2: in PointInTime mode the event is still journaled and
            // escalation still refuses the open; otherwise the decoded prefix is
            // served and the discarded suffix is reported (never silently skipped).
            let (records, last_good) = match Wal::recover_span_on(&env, &wal_path) {
                Ok((records, last_good, resync_origin)) => {
                    if let Some(origin) = resync_origin {
                        // F171: a resync walk skipped damaged bytes and a
                        // later record re-anchored — the skipped region is
                        // lost from the recovered set. FailClosed refuses
                        // (never silent-wrong); PointInTime journals, reports
                        // and serves the recovered prefix.
                        let escalated = crate::corrupt::escalate_or_fail(
                            &env,
                            &dir,
                            "resync",
                            origin,
                            CoreError::Internal("WAL resync skipped damaged region mid-log".into()),
                        );
                        // RFC-0053 Y3.3: reopen outcome decided by the pure
                        // kernel (refuse / serve prefix + report), not ad hoc.
                        match crate::wal::reopen_kernel::reopen_outcome(
                            crate::wal::reopen_kernel::ReopenDamage::Resync,
                            opts.wal_recovery == WalRecovery::PointInTime,
                            matches!(escalated, CoreError::CorruptionEscalated { .. }),
                        ) {
                            crate::wal::reopen_kernel::ReopenOutcome::ServePrefixReport => {
                                let (records, last_good, _err, _resync) =
                                    Wal::recover_prefix_span_on(&env, &wal_path)?;
                                point_in_time_report = Some(RecoveryReport {
                                    kind: "resync",
                                    corrupt_offset: origin,
                                    good_through_offset: last_good,
                                    discarded_bytes: env
                                        .metadata_len(&wal_path)
                                        .unwrap_or(0)
                                        .saturating_sub(last_good),
                                });
                                (records, last_good)
                            }
                            _ => return Err(escalated),
                        }
                    } else {
                        (records, last_good)
                    }
                }
                Err(CoreError::Truncated(0)) => {
                    let len = env.metadata_len(&wal_path).unwrap_or(0);
                    if len < 64 {
                        (Vec::new(), 0)
                    } else {
                        let escalated = crate::corrupt::escalate_or_fail(
                            &env,
                            &dir,
                            "truncated_head",
                            0,
                            CoreError::Truncated(0),
                        );
                        // RFC-0053 Y3.3: reopen outcome from the pure kernel.
                        match crate::wal::reopen_kernel::reopen_outcome(
                            crate::wal::reopen_kernel::ReopenDamage::TruncatedHead,
                            opts.wal_recovery == WalRecovery::PointInTime,
                            matches!(escalated, CoreError::CorruptionEscalated { .. }),
                        ) {
                            crate::wal::reopen_kernel::ReopenOutcome::ServePrefixReport => {
                                // Re-walk collecting the decoded prefix; the
                                // stopping error is the same head error that
                                // routed us here (first error is deterministic).
                                let (records, last_good, _prefix_err, _resync) =
                                    Wal::recover_prefix_span_on(&env, &wal_path)?;
                                point_in_time_report = Some(RecoveryReport {
                                    kind: "truncated_head",
                                    corrupt_offset: 0,
                                    good_through_offset: last_good,
                                    discarded_bytes: len.saturating_sub(last_good),
                                });
                                (records, last_good)
                            }
                            _ => return Err(escalated),
                        }
                    }
                }
                Err(e @ CoreError::Crc { offset, .. }) => {
                    // Mid-WAL bitflip: fail-stop (silent skip is G8-forbidden),
                    // journaled; the Nth event escalates (RFC-0038 D).
                    let escalated = crate::corrupt::escalate_or_fail(&env, &dir, "crc", offset, e);
                    // RFC-0053 Y3.3: reopen outcome from the pure kernel.
                    match crate::wal::reopen_kernel::reopen_outcome(
                        crate::wal::reopen_kernel::ReopenDamage::Crc,
                        opts.wal_recovery == WalRecovery::PointInTime,
                        matches!(escalated, CoreError::CorruptionEscalated { .. }),
                    ) {
                        crate::wal::reopen_kernel::ReopenOutcome::ServePrefixReport => {
                            let (records, last_good, _prefix_err, _resync) =
                                Wal::recover_prefix_span_on(&env, &wal_path)?;
                            point_in_time_report = Some(RecoveryReport {
                                kind: "crc",
                                corrupt_offset: offset,
                                good_through_offset: last_good,
                                discarded_bytes: env
                                    .metadata_len(&wal_path)
                                    .unwrap_or(0)
                                    .saturating_sub(last_good),
                            });
                            (records, last_good)
                        }
                        _ => return Err(escalated),
                    }
                }
                Err(e @ CoreError::WalZeroHeader { offset }) => {
                    // F170: zero header at a fresh alignment with a non-zero
                    // tail — corruption, never padding. Journaled (RFC-0038)
                    // so repeated events escalate; PointInTime serves the
                    // decoded prefix and reports the discard.
                    let escalated =
                        crate::corrupt::escalate_or_fail(&env, &dir, "zero_header", offset, e);
                    // RFC-0053 Y3.3: reopen outcome from the pure kernel.
                    match crate::wal::reopen_kernel::reopen_outcome(
                        crate::wal::reopen_kernel::ReopenDamage::ZeroHeader,
                        opts.wal_recovery == WalRecovery::PointInTime,
                        matches!(escalated, CoreError::CorruptionEscalated { .. }),
                    ) {
                        crate::wal::reopen_kernel::ReopenOutcome::ServePrefixReport => {
                            let (records, last_good, _prefix_err, _resync) =
                                Wal::recover_prefix_span_on(&env, &wal_path)?;
                            point_in_time_report = Some(RecoveryReport {
                                kind: "zero_header",
                                corrupt_offset: offset,
                                good_through_offset: last_good,
                                discarded_bytes: env
                                    .metadata_len(&wal_path)
                                    .unwrap_or(0)
                                    .saturating_sub(last_good),
                            });
                            (records, last_good)
                        }
                        _ => return Err(escalated),
                    }
                }
                Err(e) => return Err(e),
            };
            // RFC-0048 P1.1: mid-log resync damage cannot be healed by the
            // tail-cut below (the damage sits before `last_good` after a
            // re-anchor) — PointInTime rewrites the WAL from the recovered
            // records so the next open, even fail-closed, is clean.
            if point_in_time_report
                .as_ref()
                .is_some_and(|r| r.kind == "resync")
            {
                let repair = dir.join(format!("{WAL_FILE_NAME}.repair"));
                let mut w = Wal::create_on(&env, &repair)?;
                w.set_full_fsync(opts.wal_full_fsync);
                for raw in &records {
                    w.append_record(raw)?;
                }
                w.sync_data()?;
                drop(w);
                env.rename(&repair, &wal_path)?;
                env.sync_dir(&dir)?;
            }
            let feed_max = change_log.max_sequence().unwrap_or(0);
            for raw in records {
                let rec = WriteRecord::decode(&raw)?;
                apply_record(&mut mem, &rec);
                if let Some(s) = rec.max_sequence() {
                    max_seq = max_seq.max(s);
                }
                // Rebuild feed entries present in WAL but missing from CHANGELOG
                // (crash between WAL sync and changelog persist).
                let mut missing = Vec::new();
                for op in &rec.ops {
                    if op.sequence > feed_max {
                        missing.push(ChangeEntry::from_write_op(op));
                    }
                }
                if !missing.is_empty() {
                    change_log.extend(missing);
                }
            }
            if change_log.max_sequence().unwrap_or(0) > feed_max {
                change_log.store_on(&env, &dir)?;
            }
            // RFC-0038 D: cut a torn tail to the last known-good offset so
            // new appends never sit on top of the damaged region (re-opening
            // would then fail-stop on its garbage as if it were records).
            let wal_len = env.metadata_len(&wal_path).unwrap_or(0);
            if wal_len > last_good {
                let mut wal_file = env.open_append(&wal_path)?;
                wal_file.set_len(last_good)?;
                wal_file.sync_data()?;
            }
        }

        let wal = Arc::new(Mutex::new({
            let mut w = if env.exists(&wal_path) {
                Wal::append_on(&env, &wal_path)?
            } else {
                Wal::create_on(&env, &wal_path)?
            };
            w.set_full_fsync(opts.wal_full_fsync);
            w
        }));

        // Watermark may exceed max sequence still present in SSTs (e.g. latest_only
        // dropped a high-seq tombstone). Keep last_sequence ≥ earliest so current
        // gets never look "too old" after reopen.
        let next_seq = max_seq.max(earliest_readable_seq).saturating_add(1).max(1);
        if next_seq > MAX_SEQUENCE_NUMBER {
            return Err(CoreError::Internal(
                "sequence number space exhausted".into(),
            ));
        }

        let large_value_threshold = opts.large_value_threshold.filter(|n| *n > 0);
        let vlog_path = dir.join(VLOG_FILE_NAME);
        let vlog_new = dir.join(crate::vlog::VLOG_NEW_NAME);
        let blob_nums = vlog::list_blob_nums(&env, &dir);
        let blob_active = blob_nums.last().copied().unwrap_or(0);
        // RFC-0056 P1.4: vlog swing decision comes from the pure kernel; the
        // flag-resolved arms delegate to `open_with_flag`, which encodes the
        // same F51 refuse / empty-create rules as defense-in-depth.
        let vlog = match crate::vlog_gc_kernel::vlog_recover_action(
            blob_active > 0,
            large_value_threshold.is_some(),
            env.exists(&vlog_path),
            vlog_use_new,
            env.exists(&vlog_new),
        ) {
            crate::vlog_gc_kernel::VlogRecoverAction::NoVlog => None,
            crate::vlog_gc_kernel::VlogRecoverAction::OpenBlob => {
                Some(Mutex::new(ValueLog::open_blob(&env, &dir, blob_active)?))
            }
            crate::vlog_gc_kernel::VlogRecoverAction::OpenNew
            | crate::vlog_gc_kernel::VlogRecoverAction::OpenPrimary
            | crate::vlog_gc_kernel::VlogRecoverAction::CreateEmptyPrimary
            | crate::vlog_gc_kernel::VlogRecoverAction::RefuseOpen => Some(Mutex::new(
                ValueLog::open_with_flag(&env, &dir, vlog_use_new)?,
            )),
        };

        let mut db = Self {
            dir,
            env,
            wal,
            commit_inflight: Arc::new(AtomicUsize::new(0)),
            unapplied: Vec::new(),
            mem,
            imm: None,
            flush_read_pin: None,
            parked_unflushed: Vec::new(),
            bulk_latch: crate::bulk_ingest::BulkLatch::new(),
            bulk_route_enabled: crate::bulk_ingest::bulk_enabled(),
            bulk_runs: HashMap::new(),
            parked_bulk: VecDeque::new(),
            bulk_encoding: None,
            bulk_manifest_debt: 0,
            fold_pair_expected: None,
            retired_pending: Vec::new(),
            retired_fold: MemTable::new(),
            retired_l0s: 0,
            sst_order_newest: Vec::new(),
            sst_runs: Vec::new(),
            sst_user_lo: None,
            sst_user_hi: None,
            sst_envelope: Arc::new(RwLock::new(Vec::new())),
            settled_sst_only: Arc::new(AtomicBool::new(false)),
            ssts,
            sst_levels,
            physical_cfs: Vec::new(),
            next_file_num,
            manifest_file_num,
            manifest_epoch: 0,
            manifest_write_gate: Arc::new(Mutex::new(0)),
            vlog_use_new,
            next_seq,
            published_seq: Arc::new(AtomicU64::new(next_seq.saturating_sub(1))),
            sync: opts.sync,
            auto_flush_bytes: opts.auto_flush_bytes.filter(|n| *n > 0),
            cf_write_buffer: std::collections::BTreeMap::new(),
            auto_compact_sst_count: opts.auto_compact_sst_count.filter(|n| *n > 0),
            auto_compact_sst_bytes: opts.auto_compact_sst_bytes.filter(|n| *n > 0),
            table_cache,
            block_cache,
            sst_payload_pool,
            sst_source: source,
            sst_file_cache,
            point_cache,
            last_prefix_cache,
            count_cache,
            read_cache_epoch: Arc::new(AtomicU64::new(1)),
            point_tls_epoch: Arc::new(AtomicU64::new(1)),
            key_gen: Arc::new(KeyGenMap::new()),
            phase_stats: std::env::var_os("PEDRA_WRITE_PHASE_STATS")
                .map(|_| Arc::new(WritePhaseStats::default())),
            last_recovery: point_in_time_report,
            dirty_points: Mutex::new(Vec::new()),
            point_cache_reset: AtomicBool::new(false),
            dir_lock: lock,
            durability_fenced: false,
            fence_report: None,
            occ_floor_registry: None,
            open_opts: opts,
            auto_compact_failures: 0,
            last_auto_compact_error: None,
            rewrite_chunk_target_bytes: REWRITE_CHUNK_TARGET_BYTES,
            large_value_threshold,
            vlog,
            vlog_rotate_bytes: None,
            blob_active,
            scan_prefetch: 4,
            prefetch_hits: AtomicU64::new(0),
            latest_ops: AtomicU64::new(0),
            latest_mem_hit: AtomicU64::new(0),
            latest_sst_fallback: AtomicU64::new(0),
            latest_sst_probed: AtomicU64::new(0),
            scan_ops: AtomicU64::new(0),
            scan_sst_probed: AtomicU64::new(0),
            get_mem_hit: AtomicU64::new(0),
            get_sst_fallback: AtomicU64::new(0),
            lookup_sst_considered: AtomicU64::new(0),
            get_inline: AtomicU64::new(0),
            get_vlog: AtomicU64::new(0),
            mvcc_split_ops: AtomicU64::new(0),
            mvcc_ns_encode: AtomicU64::new(0),
            mvcc_ns_last: AtomicU64::new(0),
            mvcc_ns_get: AtomicU64::new(0),
            mvcc_ns_copy: AtomicU64::new(0),
            auto_blob_gc_min_ratio: None,
            auto_reclaim: false,
            defer_auto_compact: false,
            write_stall_l0: None,
            write_stall_mem_bytes: None,
            write_pressure_l0: None,
            write_stall_drain: false,
            write_stall_count: 0,
            write_pressure_count: 0,
            snapshot_pins: std::collections::BTreeMap::new(),
            next_snapshot_pin_id: 1,
            earliest_readable_seq,
            history: opts.history,
            seq_times: Mutex::new(std::collections::VecDeque::new()),
            seq_time_counter: AtomicU64::new(0),
            history_tier: None,
            remote_history: None,
            uploaded_history_segs: std::collections::HashSet::new(),
            upload_bandwidth: None,
            remote_read_cache: Mutex::new(SegmentCache {
                budget: 64 * 1024 * 1024,
                ..Default::default()
            }),
            last_archive_millis: None,
            last_horizon_reclaim: None,
            wal_sync_count: AtomicU64::new(0),
            bytes_ingested: 0,
            bytes_written_wal: 0,
            bytes_written_sst: 0,
            compact_count: 0,
            vlog_gc_count: 0,
            change_log,
            changelog_interval: changelog_interval_from_env(),
            changelog_rebuild_budget_entries:
                crate::changelog_kernel::DEFAULT_CHANGELOG_REBUILD_BUDGET_ENTRIES,
            compact_target_file_bytes: crate::compact_kernel::COMPACT_TARGET_FILE_BYTES,
            l1_target_bytes: crate::compact_kernel::COMPACT_TARGET_FILE_BYTES,
            parallel_merge: None,
            parallel_jobs: 1,
            commits_since_changelog: 0,
            changelog_store_count: 0,
            unsynced_ssts: Vec::new(),
        };
        // RFC-0042 v18: `ScanAndInstall` recovery (legacy dirs, no MANIFEST)
        // opens tables outside the table cache — attach + register them here
        // so the armed pool bounds them too.
        if let Some(src) = &db.sst_source {
            for t in &db.ssts {
                t.attach_payload_kit(src, &db.sst_payload_pool);
            }
        }
        db.rebuild_sst_order();
        db.maybe_rebuild_feed_from_live();
        // RFC-0046 P0.2: restore the archive floor across reopens (a cap
        // overflow in a previous life must keep failing old snaps closed).
        let tier = crate::history::HistoryTier::open(&db.env, &db.dir)?;
        db.raise_earliest_readable(tier.archive_floor());
        db.history_tier = Some(tier);
        Ok(db)
    }

    /// Default WAL sync policy from open options (`true` unless opened with `sync: false`).
    #[must_use]
    pub fn default_write_sync(&self) -> bool {
        self.sync
    }

    /// Number of successful WAL fsyncs since open (group-commit amortization metric).
    #[must_use]
    pub fn wal_sync_count(&self) -> u64 {
        self.wal_sync_count.load(Ordering::Relaxed)
    }

    /// Share `ConcurrentDb`'s OCC snapshot registry so GC floors honor open
    /// transactions (F201). Called once at `ConcurrentDb` construction.
    pub(crate) fn set_occ_floor_registry(
        &mut self,
        registry: Arc<Mutex<std::collections::BTreeMap<u64, SequenceNumber>>>,
    ) {
        self.occ_floor_registry = Some(registry);
    }

    /// Oldest live OCC snapshot bound (`None` when no transaction is open or
    /// this `Db` has no registry). F201: reclaim/auto-compact floors take
    /// `min(pin floor, this)` — a fold with GC must not drop a version an
    /// open transaction can still read.
    fn occ_registry_floor(&self) -> Option<SequenceNumber> {
        self.occ_floor_registry
            .as_ref()?
            .lock()
            .values()
            .copied()
            .min()
    }

    /// Whether this handle refused further writes after a failed required WAL sync.
    ///
    /// Cleared only by constructing a new `Db` (reopen).
    #[must_use]
    pub fn is_durability_fenced(&self) -> bool {
        self.durability_fenced
    }

    /// F196: fence from the off-lock persist path after a post-commit
    /// (`CURRENT` swung) error — the caller cannot undo past the commit
    /// point, so writes must stop instead.
    pub(crate) fn fence_durability_post_commit(&mut self, io_error: &dyn std::fmt::Display) {
        self.fence_durability(io_error, FenceClass::Unknown);
    }

    /// Directory this DB was opened on.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// Latest sequence that has been written (0 if empty).
    #[must_use]
    pub fn last_sequence(&self) -> SequenceNumber {
        self.next_seq.saturating_sub(1)
    }

    /// RFC-0078: admit a “this `fdatasync` Ok proves the drive” claim.
    ///
    /// Always false. POSIX rc==0 (RFC-0073) is the product barrier, not a
    /// media theorem. AS-IS
    /// [`crate::group_commit_kernel::media_durable_admitted_as_is`] would
    /// admit after a successful sync.
    #[must_use]
    pub fn claim_media_durable(&self) -> bool {
        let _ = self.last_sequence();
        crate::group_commit_kernel::media_durable_admitted(true)
    }

    /// RFC-0155: admit a “zero remaining glue” claim.
    ///
    /// Always false. SST CRC fate is cataloged; handler glue stays TCB
    /// (`R-glue`). AS-IS [`crate::sst::zero_glue_admitted_as_is`] would
    /// admit after a successful put.
    #[must_use]
    pub fn claim_zero_glue(&self) -> bool {
        let _ = self.last_sequence();
        crate::sst::zero_glue_admitted()
    }

    /// Latest sequence default reads may observe (durable or no-sync apply).
    #[must_use]
    pub fn visible_sequence(&self) -> SequenceNumber {
        self.published_seq.load(Ordering::Acquire)
    }

    /// Shared point-cache handle (ConcurrentDb hit path, no Db read lock).
    #[must_use]
    pub fn point_cache_handle(&self) -> Arc<PointCache> {
        Arc::clone(&self.point_cache)
    }

    /// Shared per-CF SST envelope list (RFC-0164 miss fast path).
    #[must_use]
    pub fn sst_envelope_handle(&self) -> Arc<RwLock<Vec<(Bytes, Bytes)>>> {
        Arc::clone(&self.sst_envelope)
    }

    /// Shared settled-SSTs-only flag (RFC-0164 miss fast path gate).
    #[must_use]
    pub fn settled_sst_only_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.settled_sst_only)
    }

    /// Shared count-cache handle (ConcurrentDb `deps_scan` hit, no Db lock).
    #[must_use]
    pub fn count_cache_handle(&self) -> Arc<crate::cache::CountCache> {
        Arc::clone(&self.count_cache)
    }

    /// Shared invalidate epoch for TLS last-count / last-get (RFC-0041).
    #[must_use]
    pub fn read_cache_epoch_handle(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.read_cache_epoch)
    }

    /// Fat-apply epoch for compat TLS last-get (RFC-0154 P1.5).
    #[must_use]
    pub fn point_tls_epoch_handle(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.point_tls_epoch)
    }

    /// Per-key TLS generation map (RFC-0154 P1.5).
    #[must_use]
    pub(crate) fn key_gen_handle(&self) -> Arc<KeyGenMap> {
        Arc::clone(&self.key_gen)
    }

    /// Published sequence handle (ConcurrentDb OCC begin, no Db lock).
    #[must_use]
    pub fn published_seq_handle(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.published_seq)
    }

    /// RFC-0045 P0.1: shared write-phase timings when
    /// `PEDRA_WRITE_PHASE_STATS=1` was set at open (`None` otherwise).
    #[must_use]
    pub fn write_phase_stats(&self) -> Option<Arc<WritePhaseStats>> {
        self.phase_stats.clone()
    }

    /// Fold the unsorted memtable tail into the BTree (no SST I/O).
    ///
    /// RFC-0054: compat CFs share one memtable; raftdb is a separate Rocks
    /// instance. Folding the tail between shapes restores an empty insert
    /// path for the next CF without a flush. No-op if the tail is empty.
    pub fn fold_active_tail(&mut self) {
        if self.mem.has_tail() {
            self.mem.spill_tail();
        }
    }

    /// Unsorted tail length (RFC-0054 probe).
    #[must_use]
    pub fn mem_tail_len(&self) -> usize {
        self.mem.tail_len()
    }

    /// RFC-0047 P0.2: what a [`WalRecovery::PointInTime`] open discarded
    /// (`None` = nothing discarded / [`WalRecovery::FailClosed`] mode).
    #[must_use]
    pub fn last_recovery_report(&self) -> Option<&RecoveryReport> {
        self.last_recovery.as_ref()
    }

    /// Publish `seq` as visible and drop read caches (after WAL is durable).
    pub(crate) fn publish_sequence(&self, seq: SequenceNumber) {
        // F198: drop stale cached answers BEFORE `seq` becomes visible —
        // the point-cache OCC double-check (`get_at` with snap == published)
        // accepts any hit while `published` is unchanged, so invalidating
        // after the CAS leaves a window that serves the pre-write value.
        self.invalidate_read_answers(seq);
        let mut cur = self.published_seq.load(Ordering::Relaxed);
        while seq > cur {
            match self.published_seq.compare_exchange_weak(
                cur,
                seq,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => cur = actual,
            }
        }
        // RFC-0046 P0.1: sample (seq, time) every 32 publishes while a window
        // horizon is active — the cutoff only needs ±32-seq granularity.
        if !matches!(self.history.horizon, HistoryHorizon::All) {
            if self.seq_time_counter.fetch_add(1, Ordering::Relaxed) % 32 == 0 {
                let t = self.env.unix_millis();
                let mut ring = self.seq_times.lock();
                ring.push_back((seq, t));
                // Keep 2× the horizon of samples, and a hard count cap:
                // the time rule alone bounds the *span*, not the *count* —
                // a long window (default 24 h) under sustained writes would
                // otherwise grow the ring without bound (one sample per 32
                // publishes). Dropping oldest only coarsens the cutoff
                // (it lags), never advances it — retention stays safe.
                if let HistoryHorizon::Window(d) = self.history.horizon {
                    let keep = t.saturating_sub(2 * d.as_millis() as u64);
                    while ring.len() > 2 && ring.front().is_some_and(|&(_, t0)| t0 < keep) {
                        ring.pop_front();
                    }
                }
                while ring.len() > HORIZON_SAMPLE_RING_CAP {
                    ring.pop_front();
                }
            }
        }
    }

    fn note_dirty_points(&self, ops: &[WriteOp]) {
        if ops.len() >= 32 || ops.iter().any(|op| op.kind == ValueType::RangeDeletion) {
            // Fat apply / range: gen-bump at publish. Do not clone 64 keys
            // under the write lock just to discard them (RFC-0041 apply_mc4).
            self.point_cache_reset.store(true, Ordering::Relaxed);
            self.dirty_points.lock().clear();
            return;
        }
        // RFC-0062 P0.4: 16-insert raftlog on a write-only cache (empty
        // point+count maps) cloned every user key into `dirty_points` then
        // discarded at publish (`record_dirty` short-circuits on empty
        // map). The write lock excludes readers, so empty now stays empty
        // until we return. 1-key puts skip the is_empty peek.
        if ops.len() >= 16 && self.point_cache.is_empty() && self.count_cache.is_empty() {
            return;
        }
        let mut g = self.dirty_points.lock();
        g.extend(ops.iter().map(|op| op.key.clone()));
    }

    /// Zero RFC-0035 latest/scan counters, block-cache stats, and the
    /// thread-local decode counter.
    pub fn reset_read_probe(&self) {
        self.latest_ops.store(0, Ordering::Relaxed);
        self.latest_mem_hit.store(0, Ordering::Relaxed);
        self.latest_sst_fallback.store(0, Ordering::Relaxed);
        self.latest_sst_probed.store(0, Ordering::Relaxed);
        self.scan_ops.store(0, Ordering::Relaxed);
        self.scan_sst_probed.store(0, Ordering::Relaxed);
        self.get_mem_hit.store(0, Ordering::Relaxed);
        self.get_sst_fallback.store(0, Ordering::Relaxed);
        self.lookup_sst_considered.store(0, Ordering::Relaxed);
        self.get_inline.store(0, Ordering::Relaxed);
        self.get_vlog.store(0, Ordering::Relaxed);
        self.mvcc_split_ops.store(0, Ordering::Relaxed);
        self.mvcc_ns_encode.store(0, Ordering::Relaxed);
        self.mvcc_ns_last.store(0, Ordering::Relaxed);
        self.mvcc_ns_get.store(0, Ordering::Relaxed);
        self.mvcc_ns_copy.store(0, Ordering::Relaxed);
        self.block_cache.reset_stats();
        crate::sst::reset_sst_blocks_decoded();
        crate::sst::reset_sst_block_crc_skipped();
    }

    /// Snapshot of latest/scan counters + LSM shape (RFC-0035 P0).
    #[must_use]
    pub fn read_probe(&self) -> ReadProbeSnap {
        ReadProbeSnap {
            latest_ops: self.latest_ops.load(Ordering::Relaxed),
            latest_mem_hit: self.latest_mem_hit.load(Ordering::Relaxed),
            latest_sst_fallback: self.latest_sst_fallback.load(Ordering::Relaxed),
            latest_sst_probed: self.latest_sst_probed.load(Ordering::Relaxed),
            scan_ops: self.scan_ops.load(Ordering::Relaxed),
            scan_sst_probed: self.scan_sst_probed.load(Ordering::Relaxed),
            sst_count: self.ssts.len(),
            l0_files: self.level_file_count(0),
            level1_files: self.level_file_count(1),
            mem_entries: self.mem.len(),
            block_cache_hits: self.block_cache.hits(),
            block_cache_misses: self.block_cache.misses(),
            blocks_decoded: crate::sst::sst_blocks_decoded() as u64,
            blocks_crc_skipped: crate::sst::sst_block_crc_skipped() as u64,
            get_mem_hit: self.get_mem_hit.load(Ordering::Relaxed),
            get_sst_fallback: self.get_sst_fallback.load(Ordering::Relaxed),
            get_inline: self.get_inline.load(Ordering::Relaxed),
            get_vlog: self.get_vlog.load(Ordering::Relaxed),
            mvcc_split_ops: self.mvcc_split_ops.load(Ordering::Relaxed),
            mvcc_ns_encode: self.mvcc_ns_encode.load(Ordering::Relaxed),
            mvcc_ns_last: self.mvcc_ns_last.load(Ordering::Relaxed),
            mvcc_ns_get: self.mvcc_ns_get.load(Ordering::Relaxed),
            mvcc_ns_copy: self.mvcc_ns_copy.load(Ordering::Relaxed),
        }
    }

    /// RFC-0035 P1.2: accumulate intra-lock MVCC split (encode / last / get / copy).
    pub fn record_mvcc_split(&self, ns_encode: u64, ns_last: u64, ns_get: u64, ns_copy: u64) {
        self.mvcc_split_ops.fetch_add(1, Ordering::Relaxed);
        self.mvcc_ns_encode.fetch_add(ns_encode, Ordering::Relaxed);
        self.mvcc_ns_last.fetch_add(ns_last, Ordering::Relaxed);
        self.mvcc_ns_get.fetch_add(ns_get, Ordering::Relaxed);
        self.mvcc_ns_copy.fetch_add(ns_copy, Ordering::Relaxed);
    }

    /// Number of SST files currently loaded.
    #[must_use]
    pub fn sst_count(&self) -> usize {
        self.ssts.len()
    }

    /// Persist CHANGELOG at most every `n` durable commits (RFC-0031).
    ///
    /// `0` disables the commit-path store (flush / close / checkpoint still
    /// persist the cache). Does not change WAL fsync-before-Ok.
    pub fn set_changelog_interval(&mut self, n: u64) -> &mut Self {
        self.changelog_interval = n;
        self
    }

    /// Configured CHANGELOG store interval (RFC-0031).
    #[must_use]
    pub fn changelog_interval(&self) -> u64 {
        self.changelog_interval
    }

    /// Lazy-feed rebuild budget in entries (see
    /// [`crate::changelog_kernel::changelog_rebuild_within_budget`]). Above it
    /// the explicit flush / checkpoint / close store leaves the CHANGELOG
    /// cache stale instead of materializing the live set.
    pub fn set_changelog_rebuild_budget_entries(&mut self, n: u64) -> &mut Self {
        self.changelog_rebuild_budget_entries = n;
        self
    }

    /// Merged compaction output file split target in bytes (Rocks
    /// `target_file_size_base` role). The writer buffers one output file
    /// in memory, so this bounds compaction's peak RAM.
    pub fn set_compact_target_file_bytes(&mut self, bytes: u64) {
        self.compact_target_file_bytes = bytes.max(1);
    }

    /// Successful CHANGELOG cache stores since open.
    #[must_use]
    pub fn changelog_store_count(&self) -> u64 {
        self.changelog_store_count
    }

    /// Force a CHANGELOG cache persist (flush / close / checkpoint / WAL rotate).
    ///
    /// Best-effort: a store error is logged and never surfaces as commit failure
    /// (RFC-0019: on-disk CHANGELOG is a cache rebuilt from WAL).
    fn persist_changelog_best_effort(&mut self) {
        // Latched bulk: CHANGELOG is a cache. Collecting the live set is
        // O(n) RAM (RFC-0039). Skip until BulkRun drains on settle.
        if !self.bulk_runs.is_empty() || !self.parked_bulk.is_empty() {
            return;
        }
        if self.feed_is_lazy() && self.change_log.max_sequence().unwrap_or(0) < self.last_sequence()
        {
            // Best-effort fill (F1): a corrupt payload keeps the cache stale;
            // the feed is still rebuildable from WAL on reopen.
            // Bounded (RFC-0039 P0.3 / RFC-0041 P1.1): materializing the
            // live set costs ~3 live-set copies; above the budget the cache
            // stays stale and readers rebuild on demand.
            if crate::changelog_kernel::changelog_rebuild_within_budget(
                self.live_entry_estimate(),
                self.changelog_rebuild_budget_entries,
            ) {
                if let Ok(entries) = self.collect_feed_from_live() {
                    if !entries.is_empty() {
                        self.change_log.replace_sorted(entries);
                    }
                }
            }
        }
        match self.change_log.store_on(&self.env, &self.dir) {
            Ok(()) => {
                self.changelog_store_count = self.changelog_store_count.saturating_add(1);
                self.commits_since_changelog = 0;
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "CHANGELOG store failed; feed rebuilt on open"
                );
            }
        }
    }

    /// Debounced persist after a durable (synced) commit (RFC-0031 P0.1).
    fn maybe_persist_changelog_after_durable_commit(&mut self) {
        self.commits_since_changelog = self.commits_since_changelog.saturating_add(1);
        if changelog_should_store(self.commits_since_changelog, self.changelog_interval) {
            self.persist_changelog_best_effort();
        }
    }

    /// Explicit-flush tail (`ConcurrentDb::flush`): persist the CHANGELOG
    /// cache even when the debounce interval is 0 — the flush rotated the
    /// WAL, so reopen cannot rebuild the flushed keys from it (mirrors the
    /// `Db::flush` tail).
    pub(crate) fn persist_changelog_after_explicit_flush(&mut self) {
        if self.changelog_interval == 0 {
            self.persist_changelog_best_effort();
        }
    }

    /// LSM level of each live SST (parallel to inventory order).
    #[must_use]
    pub fn sst_levels(&self) -> &[u32] {
        &self.sst_levels
    }

    /// Highest level that currently holds ≥1 SST (`0` if empty or only L0).
    #[must_use]
    pub fn max_level(&self) -> u32 {
        self.sst_levels.iter().copied().max().unwrap_or(0)
    }

    /// Count of SSTs at `level`.
    #[must_use]
    pub fn level_file_count(&self, level: u32) -> usize {
        self.sst_levels.iter().filter(|&&l| l == level).count()
    }

    /// Register CF names so flush/compact split by family (RFC-0065). Empty
    /// keeps the kernel one-LSM behaviour (keys with accidental NULs stay
    /// in one SST forest).
    pub fn set_physical_cfs(&mut self, names: Vec<String>) {
        self.physical_cfs = names;
    }

    /// Per-CF memtable flush threshold (RFC-0065 P1.1). `0` removes the override.
    pub fn set_cf_write_buffer(&mut self, cf: impl Into<String>, bytes: usize) {
        let cf = cf.into();
        if bytes == 0 {
            self.cf_write_buffer.remove(&cf);
        } else {
            self.cf_write_buffer.insert(cf, bytes);
        }
    }

    fn write_buffer_for(&self, family: &str) -> Option<usize> {
        self.cf_write_buffer
            .get(family)
            .copied()
            .filter(|n| *n > 0)
            .or(self.auto_flush_bytes)
    }

    /// L0 files tagged with `cf` (empty physical set = global L0).
    #[must_use]
    pub fn level_file_count_cf(&self, cf: &str) -> usize {
        if self.physical_cfs.is_empty() {
            return self.level_file_count(0);
        }
        self.ssts
            .iter()
            .zip(self.sst_levels.iter())
            .filter(|(t, &lvl)| lvl == 0 && t.cf() == cf)
            .count()
    }

    fn family_of_user_key<'a>(&'a self, key: &[u8]) -> &'a str {
        if self.physical_cfs.is_empty() {
            return "default";
        }
        let p = crate::memtable::cf_prefix(key);
        if p.is_empty() {
            return "default";
        }
        self.physical_cfs
            .iter()
            .find(|n| n.as_bytes() == p)
            .map(String::as_str)
            .unwrap_or("default")
    }

    fn batch_families(&self, ops: &[BatchOp]) -> Vec<String> {
        if self.physical_cfs.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(4);
        for op in ops {
            match op {
                BatchOp::Put { key, .. } | BatchOp::Delete { key } => {
                    let f = self.family_of_user_key(key.as_ref());
                    if !out.iter().any(|s| s == f) {
                        out.push(f.to_string());
                    }
                }
                BatchOp::DeleteRange { start, end } => {
                    for k in [start.as_ref(), end.as_ref()] {
                        let f = self.family_of_user_key(k);
                        if !out.iter().any(|s| s == f) {
                            out.push(f.to_string());
                        }
                    }
                }
            }
        }
        out
    }

    /// Stall knobs off (parity default): collecting CF families is unused.
    /// RFC-0149 P2.1: `batch_families` used to `to_string()` on every 1c put.
    fn write_admission_idle(&self) -> bool {
        self.write_stall_mem_bytes.is_none()
            && self.write_pressure_l0.is_none()
            && self.write_stall_l0.is_none()
    }

    /// Whether flush/compact split by CF family.
    #[must_use]
    pub fn physical_cfs(&self) -> &[String] {
        &self.physical_cfs
    }

    /// Compact grouping key: all files share `""` until CFs are registered.
    fn compact_family_key<'a>(&self, table: &'a SstTable) -> &'a str {
        if self.physical_cfs.is_empty() {
            ""
        } else {
            table.cf()
        }
    }

    /// Shared table cache (open reuse + hit stats).
    #[must_use]
    pub fn table_cache(&self) -> &TableCache {
        &self.table_cache
    }

    /// Shared block cache (hit stats).
    #[must_use]
    pub fn block_cache(&self) -> &BlockCache {
        &self.block_cache
    }

    /// Replace the SST block cache (RFC-0153). Empty — call before serving
    /// reads. `None` restores the 8192-entry default; `Some(n)` is a byte budget.
    pub fn install_block_cache(&mut self, cache: BlockCache) {
        self.block_cache = cache;
    }

    /// Attach + register a newly installed table's payload with the pool
    /// (RFC-0042 v18). No-op on a legacy open (no source) or a v1/eager
    /// table. Call at every point a table enters `self.ssts`.
    fn adopt_sst(&self, table: &SstTable) {
        if let Some(src) = &self.sst_source {
            table.attach_payload_kit(src, &self.sst_payload_pool);
        }
    }

    /// Shared payload pool (observability).
    #[must_use]
    pub fn sst_payload_pool(&self) -> &crate::cache::SstPayloadPool {
        &self.sst_payload_pool
    }

    /// Total entries held in per-table decoded-entries caches across all
    /// installed SSTs (observability — the caches are unbounded per table).
    #[must_use]
    pub fn sst_cached_entries(&self) -> usize {
        self.ssts.iter().map(SstTable::cached_entries_count).sum()
    }

    /// Capture a read snapshot of currently committed state (sequence export).
    ///
    /// Does **not** register a pin — use [`Self::pin_snapshot`] when you need
    /// [`Self::compact_reclaim`] to preserve history for this sequence.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            seq: self.visible_sequence(),
        }
    }

    /// Register a read pin at the current last sequence (open-items §2.1).
    ///
    /// [`Self::compact_reclaim`] will not drop versions still required for this
    /// pin. Call [`Self::release_snapshot_pin`] when done.
    pub fn pin_snapshot(&mut self) -> SnapshotPin {
        let seq = self.visible_sequence();
        let id = self.next_snapshot_pin_id;
        self.next_snapshot_pin_id = id.saturating_add(1);
        self.snapshot_pins.insert(id, seq);
        SnapshotPin { id, seq }
    }

    /// Drop a pin previously returned by [`Self::pin_snapshot`].
    ///
    /// Unknown ids are ignored (idempotent).
    pub fn release_snapshot_pin(&mut self, pin: SnapshotPin) {
        self.snapshot_pins.remove(&pin.id);
    }

    /// Minimum sequence among open pins, if any.
    #[must_use]
    pub fn oldest_pinned_sequence(&self) -> Option<SequenceNumber> {
        self.snapshot_pins.values().copied().min()
    }

    /// Number of open snapshot pins (observability / tests).
    #[must_use]
    pub fn snapshot_pin_count(&self) -> usize {
        self.snapshot_pins.len()
    }

    /// Lowest sequence still guaranteed readable after version GC (0 = no floor).
    #[must_use]
    pub fn earliest_readable_sequence(&self) -> SequenceNumber {
        self.earliest_readable_seq
    }

    /// Fail closed when `snap` is below the version-GC watermark.
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`] when history for `snap` may have been dropped.
    pub fn ensure_snapshot_readable(&self, snap: Snapshot) -> Result<()> {
        if snap.seq < self.earliest_readable_seq {
            return Err(CoreError::SnapshotTooOld {
                requested: snap.seq,
                earliest: self.earliest_readable_seq,
            });
        }
        Ok(())
    }

    /// Raise the GC watermark (monotonic). Used after history-dropping compact
    /// and after a fold that dropped versions below the snapshot-list floor
    /// (in-memory only: WAL replay restores the dropped versions on reopen).
    pub(crate) fn raise_earliest_readable(&mut self, floor: SequenceNumber) {
        if floor > self.earliest_readable_seq {
            self.earliest_readable_seq = floor;
        }
    }

    /// Versions of `user_key` across every live memtable layer (fold-GC tests).
    #[cfg(test)]
    pub(crate) fn count_mem_versions(&self, user_key: &[u8]) -> usize {
        let hit = |m: &MemTable| {
            m.iter_internal()
                .filter(|(k, _)| k.user_key.as_ref() == user_key)
                .count()
        };
        let mut n = hit(&self.mem);
        if let Some(ref imm) = self.imm {
            n += hit(imm);
        }
        if let Some(ref p) = self.flush_read_pin {
            n += hit(p);
        }
        n + self.parked_unflushed.iter().map(|m| hit(m)).sum::<usize>()
    }

    /// RFC-0046 P0.1: highest published sequence at or before
    /// `now − horizon` (`None` while the horizon is `All` or nothing has aged
    /// out yet). Versions older than this cutoff are archive-then-GC
    /// candidates on the next auto-compact.
    #[must_use]
    pub fn horizon_cutoff_sequence(&self) -> Option<SequenceNumber> {
        let d = match self.history.horizon {
            HistoryHorizon::All => return None,
            HistoryHorizon::Window(d) => d,
        };
        let target = self.env.unix_millis().saturating_sub(d.as_millis() as u64);
        let mut ring = self.seq_times.lock();
        let mut cut: SequenceNumber = 0;
        for &(seq, t) in ring.iter() {
            if t <= target {
                cut = cut.max(seq);
            }
        }
        // Shed consumed samples (strictly below the returned cutoff): they
        // can never raise the cutoff again, and the cutoff is monotone —
        // losing them after a clock regression only makes the horizon lag
        // (keeps more history), never GC early.
        if cut > 0 {
            while ring.front().is_some_and(|&(s, _)| s < cut) {
                ring.pop_front();
            }
        }
        (cut > 0).then_some(cut)
    }

    /// GC decision for auto-compact (RFC-0046 P0.1): `None` =
    /// history-preserving merge this round. Two sources, two profiles:
    /// `auto_reclaim` (operator opt-in, latest-only — the Rocks storage
    /// profile on the compat face) drops to the pin floor **without
    /// archiving** (a reclaim keeps nothing); otherwise the window horizon
    /// bounds it — the product default (`Window(24 h)`) GCs only what has
    /// aged out, pin-aware, and archives what leaves (P0.2). Returns
    /// `(floor, archive_first)`.
    fn auto_gc_floor(&self) -> Option<(SequenceNumber, bool)> {
        // F211: with no pins the floor is capped at the published sequence —
        // `last_sequence()` counts applied-but-unpublished writes (write-group
        // off-lock window) and would push `earliest_readable_seq` above
        // `published_seq`, failing visible-snapshot reads until publish.
        // `for_oldest_snapshot` GC keeps every version newer than the floor,
        // so the in-flight version is retained.
        let pin_or_last = crate::compact_kernel::gc_oldest_from_pin(
            self.oldest_pinned_sequence(),
            self.last_sequence(),
            self.visible_sequence(),
        );
        // F201: an open OCC transaction holds the floor even without a pin.
        let pin_or_last = match self.occ_registry_floor() {
            Some(occ) => pin_or_last.min(occ),
            None => pin_or_last,
        };
        if self.auto_reclaim {
            return Some((pin_or_last, false));
        }
        self.horizon_cutoff_sequence()
            .map(|cutoff| (pin_or_last.min(cutoff), true))
    }

    /// RFC-0046 P0.2: archive every local version with `seq < floor` —
    /// exactly the set the following GC compact may drop — then enforce the
    /// cap. Fail-closed: on error the caller skips GC; history is never
    /// dropped unarchived. Known v0 wart: a GC round re-archives versions
    /// whose SST this round's compaction does not rewrite (duplicates are
    /// harmless for replay and age out at the cap; per-segment coverage
    /// tracking is the P1 follow-up).
    fn archive_history_below(&mut self, floor: SequenceNumber) -> Result<()> {
        const CHUNK: usize = 4096;
        let mut chunk: Vec<(Vec<u8>, Vec<u8>, u64, u8)> = Vec::with_capacity(CHUNK);
        {
            for (ik, v) in self.mem.iter_internal() {
                self.archive_note(&mut chunk, floor, &ik, &v)?;
            }
            if let Some(ref imm) = self.imm {
                for (ik, v) in imm.iter_internal() {
                    self.archive_note(&mut chunk, floor, &ik, &v)?;
                }
            }
        }
        self.archive_flush_chunk(&mut chunk)?;
        let sst_count = self.ssts.len();
        for i in 0..sst_count {
            {
                let mut stream = self.ssts[i].iter_internal_streaming();
                loop {
                    match stream.next_entry() {
                        Ok(Some((ik, v))) => self.archive_note(&mut chunk, floor, &ik, &v)?,
                        Ok(None) => break,
                        // F176: a stream error must abort the archive — the
                        // GC compact that follows this call drops everything
                        // below `floor`; treating the error as a clean EOF
                        // destroys the un-archived remainder (violates the
                        // fail-closed contract in this method's doc).
                        Err(e) => return Err(e),
                    }
                }
            }
            if chunk.len() >= CHUNK {
                self.archive_flush_chunk(&mut chunk)?;
            }
        }
        self.archive_flush_chunk(&mut chunk)?;
        // Upload pass (P1.2): ship every sealed segment + the manifest
        // generation BEFORE the cap may drop anything — the cap then only
        // reclaims segments verified present at the remote tier.
        self.upload_history_step()?;
        // Cap: drop oldest segments; advance the readable floor (typed
        // SnapshotTooOld below it). A pin holds everything at/below it.
        let pin = self.oldest_pinned_sequence();
        let cap = self.history.cap_bytes;
        let uploaded = if self.remote_history.is_some() {
            Some(self.uploaded_history_segs.clone())
        } else {
            None
        };
        let env = self.env.clone();
        let tier = self
            .history_tier
            .as_mut()
            .expect("history tier opened at open()");
        let archive_floor = tier.enforce_cap(&env, pin, cap, uploaded.as_ref())?;
        self.raise_earliest_readable(archive_floor);
        Ok(())
    }

    /// RFC-0046 P1.2: opt in to mirroring the history tier to a remote
    /// (object-storage-shaped) destination reached through any `Env`.
    /// Uploads then run inline on the auto-compact path after each archive
    /// pass, before the cap may reclaim anything: while the destination is
    /// unreachable, GC **pauses** (history-preserving) and the local cap
    /// holds un-uploaded segments — backpressure never destroys what has
    /// not shipped. Puts are idempotent, so retries/resume are free.
    pub fn set_remote_history(&mut self, env: E, root: impl Into<PathBuf>) {
        self.remote_history = Some(RemoteHistory {
            env,
            tier: crate::history::RemoteTier::new(root),
        });
    }

    /// Run one remote upload pass now (RFC-0046 P1.2). Returns the report;
    /// no-op (empty report) when no remote tier is configured.
    ///
    /// # Errors
    /// Remote I/O (fail-closed: caller decides; the auto-compact path
    /// reacts by pausing GC).
    pub fn upload_history_now(&mut self) -> Result<crate::history::UploadReport> {
        self.upload_history_step()
    }

    fn upload_history_step(&mut self) -> Result<crate::history::UploadReport> {
        let mut report = crate::history::UploadReport::default();
        let Some(remote) = self.remote_history.as_ref() else {
            return Ok(report);
        };
        let (remote_env, tier_root) = (remote.env.clone(), remote.tier.clone());
        let budget = self.upload_bandwidth;
        let metas = self
            .history_tier
            .as_ref()
            .map(|t| t.segment_metas())
            .unwrap_or_default();
        let mut shipped: u64 = 0;
        for m in metas {
            // P2.2 bandwidth limiter: once this round's byte budget is
            // spent, stop — and ship no manifest either, so the remote
            // never lists a segment it does not hold. The next step
            // resumes from the un-uploaded ids (puts are idempotent).
            if budget.is_some_and(|b| shipped >= b) {
                return Ok(report);
            }
            let path = crate::history::HistoryTier::segment_path(&self.dir, m.id);
            match tier_root.put_segment(&remote_env, &self.env, &path)? {
                crate::history::PutStatus::Uploaded => {
                    report.segments_uploaded += 1;
                    shipped += m.bytes;
                }
                crate::history::PutStatus::AlreadyPresent => report.segments_already_present += 1,
            }
            self.uploaded_history_segs.insert(m.id);
        }
        if let Some(tier) = self.history_tier.as_ref() {
            let bytes = tier.manifest_bytes();
            let generation = tier.remote_generation();
            report.manifest = Some(tier_root.put_manifest(&remote_env, &bytes, generation)?);
        }
        Ok(report)
    }

    /// RFC-0046 P2.2: bound how many segment bytes one upload step may ship
    /// (`None` = unlimited, the default). Un-shipped segments stay pending
    /// (see [`Self::history_stats`]) and the local cap holds them — the
    /// documented backpressure tradeoff: bounded upload bandwidth is paid
    /// for with local disk while the backlog drains.
    pub fn set_upload_bandwidth(&mut self, bytes_per_round: Option<u64>) {
        self.upload_bandwidth = bytes_per_round;
    }

    /// RFC-0046 P2.8: bound the in-memory read cache for remote segments
    /// (bytes of verified-decoded records; 64 MiB by default). `0`
    /// disables it — every below-watermark read of a remote-only segment
    /// then refetches the object and CRC-walks it again. Cached entries
    /// are trusted by name (content-addressed): they are verified once,
    /// on insert.
    pub fn set_remote_read_cache(&mut self, bytes: u64) {
        self.remote_read_cache.lock().set_budget(bytes);
    }

    /// RFC-0046 P2.2: local tier, remote mirror, upload backlog and archive
    /// age in one roll-up.
    ///
    /// # Errors
    /// Remote I/O when a mirror is configured (the summary reads the
    /// destination's manifest — fail-closed, not a silent `None`).
    pub fn history_stats(&self) -> Result<crate::history::HistoryStats> {
        let mut stats = crate::history::HistoryStats {
            earliest_readable: self.earliest_readable_seq,
            ..Default::default()
        };
        if let Some(tier) = self.history_tier.as_ref() {
            stats.local_segments = tier.segment_metas().len();
            stats.local_bytes = tier.bytes();
            stats.archive_floor = tier.archive_floor();
        }
        if self.remote_history.is_some() {
            stats.pending_uploads = self
                .history_tier
                .as_ref()
                .map(|t| t.segment_ids())
                .unwrap_or_default()
                .into_iter()
                .filter(|id| !self.uploaded_history_segs.contains(id))
                .count();
        }
        if let Some(remote) = self.remote_history.as_ref() {
            stats.remote = remote.tier.latest_summary(&remote.env)?;
        }
        stats.last_archive_age_millis = self
            .last_archive_millis
            .map(|t| self.env.unix_millis().saturating_sub(t));
        stats.seq_time_samples = self.seq_times.lock().len();
        let cache = self.remote_read_cache.lock();
        stats.remote_cache_entries = cache.map.len();
        stats.remote_cache_bytes = cache.used;
        Ok(stats)
    }

    fn archive_note(
        &self,
        chunk: &mut Vec<(Vec<u8>, Vec<u8>, u64, u8)>,
        floor: SequenceNumber,
        ik: &InternalKey,
        stored: &Bytes,
    ) -> Result<()> {
        if ik.sequence >= floor {
            return Ok(());
        }
        // F176: a failed vlog resolve must abort the archive — falling back
        // to the raw stored bytes would archive the vlog POINTER as the
        // value, and the GC below would then destroy the only good copy
        // (replay serves pointer bytes as the value).
        let val = self.resolve_stored_value(stored.clone())?;
        let kind = match ik.kind {
            ValueType::Value => 0,
            ValueType::Deletion => 1,
            ValueType::RangeDeletion => 2,
        };
        chunk.push((ik.user_key.to_vec(), val.to_vec(), ik.sequence, kind));
        Ok(())
    }

    fn archive_flush_chunk(&mut self, chunk: &mut Vec<(Vec<u8>, Vec<u8>, u64, u8)>) -> Result<()> {
        if chunk.is_empty() {
            return Ok(());
        }
        let env = self.env.clone();
        let tier = self
            .history_tier
            .as_mut()
            .expect("history tier opened at open()");
        tier.archive_stream(&env, chunk.drain(..))?;
        self.last_archive_millis = Some(env.unix_millis());
        Ok(())
    }

    /// After a compact that ran version GC, advance the too-old watermark.
    fn note_version_gc_watermark(&mut self, gc: crate::merge::CompactGcOptions) {
        if let Some(oldest) = gc.oldest_snapshot {
            self.raise_earliest_readable(oldest);
        } else if gc.keep_only_latest {
            // Only current versions remain — anything below last_seq may miss history.
            self.raise_earliest_readable(self.last_sequence());
        } else if gc.min_sequence > 0 {
            self.raise_earliest_readable(gc.min_sequence);
        }
    }

    /// Point lookup at the latest committed sequence (MemTable ∪ SSTs).
    ///
    /// Fail-stop on a corrupt value log (F1): the `Option` shape cannot express
    /// the error, and returning `None` would make corruption indistinguishable
    /// from deletion. Use [`Self::get_at`] for an error-shaped read.
    ///
    /// # Panics
    /// If the stored value is a vlog reference whose payload fails CRC/I-O.
    #[must_use]
    pub fn get(&self, key: &[u8]) -> Option<Bytes> {
        // Latest snapshot is always ≥ watermark when watermark is raised from
        // last_sequence / pin floor after GC; fall back to None only on fence.
        if let Some(cached) = self.point_cache.get(key) {
            return cached;
        }
        self.get_after_point_miss(key)
    }

    /// SST lookup + optional point-cache fill. [`ConcurrentDb::get`] already
    /// probed the shared cache; calling [`Self::get`] again would take the
    /// cache mutex a second time on every uniform miss (lookup_100).
    pub(crate) fn get_after_point_miss(&self, key: &[u8]) -> Option<Bytes> {
        let snap = self.snapshot();
        if snap.seq == 0 {
            return None;
        }
        if snap.seq < self.earliest_readable_seq {
            return match self.get_at(snap, key) {
                Ok(v) => v,
                Err(CoreError::SnapshotTooOld { .. }) => None,
                Err(e) => fail_stop_corrupt_value(&format!("get key {key:?}"), &e),
            };
        }
        // Already cache-missed. Do not re-lock `point_cache` inside `get_at`.
        let got =
            match self.lookup(key, snap.seq) {
                Lookup::Found(v) => {
                    if vlog::decode_vlog_ptr(v.as_ref()).is_some() {
                        self.get_vlog.fetch_add(1, Ordering::Relaxed);
                    } else {
                        self.get_inline.fetch_add(1, Ordering::Relaxed);
                    }
                    Some(self.resolve_stored_value(v).unwrap_or_else(|e| {
                        fail_stop_corrupt_value(&format!("get key {key:?}"), &e)
                    }))
                }
                Lookup::Deleted | Lookup::NotFound => None,
            };
        // Do not cache absence: unique probe_miss keys would freeze 8192
        // with Nones and take the mutex on every subsequent unique miss.
        if got.is_some() && self.published_seq.load(Ordering::Acquire) == snap.seq {
            self.point_cache.insert(key, got.clone());
        }
        got
    }

    /// Point lookup at an explicit [`Snapshot`].
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`] if history for `snap` was dropped and
    /// no tier copy can decide the key (RFC-0046 P2.1: below-watermark
    /// point reads fall back to the history tier — local segments first,
    /// remote mirror second — and only fail when that history is gone).
    pub fn get_at(&self, snap: Snapshot, key: &[u8]) -> Result<Option<Bytes>> {
        if snap.seq == 0 {
            return Ok(None);
        }
        if snap.seq < self.earliest_readable_seq {
            // RFC-0046 P2.3: the global watermark advances with the cap and
            // the reported GC floor, not per key — a version that survived
            // the rewrite (single-version keys are never dropped) can sit
            // below it while the archive already dropped its segment. When
            // the tier cannot cover the read, fall back to the LSM and
            // serve only a physically-present decisive record.
            return match self.get_at_from_archive(snap, key) {
                Err(CoreError::SnapshotTooOld { .. }) => self.get_at_below_watermark_lsm(snap, key),
                other => other,
            };
        }
        // Snapshot == published: the point cache already answers at exactly
        // this seq (it only ever holds latest-published values; publish
        // invalidates dirty keys before new inserts can refill them).
        // Double-checked `published_seq`: a racing publish bumps it before any
        // newer value can enter the cache, so the recheck rejects the hit and
        // falls to the full walk (OCC rmw reads become cache hits).
        if self.published_seq.load(Ordering::Acquire) == snap.seq {
            if let Some(v) = self.point_cache.get(key) {
                if self.published_seq.load(Ordering::Acquire) == snap.seq {
                    return Ok(v);
                }
            }
        }
        Ok(match self.lookup(key, snap.seq) {
            Lookup::Found(v) => {
                if vlog::decode_vlog_ptr(v.as_ref()).is_some() {
                    self.get_vlog.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.get_inline.fetch_add(1, Ordering::Relaxed);
                }
                // F1: corruption surfaces as Err, never as a miss.
                Some(self.resolve_stored_value(v)?)
            }
            Lookup::Deleted | Lookup::NotFound => None,
        })
    }

    /// RFC-0046 P2.1: point read below the version-GC watermark, served
    /// lazy from the history tier (local segments first, the remote mirror
    /// second — it retains what the local cap already dropped).
    ///
    /// A decisive record (newest put/delete/range-delete at `seq ≤ snap`)
    /// answers even when coverage has gaps. A no-match answers `None` only
    /// when the retained segments provably cover `[1, snap]` with nothing
    /// dropped — otherwise fail-closed [`CoreError::SnapshotTooOld`]
    /// (never-written and dropped are indistinguishable). Read cost: the
    /// P2.5 key-coverage bound and the P2.6/P2.7 bloom sidecar prune
    /// segments (local and remote alike) before their bytes are read;
    /// only may-affect segments are fetched and CRC-walked.
    fn get_at_from_archive(&self, snap: Snapshot, key: &[u8]) -> Result<Option<Bytes>> {
        let too_old = || CoreError::SnapshotTooOld {
            requested: snap.seq,
            earliest: self.earliest_readable_seq,
        };
        let Some(tier) = self.history_tier.as_ref() else {
            return Err(too_old());
        };
        // Candidate segments: the local manifest plus (content-addressed
        // dedup by name) everything the remote manifest still lists.
        // Entries carry the P2.5 key-coverage bound (range-delete aware)
        // — segments whose bound excludes the key are skipped without a
        // walk. Remote listings carry the bound of their manifest
        // generation (v3+); a pre-P2.5 remote manifest decodes with
        // `None` (walk).
        #[allow(clippy::type_complexity)]
        let mut cands: Vec<(u64, u64, String, Option<u64>, Option<(Vec<u8>, Vec<u8>)>)> = tier
            .segment_metas()
            .into_iter()
            .map(|m| {
                (
                    m.from_seq,
                    m.through_seq,
                    m.name,
                    Some(m.id),
                    m.key_lo.zip(m.key_hi),
                )
            })
            .collect();
        if let Some(remote) = self.remote_history.as_ref() {
            for seg in remote.tier.latest_segments(&remote.env)? {
                if !cands.iter().any(|(_, _, name, _, _)| *name == seg.name) {
                    cands.push((
                        seg.from_seq,
                        seg.through_seq,
                        seg.name,
                        None,
                        seg.key_lo.zip(seg.key_hi),
                    ));
                }
            }
        }
        let mut best: Option<crate::history::HistoryRecord> = None;
        let mut missing_below_snap = false;
        for (from, _, name, local_id, coverage) in &cands {
            if *from > snap.seq {
                continue; // cannot hold a record this snapshot can see
            }
            if let Some((lo, hi)) = coverage {
                if key < lo.as_slice() || key > hi.as_slice() {
                    continue; // outside the segment's key coverage — sound skip
                }
            }
            // P2.6 bloom sidecar: skip the record walk when the segment
            // provably cannot decide this key (helps the overlapping-key
            // case the manifest bound can't prune). Coverage spans below
            // still use `cands`, so the None-proof is unaffected by skips.
            if let Some(id) = local_id {
                if !tier.segment_may_affect(&self.env, *id, key) {
                    continue;
                }
            }
            let records = match local_id.map(|id| tier.read_local_segment(&self.env, id)) {
                Some(Ok(Some(bytes))) => Some(std::sync::Arc::new(
                    crate::history::walk_segment_records(&bytes)?,
                )),
                Some(Ok(None)) | None => None,
                Some(Err(e)) => return Err(e),
            };
            let records = match records {
                Some(records) => Some(records),
                // Local copy absent: consult the remote mirror. The P2.7
                // sidecar object (KBs) prunes the segment fetch (100s of
                // KB) when the segment provably cannot decide this key —
                // any sidecar problem (absent = pre-P2.7 upload or
                // pre-P2.6 segment, unreadable, corrupt) fails open to
                // the fetch+walk, exactly like the local sidecar.
                None => {
                    let Some(remote) = self.remote_history.as_ref() else {
                        missing_below_snap = true;
                        continue;
                    };
                    if let Ok(Some(buf)) = remote.tier.read_sidecar(&remote.env, name) {
                        if !crate::history::HistoryTier::sidecar_may_affect(&buf, key) {
                            continue; // sound skip — spans below still count it
                        }
                    }
                    // P2.8 read cache: a hit skips the fetch and the walk
                    // (entries are CRC-verified on insert and the name is
                    // content-addressed — stable for the bytes). The
                    // guard is dropped at the `let` so the miss path can
                    // re-lock for the insert.
                    let cached = self.remote_read_cache.lock().get(name);
                    if let Some(records) = cached {
                        Some(records)
                    } else {
                        // A missing object is a coverage gap (fail-closed
                        // below); anything else (corrupt read-back)
                        // propagates typed.
                        match remote.tier.read_segment(&remote.env, name) {
                            Ok(bytes) => {
                                let records = std::sync::Arc::new(
                                    crate::history::walk_segment_records(&bytes)?,
                                );
                                let cost = bytes.len() as u64;
                                self.remote_read_cache
                                    .lock()
                                    .insert(name, records.clone(), cost);
                                Some(records)
                            }
                            Err(CoreError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                                missing_below_snap = true;
                                None
                            }
                            Err(e) => return Err(e),
                        }
                    }
                }
            };
            let Some(records) = records else { continue };
            if let Some(rec) = crate::history::decide_at(&records, key, snap.seq) {
                if best.as_ref().map_or(true, |b| rec.seq > b.seq) {
                    best = Some(rec.clone());
                }
            }
        }
        if let Some(rec) = best {
            return Ok(match rec.kind {
                0 => Some(Bytes::from(rec.val)),
                _ => None, // delete / range delete covering the key
            });
        }
        // No deciding record. `None` is provable only when the retained
        // segments cover [1, snap] contiguously and the cap never dropped
        // anything at/below snap (archive_floor is the drop high-water).
        if missing_below_snap || tier.archive_floor() > snap.seq {
            return Err(too_old());
        }
        let mut spans: Vec<(u64, u64)> = cands.iter().map(|(f, t, _, _, _)| (*f, *t)).collect();
        spans.sort_unstable();
        let mut covered_to = 1u64;
        for (from, through) in spans {
            if from > covered_to {
                return Err(too_old()); // gap below snap
            }
            covered_to = covered_to.max(through + 1);
            if covered_to > snap.seq {
                break;
            }
        }
        if covered_to > snap.seq {
            Ok(None)
        } else {
            Err(too_old())
        }
    }

    /// RFC-0046 P2.3: LSM leg of a below-watermark read whose archive
    /// coverage failed. Soundness: a found version (put or tombstone) at
    /// `seq ≤ snap` is physically present — serve it. Anything else keeps
    /// the tier's `SnapshotTooOld`: the LSM cannot prove a key was never
    /// written (all its versions may have been GC'd and tombstone-cleaned),
    /// so `None` here would be a silent destroy.
    fn get_at_below_watermark_lsm(&self, snap: Snapshot, key: &[u8]) -> Result<Option<Bytes>> {
        let too_old = || CoreError::SnapshotTooOld {
            requested: snap.seq,
            earliest: self.earliest_readable_seq,
        };
        match self.lookup(key, snap.seq) {
            Lookup::Found(v) => {
                if vlog::decode_vlog_ptr(v.as_ref()).is_some() {
                    self.get_vlog.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.get_inline.fetch_add(1, Ordering::Relaxed);
                }
                // F1: corruption surfaces as Err, never as a miss.
                Ok(Some(self.resolve_stored_value(v)?))
            }
            Lookup::Deleted => Ok(None),
            Lookup::NotFound => Err(too_old()),
        }
    }

    /// Whether any version of `key` has `sequence > snapshot` (OCC conflict probe).
    ///
    /// Includes point puts/deletes **and** range tombstones that cover `key`
    /// (F30: a concurrent `delete_range` that covers a read/write key must conflict).
    #[must_use]
    pub fn key_has_write_after(&self, key: &[u8], snapshot: SequenceNumber) -> bool {
        // RFC-0045 P2.1: WAL-encoded ops sit here during off-lock fsync,
        // before memtable apply. First-committer-wins must see them.
        for u in &self.unapplied {
            if u.seq <= snapshot {
                continue;
            }
            if u.kind == ValueType::RangeDeletion {
                if range_tombstone_covers(u.key.as_ref(), u.end.as_ref(), key) {
                    return true;
                }
            } else if u.key.as_ref() == key {
                return true;
            }
        }
        // Point lookup per layer (not a full memtable walk). Range tombs
        // only when the layer actually has any (OCC 1c / Surreal commit).
        for table in self.mem_layers() {
            if let Some((seq, _)) = table.get_entry(key, MAX_SEQUENCE_NUMBER) {
                if seq > snapshot {
                    return true;
                }
            }
            if table.has_range_tombstones() {
                let mut tombs = Vec::new();
                table.collect_range_tombstones(MAX_SEQUENCE_NUMBER, &mut tombs);
                for t in tombs {
                    if t.sequence > snapshot
                        && range_tombstone_covers(t.start.as_ref(), t.end.as_ref(), key)
                    {
                        return true;
                    }
                }
            }
        }
        for table in &self.ssts {
            if let Some((seq, _)) = table.point_at(key, MAX_SEQUENCE_NUMBER) {
                if seq > snapshot {
                    return true;
                }
            }
            if !table.has_range_tombstones() {
                continue;
            }
            let mut tombs = Vec::new();
            table.collect_range_tombstones(MAX_SEQUENCE_NUMBER, &mut tombs);
            for t in tombs {
                if t.sequence > snapshot
                    && range_tombstone_covers(t.start.as_ref(), t.end.as_ref(), key)
                {
                    return true;
                }
            }
        }
        false
    }

    /// Resolve vlog pointer to payload (or return inline value).
    fn resolve_stored_value(&self, stored: Bytes) -> Result<Bytes> {
        if stored.first() == Some(&INLINE_ESCAPE) {
            // F188: escaped inline value — strip the marker byte.
            return Ok(stored.slice(1..));
        }
        let Some(ptr) = vlog::decode_vlog_ptr(stored.as_ref()) else {
            return Ok(stored);
        };
        let Some(ref vlog) = self.vlog else {
            return Err(CoreError::Internal(
                "vlog ref in DB but VALUES.vlog not open".into(),
            ));
        };
        let guard = vlog.lock();
        guard.read_ptr_on(&self.env, &self.dir, ptr, self.vlog_use_new)
    }

    /// Enable blob rotation after `bytes` on the active file (RFC-0029). `None` disables.
    pub fn set_vlog_rotate_bytes(&mut self, bytes: Option<u64>) {
        self.vlog_rotate_bytes = bytes.filter(|n| *n > 0);
    }

    /// Set scan vlog prefetch window size (RFC-0029 P0.3 / P2.2).
    ///
    /// `0` or `1` = resolve one-by-one; larger windows issue `Env::advise` then
    /// resolve up to `n` pointers before advancing. Default is **4** (measured
    /// lab default; see `scan_prefetch_n_window_measure`). Cap is 64 to avoid
    /// unbounded stacks of in-flight resolve work on a single thread.
    pub fn set_scan_prefetch(&mut self, n: usize) {
        self.scan_prefetch = n.min(64);
    }

    /// Current scan prefetch window (RFC-0029).
    #[must_use]
    pub fn scan_prefetch(&self) -> usize {
        self.scan_prefetch
    }

    /// Enable or disable best-effort blob GC after flush / `latest_only` compact.
    ///
    /// When `Some(θ)`, after those paths the engine calls
    /// [`Self::compact_blob_auto`] with that θ (Titan-shaped default **0.5**).
    /// Failures are logged and do **not** fail the flush/compact. Off by default
    /// (no background thread — write-path only; RFC-0026 residual).
    pub fn set_auto_blob_gc_min_ratio(&mut self, min_dead_ratio: Option<f64>) {
        self.auto_blob_gc_min_ratio = min_dead_ratio.map(|r| r.clamp(0.0, 1.0));
    }

    /// Opt-in: auto-compact runs snapshot-safe version reclaim (open-items §2.1).
    ///
    /// When **on**, threshold auto-compact uses
    /// [`CompactGcOptions::for_oldest_snapshot`] (oldest open [`SnapshotPin`], or
    /// last sequence if none) and advances the too-old watermark. Bare
    /// [`Snapshot`] tokens without pins become [`CoreError::SnapshotTooOld`] after
    /// reclaim — use [`Self::pin_snapshot`] for long-lived reads.
    ///
    /// When **off** (default, F20): auto-compact only merges levels and keeps
    /// all versions. Explicit reclaim remains [`Self::compact_reclaim`] /
    /// [`CompactOptions::latest_only`].
    pub fn set_auto_reclaim(&mut self, enabled: bool) {
        self.auto_reclaim = enabled;
    }

    /// Whether auto-compact uses snapshot-safe reclaim GC.
    #[must_use]
    pub fn auto_reclaim(&self) -> bool {
        self.auto_reclaim
    }

    /// Skip inline auto-compact after flush (RFC-0037). Host drains L0.
    pub fn set_defer_auto_compact(&mut self, enabled: bool) {
        self.defer_auto_compact = enabled;
    }

    /// Whether flush leaves L0 for a host worker to compact.
    #[must_use]
    pub fn defer_auto_compact(&self) -> bool {
        self.defer_auto_compact
    }

    /// Opt-in write stall when L0 SST count ≥ `limit` (open-items §2.3).
    ///
    /// `None` or `0` disables (default). When enabled, [`Self::put`] /
    /// [`Self::apply_batch`] / group commit fail with
    /// [`CoreError::WriteStall`] instead of letting L0 grow unbounded.
    /// No sleep — honest signal; compact then retry (or enable
    /// [`Self::set_write_stall_drain`]).
    pub fn set_write_stall_l0(&mut self, limit: Option<usize>) {
        self.write_stall_l0 = limit.filter(|n| *n > 0);
    }

    /// Current L0 write-stall threshold, if enabled.
    #[must_use]
    pub fn write_stall_l0(&self) -> Option<usize> {
        self.write_stall_l0
    }

    /// When stall limit is set: try one flush + leveled compact before refusing.
    ///
    /// Default **false** (immediate `WriteStall`). Enable for single-writer
    /// embeds that prefer self-help drain over surfacing the error.
    pub fn set_write_stall_drain(&mut self, enabled: bool) {
        self.write_stall_drain = enabled;
    }

    /// Whether one compact drain is attempted before `WriteStall`.
    #[must_use]
    pub fn write_stall_drain(&self) -> bool {
        self.write_stall_drain
    }

    /// Times a write was refused for L0 or mem stall (observability).
    #[must_use]
    pub fn write_stall_count(&self) -> u64 {
        self.write_stall_count
    }

    /// Opt-in write stall when active memtable ≈ ≥ `bytes` (open-items §2.3 c).
    ///
    /// `None` or `0` disables (default). Bounds mem growth when auto-flush cannot
    /// keep up. With [`Self::set_write_stall_drain`], one flush is tried first.
    pub fn set_write_stall_mem_bytes(&mut self, bytes: Option<usize>) {
        self.write_stall_mem_bytes = bytes.filter(|n| *n > 0);
    }

    /// Current memtable stall threshold in bytes, if enabled.
    #[must_use]
    pub fn write_stall_mem_bytes(&self) -> Option<usize> {
        self.write_stall_mem_bytes
    }

    /// Soft L0 pressure: when L0 ≥ `n`, run one flush+compact before admitting
    /// the write (open-items §2.3 option b — no sleep, no refuse).
    ///
    /// Typically set **below** [`Self::set_write_stall_l0`] so the engine self-helps
    /// under load and only hard-stalls if still over the hard limit. Default off.
    pub fn set_write_pressure_l0(&mut self, limit: Option<usize>) {
        self.write_pressure_l0 = limit.filter(|n| *n > 0);
    }

    /// Current soft L0 pressure threshold, if enabled.
    #[must_use]
    pub fn write_pressure_l0(&self) -> Option<usize> {
        self.write_pressure_l0
    }

    /// Times soft pressure triggered a drain pass.
    #[must_use]
    pub fn write_pressure_count(&self) -> u64 {
        self.write_pressure_count
    }

    /// Enable Pebble-shaped L0 backpressure defaults (open-items §2.3).
    ///
    /// - Soft pressure at [`L0_COMPACTION_TRIGGER`] (one drain, still admit)
    /// - Hard stall at `2 × L0_COMPACTION_TRIGGER` with drain before refuse
    ///
    /// Mem stall remains off (configure separately). No artificial sleep.
    pub fn enable_write_backpressure_defaults(&mut self) {
        self.set_write_pressure_l0(Some(L0_COMPACTION_TRIGGER));
        self.set_write_stall_l0(Some(L0_COMPACTION_TRIGGER.saturating_mul(2)));
        self.set_write_stall_drain(true);
    }

    /// One flush + leveled compact (shared by pressure and stall-drain).
    fn drain_l0_once(&mut self) {
        if !self.mem.is_empty() || self.imm.is_some() {
            let _ = self.flush();
        }
        let _ = self.compact_with_ssts_only(CompactOptions::default());
    }

    /// Current auto blob-GC threshold, if enabled.
    #[must_use]
    pub fn auto_blob_gc_min_ratio(&self) -> Option<f64> {
        self.auto_blob_gc_min_ratio
    }

    /// Active blob generation (`0` = single `VALUES.vlog`).
    #[must_use]
    pub fn blob_active(&self) -> u32 {
        self.blob_active
    }

    /// Sealed + active blob file numbers on disk.
    #[must_use]
    pub fn blob_file_nums(&self) -> Vec<u32> {
        vlog::list_blob_nums(&self.env, &self.dir)
    }

    fn rotate_blob(&mut self) -> Result<()> {
        self.vlog_sync_pending()?;
        let next = if self.blob_active == 0 {
            1
        } else {
            self.blob_active
                .checked_add(1)
                .ok_or_else(|| CoreError::Internal("blob generation overflow".into()))?
        };
        let log = ValueLog::open_blob(&self.env, &self.dir, next)?;
        self.vlog = Some(Mutex::new(log));
        self.blob_active = next;
        Ok(())
    }

    /// Ingest accounting for write paths outside `apply_batch` (TX staging).
    pub(crate) fn note_ingested(&mut self, n: usize) {
        self.bytes_ingested = self.bytes_ingested.saturating_add(n as u64);
    }

    /// Maybe rewrite a large put value into the vlog; returns stored value bytes.
    pub(crate) fn maybe_spill_large_value(&mut self, value: Bytes) -> Result<Bytes> {
        let Some(threshold) = self.large_value_threshold else {
            // F188: inline values are stored escaped so an honest `VLG…`
            // payload can never be resolved as a pointer on read.
            return Ok(escape_inline_value(value));
        };
        if value.len() < threshold {
            return Ok(escape_inline_value(value));
        }
        if self.vlog.is_none() {
            if self.vlog_rotate_bytes.is_some() {
                self.rotate_blob()?;
            } else {
                self.vlog = Some(Mutex::new(ValueLog::open_with_flag(
                    &self.env,
                    &self.dir,
                    self.vlog_use_new,
                )?));
            }
        }
        if let Some(cap) = self.vlog_rotate_bytes {
            // Open already created VALUES.vlog when the threshold is set.
            // Rotation mode must start at 000001.blob — otherwise the first
            // spills are VLG1 on file 0 and the first get after rotate misses.
            if self.blob_active == 0 {
                self.rotate_blob()?;
            } else {
                let len = self.vlog.as_ref().map_or(0, |v| v.lock().len_bytes());
                if len >= cap {
                    self.rotate_blob()?;
                }
            }
        }
        let vlog = self.vlog.as_ref().expect("just opened");
        let mut guard = vlog.lock();
        let (off, len, crc) = guard.append_pending_bytes(value)?;
        drop(guard);
        Ok(vlog::encode_vlog_ptr(vlog::VlogPtr {
            file_num: self.blob_active,
            offset: off,
            len,
            crc,
        }))
    }

    /// `write()` vlog tail so WAL pointers cannot outrun the payload (async).
    fn vlog_flush_pending(&mut self) -> Result<()> {
        if let Some(v) = &self.vlog {
            v.lock().flush_pending()?;
        }
        Ok(())
    }

    /// Flush + fsync vlog. G1: must return before the WAL pointer is durable.
    fn vlog_sync_pending(&mut self) -> Result<()> {
        if let Some(v) = &self.vlog {
            v.lock().sync_pending()?;
        }
        Ok(())
    }

    /// Async: `write()` only. G1: `fsync` so a crash after Ok still resolves.
    fn vlog_prepare_wal(&mut self, do_sync: bool) -> Result<()> {
        if do_sync {
            self.vlog_sync_pending()
        } else {
            self.vlog_flush_pending()
        }
    }

    /// Range scan at the latest committed snapshot over MemTable ∪ SSTs.
    ///
    /// Yields `(user_key, value)` in ascending user-key order. Only the newest
    /// non-deleted version per key with `sequence <= last_sequence` is returned.
    ///
    /// # Production note (RFC-0015 M1)
    ///
    /// Unbounded `range` materialises every live key in the interval into a `Vec`
    /// — fine for small DBs and tests, an **OOM footgun** on large keyspaces.
    /// Prefer [`Self::range_limited`] or streaming [`Self::scan`] / [`Self::scan_at`]
    /// for pagination and large scans.
    #[deprecated(
        since = "0.1.0",
        note = "materialises the whole interval into RAM (OOM footgun on large DBs); \
                use `scan`/`scan_at` for streaming or `range_limited`/`range_at_limited` \
                for a bounded collect"
    )]
    #[must_use]
    pub fn range(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> Vec<(Bytes, Bytes)> {
        // Latest sequence is always ≥ the GC watermark.
        self.range_at_limited(self.visible_sequence(), start, end, None)
            .unwrap_or_else(|_| Vec::new())
    }

    /// Range scan at an explicit snapshot sequence.
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`] if `snapshot` is below the version-GC watermark.
    #[deprecated(
        since = "0.1.0",
        note = "materialises the whole interval into RAM (OOM footgun on large DBs); \
                use `scan_at`/`try_scan_at` for streaming or `range_at_limited` \
                for a bounded collect"
    )]
    pub fn range_at(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
    ) -> Result<Vec<(Bytes, Bytes)>> {
        self.range_at_limited(snapshot, start, end, None)
    }

    /// Like [`range`](Self::range) but stops after `limit` live keys when `Some`.
    ///
    /// Prefer this for large keyspaces / pagination (RocksDB iterator limit class).
    #[must_use]
    pub fn range_limited(
        &self,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
    ) -> Vec<(Bytes, Bytes)> {
        self.range_at_limited(self.visible_sequence(), start, end, limit)
            .unwrap_or_else(|_| Vec::new())
    }

    /// Largest user key that starts with `prefix` and is visible at `snapshot`.
    ///
    /// RFC-0033: per-layer last key in `[prefix, before)`, then confirm with
    /// [`lookup`] so a newer tombstone in another layer cannot leak. Empty
    /// prefix means the whole keyspace. Does not run `StreamingVisibleIter`
    /// over the prefix. WAL / fencing / accept-set are untouched (read path).
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`].
    pub fn last_under_prefix(
        &self,
        snapshot: SequenceNumber,
        prefix: &[u8],
    ) -> Result<Option<Bytes>> {
        self.ensure_snapshot_readable(Snapshot::at(snapshot))?;
        if snapshot == 0 {
            return Ok(None);
        }
        self.latest_sst_probed
            .fetch_add(self.ssts.len() as u64, Ordering::Relaxed);
        let mut before = crate::prefix::prefix_exclusive_end(prefix);
        loop {
            let mut cand: Option<Bytes> = None;
            let mut consider = |k: Bytes| {
                if !prefix.is_empty() && !k.starts_with(prefix) {
                    return;
                }
                if let Some(h) = before.as_deref() {
                    if k.as_ref() >= h {
                        return;
                    }
                }
                if cand.as_ref().is_none_or(|c| k.as_ref() > c.as_ref()) {
                    cand = Some(k);
                }
            };
            let hi = before.as_deref();
            let mut newest_mem_last: Option<Bytes> = None;
            for (i, table) in self.mem_layers().enumerate() {
                if let Some((k, _)) = table.last_visible_under_prefix(prefix, snapshot, hi) {
                    if i == 0 {
                        newest_mem_last = Some(k.clone());
                    }
                    consider(k);
                }
            }
            for table in &self.ssts {
                if let Some((k, _)) =
                    table.last_visible_under_prefix_with(prefix, snapshot, hi, |bi| {
                        Some(self.block_cache.get_or_insert_with(table.path(), bi, || {
                            table.decode_block(bi).unwrap_or_default()
                        }))
                    })
                {
                    consider(k);
                }
            }
            let Some(k) = cand else {
                return Ok(None);
            };
            // Newest mem already applied get_entry; an older layer cannot hide
            // a newer live key (G2). Skip the second full LSM walk.
            if newest_mem_last.as_ref() == Some(&k) {
                return Ok(Some(k));
            }
            match self.lookup(k.as_ref(), snapshot) {
                Lookup::Found(_) => return Ok(Some(k)),
                Lookup::Deleted | Lookup::NotFound => {
                    before = Some(k.to_vec());
                }
            }
        }
    }

    /// Last live key under an **MVCC user prefix** (`user || version`).
    ///
    /// If the newest memtable has a live key under `prefix`, that is the
    /// latest write (single-writer; newer suffixes are assigned in mem).
    /// Older layers cannot hold a bytewise-larger live key of the same user.
    /// When mem misses, falls through to [`last_under_prefix`] (full merge +
    /// lookup) so a flushed version + mem tombstone still resolves.
    ///
    /// Do **not** use this for a prefix that spans many users (`"u/"`): an
    /// older layer may hold a larger sibling. WAL / fencing unchanged.
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`].
    pub fn last_under_user_prefix(
        &self,
        snapshot: SequenceNumber,
        prefix: &[u8],
    ) -> Result<Option<Bytes>> {
        self.ensure_snapshot_readable(Snapshot::at(snapshot))?;
        if snapshot == 0 {
            return Ok(None);
        }
        self.latest_ops.fetch_add(1, Ordering::Relaxed);
        let latest = snapshot == self.visible_sequence();
        if latest {
            if let Some(hit) = self.last_prefix_cache.get(prefix) {
                return Ok(hit);
            }
        }
        // Newest layer first (active → imm → pin → retired newest). The first
        // hit is the latest write of this user (RFC-0041 retired L0 cache).
        let mut best: Option<(Bytes, bool)> = None;
        for (i, table) in self.mem_layers().enumerate() {
            if let Some((k, _)) = table.last_visible_under_prefix(prefix, snapshot, None) {
                best = Some((k, i == 0));
                break;
            }
        }
        let out = if let Some((k, from_newest_mem)) = best {
            self.latest_mem_hit.fetch_add(1, Ordering::Relaxed);
            if from_newest_mem {
                // `get_entry` already merged this newest layer (range
                // tombstones included); nothing newer can hide it.
                Some(k)
            } else {
                // Older mem layer: a newer layer's range tombstone must
                // still be allowed to hide the candidate (F172) — confirm
                // with the full point merge before accepting.
                match self.lookup(k.as_ref(), snapshot) {
                    Lookup::Found(_) => Some(k),
                    Lookup::Deleted | Lookup::NotFound => {
                        self.latest_sst_fallback.fetch_add(1, Ordering::Relaxed);
                        self.last_under_user_prefix_sst(snapshot, prefix)?
                    }
                }
            }
        } else {
            self.latest_sst_fallback.fetch_add(1, Ordering::Relaxed);
            self.last_under_user_prefix_sst(snapshot, prefix)?
        };
        if latest {
            // F207 (F198 shape): the fill runs under the read lock, which does
            // not exclude `publish_sequence` (also read-locked). A publish
            // landing mid-walk clears the cache between the entry check above
            // and this insert; the pre-publish answer would then carry the
            // post-clear generation and validate until the next write. Only
            // fill while `published` still matches the seq the answer was
            // computed at.
            if self.published_seq.load(Ordering::Acquire) == snapshot {
                self.last_prefix_cache.insert(prefix, out.clone());
            }
        }
        Ok(out)
    }

    /// L0 newest → older → L1+ (same single-writer invariant as the mem hit).
    #[must_use]
    pub fn sst_indices_newest_first(&self) -> &[usize] {
        &self.sst_order_newest
    }

    /// Rebuild the newest-first SST order after level changes.
    fn rebuild_sst_order(&mut self) {
        let mut idx: Vec<usize> = (0..self.ssts.len()).collect();
        idx.sort_by(|&a, &b| {
            let la = self.sst_levels.get(a).copied().unwrap_or(0);
            let lb = self.sst_levels.get(b).copied().unwrap_or(0);
            match la.cmp(&lb) {
                std::cmp::Ordering::Equal => b.cmp(&a),
                o => o,
            }
        });
        self.sst_order_newest = idx;
        self.rebuild_sst_runs();
    }

    /// Regroup [`Self::sst_order_newest`] into per-level runs. That order is
    /// sorted (level asc, index desc), so levels come out contiguous and
    /// newest-first inside each run — the linear fallback inside `lookup`
    /// iterates exactly the flat order.
    fn rebuild_sst_runs(&mut self) {
        let mut runs: Vec<SstRun> = Vec::new();
        for &sst_i in &self.sst_order_newest {
            let level = self.sst_levels.get(sst_i).copied().unwrap_or(0);
            match runs.last_mut() {
                Some(run) if run.level == level => run.tables_newest_first.push(sst_i),
                _ => runs.push(SstRun {
                    level,
                    tables_newest_first: vec![sst_i],
                    disjoint_by_lo: None,
                    sorted_by_lo: None,
                    packed_lo: None,
                    packed_hi: None,
                    disjoint_los: None,
                    any_range_tombstones: false,
                }),
            }
        }
        for run in &mut runs {
            run.sorted_by_lo = SstRun::sorted_by_lo(&self.ssts, &run.tables_newest_first);
            run.disjoint_by_lo = run.sorted_by_lo.as_ref().and_then(|by_lo| {
                SstRun::pairwise_disjoint(&self.ssts, by_lo).then(|| by_lo.clone())
            });
            run.packed_lo = run
                .sorted_by_lo
                .as_ref()
                .map(|by_lo| DisjointLos::from_tables(&self.ssts, by_lo));
            run.packed_hi = run.sorted_by_lo.as_ref().map(|by_lo| {
                let mut bytes = Vec::new();
                let mut ends = Vec::with_capacity(by_lo.len());
                for &i in by_lo {
                    bytes.extend_from_slice(self.ssts[i].largest_user_key().unwrap());
                    ends.push(u32::try_from(bytes.len()).expect("packed hi fits u32"));
                }
                DisjointLos { bytes, ends }
            });
            run.disjoint_los = run.packed_lo.clone();
            run.any_range_tombstones = run
                .tables_newest_first
                .iter()
                .any(|&i| self.ssts[i].has_range_tombstones());
        }
        let mut glo: Option<&[u8]> = None;
        let mut ghi: Option<&[u8]> = None;
        for t in &self.ssts {
            if let Some(lo) = t.smallest_user_key() {
                if glo.is_none_or(|g| lo < g) {
                    glo = Some(lo);
                }
            }
            if let Some(hi) = t.largest_user_key() {
                if ghi.is_none_or(|g| hi > g) {
                    ghi = Some(hi);
                }
            }
        }
        self.sst_user_lo = glo.map(Bytes::copy_from_slice);
        self.sst_user_hi = ghi.map(Bytes::copy_from_slice);
        let mut per_cf: std::collections::BTreeMap<String, (Bytes, Bytes)> =
            std::collections::BTreeMap::new();
        let mut mixed: Vec<(Bytes, Bytes)> = Vec::new();
        for t in &self.ssts {
            let Some(lo) = t.smallest_user_key() else {
                continue;
            };
            let Some(hi) = t.largest_user_key() else {
                continue;
            };
            let fam = t.cf();
            if fam.is_empty() {
                mixed.push((Bytes::copy_from_slice(lo), Bytes::copy_from_slice(hi)));
                continue;
            }
            per_cf
                .entry(fam.to_string())
                .and_modify(|(clo, chi)| {
                    if lo < clo.as_ref() {
                        *clo = Bytes::copy_from_slice(lo);
                    }
                    if hi > chi.as_ref() {
                        *chi = Bytes::copy_from_slice(hi);
                    }
                })
                .or_insert_with(|| (Bytes::copy_from_slice(lo), Bytes::copy_from_slice(hi)));
        }
        let mut envelopes: Vec<(Bytes, Bytes)> = per_cf.into_values().collect();
        envelopes.extend(mixed);
        *self.sst_envelope.write() = envelopes;
        let settled = self.bulk_live_bytes() == 0
            && self.mem.is_empty()
            && self.imm.is_none()
            && self.parked_unflushed.is_empty();
        self.settled_sst_only.store(settled, Ordering::Release);
        self.sst_runs = runs;
    }

    /// Drop the retired read cache when no L0 remains to cover.
    fn sync_retired_to_l0(&mut self) {
        let l0 = self.level_file_count(0);
        if l0 == 0 {
            self.retired_pending.clear();
            self.retired_fold = MemTable::new();
            self.retired_l0s = 0;
        } else if self.retired_l0s > l0 {
            self.retired_l0s = l0;
        }
    }

    fn note_sst_inventory_changed(&mut self) {
        self.rebuild_sst_order();
        self.sync_retired_to_l0();
    }

    /// SST fallback for an MVCC user prefix: first file (newest) with a live
    /// key wins. Older files cannot hold a bytewise-larger suffix (same
    /// contract as the mem hit). A newer tombstone of that exact key still
    /// falls through (`before` retry).
    fn last_under_user_prefix_sst(
        &self,
        snapshot: SequenceNumber,
        prefix: &[u8],
    ) -> Result<Option<Bytes>> {
        let order = self.sst_order_newest.clone();
        let mut before = crate::prefix::prefix_exclusive_end(prefix);
        loop {
            let mut cand: Option<Bytes> = None;
            for &sst_i in order.iter() {
                let table = &self.ssts[sst_i];
                self.latest_sst_probed.fetch_add(1, Ordering::Relaxed);
                if let Some((k, _)) = table.last_visible_under_prefix_with(
                    prefix,
                    snapshot,
                    before.as_deref(),
                    |bi| {
                        Some(self.block_cache.get_or_insert_with(table.path(), bi, || {
                            table.decode_block(bi).unwrap_or_default()
                        }))
                    },
                ) {
                    cand = Some(k);
                    break;
                }
            }
            let Some(k) = cand else {
                return Ok(None);
            };
            // Confirm with the full point merge (same ruler as
            // `last_under_prefix`): a range tombstone in a newer SST — or an
            // older mem layer's put under a newer layer's tombstone — must
            // hide the candidate. `point_at` alone misses RangeDeletion
            // entries (F172).
            match self.lookup(k.as_ref(), snapshot) {
                Lookup::Found(_) => return Ok(Some(k)),
                Lookup::Deleted | Lookup::NotFound => {
                    before = Some(k.to_vec());
                    continue;
                }
            }
        }
    }

    /// Newer mem / L0 has a point tombstone (or newer point) for `key`.

    /// Range at `snapshot` with optional live-key `limit`.
    ///
    /// Uses the streaming merge path ([`Self::try_scan_at`]) so the full keyspace is
    /// not required as a single materialised `Vec` of all live pairs.
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`] if `snapshot` is below the version-GC watermark.
    ///
    /// # Panics
    /// Fail-stop on a corrupt vlog payload (F1): the resolving stream refuses
    /// to serve corruption as an absent or empty key.
    pub fn range_at_limited(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
    ) -> Result<Vec<(Bytes, Bytes)>> {
        Ok(self
            .try_scan_at(snapshot, start, end, limit)?
            .map(|VisibleKv { key, value }| (key, value))
            .collect())
    }

    /// Streaming range scan at the latest snapshot (public bound-memory path).
    ///
    /// Pulls sorted per-layer streams and merges with a heap — does not allocate
    /// one `Vec` of every live KV before yielding. Prefer this over collecting a
    /// huge `range` result when the keyspace is large; use [`Iterator::take`] or
    /// the `limit` on [`Self::scan_at`] for pagination.
    pub fn scan(
        &self,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
    ) -> impl Iterator<Item = VisibleKv> + '_ {
        self.scan_projected(start, end, ScanProjection::Full)
    }

    /// Scan with [`ScanProjection`] (RFC-0019 `KeyOnly` skips vlog resolve).
    pub fn scan_projected(
        &self,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        projection: ScanProjection,
    ) -> impl Iterator<Item = VisibleKv> + '_ {
        self.scan_at_projected(self.visible_sequence(), start, end, None, projection)
    }

    /// Streaming range at `snapshot` with optional live-key `limit`.
    ///
    /// Prefer [`Self::try_scan_at`] when the snapshot may predate version GC.
    /// This convenience path **panics** on [`CoreError::SnapshotTooOld`] so a
    /// too-old scan cannot silently look like an empty range.
    pub fn scan_at(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
    ) -> impl Iterator<Item = VisibleKv> + '_ {
        self.try_scan_at(snapshot, start, end, limit)
            .unwrap_or_else(|e| {
                panic!("scan_at: {e}; use try_scan_at for recoverable SnapshotTooOld")
            })
    }

    /// Fail-closed streaming scan at `snapshot` (open-items §2.1 (c) range path).
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`] if history for `snapshot` may have been dropped.
    pub fn try_scan_at(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
    ) -> Result<impl Iterator<Item = VisibleKv> + '_> {
        self.try_scan_at_projected(snapshot, start, end, limit, ScanProjection::Full)
    }

    /// Newest version per user key at `snapshot`, with [`crate::WindowKv::snapshot_live`].
    ///
    /// Does not apply [`crate::iter_window_keep`] — the caller (compat iterator
    /// window) keeps or drops from that bit. Hidden rows (deletion / covering
    /// range tombstone) are included with `snapshot_live == false`.
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`].
    pub fn try_scan_window_at(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
    ) -> Result<impl Iterator<Item = crate::WindowKv> + '_> {
        self.ensure_snapshot_readable(Snapshot::at(snapshot))?;
        Ok(self
            .scan_at_raw(snapshot, start, end, None, true)
            .into_window_kvs())
    }

    /// [`scan_at`](Self::scan_at) with projection (panics on too-old snapshot).
    pub fn scan_at_projected(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
        projection: ScanProjection,
    ) -> impl Iterator<Item = VisibleKv> + '_ {
        self.try_scan_at_projected(snapshot, start, end, limit, projection)
            .unwrap_or_else(|e| {
                panic!("scan_at_projected: {e}; use try_scan_at_projected for recoverable SnapshotTooOld")
            })
    }

    /// Fail-closed projected scan (see [`Self::try_scan_at`]).
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`].
    pub fn try_scan_at_projected(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
        projection: ScanProjection,
    ) -> Result<impl Iterator<Item = VisibleKv> + '_> {
        self.ensure_snapshot_readable(Snapshot::at(snapshot))?;
        let resolve = matches!(projection, ScanProjection::Full);
        let raw = self.scan_at_raw(snapshot, start, end, limit, resolve);
        Ok(raw.map(move |VisibleKv { key, value }| match projection {
            ScanProjection::Full => VisibleKv { key, value },
            ScanProjection::KeyOnly => VisibleKv {
                key,
                value: Bytes::new(),
            },
        }))
    }

    /// Count live keys in `[start, end)` at `snapshot`, stopping at `limit`.
    ///
    /// Same visibility as [`Self::try_scan_at_projected`] with
    /// [`ScanProjection::KeyOnly`] (no value resolve).
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`].
    pub fn count_in_range(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
    ) -> Result<usize> {
        self.ensure_snapshot_readable(Snapshot::at(snapshot))?;
        if snapshot == 0 {
            return Ok(0);
        }
        let latest = snapshot == self.visible_sequence();
        if latest {
            if let Some(n) = self.count_cache.get(start, end, limit) {
                self.scan_ops.fetch_add(1, Ordering::Relaxed);
                return Ok(n);
            }
        }
        // `snapshot` (== visible sequence here) read BEFORE computing: any
        // write published after the answer carries a strictly greater
        // sequence, so the dirty-log check in `CountCache::get` sees it.
        let n = self.count_visible(snapshot, start, end, limit);
        if latest {
            self.count_cache.insert(start, end, limit, n, snapshot);
        }
        Ok(n)
    }

    fn invalidate_read_answers(&self, seq: SequenceNumber) {
        let reset = self.point_cache_reset.swap(false, Ordering::Relaxed);
        let keys = std::mem::take(&mut *self.dirty_points.lock());
        // Do not insert WriteOp.value: large values are vlog pointers.
        // Fat apply gen-bumps; small writes drop only the dirty keys.
        if reset || keys.len() > 32 || keys.is_empty() {
            self.point_cache.clear();
        } else {
            self.point_cache.invalidate_many(&keys);
        }
        // Count answers are window-scoped: range-check the dirty keys
        // instead of clearing every window (RFC-0044 `ycsb-longwindow`).
        // Fat apply / range deletion / unknown dirt still clear wholesale.
        if reset || keys.is_empty() {
            self.count_cache.clear();
        } else {
            self.count_cache.record_dirty(seq, &keys);
        }
        self.last_prefix_cache.clear();
        self.read_cache_epoch.fetch_add(1, Ordering::Release);
        // RFC-0154 P1.5: 1-key put bumps one TLS gen bucket so zipf gets of
        // other keys stay cached. Fat apply / unknown dirt still epoch-bumps.
        if reset || keys.len() != 1 {
            self.point_tls_epoch.fetch_add(1, Ordering::Release);
        } else {
            self.key_gen.touch(&keys[0]);
        }
        let settled = self.bulk_live_bytes() == 0
            && self.mem.is_empty()
            && self.imm.is_none()
            && self.parked_unflushed.is_empty();
        self.settled_sst_only.store(settled, Ordering::Release);
    }

    /// Distinct visible user keys in `[start, end)` at `snapshot`, capped at
    /// `limit` (RFC-0037 P1.3). Borrowed-cursor merge — same visibility as
    /// [`Self::scan_at_raw`] without materializing owned key clones per
    /// entry (count windows walk every MVCC version).
    fn count_visible(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
    ) -> usize {
        if snapshot == 0 {
            return 0;
        }
        let cap = limit.unwrap_or(usize::MAX);
        // deps_scan / kvrocks_scan: one memtable, no SST, latest snapshot —
        // count the live tail index (RFC-0154).
        if self.ssts.is_empty() {
            let mut only: Option<&MemTable> = None;
            let mut many = false;
            for t in self.scan_mem_layers() {
                if t.is_empty() {
                    continue;
                }
                if only.is_some() {
                    many = true;
                    break;
                }
                only = Some(t);
            }
            if !many {
                if let Some(t) = only {
                    if let Some(n) = t.count_latest_in_range(start, end, cap, snapshot) {
                        self.scan_ops.fetch_add(1, Ordering::Relaxed);
                        return n;
                    }
                } else {
                    return 0;
                }
            }
        }
        self.scan_ops.fetch_add(1, Ordering::Relaxed);
        // Range tombstones first (G2), exactly like `scan_at_raw`.
        let mut range_dels = Vec::new();
        // First non-empty layer stays on the stack (the deps-scan state is
        // one memtable + no SSTs — no cursor Vec alloc on that path);
        // a second layer migrates to the Vec (RFC-0054 P1.3).
        let mut single: Option<CountCursor<'_>> = None;
        let mut cursors: Vec<CountCursor<'_>> = Vec::new();
        // Live + parked-without-SST only. Retired BTrees are a point/MVCC
        // cache; their L0 files are in `ssts` (retire2 scan died merging them).
        for table in self.scan_mem_layers() {
            if table.has_range_tombstones() {
                table.collect_range_tombstones(snapshot, &mut range_dels);
            }
            if table.is_empty() {
                continue;
            }
            let c = CountCursor::Mem(MemCountCursor::new(table, start, end, snapshot));
            if let Some(first) = single.take() {
                cursors.reserve_exact(4 + self.ssts.len());
                cursors.push(first);
                cursors.push(c);
            } else {
                single = Some(c);
            }
        }
        let any_sst_tombs = self.sst_runs.iter().any(|r| r.any_range_tombstones);
        for table in self.ssts.iter() {
            if any_sst_tombs {
                table.collect_range_tombstones(snapshot, &mut range_dels);
            }
            if !table.overlaps_user_range(start, end) {
                continue;
            }
            self.scan_sst_probed.fetch_add(1, Ordering::Relaxed);
            let c = CountCursor::Sst(SstCountCursor::new(
                table,
                start,
                end,
                snapshot,
                &self.block_cache,
            ));
            if let Some(first) = single.take() {
                cursors.push(first);
                cursors.push(c);
            } else if cursors.is_empty() {
                single = Some(c);
            } else {
                cursors.push(c);
            }
        }
        let mut count = 0usize;
        // Single-cursor fast path: the k-way min-head scan is pure overhead
        // when only one layer overlaps the window (deps-scan state: one
        // memtable, no SSTs — RFC-0054 P1.3).
        if let Some(c) = single.as_mut() {
            while count < cap {
                let Some(head) = c.head() else { break };
                let kind = head.kind;
                let seq = head.sequence;
                // Step even on tombstone heads — visibility and cursor
                // advance must not be coupled (short-circuit spin).
                let visible = kind == ValueType::Value
                    && (range_dels.is_empty()
                        || !crate::merge::range_deleted(head.user_key.as_ref(), seq, &range_dels));
                c.step_current_user();
                if visible {
                    count += 1;
                }
            }
            return count;
        }
        while count < cap {
            // Min head across layers by InternalKey order (user asc, seq
            // desc, kind desc) — the global newest version of that user.
            let mut best: Option<usize> = None;
            let mut best_h: Option<&crate::key::InternalKey> = None;
            for (i, c) in cursors.iter().enumerate() {
                let Some(h) = c.head() else { continue };
                match best_h {
                    None => {
                        best = Some(i);
                        best_h = Some(h);
                    }
                    Some(bh) if internal_less(h, bh) => {
                        best = Some(i);
                        best_h = Some(h);
                    }
                    _ => {}
                }
            }
            let Some(bi) = best else { break };
            let visible = {
                let head = cursors[bi].head().expect("best head");
                head.kind == ValueType::Value
                    && (range_dels.is_empty()
                        || !crate::merge::range_deleted(
                            head.user_key.as_ref(),
                            head.sequence,
                            &range_dels,
                        ))
            };
            // Split the winner out so other cursors can step while its
            // user-key borrow is live, then step the winner (RFC-0039 P2.1).
            let (left, rest) = cursors.split_at_mut(bi);
            let (win, right) = rest.split_at_mut(1);
            {
                let user = win[0].head().expect("best head").user_key.as_ref();
                for c in left.iter_mut().chain(right.iter_mut()) {
                    if c.head().is_some_and(|h| h.user_key.as_ref() == user) {
                        c.step_current_user();
                    }
                }
            }
            win[0].step_current_user();
            if visible {
                count += 1;
            }
        }
        count
    }

    /// Settled one-run prefix page: no mem/bulk, no range tombs, exactly one
    /// overlapping disjoint level. Skips the boxed merge heap the windowed
    /// iterator otherwise builds per 512-row refill (prefix_scan).
    #[must_use]
    pub fn try_disjoint_scan_page(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: usize,
    ) -> Option<Vec<(Bytes, Bytes)>> {
        if snapshot == 0 || limit == 0 {
            return Some(Vec::new());
        }
        if snapshot < self.earliest_readable_seq {
            return None;
        }
        if self.bulk_live_bytes() != 0
            || !self.mem.is_empty()
            || self.imm.is_some()
            || !self.parked_unflushed.is_empty()
        {
            return None;
        }
        let mut picked: Vec<&[usize]> = Vec::new();
        for run in &self.sst_runs {
            if run.any_range_tombstones {
                return None;
            }
            if let Some(by_lo) = run.disjoint_by_lo.as_ref() {
                let i = first_overlapping_disjoint_file(&self.ssts, by_lo, start);
                if i < by_lo.len() && self.ssts[by_lo[i]].overlaps_user_range(start, end) {
                    picked.push(by_lo.as_slice());
                }
            } else if run
                .tables_newest_first
                .iter()
                .any(|&ti| self.ssts[ti].overlaps_user_range(start, end))
            {
                return None;
            }
        }
        if picked.is_empty() {
            return Some(Vec::new());
        }
        self.scan_ops.fetch_add(1, Ordering::Relaxed);
        Some(merge_disjoint_level_page(
            self, &picked, start, end, snapshot, limit,
        ))
    }

    fn scan_at_raw(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
        resolve_values: bool,
    ) -> StreamingVisibleIter<'_> {
        if snapshot == 0 {
            return StreamingVisibleIter::new(Vec::new(), 0, start, end, limit);
        }
        self.scan_ops.fetch_add(1, Ordering::Relaxed);
        let scan_diag = crate::merge::scan_diag_enabled();
        let scan_diag_t0 = scan_diag.then(Instant::now);
        // Range tombstones first (G2): a covering delete whose start sits
        // before `start` must still hide keys in the window. Point streams
        // are lazy — later SST blocks are not decoded after `limit` emits.
        let mut range_dels = Vec::new();
        let mut streams: Vec<crate::merge::LayerStream<'_>> =
            Vec::with_capacity(4 + self.sst_runs.len());
        for table in self.scan_mem_layers() {
            if table.has_range_tombstones() {
                table.collect_range_tombstones(snapshot, &mut range_dels);
            }
            if table.is_empty() {
                continue;
            }
            streams.push(self.memtable_stream(table, start, end, snapshot, resolve_values));
        }
        // Range tombstones from EVERY table (G2): a covering delete whose
        // start sits before `start` must still hide keys in the window,
        // including tables a grouped stream has not pulled from yet.
        // Bulk hydrate has none — skip the O(#SST) walk (prefix_scan).
        if self.sst_runs.iter().any(|r| r.any_range_tombstones) {
            for table in self.ssts.iter() {
                table.collect_range_tombstones(snapshot, &mut range_dels);
            }
        }
        // A strictly disjoint level collapses into ONE lazy concatenated
        // stream ([`LevelRunStream`]): heap width drops from #SSTs to
        // #levels + L0 + memtables. L0 and overlapping levels keep one
        // stream per overlapping file (identical load/probe semantics).
        for run in self.sst_runs.iter() {
            if let Some(by_lo) = run.disjoint_by_lo.as_ref() {
                streams.push(Box::new(LevelRunStream::new(
                    self,
                    by_lo,
                    start,
                    end,
                    snapshot,
                    resolve_values,
                )));
                continue;
            }
            for &ti in run.tables_newest_first.iter() {
                let table = &self.ssts[ti];
                if !table.overlaps_user_range(start, end) {
                    continue;
                }
                self.scan_sst_probed.fetch_add(1, Ordering::Relaxed);
                // Hash the path once per stream, not once per block fetch, and
                // keep value-resolved blocks under a tagged id: a full scan then
                // resolves each block once (on miss) and later loads are a pure
                // Arc clone — no per-load deep clone + vlog re-resolve. Re-resolve
                // is NOT identity (F188 strips an escape byte), so resolved slots
                // must never flow into a raw-keyed load.
                let id = crate::cache::path_id(table.path())
                    ^ if resolve_values {
                        RESOLVED_BLOCK_TAG
                    } else {
                        0
                    };
                let db = self;
                let load: Box<
                    dyn FnMut(usize) -> Option<std::sync::Arc<Vec<(InternalKey, Bytes)>>> + '_,
                > = Box::new(move |bi| {
                    Some(crate::sst::scan_block_get_or_insert(id, bi, || {
                        let mut entries = match table.decode_block(bi) {
                            Ok(entries) => entries,
                            // F1: a CRC/IO-faulted block must fail loudly —
                            // `unwrap_or_default` would silently skip its keys.
                            Err(e) => fail_stop_corrupt_block(table.path(), &e),
                        };
                        if resolve_values {
                            db.prefetch_resolve_stream(&mut entries);
                        }
                        entries
                    }))
                });
                streams.push(Box::new(table.iter_user_range(
                    start,
                    end,
                    snapshot,
                    resolve_values,
                    load,
                )));
            }
        }
        if let Some(t0) = scan_diag_t0 {
            self.scan_diag_note(t0, streams.len());
        }
        StreamingVisibleIter::from_point_streams(streams, range_dels, snapshot, start, end, limit)
    }

    /// `PEDRA_SCAN_DIAG=1`: one aggregate line every 2048 scans — streams
    /// merged, core setup ns/op, per-row ns (crate::merge counters) and
    /// block-cache hit/miss deltas. The cache counters are DB-global
    /// (point reads share the cache), so the per-op numbers are only
    /// attributable to scans on a scan-only bench leg.
    fn scan_diag_note(&self, t0: Instant, streams: usize) {
        static OPS: AtomicU64 = AtomicU64::new(0);
        static STREAMS: AtomicU64 = AtomicU64::new(0);
        static SETUP_NS: AtomicU64 = AtomicU64::new(0);
        static LAST_OPS: AtomicU64 = AtomicU64::new(0);
        static LAST_STREAMS: AtomicU64 = AtomicU64::new(0);
        static LAST_SETUP_NS: AtomicU64 = AtomicU64::new(0);
        static LAST_ROWS: AtomicU64 = AtomicU64::new(0);
        static LAST_ROW_NS: AtomicU64 = AtomicU64::new(0);
        static LAST_SINGLE: AtomicU64 = AtomicU64::new(0);
        static LAST_EVICTS: AtomicU64 = AtomicU64::new(0);
        static LAST_HITS: AtomicU64 = AtomicU64::new(0);
        static LAST_MISSES: AtomicU64 = AtomicU64::new(0);

        SETUP_NS.fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
        STREAMS.fetch_add(streams as u64, Ordering::Relaxed);
        let ops = OPS.fetch_add(1, Ordering::Relaxed) + 1;
        if ops % 2048 != 0 {
            return;
        }
        let rows = crate::merge::SCAN_DIAG_ROWS.load(Ordering::Relaxed);
        let row_ns = crate::merge::SCAN_DIAG_ROW_NS.load(Ordering::Relaxed);
        let hits = self.block_cache.hits();
        let misses = self.block_cache.misses();
        let d = ops - LAST_OPS.swap(ops, Ordering::Relaxed);
        if d == 0 {
            return;
        }
        let total = STREAMS.load(Ordering::Relaxed);
        let d_streams = total - LAST_STREAMS.swap(total, Ordering::Relaxed);
        let total = SETUP_NS.load(Ordering::Relaxed);
        let d_setup = total - LAST_SETUP_NS.swap(total, Ordering::Relaxed);
        let d_rows = rows - LAST_ROWS.swap(rows, Ordering::Relaxed);
        let d_row_ns = row_ns - LAST_ROW_NS.swap(row_ns, Ordering::Relaxed);
        let single = crate::merge::SCAN_DIAG_SINGLE_ROWS.load(Ordering::Relaxed);
        let evicts = crate::merge::SCAN_DIAG_STREAM_EVICTS.load(Ordering::Relaxed);
        let d_single = single - LAST_SINGLE.swap(single, Ordering::Relaxed);
        let d_evicts = evicts - LAST_EVICTS.swap(evicts, Ordering::Relaxed);
        let d_hits = hits - LAST_HITS.swap(hits, Ordering::Relaxed);
        let d_misses = misses - LAST_MISSES.swap(misses, Ordering::Relaxed);
        println!(
            "SCANDIAG ops={} streams/op={:.1} setup_ns/op={:.0} rows/op={:.1} row_ns/row={:.0} single={:.0}% evict/op={:.2} cache_hits/op={:.2} cache_misses/op={:.2}",
            ops,
            d_streams as f64 / d as f64,
            d_setup as f64 / d as f64,
            d_rows as f64 / d as f64,
            if d_rows > 0 { d_row_ns as f64 / d_rows as f64 } else { 0.0 },
            if d_rows > 0 { 100.0 * d_single as f64 / d_rows as f64 } else { 0.0 },
            d_evicts as f64 / d as f64,
            d_hits as f64 / d as f64,
            d_misses as f64 / d as f64,
        );
    }

    fn memtable_stream<'a>(
        &'a self,
        table: &'a MemTable,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        snapshot: SequenceNumber,
        resolve_values: bool,
    ) -> crate::merge::LayerStream<'a> {
        if table.has_range_tombstones() {
            // Range-tombstone layout walks the full table (no cursor
            // bounds) — keep the materialized shape; range deletes are
            // rare and this path is correctness-first.
            let mut stream: Vec<(InternalKey, Bytes)> = Vec::new();
            let mut last: Option<Bytes> = None;
            for (k, v) in table.iter_internal() {
                if !crate::merge::user_key_in_range(k.user_key.as_ref(), start, end) {
                    continue;
                }
                if k.kind == ValueType::RangeDeletion || k.sequence > snapshot {
                    continue;
                }
                if last.as_ref().is_some_and(|u| u == &k.user_key) {
                    continue;
                }
                last = Some(k.user_key.clone());
                let value = if resolve_values {
                    v.clone()
                } else {
                    Bytes::new()
                };
                stream.push((k.clone(), value));
            }
            if resolve_values {
                self.prefetch_resolve_stream(&mut stream);
            }
            return Box::new(stream.into_iter());
        }
        Box::new(MemChunkStream {
            table,
            start: crate::merge::bound_to_owned(start),
            end: crate::merge::bound_to_owned(end),
            last: None,
            snapshot,
            resolve: resolve_values,
            db: self,
            buf: Vec::new().into_iter(),
        })
    }

    /// Resolve vlog pointers in windows of [`Self::scan_prefetch`] (RFC-0029 P0.3).
    ///
    /// Single-threaded: each window issues up to N `Env` reads then continues.
    /// Before resolving a window, best-effort [`Env::advise`] `WillNeed` on each
    /// vlog pointer range (RFC-0029 P1.2). Order of `stream` is unchanged.
    /// A corrupt payload fail-stops via [`Self::resolve_stream_value`] (F1).
    fn prefetch_resolve_stream(&self, stream: &mut [(InternalKey, Bytes)]) {
        let n = self.scan_prefetch.max(1);
        let mut i = 0;
        while i < stream.len() {
            let end = (i + n).min(stream.len());
            let mut issued = 0u64;
            // Kernel readahead hints (StdEnv: Linux posix_fadvise; sim no-op).
            for slot in &stream[i..end] {
                if slot.0.kind == ValueType::RangeDeletion {
                    continue;
                }
                if let Some(ptr) = vlog::decode_vlog_ptr(slot.1.as_ref()) {
                    let path = if ptr.file_num == 0 {
                        self.dir.join(VLOG_FILE_NAME)
                    } else {
                        vlog::blob_path(&self.dir, ptr.file_num)
                    };
                    let _ = self.env.advise(
                        &path,
                        ptr.offset,
                        u64::from(ptr.len),
                        crate::env::AdviseKind::WillNeed,
                    );
                }
            }
            for slot in &mut stream[i..end] {
                if slot.0.kind == ValueType::RangeDeletion {
                    continue;
                }
                if vlog::decode_vlog_ptr(slot.1.as_ref()).is_some() {
                    issued = issued.saturating_add(1);
                }
                slot.1 = self.resolve_stream_value(slot.0.kind, slot.1.clone());
            }
            if issued > 0 && self.scan_prefetch > 1 {
                self.prefetch_hits.fetch_add(1, Ordering::Relaxed);
            }
            i = end;
        }
    }

    /// Resolve VLG1 for user values; leave range-tombstone end keys untouched.
    ///
    /// Fail-stop on corruption (F1): the streaming scan shape cannot express
    /// an error, and serving an empty `Bytes` would dress corruption up as a
    /// real (empty) value.
    fn resolve_stream_value(&self, kind: ValueType, stored: Bytes) -> Bytes {
        if kind == ValueType::RangeDeletion {
            return stored;
        }
        match self.resolve_stored_value(stored) {
            Ok(v) => v,
            Err(e) => fail_stop_corrupt_value("scan stream entry", &e),
        }
    }

    /// Observability snapshot: sizes, counts, WAL length (RFC-0014 / RFC-0016).
    #[must_use]
    pub fn stats(&self) -> DbStats {
        let mut sst_entries = 0usize;
        let mut sst_bytes = 0u64;
        for t in &self.ssts {
            sst_entries = sst_entries.saturating_add(t.len());
            if let Ok(len) = self.env.metadata_len(t.path()) {
                sst_bytes = sst_bytes.saturating_add(len);
            }
        }
        let wal_path = self.dir.join(WAL_FILE_NAME);
        let wal_bytes = self.env.metadata_len(&wal_path).unwrap_or(0);
        let (vlog_bytes, vlog_live_bytes, vlog_live_records) = self.vlog_size_stats();
        DbStats {
            last_sequence: self.last_sequence(),
            mem_approx_bytes: self.mem.approx_memory_usage()
                + self.imm.as_ref().map_or(0, MemTable::approx_memory_usage),
            mem_entries: self.mem.len() + self.imm.as_ref().map_or(0, MemTable::len),
            sst_count: self.ssts.len(),
            sst_entries,
            sst_bytes,
            wal_bytes,
            max_level: self.max_level(),
            table_cache_hits: self.table_cache.hits(),
            table_cache_misses: self.table_cache.misses(),
            block_cache_hits: self.block_cache.hits(),
            block_cache_misses: self.block_cache.misses(),
            block_cache_bytes: self.block_cache.used_bytes(),
            auto_compact_failures: self.auto_compact_failures,
            last_auto_compact_error: self.last_auto_compact_error.clone().unwrap_or_default(),
            wal_sync_count: self.wal_sync_count.load(Ordering::Relaxed),
            vlog_bytes,
            vlog_live_bytes,
            vlog_live_records,
            bytes_ingested: self.bytes_ingested,
            bytes_written_wal: self.bytes_written_wal,
            bytes_written_sst: self.bytes_written_sst,
            compact_count: self.compact_count,
            vlog_gc_count: self.vlog_gc_count,
            blob_files: u32::try_from(vlog::list_blob_nums(&self.env, &self.dir).len())
                .unwrap_or(u32::MAX),
            scan_prefetch_hits: self.prefetch_hits.load(Ordering::Relaxed),
            earliest_readable_seq: self.earliest_readable_seq,
            snapshot_pin_count: self.snapshot_pins.len(),
            auto_reclaim: self.auto_reclaim,
            write_stall_count: self.write_stall_count,
            write_pressure_count: self.write_pressure_count,
            write_stall_l0: self.write_stall_l0.unwrap_or(0) as u64,
            write_stall_mem_bytes: self.write_stall_mem_bytes.unwrap_or(0) as u64,
            write_pressure_l0: self.write_pressure_l0.unwrap_or(0) as u64,
            l0_files: self.level_file_count(0) as u64,
            changelog_interval: self.changelog_interval,
            changelog_store_count: self.changelog_store_count,
        }
    }

    /// `(vlog_bytes, live_bytes, live_records)` for observability.
    fn vlog_size_stats(&self) -> (u64, u64, u64) {
        let mut vlog_bytes = if let Some(ref v) = self.vlog {
            v.lock().len_bytes()
        } else {
            let p = self.dir.join(VLOG_FILE_NAME);
            self.env.metadata_len(&p).unwrap_or(0)
        };
        for n in vlog::list_blob_nums(&self.env, &self.dir) {
            if n == self.blob_active {
                continue;
            }
            let p = vlog::blob_path(&self.dir, n);
            vlog_bytes = vlog_bytes.saturating_add(self.env.metadata_len(&p).unwrap_or(0));
        }
        if self.blob_active > 0 {
            let legacy = self.dir.join(VLOG_FILE_NAME);
            if self.env.exists(&legacy) {
                vlog_bytes = vlog_bytes.saturating_add(self.env.metadata_len(&legacy).unwrap_or(0));
            }
        }
        let mut live_bytes = 0u64;
        let mut seen: std::collections::HashSet<(u32, u64)> = std::collections::HashSet::new();
        let mut consider = |stored: &Bytes| {
            if let Some(ptr) = vlog::decode_vlog_ptr(stored.as_ref()) {
                if seen.insert((ptr.file_num, ptr.offset)) {
                    live_bytes = live_bytes.saturating_add(u64::from(ptr.len));
                }
            }
        };
        for (_, v) in self.mem.iter_internal() {
            consider(v);
        }
        if let Some(ref imm) = self.imm {
            for (_, v) in imm.iter_internal() {
                consider(v);
            }
        }
        for t in &self.ssts {
            for (_, v) in t.entries_cloned().unwrap_or_else(|e| {
                panic!(
                    "pedradb: corrupt SST in {} on vlog stats: {e}",
                    t.path().display()
                )
            }) {
                consider(&v);
            }
        }
        (vlog_bytes, live_bytes, seen.len() as u64)
    }

    /// Re-validate on-disk integrity of live SSTs + MANIFEST (fail-stop on CRC/format).
    ///
    /// RocksDB/Redwood-class ops primitive: detect bitrot before relying on reads.
    ///
    /// # Errors
    /// Corrupt SST/MANIFEST or I/O.
    pub fn verify_checksums(&self) -> Result<()> {
        if let Some(vs) = manifest::load(&self.env, &self.dir)? {
            if vs.sst_file_nums.len() != self.ssts.len() {
                return Err(CoreError::CorruptManifest(format!(
                    "in-memory SST count {} != MANIFEST {}",
                    self.ssts.len(),
                    vs.sst_file_nums.len()
                )));
            }
        }
        for table in &self.ssts {
            // F217: read the file from disk, not the table cache — the cache
            // is primed by the very paths that install each table (flush via
            // `apply_l0_install`, compact, blob rewrite, vlog GC), so a cache
            // hit would serve the decoded in-memory bytes and never see
            // in-process bitrot on a live file.
            let re = SstTable::open_on(&self.env, table.path())?;
            if re.len() != table.len() {
                return Err(CoreError::Internal(format!(
                    "SST {} entry count drift after re-open",
                    table.path().display()
                )));
            }
        }
        let wal_path = self.dir.join(WAL_FILE_NAME);
        if self.env.exists(&wal_path) {
            // Full WAL recover checks record CRCs without applying.
            let _ = Wal::recover_on(&self.env, &wal_path)?;
        }
        Ok(())
    }

    /// RFC-0060 at-rest scrub over this DB's directory (same walk as
    /// [`crate::verify_at_rest`] / `pedra verify`).
    #[must_use]
    pub fn verify_at_rest(&self) -> crate::VerifyReport {
        crate::verify_at_rest(&self.env, &self.dir)
    }

    /// Create a point-in-time checkpoint under `dest` (RocksDB Checkpoint class).
    ///
    /// Flushes the MemTable first so the checkpoint is self-contained: live SSTs
    /// + empty/rotated WAL + CURRENT/MANIFEST + [`CHECKPOINT_META_FILE`].
    ///
    /// `dest` must not exist, or must be an empty directory.
    ///
    /// # Errors
    /// I/O, non-empty dest, or flush/manifest failures.
    pub fn create_checkpoint(&mut self, dest: impl AsRef<Path>) -> Result<CheckpointMeta> {
        self.flush()?;
        let dest = dest.as_ref();
        if self.env.exists(dest) {
            let names = self.env.read_dir_names(dest)?;
            if !names.is_empty() {
                return Err(CoreError::Internal(format!(
                    "checkpoint destination not empty: {}",
                    dest.display()
                )));
            }
        } else {
            self.env.create_dir_all(dest)?;
        }

        // Copy live inventory files.
        let current = self.dir.join(manifest::CURRENT_FILE);
        if self.env.exists(&current) {
            self.env
                .copy_file(&current, &dest.join(manifest::CURRENT_FILE))?;
        }
        // Active MANIFEST name from CURRENT contents, or copy all MANIFEST-*.
        for name in self.env.read_dir_names(&self.dir)? {
            // Inventory files only (`MANIFEST-000001`); skip `MANIFEST-*.tmp` install temps.
            if name.starts_with(manifest::MANIFEST_PREFIX)
                && !name
                    .rsplit_once('.')
                    .is_some_and(|(_, e)| e.eq_ignore_ascii_case("tmp"))
            {
                self.env
                    .copy_file(&self.dir.join(&name), &dest.join(&name))?;
            }
        }
        for table in &self.ssts {
            let name = table
                .path()
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| CoreError::Internal("sst path missing name".into()))?;
            self.env.copy_file(table.path(), &dest.join(name))?;
        }
        let wal_src = self.dir.join(WAL_FILE_NAME);
        if self.env.exists(&wal_src) {
            // F206: drain the WAL handle under its mutex before copying — a
            // write group parked between `begin_commit` and `end_commit`
            // holds an acked key's only durable bytes out of the file, and
            // completed async groups buffer up to 64 KiB in userspace. Taking
            // the lock waits out the former; `flush` pushes the latter, so
            // the copy carries every commit acked before the checkpoint.
            self.wal.lock().flush()?;
            self.env.copy_file(&wal_src, &dest.join(WAL_FILE_NAME))?;
        }
        // Large-value spill (RFC-0014 P2.2): SST/WAL may hold only VLG1 pointers.
        // F44: mid-GC MANIFEST may set `vlog_use_new` with live data in VALUES.vlog.new
        // and remapped SST offsets. Copying only the primary file leaves open falling
        // back to stale primary bytes → missing/wrong large values after restore.
        let vlog_src = self.dir.join(VLOG_FILE_NAME);
        if self.env.exists(&vlog_src) {
            self.env.copy_file(&vlog_src, &dest.join(VLOG_FILE_NAME))?;
        }
        for num in vlog::list_blob_nums(&self.env, &self.dir) {
            let src = vlog::blob_path(&self.dir, num);
            let name = src
                .file_name()
                .ok_or_else(|| CoreError::Internal("blob path missing file name".into()))?;
            self.env.copy_file(&src, &dest.join(name))?;
        }
        let vlog_new_src = self.dir.join(crate::vlog::VLOG_NEW_NAME);
        if self.vlog_use_new && self.env.exists(&vlog_new_src) {
            self.env
                .copy_file(&vlog_new_src, &dest.join(crate::vlog::VLOG_NEW_NAME))?;
        }
        // Adopt marker (if present) so open prefers the same vlog generation.
        let adopt = self.dir.join(crate::vlog::VLOG_ADOPT_NAME);
        if self.env.exists(&adopt) {
            self.env
                .copy_file(&adopt, &dest.join(crate::vlog::VLOG_ADOPT_NAME))?;
        }
        // F46: CHANGELOG is the durable change-feed cache. After flush the WAL is
        // empty/rotated — omit CHANGELOG from the checkpoint → silent feed loss.
        // RFC-0031: force a store so a checkpoint mid-debounce still copies the feed.
        self.persist_changelog_best_effort();
        let chlog = self.dir.join(crate::change_feed::CHANGELOG_FILE_NAME);
        if self.env.exists(&chlog) {
            self.env
                .copy_file(&chlog, &dest.join(crate::change_feed::CHANGELOG_FILE_NAME))?;
        }

        // F175: after horizon GC, archived versions exist only in `history/`
        // (tier MANIFEST + `seg-*.hist` + `seg-*.bloom` sidecars) — the read
        // path serves them via the archive fallback. A checkpoint that omits
        // the tier restores a copy that answers `SnapshotTooOld` where the
        // source served archived data.
        let hist = self.dir.join("history");
        if self.env.is_dir(&hist).unwrap_or(false) {
            let dest_hist = dest.join("history");
            self.env.create_dir_all(&dest_hist)?;
            for name in self.env.read_dir_names(&hist)? {
                self.env
                    .copy_file(&hist.join(&name), &dest_hist.join(&name))?;
            }
        }

        let meta = CheckpointMeta {
            last_sequence: self.last_sequence(),
            sst_count: self.ssts.len(),
            earliest_readable_seq: self.earliest_readable_seq,
        };
        write_checkpoint_meta(&self.env, dest, &meta)?;
        self.sync_dir_if_required(dest)?;
        Ok(meta)
    }

    /// Flush MemTable(s) to L0 SST(s) using dual-memtable switch (pipeline).
    ///
    /// Active mem is swapped to immutable; new writes go to a fresh mem while
    /// imm is written to SST. WAL is rotated only when mem, imm, **and** the
    /// off-lock flush read pin are empty (so a concurrent checkpoint cannot
    /// copy a truncated WAL while acked keys live only in the pin).
    ///
    /// # Errors
    /// I/O while writing SST or recreating the WAL.
    pub fn flush(&mut self) -> Result<()> {
        self.ensure_not_fenced()?;
        if let Some(persist) = self.flush_all_bulk_runs()? {
            persist.write()?;
        }
        self.vlog_sync_pending()?;
        crate::buggify_hooks::inject_checked(crate::buggify_hooks::sites::BEFORE_SST_RENAME)?;
        crate::buggify_hooks::inject_checked(crate::buggify_hooks::sites::BEFORE_MANIFEST_RENAME)?;
        // Flush plan decided by the pure kernel (RFC-0056 P0.2): the mem
        // tail is always written to an SST before any WAL rotate.
        match crate::flush_kernel::flush_plan(self.mem.is_empty(), self.imm.is_some()) {
            crate::flush_kernel::FlushPlan::FinishImmThenFlush
            | crate::flush_kernel::FlushPlan::WriteSstThenRotate => {
                // Finish any in-flight imm first (single-flight).
                if self.imm.is_some() {
                    self.flush_imm_to_l0()?;
                }
                if self.mem.is_empty() {
                    self.try_rotate_wal()?;
                    return Ok(());
                }
                // Switch: active → imm; new empty active (writers can continue after return
                // on ConcurrentDb once this returns; single-threaded Db flushes imm next).
                self.imm = Some(std::mem::replace(&mut self.mem, MemTable::new()));
                self.flush_imm_to_l0()?;
                self.finish_flush_pipeline()?;
            }
            crate::flush_kernel::FlushPlan::RotateOnly => {
                self.try_rotate_wal()?;
                return Ok(());
            }
        }
        // Explicit flush: persist the cache even when interval is 0 (WAL gone).
        if self.changelog_interval == 0 {
            self.persist_changelog_best_effort();
        }
        Ok(())
    }

    /// RFC-0159 P0.2: family key of a table for bulk routing. Matches
    /// `family_of_user_key` ("default" when no physical CFs are
    /// registered) so observation and install agree on family identity.
    pub(crate) fn bulk_family_of_table<'a>(&self, table: &'a SstTable) -> &'a str {
        if self.physical_cfs.is_empty() {
            "default"
        } else {
            table.cf()
        }
    }

    /// Largest user key `family` holds on disk or in any memtable layer —
    /// the `family_max_in_db` input for the latch's **first** observation
    /// of a family (everything committed after open is observed by the
    /// latch itself). Free-standing so the borrow checker sees disjoint
    /// field access next to `&mut self.bulk_latch`.
    fn bulk_family_max_in_db_parts(
        ssts: &[SstTable],
        physical_empty: bool,
        mems: &[&MemTable],
        family: &str,
    ) -> Option<Bytes> {
        let mut max: Option<Bytes> = None;
        let bump = |max: &mut Option<Bytes>, k: &[u8]| {
            if max.as_deref().map_or(true, |m: &[u8]| k > m) {
                *max = Some(Bytes::copy_from_slice(k));
            }
        };
        for t in ssts {
            let matches = if physical_empty {
                family == "default"
            } else {
                t.cf() == family
            };
            if matches {
                if let Some(k) = t.largest_user_key() {
                    bump(&mut max, k);
                }
            }
        }
        for m in mems {
            if let Some(k) = m.max_user_key_in_family(family) {
                bump(&mut max, k.as_ref());
            }
        }
        max
    }

    /// Observe one committed batch through the sorted-ingest latch.
    /// Every write funnel calls this exactly once per batch (the
    /// memtable-apply sites are NOT the choke point — recovery replay
    /// must stay unobserved so `family_max_in_db` covers it instead).
    fn observe_bulk_batch(&mut self, batch: &[BatchOp]) {
        if batch.is_empty() || !self.bulk_route_enabled {
            return;
        }
        // Field-borrowing family resolver (a `&self` method would borrow
        // `bulk_latch` into the ops vec): mirrors `family_of_user_key`.
        let physical = &self.physical_cfs;
        let fam_of = |key: &[u8]| -> &str {
            if physical.is_empty() {
                return "default";
            }
            let p = crate::memtable::cf_prefix(key);
            if p.is_empty() {
                return "default";
            }
            physical
                .iter()
                .find(|n| n.as_bytes() == p)
                .map(String::as_str)
                .unwrap_or("default")
        };
        // Raftlog / pipeline: one family, all puts. Skip the HashMap
        // classify_batch and the memtable-chain collect after the family's
        // first observation (high-water already covers it).
        if let Some(family) = Self::single_put_family(batch, &fam_of) {
            const STACK: usize = 32;
            let mut stack = [(false, &[] as &[u8]); STACK];
            let mut n = 0usize;
            let mut heap: Vec<(bool, &[u8])> = Vec::new();
            for op in batch {
                if let BatchOp::Put { key, .. } = op {
                    let item = (true, key.as_ref());
                    if n < STACK && heap.is_empty() {
                        stack[n] = item;
                        n += 1;
                    } else {
                        if heap.is_empty() {
                            heap.extend_from_slice(&stack[..n]);
                        }
                        heap.push(item);
                    }
                }
            }
            let keys: &[(bool, &[u8])] = if heap.is_empty() { &stack[..n] } else { &heap };
            if self.bulk_latch.has_high_water(family) {
                let _ = self
                    .bulk_latch
                    .classify_family(family, keys, false, || None);
            } else {
                let ssts = &self.ssts;
                let physical_empty = self.physical_cfs.is_empty();
                let mems: Vec<&MemTable> = std::iter::once(&self.mem)
                    .chain(self.imm.as_ref())
                    .chain(self.flush_read_pin.as_ref())
                    .chain(self.parked_unflushed.iter().map(|t| t.as_ref()))
                    .collect();
                let _ = self.bulk_latch.classify_family(family, keys, false, || {
                    Self::bulk_family_max_in_db_parts(ssts, physical_empty, &mems, family)
                });
            }
            return;
        }
        let ops: Vec<crate::bulk_ingest::BulkOp> = batch
            .iter()
            .map(|op| match op {
                BatchOp::Put { key, .. } => crate::bulk_ingest::BulkOp::Put {
                    family: fam_of(key.as_ref()),
                    key: key.as_ref(),
                },
                BatchOp::Delete { key } => crate::bulk_ingest::BulkOp::Delete {
                    family: fam_of(key.as_ref()),
                    key: key.as_ref(),
                },
                BatchOp::DeleteRange { start, end } => crate::bulk_ingest::BulkOp::DeleteRange {
                    start_family: fam_of(start.as_ref()),
                    start: start.as_ref(),
                    end_family: fam_of(end.as_ref()),
                    end: end.as_ref(),
                },
            })
            .collect();
        let ssts = &self.ssts;
        let physical_empty = self.physical_cfs.is_empty();
        // Field-direct memtable chain (not `scan_mem_layers`, whose `&self`
        // receiver would borrow `bulk_latch` too): disjoint-field borrows
        // let the latch mutate next to these.
        let mems: Vec<&MemTable> = std::iter::once(&self.mem)
            .chain(self.imm.as_ref())
            .chain(self.flush_read_pin.as_ref())
            .chain(self.parked_unflushed.iter().map(|t| t.as_ref()))
            .collect();
        let _routes = self.bulk_latch.classify_batch(&ops, &|f| {
            Self::bulk_family_max_in_db_parts(ssts, physical_empty, &mems, f)
        });
    }

    fn single_put_family<'a>(
        batch: &'a [BatchOp],
        fam_of: &dyn Fn(&[u8]) -> &'a str,
    ) -> Option<&'a str> {
        let mut family = None;
        for op in batch {
            match op {
                BatchOp::Put { key, .. } => {
                    let f = fam_of(key.as_ref());
                    match family {
                        None => family = Some(f),
                        Some(prev) if prev == f => {}
                        Some(_) => return None,
                    }
                }
                BatchOp::Delete { .. } | BatchOp::DeleteRange { .. } => return None,
            }
        }
        family
    }

    /// Single-op form of [`Self::observe_bulk_batch`] (iterator-based
    /// write funnels observe op-granular; the span-level ascending check
    /// at flush time is the real gate, so granularity loses nothing).
    fn observe_bulk_op(&mut self, op: &BatchOp) {
        if self.bulk_route_enabled {
            self.observe_bulk_batch(std::slice::from_ref(op));
        }
    }

    /// Staged-transaction form ([`crate::tx`]): keys arrive without a
    /// `BatchOp`; observe them so the high-water stays complete.
    pub(crate) fn observe_bulk_staged(&mut self, key: &[u8], is_put: bool) {
        if !self.bulk_route_enabled {
            return;
        }
        // Field-borrowing resolver (see `observe_bulk_batch`).
        let physical = &self.physical_cfs;
        let fam_of = |key: &[u8]| -> &str {
            if physical.is_empty() {
                return "default";
            }
            let p = crate::memtable::cf_prefix(key);
            if p.is_empty() {
                return "default";
            }
            physical
                .iter()
                .find(|n| n.as_bytes() == p)
                .map(String::as_str)
                .unwrap_or("default")
        };
        let family = fam_of(key);
        let op = if is_put {
            crate::bulk_ingest::BulkOp::Put { family, key }
        } else {
            crate::bulk_ingest::BulkOp::Delete { family, key }
        };
        let ssts = &self.ssts;
        let physical_empty = self.physical_cfs.is_empty();
        // Same field-direct chain as `observe_bulk_batch`.
        let mems: Vec<&MemTable> = std::iter::once(&self.mem)
            .chain(self.imm.as_ref())
            .chain(self.flush_read_pin.as_ref())
            .chain(self.parked_unflushed.iter().map(|t| t.as_ref()))
            .collect();
        let _routes = self.bulk_latch.classify_batch(&[op], &|f| {
            Self::bulk_family_max_in_db_parts(ssts, physical_empty, &mems, f)
        });
    }

    /// RFC-0159 P0.2: install level for one flushed family span.
    /// `MAX_LSM_LEVEL` only when every gate holds: the family is latched,
    /// the span is strictly-ascending puts with no point/range tombstones,
    /// and the span hull does not overlap the family's existing files at
    /// levels ≥ 1 (those levels would merge it back down; the max level is
    /// never a pushdown source, so a qualifying span is written exactly
    /// once). Anything else stays L0 — identical to the pre-bulk path.
    pub(crate) fn bulk_span_level(&self, family: &str, mem: &MemTable) -> u32 {
        if !self.bulk_route_enabled || !self.bulk_latch.is_latched(family) {
            return 0;
        }
        if mem.has_range_tombstones() {
            return 0;
        }
        // RFC-0159 P1.4: incremental per-prefix span state — one map lookup
        // instead of a full parked-table rescan per output file (run #33:
        // 4.42 s of the 4.7 s install stage at 25M). Absorbed tables and
        // exotic family names keep the legacy scan below.
        let (lo, hi) = match mem.bulk_span(family) {
            crate::memtable::BulkSpan::Absent | crate::memtable::BulkSpan::Impure => return 0,
            crate::memtable::BulkSpan::Unknown => return self.bulk_span_level_scan(family, mem),
            crate::memtable::BulkSpan::Pure { lo, hi } => (lo, hi),
        };
        for (t, &lvl) in self.ssts.iter().zip(self.sst_levels.iter()) {
            if lvl == 0 || self.bulk_family_of_table(t) != family {
                continue;
            }
            let (Some(tlo), Some(thi)) = (t.smallest_user_key(), t.largest_user_key()) else {
                continue;
            };
            if tlo <= hi.as_ref() && thi >= lo.as_ref() {
                return 0; // would stack over an existing lower-level file
            }
        }
        MAX_LSM_LEVEL
    }

    /// Legacy whole-memtable scan for [`Self::bulk_span_level`] — fallback
    /// when the incremental span state is not tracked (absorbed tables).
    fn bulk_span_level_scan(&self, family: &str, mem: &MemTable) -> u32 {
        let mut prev: Option<&[u8]> = None;
        let mut lo: Option<&[u8]> = None;
        let mut hi: &[u8] = &[];
        for (ik, _) in mem.iter_internal() {
            if !crate::cf_kernel::key_in_cf_family(ik.user_key.as_ref(), family) {
                continue;
            }
            if ik.kind != crate::key::ValueType::Value {
                return 0; // tombstone in the span: ladder
            }
            let uk = ik.user_key.as_ref();
            if let Some(p) = prev {
                if uk <= p {
                    return 0; // duplicate / descent: not a pure append span
                }
            }
            prev = Some(uk);
            if lo.is_none() {
                lo = Some(uk);
            }
            hi = uk;
        }
        let Some(lo) = lo else {
            return 0; // family absent from this span
        };
        for (t, &lvl) in self.ssts.iter().zip(self.sst_levels.iter()) {
            if lvl == 0 || self.bulk_family_of_table(t) != family {
                continue;
            }
            let (Some(tlo), Some(thi)) = (t.smallest_user_key(), t.largest_user_key()) else {
                continue;
            };
            if tlo <= hi && thi >= lo {
                return 0; // would stack over an existing lower-level file
            }
        }
        MAX_LSM_LEVEL
    }

    /// Flush only `family` to L0 (RFC-0065 P1.1). Other families stay in mem.
    ///
    /// # Errors
    /// SST / MANIFEST I/O.
    pub fn flush_cf(&mut self, family: &str) -> Result<()> {
        self.ensure_not_fenced()?;
        let mut taken = self.mem.take_family(family);
        if let Some(ref mut imm) = self.imm {
            taken.absorb(imm.take_family(family));
            if imm.is_empty() {
                self.imm = None;
            }
        }
        if taken.is_empty() {
            return Ok(());
        }
        let nums = vec![self.alloc_file_num()];
        let files = match Self::write_imm_l0_files(&self.env, &self.dir, self.sync, &taken, &nums) {
            Ok(f) => f,
            Err(e) => {
                for (k, v) in taken.iter_internal() {
                    self.mem.insert(k.clone(), v.clone());
                }
                return Err(self.fence_io_err(e));
            }
        };
        // RFC-0159 P0.2: a latched family's pure-append span installs
        // directly at the bottom level (written once, never re-laddered).
        let level = self.bulk_span_level(family, &taken);
        let pairs: Vec<_> = files.into_iter().map(|(t, num, _)| (t, num)).collect();
        if let Err(e) = self.install_ssts_at_levels(pairs, &[level]) {
            for (k, v) in taken.iter_internal() {
                self.mem.insert(k.clone(), v.clone());
            }
            return Err(self.fence_io_err(e));
        }
        if level != 0 {
            self.bulk_diag("install_cf", family, level);
        }
        Ok(())
    }

    /// `PEDRA_BULK_DIAG` line for a bulk install decision.
    pub(crate) fn bulk_diag(&self, tag: &str, family: &str, level: u32) {
        if std::env::var_os("PEDRA_BULK_DIAG").is_some() {
            eprintln!(
                "BULKDIAG {tag} family={family} level={level} ssts={} l0={} max={}",
                self.ssts.len(),
                self.level_file_count(0),
                self.level_file_count(MAX_LSM_LEVEL)
            );
        }
    }

    fn bulk_family_of_key(&self, key: &[u8]) -> &str {
        if self.physical_cfs.is_empty() {
            return "default";
        }
        let p = crate::memtable::cf_prefix(key);
        if p.is_empty() {
            return "default";
        }
        self.physical_cfs
            .iter()
            .find(|n| n.as_bytes() == p)
            .map_or("default", String::as_str)
    }

    /// Sorted-ingest latch is live for `family` (RFC-0159).
    pub(crate) fn family_is_latched(&self, family: &str) -> bool {
        self.bulk_route_enabled && self.bulk_latch.is_latched(family)
    }

    /// Latched-family puts (already encoded) plus an optional ladder tail
    /// (hydrate: 1024 data + 1 meta cursor). No `BatchOp` / WAL for the
    /// latched span. Descent kills the latch and falls back to
    /// [`Self::commit_async_ops`].
    pub(crate) fn apply_latched_bulk_puts(
        &mut self,
        family: &str,
        keys: Vec<Bytes>,
        vals: Vec<Bytes>,
        tail: Vec<BatchOp>,
    ) -> Result<SequenceNumber> {
        if keys.len() != vals.len() {
            return Err(CoreError::Internal(
                "latched bulk keys/values length mismatch".into(),
            ));
        }
        if !self.write_admission_idle() {
            let fams = [family.to_string()];
            self.ensure_write_admitted_for(&fams)?;
        }
        if keys.is_empty() {
            return if tail.is_empty() {
                Ok(self.last_sequence())
            } else {
                self.commit_async_ops(tail)
            };
        }
        if !self.bulk_route_enabled || !self.bulk_latch.is_latched(family) {
            return self.commit_async_ops(Self::latched_to_ops(keys, vals, tail));
        }
        let route = self.bulk_latch.observe_latched_span(family, &keys);
        if route != crate::bulk_ingest::FamilyRoute::Bulk {
            return self.commit_async_ops(Self::latched_to_ops(keys, vals, tail));
        }
        if !self.bulk_runs.contains_key(family) {
            self.flush_dead_bulk_runs()?;
            self.absorb_mem_family_into_run(family)?;
        }
        self.bulk_append_puts(family, keys, vals)?;
        if tail.is_empty() {
            let seq = self.last_sequence();
            self.publish_sequence(seq);
            return Ok(seq);
        }
        // Hydrate's extra op is a 1-key meta cursor, overwritten every
        // batch. WAL of 24k versions of the same key is envelope the
        // data path already skipped (disableWAL class). Memtable holds
        // the live value; flush/settle persists it.
        if tail.len() == 1 {
            match tail.into_iter().next().unwrap() {
                BatchOp::Put { key, value } => {
                    let seq = self.alloc_seq()?;
                    self.mem
                        .insert(InternalKey::new(key, seq, ValueType::Value), value);
                    self.publish_sequence(seq);
                    return Ok(seq);
                }
                other => return self.commit_async_ops(vec![other]),
            }
        }
        self.commit_async_ops(tail)
    }

    fn latched_to_ops(keys: Vec<Bytes>, vals: Vec<Bytes>, tail: Vec<BatchOp>) -> Vec<BatchOp> {
        let mut ops = Vec::with_capacity(keys.len() + tail.len());
        ops.extend(
            keys.into_iter()
                .zip(vals)
                .map(|(key, value)| BatchOp::Put { key, value }),
        );
        ops.extend(tail);
        ops
    }

    fn flush_dead_bulk_runs(&mut self) -> Result<()> {
        let dead: Vec<String> = self
            .bulk_runs
            .keys()
            .filter(|f| !self.bulk_latch.is_latched(f))
            .cloned()
            .collect();
        for f in dead {
            self.flush_bulk_run(&f)?;
        }
        Ok(())
    }

    pub(crate) fn flush_all_bulk_runs(&mut self) -> Result<Option<ManifestPersist<E>>> {
        while let Some((fam, run)) = self.parked_bulk.pop_front() {
            self.install_bulk_run(&fam, run.as_ref())?;
        }
        let fams: Vec<String> = self.bulk_runs.keys().cloned().collect();
        for f in fams {
            self.flush_bulk_run(&f)?;
        }
        self.persist_bulk_manifest(true)
    }

    #[must_use]
    pub(crate) fn has_parked_bulk(&self) -> bool {
        !self.parked_bulk.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn bulk_manifest_debt(&self) -> u8 {
        self.bulk_manifest_debt
    }

    #[cfg(test)]
    pub(crate) fn bulk_latch_is_latched(&self, family: &str) -> bool {
        self.bulk_latch.is_latched(family)
    }

    /// Pop one parked bulk chunk for off-lock SST write. Pins it in
    /// `bulk_encoding` so get still hits until [`Self::finish_bulk_sst`].
    pub(crate) fn pop_parked_bulk_job(
        &mut self,
    ) -> Option<(String, Arc<crate::bulk_run::BulkRun>, u64, PathBuf, E, bool)> {
        let (fam, run) = self.parked_bulk.pop_front()?;
        self.bulk_encoding = Some((fam.clone(), Arc::clone(&run)));
        let num = self.alloc_file_num();
        let final_path = self.dir.join(format!("{num:06}.sst"));
        let (env, _, sync) = self.l0_write_ctx();
        Some((fam, run, num, final_path, env, sync))
    }

    pub(crate) fn take_bulk_encoding(&mut self) -> Option<(String, Arc<crate::bulk_run::BulkRun>)> {
        self.bulk_encoding.take()
    }

    pub(crate) fn push_parked_bulk_front(&mut self, pin: (String, Arc<crate::bulk_run::BulkRun>)) {
        self.parked_bulk.push_front(pin);
    }

    pub(crate) fn finish_bulk_sst(
        &mut self,
        family: &str,
        table: SstTable,
        num: u64,
    ) -> Result<Option<ManifestPersist<E>>> {
        if let Err(e) = self.install_ssts_at_levels(vec![(table, num)], &[MAX_LSM_LEVEL]) {
            return Err(self.fence_io_err(e));
        }
        if self.sst_source.is_some() {
            if let Some(t) = self.ssts.last() {
                t.release_resident();
            }
        }
        if self
            .bulk_encoding
            .as_ref()
            .is_some_and(|(f, _)| f == family)
        {
            self.bulk_encoding = None;
        }
        self.bulk_diag("run_install", family, MAX_LSM_LEVEL);
        self.persist_bulk_manifest(false)
    }

    /// Persist MANIFEST every [`BULK_MANIFEST_EVERY`] async bulk installs
    /// (off the write lock). `force` flushes leftover debt (settle).
    fn persist_bulk_manifest(&mut self, force: bool) -> Result<Option<ManifestPersist<E>>> {
        if self.sync {
            self.persist_manifest()?;
            self.bulk_manifest_debt = 0;
            return Ok(None);
        }
        self.unsynced_ssts.clear();
        if force {
            if self.bulk_manifest_debt == 0 {
                return Ok(None);
            }
        } else {
            self.bulk_manifest_debt = self.bulk_manifest_debt.saturating_add(1);
            if self.bulk_manifest_debt < BULK_MANIFEST_EVERY {
                return Ok(None);
            }
        }
        self.bulk_manifest_debt = 0;
        Ok(Some(self.take_manifest_persist()?))
    }

    fn flush_bulk_run(&mut self, family: &str) -> Result<()> {
        let Some(run) = self.bulk_runs.remove(family) else {
            return Ok(());
        };
        self.install_bulk_run(family, &run)
    }

    fn install_bulk_run(&mut self, family: &str, run: &crate::bulk_run::BulkRun) -> Result<()> {
        if run.is_empty() {
            return Ok(());
        }
        let num = self.alloc_file_num();
        let (env, dir, sync) = self.l0_write_ctx();
        let (table, num) =
            match Self::write_bulk_run_sst(&env, &dir, num, &run, family, sync, "inline") {
                Ok(t) => t,
                Err(e) => return Err(self.fence_io_err(e)),
            };
        if let Some(persist) = self.finish_bulk_sst(family, table, num)? {
            persist.write()?;
        }
        Ok(())
    }

    /// Off-lock SST write for a parked bulk chunk (no `Db` borrow).
    /// `caller` labels the RFC-0162 BSTAGE line ("worker" vs "inline").
    pub(crate) fn write_bulk_run_sst(
        env: &E,
        dir: &std::path::Path,
        num: u64,
        run: &crate::bulk_run::BulkRun,
        family: &str,
        sync: bool,
        caller: &'static str,
    ) -> Result<(SstTable, u64)> {
        if run.is_empty() {
            return Err(CoreError::Internal("empty bulk run".into()));
        }
        let t0 = bulk_stage_timing_on().then(std::time::Instant::now);
        let final_path = dir.join(format!("{num:06}.sst"));
        let tmp_path = dir.join(format!("{num:06}.sst.tmp"));
        let table = match write_sst_bulk_arrays(env, &tmp_path, run.keys(), run.vals(), run.seqs())
        {
            Ok(t) => t,
            Err(e) => {
                let _ = env.remove_file(&tmp_path);
                return Err(e);
            }
        };
        if let Err(e) = env.rename(&tmp_path, &final_path) {
            let _ = env.remove_file(&tmp_path);
            let _ = env.remove_file(&final_path);
            return Err(CoreError::Io(e));
        }
        // RFC-0162 P1.1 (H1): chunk is synced — drop its pages so writeback
        // debt never throttles later chunks. Best-effort by the advise contract.
        let _ = env.advise(&final_path, 0, 0, AdviseKind::DontNeed);
        if let Some(t0) = t0 {
            let epoch = BSTAGE_EPOCH.get_or_init(std::time::Instant::now);
            eprintln!(
                "{}",
                bulk_stage_line(
                    BSTAGE_IDX.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                    epoch.elapsed().as_millis(),
                    t0.elapsed().as_millis(),
                    run.len(),
                    run.bytes(),
                    caller,
                    sync,
                )
            );
        }
        Ok((table.with_path(final_path).with_cf(family.to_string()), num))
    }

    fn absorb_mem_family_into_run(&mut self, family: &str) -> Result<()> {
        let taken = self.mem.take_family(family);
        if taken.is_empty() {
            return Ok(());
        }
        let run = self.bulk_runs.entry(family.to_string()).or_default();
        for (ik, v) in taken.iter_internal() {
            if ik.kind != ValueType::Value {
                continue;
            }
            run.push(ik.user_key.clone(), v.clone(), ik.sequence);
        }
        Ok(())
    }

    fn bulk_append_puts(
        &mut self,
        family: &str,
        mut keys: Vec<Bytes>,
        mut vals: Vec<Bytes>,
    ) -> Result<()> {
        let n = keys.len();
        if n == 0 {
            return Ok(());
        }
        crate::bulk_run::sort_bulk_key_vals(&mut keys, &mut vals);
        let n64 = n as u64;
        let last = self.next_seq.saturating_add(n64.saturating_sub(1));
        if last > MAX_SEQUENCE_NUMBER {
            return Err(CoreError::Internal(
                "sequence number space exhausted".into(),
            ));
        }
        let mut seq = self.next_seq;
        self.next_seq = last + 1;
        let cap = self.bulk_chunk_cap();
        let over = {
            let run = self.bulk_runs.entry(family.to_string()).or_default();
            run.reserve(n);
            for (k, v) in keys.into_iter().zip(vals) {
                self.bytes_ingested = self.bytes_ingested.saturating_add(v.len() as u64);
                run.push(k, v, seq);
                seq += 1;
            }
            run.bytes() >= cap
        };
        if over {
            if let Some(run) = self.bulk_runs.remove(family) {
                // Park even while the worker is encoding the previous
                // chunk so fill overlaps SST. One parked + one encoding
                // + the open tail is the RAM bound; a second overflow
                // while parked is still full encodes inline.
                if self.parked_bulk.is_empty() {
                    self.parked_bulk
                        .push_back((family.to_string(), Arc::new(run)));
                } else {
                    self.install_bulk_run(family, &run)?;
                }
            }
        }
        Ok(())
    }

    /// Rotate a full active mem into the imm slot **without** taking it out.
    ///
    /// Host workers (RFC-0037) stage here so `has_imm` stays true until
    /// [`Self::prepare_flush_imm`]. Does nothing when imm is already occupied
    /// (worker behind) — active mem may grow until the slot frees.
    ///
    /// # Errors
    /// [`CoreError::DurabilityFenced`].
    pub fn stage_flush_imm(&mut self) -> Result<bool> {
        self.ensure_not_fenced()?;
        if self.imm.is_some() || self.mem.is_empty() {
            return Ok(false);
        }
        self.imm = Some(std::mem::replace(&mut self.mem, MemTable::new()));
        Ok(true)
    }

    /// Switch active mem → imm if free; returns taken imm for out-of-lock SST write.
    ///
    /// Used by [`ConcurrentDb`] to release the write lock during SST I/O.
    ///
    /// # Errors
    /// [`CoreError::DurabilityFenced`].
    pub fn prepare_flush_imm(&mut self) -> Result<Option<MemTable>> {
        self.ensure_not_fenced()?;
        let taken = if self.imm.is_some() {
            // Still flushing previous imm — caller should finish that first.
            self.imm.take()
        } else if self.mem.is_empty() {
            None
        } else {
            Some(std::mem::replace(&mut self.mem, MemTable::new()))
        };
        // Keep a read pin so get/scan still see acked keys during off-lock SST I/O.
        if let Some(table) = taken {
            self.flush_read_pin = Some(table.clone());
            Ok(Some(table))
        } else {
            Ok(None)
        }
    }

    /// Drop the off-lock flush read pin (after a test wants the pre-fix hole).
    pub fn clear_flush_read_pin(&mut self) {
        self.flush_read_pin = None;
    }

    /// Park the flush pin for later off-lock fold (no BTree merge here).
    pub fn retire_flush_pin(&mut self) {
        if let Some(pin) = self.flush_read_pin.take() {
            if !pin.is_empty() {
                self.retired_pending.push(pin);
                self.retired_l0s = self.retired_l0s.saturating_add(1);
                self.trim_retired_cache();
            }
        }
    }

    /// Take pending pins so the host can fold them without the write lock.
    pub fn take_retired_pending(&mut self) -> Vec<MemTable> {
        std::mem::take(&mut self.retired_pending)
    }

    /// Install a fold built off-lock (union of pending pins). The union is
    /// one undividable cache layer: over cap it is dropped whole — reads
    /// fall back to the covered SSTs.
    pub fn install_retired_fold(&mut self, built: MemTable) {
        if self.retired_fold.is_empty() {
            self.retired_fold = built;
        } else {
            self.retired_fold.absorb(built);
        }
        if self.retired_fold.approx_memory_usage() > self.retired_cache_cap() {
            self.retired_fold = MemTable::new();
        }
    }

    /// How many L0 files the retired cache covers (tests / probes).
    #[must_use]
    pub fn retired_mem_count(&self) -> usize {
        self.retired_l0s
    }

    /// Rotate WAL even if [`Self::flush_read_pin`] is live (pre-fix hole).
    ///
    /// Production [`Self::try_rotate_wal`] must refuse while a pin holds the
    /// only copy of acked keys. Tests use this to replay the truncate.
    ///
    /// # Errors
    /// WAL create / close I/O.
    pub fn rotate_wal_ignoring_pin(&mut self) -> Result<()> {
        self.rotate_wal_now()
    }

    /// Reserve the next SST file number (must hold exclusive write lock).
    ///
    /// Call **before** off-lock SST I/O so concurrent flushes cannot race on
    /// the same `next_file_num` (F43).
    pub fn alloc_file_num(&mut self) -> u64 {
        let n = self.next_file_num;
        self.next_file_num = n.saturating_add(1);
        n
    }

    /// Remove a DB-owned file and drop any cached read handle for its path.
    ///
    /// Invalidation is part of the delete, not an optimization: a cached
    /// handle keeps an unlinked inode (and its disk space) alive, and a
    /// failed SST write rolls `next_file_num` back, so the path can be
    /// re-allocated with different bytes. Every removal of a file that was
    /// adopted (visible to reads) routes here; write-path cleanup of files
    /// that were never adopted never entered the cache.
    fn remove_db_file(&self, path: &Path) -> std::io::Result<()> {
        self.env.remove_file(path)?;
        self.sst_file_cache.invalidate(path);
        Ok(())
    }

    /// Reserve SST numbers for a flush of `imm` (RFC-0065 P0).
    ///
    /// One number when CFs are not registered; one per family otherwise.
    #[must_use]
    pub fn alloc_file_nums_for_imm(&mut self, imm: &crate::memtable::MemTable) -> Vec<u64> {
        let n = if self.physical_cfs.is_empty() {
            1
        } else {
            imm.cf_families().len().max(1)
        };
        (0..n).map(|_| self.alloc_file_num()).collect()
    }

    /// Snapshot of what off-lock L0 write needs (`Env` is [`Clone`]).
    #[must_use]
    pub fn l0_write_ctx(&self) -> (E, PathBuf, bool) {
        (self.env.clone(), self.dir.clone(), self.sync)
    }

    /// Write `imm` to `{num:06}.sst` without borrowing `Db` (caller drops the lock).
    ///
    /// Whole-memtable (no CF split). Prefer [`Self::write_imm_l0_files`] for
    /// the flush path.
    ///
    /// # Errors
    /// SST I/O.
    pub fn write_imm_l0_file(
        env: &E,
        dir: &Path,
        sync: bool,
        imm: &MemTable,
        num: u64,
    ) -> Result<(SstTable, u64, PathBuf)> {
        let final_path = dir.join(format!("{num:06}.sst"));
        let tmp_path = dir.join(format!("{num:06}.sst.tmp"));
        // L0 is not WAL-durable until rotate: skip file `fdatasync` here.
        // `sync` only dir-syncs after rename (used by tests that want the
        // name visible); the file bytes stay lazy.
        match write_l0_sst(env, &tmp_path, imm, false) {
            Ok(table) => {
                // PEDRA_PARK_DIAG2: rename off the tmp name (same dir).
                let t_r = std::time::Instant::now();
                env.rename(&tmp_path, &final_path)?;
                if std::env::var_os("PEDRA_PARK_DIAG2").is_some() {
                    eprintln!(
                        "PARKDIAG2 file rename_ms={:.1}",
                        t_r.elapsed().as_secs_f64() * 1e3
                    );
                }
                if sync {
                    env.sync_dir(dir)?;
                }
                // Keep the writer's in-place table (rename does not change
                // the bytes). Re-opening paid a full read + per-block
                // decompress + per-entry decode of every flushed file —
                // the caller-side half of the read-back the v21p writer
                // fix removed. Reopens at recovery still verify fully.
                Ok((table.with_path(final_path.clone()), num, final_path))
            }
            Err(e) => {
                let _ = env.remove_file(&tmp_path);
                let _ = env.remove_file(&final_path);
                Err(e)
            }
        }
    }

    /// Write one L0 SST containing only `family` keys. `None` if the family
    /// has no entries (no file left behind).
    ///
    /// # Errors
    /// SST I/O.
    pub fn write_imm_l0_file_for_family(
        env: &E,
        dir: &Path,
        sync: bool,
        imm: &MemTable,
        num: u64,
        family: &str,
    ) -> Result<Option<(SstTable, u64, PathBuf)>> {
        let final_path = dir.join(format!("{num:06}.sst"));
        let tmp_path = dir.join(format!("{num:06}.sst.tmp"));
        match write_l0_sst_for_family(env, &tmp_path, imm, family, false) {
            Ok(table) => {
                if table.is_empty() {
                    drop(table);
                    let _ = env.remove_file(&tmp_path);
                    return Ok(None);
                }
                // PEDRA_PARK_DIAG2: rename off the tmp name (same dir).
                let t_r = std::time::Instant::now();
                env.rename(&tmp_path, &final_path)?;
                if std::env::var_os("PEDRA_PARK_DIAG2").is_some() {
                    eprintln!(
                        "PARKDIAG2 file rename_ms={:.1}",
                        t_r.elapsed().as_secs_f64() * 1e3
                    );
                }
                if sync {
                    env.sync_dir(dir)?;
                }
                // In-place table kept (see `write_imm_l0_file`): no
                // post-rename re-read of the freshly written bytes.
                let table = table
                    .with_path(final_path.clone())
                    .with_cf(family.to_string());
                Ok(Some((table, num, final_path)))
            }
            Err(e) => {
                let _ = env.remove_file(&tmp_path);
                let _ = env.remove_file(&final_path);
                Err(e)
            }
        }
    }

    /// Split `imm` into one L0 SST per CF family (RFC-0065 P0).
    ///
    /// One reserved number ⇒ one SST (kernel). Several numbers ⇒ one file
    /// per CF family.
    ///
    /// # Errors
    /// SST I/O, or fewer file numbers than families.
    pub fn write_imm_l0_files(
        env: &E,
        dir: &Path,
        sync: bool,
        imm: &MemTable,
        nums: &[u64],
    ) -> Result<Vec<(SstTable, u64, PathBuf)>> {
        // PEDRA_FLUSH_DIAG: per-memtable SST encode+write wall time (the
        // drain the hydrate writer parks behind), regardless of caller.
        let t0 = std::env::var_os("PEDRA_FLUSH_DIAG").map(|_| Instant::now());
        let out = Self::write_imm_l0_files_inner(env, dir, sync, imm, nums);
        if let Some(t0) = t0 {
            println!("FLUSHDUR ms={}", t0.elapsed().as_millis());
        }
        out
    }

    fn write_imm_l0_files_inner(
        env: &E,
        dir: &Path,
        sync: bool,
        imm: &MemTable,
        nums: &[u64],
    ) -> Result<Vec<(SstTable, u64, PathBuf)>> {
        let families = if nums.len() > 1 {
            imm.cf_families()
        } else {
            Vec::new()
        };
        if families.is_empty() || nums.is_empty() {
            let num = nums.first().copied().unwrap_or(1);
            let one = Self::write_imm_l0_file(env, dir, sync, imm, num)?;
            return Ok(vec![one]);
        }
        if nums.len() < families.len() {
            return Err(CoreError::Internal(format!(
                "need {} SST file numbers for CF split, got {}",
                families.len(),
                nums.len()
            )));
        }
        let mut out = Vec::new();
        let mut written = Vec::new();
        for (fam, &num) in families.iter().zip(nums.iter()) {
            match Self::write_imm_l0_file_for_family(env, dir, sync, imm, num, fam) {
                Ok(Some(t)) => {
                    written.push(t.2.clone());
                    out.push(t);
                }
                Ok(None) => {}
                Err(e) => {
                    for p in &written {
                        let _ = env.remove_file(p);
                    }
                    return Err(e);
                }
            }
        }
        if out.is_empty() {
            let one = Self::write_imm_l0_file(env, dir, sync, imm, nums[0])?;
            return Ok(vec![one]);
        }
        Ok(out)
    }

    /// Write `imm` to L0 using a **pre-allocated** file number (no Db write lock).
    ///
    /// Prefer [`Self::alloc_file_num`] under the write lock, then this for I/O.
    /// Holding `&self` across this call (a read lock) **blocks writers** —
    /// use [`Self::l0_write_ctx`] + [`Self::write_imm_l0_file`] instead.
    ///
    /// # Errors
    /// SST I/O.
    pub fn write_memtable_to_l0_file_num(
        &self,
        imm: &MemTable,
        num: u64,
    ) -> Result<(SstTable, u64, PathBuf)> {
        Self::write_imm_l0_file(&self.env, &self.dir, self.sync, imm, num)
    }

    /// Write `imm` to a new L0 SST (exclusive path: peeks `next_file_num`, no bump).
    ///
    /// Concurrent callers must use [`Self::alloc_file_num`] +
    /// [`Self::write_memtable_to_l0_file_num`] instead.
    ///
    /// # Errors
    /// SST I/O.
    pub fn write_memtable_to_l0_file(&self, imm: &MemTable) -> Result<(SstTable, u64, PathBuf)> {
        self.write_memtable_to_l0_file_num(imm, self.next_file_num)
    }

    fn note_sst_bytes_written(&mut self, path: &Path) {
        if let Ok(len) = self.env.metadata_len(path) {
            self.bytes_written_sst = self.bytes_written_sst.saturating_add(len);
        }
    }

    /// Install a flushed L0 SST (MANIFEST before success).
    ///
    /// Does **not** clear [`Self::imm`]: the caller already took the imm via
    /// [`Self::prepare_flush_imm`] / `flush_imm_to_l0`. Clearing here would drop a
    /// concurrently restored or second pipeline imm (F45).
    ///
    /// If `file_num` was pre-allocated via [`Self::alloc_file_num`], `next_file_num`
    /// is already past it and is left unchanged. On exclusive paths that only peeked
    /// the number, advances `next_file_num` to `file_num + 1`.
    ///
    /// # Errors
    /// MANIFEST I/O (rolls back inventory).
    pub fn install_l0_sst(&mut self, table: SstTable, file_num: u64) -> Result<()> {
        self.install_l0_ssts(vec![(table, file_num)])
    }

    /// Install one or more flushed SSTs (MANIFEST before success).
    ///
    /// # Errors
    /// MANIFEST I/O (rolls back inventory).
    pub fn install_l0_ssts(&mut self, files: Vec<(SstTable, u64)>) -> Result<()> {
        self.install_ssts_at_levels(files, &[])
    }

    /// Level-explicit flush install (RFC-0159 P0.2): `levels[i]` is the
    /// level of `files[i]`; an empty / short slice defaults to L0.
    ///
    /// # Errors
    /// MANIFEST I/O (rolls back inventory).
    pub fn install_ssts_at_levels(
        &mut self,
        files: Vec<(SstTable, u64)>,
        levels: &[u32],
    ) -> Result<()> {
        // In-memory only. MANIFEST + SST `fdatasync` wait for WAL rotate so a
        // write burst is not charged one extra fd per 64 MiB flush (RFC-0041).
        let _undo = self.apply_sst_installs(files, levels);
        self.retire_flush_pin();
        Ok(())
    }

    /// Restore an imm memtable after a failed off-lock flush ([`crate::concurrent::ConcurrentDb`]).
    ///
    /// If another imm is already present (dual-flush race), fold this table's
    /// entries into the **active** mem so neither pipeline's data is dropped (F45).
    pub fn restore_imm(&mut self, imm: MemTable) {
        self.flush_read_pin = None;
        if self.imm.is_some() {
            for (k, v) in imm.iter_internal() {
                self.mem.insert(k.clone(), v.clone());
            }
            return;
        }
        self.imm = Some(imm);
    }

    /// Active mem empty and no imm/pin/parked (the WAL-pin half of
    /// `flush_kernel::wal_rotate_decision`, minus the in-flight-commit
    /// guard).
    #[must_use]
    pub fn mem_is_empty_for_rotate(&self) -> bool {
        let s = self.wal_pin_state();
        s.mem_empty && !s.imm_present && !s.pin_live && !s.parked_unflushed
    }

    /// Whether an immutable memtable is present.
    #[must_use]
    pub fn has_imm(&self) -> bool {
        self.imm.is_some()
    }

    /// Active memtable approximate bytes (host stages when this hits the flush cap).
    #[must_use]
    pub fn active_mem_usage(&self) -> usize {
        self.mem.approx_memory_usage()
    }

    /// Configured auto-flush threshold, if any.
    ///
    /// RFC-0159 P1.3: per-CF buffers raise the shared stage threshold the
    /// same way they raise the flush-debt cap — the host worker stages one
    /// shared active mem, so a per-CF buffer above the global cap must not
    /// be cut down to the global cap (bench: global 64 MiB, data CF
    /// 256 MiB, chunks staged at 64 MiB). Per-CF limits for smaller
    /// families stay enforced by the `maybe_auto_flush` walk.
    #[must_use]
    pub fn auto_flush_threshold(&self) -> Option<usize> {
        let mut cap = self.auto_flush_bytes.filter(|n| *n > 0);
        for &n in self.cf_write_buffer.values() {
            if n > 0 && cap.is_none_or(|c| n > c) {
                cap = Some(n);
            }
        }
        // RFC-0159 P1.3 sweep knob: clamp the stage threshold for
        // chunk-size experiments without touching caller buffers
        // (`PEDRA_STAGE_MAX_BYTES`, e.g. 67108864 for 64 MiB chunks;
        // 0 / unparseable = unset). A smaller clamp also moves parking
        // back to whole-memtable staging (host worker) before any
        // per-CF `take_family` limit can fire on the writer.
        if let Some(c) = cap {
            if let Ok(v) = std::env::var("PEDRA_STAGE_MAX_BYTES") {
                if let Ok(max) = v.parse::<usize>() {
                    if max > 0 && max < c {
                        return Some(max);
                    }
                }
            }
        }
        cap
    }

    /// Bulk-run flush size: per-CF / global write buffer, **not**
    /// `PEDRA_STAGE_MAX_BYTES` (that clamp is for memtable staging).
    ///
    /// Always a real cap (RFC-0160 P0.5). `None` flush thresholds used to
    /// let the open tail grow with n (100M SIGKILL).
    ///
    /// RFC-0161 P0.5: never exceed [`DEFAULT_BULK_CHUNK_BYTES`]. Slipstream
    /// sets a 256 MiB write buffer; three of those (tail+park+encode) is
    /// 768 MiB on a 3.9 GiB guest before indexes. Cap the bulk chunk at
    /// 64 MiB regardless of the memtable threshold.
    pub(crate) fn bulk_chunk_cap(&self) -> usize {
        let mut cap = self.auto_flush_bytes.filter(|n| *n > 0);
        for &n in self.cf_write_buffer.values() {
            if n > 0 && cap.is_none_or(|c| n > c) {
                cap = Some(n);
            }
        }
        cap.unwrap_or(DEFAULT_BULK_CHUNK_BYTES)
            .min(DEFAULT_BULK_CHUNK_BYTES)
    }

    /// Open BulkRun + parked chunk + in-flight encode (RFC-0160 P0.5).
    /// Does not include SST payloads (those must stay empty on the bulk
    /// path) or kernel page-cache.
    #[must_use]
    #[allow(dead_code)] // guest MEMDIAG / tests
    pub(crate) fn bulk_live_bytes(&self) -> usize {
        let mut n = 0usize;
        for run in self.bulk_runs.values() {
            n = n.saturating_add(run.bytes());
        }
        for (_, run) in &self.parked_bulk {
            n = n.saturating_add(run.bytes());
        }
        if let Some((_, run)) = &self.bulk_encoding {
            n = n.saturating_add(run.bytes());
        }
        n
    }

    /// Sparse-index RAM of installed SSTs (RFC-0161 P0.5).
    #[must_use]
    pub fn sst_index_bytes(&self) -> usize {
        self.ssts.iter().map(SstTable::index_memory_bytes).sum()
    }

    /// Pedra-side RAM the 100M guest can charge us for: BulkRun tail +
    /// resident payloads + SST indexes + mem/imm. Not kernel page-cache
    /// and not a peer Rocks still live in the same process (v74 SIGKILL
    /// at 3.64 GiB was that sum).
    #[must_use]
    pub fn hydrate_resident_bytes(&self) -> usize {
        self.bulk_live_bytes()
            .saturating_add(self.sst_payload_pool.resident_bytes() as usize)
            .saturating_add(self.sst_index_bytes())
            .saturating_add(self.mem.approx_memory_usage())
            .saturating_add(
                self.imm
                    .as_ref()
                    .map(MemTable::approx_memory_usage)
                    .unwrap_or(0),
            )
    }

    /// Flush-debt cap for concurrent writer backpressure: one parked
    /// table's worth — the max of the global auto-flush and per-CF buffer
    /// thresholds (whichever one parks tables). Writers above it wait for
    /// the host flush worker instead of parking faster than it drains
    /// (25M slipstream: 185 MB/s ingest vs ~100 MB/s materialize OOMed a
    /// 3892 MB box with nothing bounding `parked_unflushed`).
    pub(crate) fn flush_debt_cap(&self) -> Option<usize> {
        // Two thresholds = one chunk of runway: the writer keeps filling
        // chunk N+1 while the worker materializes chunk N. cap ==
        // threshold made every park stop-and-wait (writer queued on
        // flush_lock; local 15M profile 8 s lock_slow per 25 s window,
        // guest run #29 61.5 s unattributed of a 112.5 s wall).
        self.auto_flush_threshold().map(|t| t.saturating_mul(2))
    }

    /// Mem / imm / pin / parked (no SST yet) / folded retired / pending pins.
    fn mem_layers(&self) -> impl Iterator<Item = &MemTable> {
        // F184: the fold is the union of drained pins — always the OLDEST
        // retired layer. Pins parked after that drain are newer, and
        // first-hit lookups ("newest layer wins") must consult them before
        // the fold, or a post-fold rewrite is served stale.
        self.scan_mem_layers()
            .chain(self.retired_pending.iter().rev())
            .chain((!self.retired_fold.is_empty()).then_some(&self.retired_fold))
    }

    /// Layers that have no covering SST: live mems + parked-unflushed.
    /// Scan/count use these plus **all** SST files (not the retired BTrees).
    fn scan_mem_layers(&self) -> impl Iterator<Item = &MemTable> {
        std::iter::once(&self.mem)
            .chain(self.imm.as_ref())
            .chain(self.flush_read_pin.as_ref())
            .chain(self.parked_unflushed.iter().rev().map(|t| t.as_ref()))
    }

    /// Take the existing imm without cloning a flush pin (park path).
    pub fn take_imm_no_pin(&mut self) -> Option<MemTable> {
        self.imm.take()
    }

    /// Park a flushed mem with no SST file. WAL still covers it (G1).
    pub fn push_parked_unflushed(&mut self, table: MemTable) {
        if !table.is_empty() {
            self.parked_unflushed.push(Arc::new(table));
        }
    }

    /// Oldest parked table (for idle materialize). Leaves it in place for reads.
    #[must_use]
    pub fn parked_front(&self) -> Option<&MemTable> {
        self.parked_unflushed.first().map(|t| t.as_ref())
    }

    /// Cheap `Arc` snapshot of the oldest parked table. Materialize streams
    /// from this **off** the Db write lock — a deep `MemTable::clone` there
    /// held the lock for a 4 MiB memcpy (apply_mc4 p99 stalls, RFC-0041).
    #[must_use]
    pub fn parked_front_arc(&self) -> Option<Arc<MemTable>> {
        self.parked_unflushed.first().map(Arc::clone)
    }

    /// Pop the oldest parked table after its L0 exists.
    pub fn take_oldest_parked(&mut self) -> Option<MemTable> {
        if self.parked_unflushed.is_empty() {
            None
        } else {
            let arc = self.parked_unflushed.remove(0);
            Some(Arc::try_unwrap(arc).unwrap_or_else(|a| (*a).clone()))
        }
    }

    /// Pop the oldest parked table **only if** it is still `expect` (by
    /// pointer). A concurrent fold may have swapped the front; the caller's
    /// L0 then covers data still held by the folded union, which stays.
    #[must_use]
    pub fn take_oldest_parked_matching(
        &mut self,
        expect: *const MemTable,
    ) -> Option<Arc<MemTable>> {
        let front = self.parked_unflushed.first()?;
        if !std::ptr::eq(Arc::as_ptr(front), expect) {
            return None;
        }
        Some(self.parked_unflushed.remove(0))
    }

    /// How many flushed mems still lack an L0 file.
    #[must_use]
    pub fn parked_unflushed_count(&self) -> usize {
        self.parked_unflushed.len()
    }

    /// Approximate bytes held by parked (no L0 yet) mems. The host worker
    /// bounds this during sustained ingest — the count alone cannot: the
    /// fold union absorbs every new table while the count stays flat.
    #[must_use]
    pub fn parked_unflushed_bytes(&self) -> usize {
        self.parked_unflushed
            .iter()
            .map(|t| t.approx_memory_usage())
            .fold(0, usize::saturating_add)
    }

    /// Cheap `Arc` snapshot of the two oldest parked tables, recorded as
    /// the in-flight fold pair (validated at swap time — F174). Fold
    /// deep-clones off the Db lock, then [`Self::replace_oldest_parked_pair`].
    pub fn parked_oldest_pair_arcs(&mut self) -> Option<(Arc<MemTable>, Arc<MemTable>)> {
        if self.parked_unflushed.len() < 2 {
            return None;
        }
        let pair = (
            Arc::clone(&self.parked_unflushed[0]),
            Arc::clone(&self.parked_unflushed[1]),
        );
        self.fold_pair_expected = Some((Arc::clone(&pair.0), Arc::clone(&pair.1)));
        Some(pair)
    }

    /// Replace the two oldest parked tables with one folded union — **only
    /// if they are still the exact pair** [`Self::parked_oldest_pair_arcs`]
    /// handed out (F174). A concurrent `materialize_parked_once` may have
    /// installed the front as an L0 and popped it while the fold built the
    /// union off-lock; removing "whatever is oldest now" would drop the
    /// table behind it from the read path. A stale union is discarded
    /// (its data is either in the new L0 or still parked). Returns whether
    /// the swap happened.
    pub fn replace_oldest_parked_pair(&mut self, built: MemTable) -> bool {
        let Some((a, b)) = self.fold_pair_expected.take() else {
            return false;
        };
        let still_pair = self.parked_unflushed.len() >= 2
            && Arc::ptr_eq(&self.parked_unflushed[0], &a)
            && Arc::ptr_eq(&self.parked_unflushed[1], &b);
        if !still_pair {
            return false;
        }
        self.parked_unflushed.remove(0);
        self.parked_unflushed.remove(0);
        if !built.is_empty() {
            self.parked_unflushed.insert(0, Arc::new(built));
        }
        true
    }

    /// Keep `mem` as a point/MVCC cache covering one newly installed L0.
    pub fn retire_mem_as_l0_cache(&mut self, mem: MemTable) {
        if !mem.is_empty() {
            self.retired_pending.push(mem);
            self.retired_l0s = self.retired_l0s.saturating_add(1);
            self.trim_retired_cache();
        }
    }

    /// Cap on the retired-BTree read cache (point/MVCC latency layer
    /// covering L0 SSTs), scaled by the write buffer. Sustained ingest
    /// installs L0s faster than compaction drains them, so the
    /// clear-on-L0-empty hook alone would let this cache grow with
    /// everything written (25M-hydrate OOM, second head).
    fn retired_cache_cap(&self) -> usize {
        self.auto_flush_bytes
            .map_or(16 * 1024 * 1024, |b| 4usize.saturating_mul(b))
    }

    /// Drop oldest retired layers until the pending pile fits the cap.
    /// Cache-only layers: every covered L0 SST stays readable, so this
    /// trades lookup latency for bounded memory, never correctness.
    fn trim_retired_cache(&mut self) {
        let cap = self.retired_cache_cap();
        let mut total: usize = self
            .retired_pending
            .iter()
            .map(|m| m.approx_memory_usage())
            .fold(0, usize::saturating_add);
        while total > cap && self.retired_pending.len() > 1 {
            // Push-ordered: front is the oldest.
            if let Some(m) = self.retired_pending.first() {
                total = total.saturating_sub(m.approx_memory_usage());
            }
            self.retired_pending.remove(0);
        }
    }

    /// Approximate bytes held by the retired read cache (pending + fold).
    #[must_use]
    pub fn retired_mem_bytes(&self) -> usize {
        self.retired_pending
            .iter()
            .map(|m| m.approx_memory_usage())
            .fold(0, usize::saturating_add)
            .saturating_add(self.retired_fold.approx_memory_usage())
    }

    /// Rotate WAL after an off-lock L0 install when mem/imm/pin are idle.
    ///
    /// # Errors
    /// WAL I/O.
    pub fn try_rotate_wal_if_idle(&mut self) -> Result<()> {
        self.try_rotate_wal()
    }

    /// After L0 install: rotate WAL if safe + opportunistic compact / blob GC.
    ///
    /// Used by [`crate::concurrent::ConcurrentDb::flush`] so the dual-mem
    /// pipeline matches single-threaded [`Self::flush`] post-steps
    /// (auto-compact + optional auto blob GC).
    ///
    /// # Errors
    /// WAL rotate I/O.
    pub fn finish_flush_pipeline(&mut self) -> Result<()> {
        self.try_rotate_wal()?;
        self.run_auto_compact_best_effort();
        self.run_auto_blob_gc_best_effort();
        Ok(())
    }

    /// Flush the immutable memtable to L0 (internal / single-threaded path).
    fn flush_imm_to_l0(&mut self) -> Result<()> {
        let Some(imm) = self.imm.take() else {
            return Ok(());
        };
        if imm.is_empty() {
            return Ok(());
        }
        let nums = self.alloc_file_nums_for_imm(&imm);
        let files = match Self::write_imm_l0_files(&self.env, &self.dir, self.sync, &imm, &nums) {
            Ok(f) => f,
            Err(e) => {
                self.imm = Some(imm);
                return Err(self.fence_io_err(e));
            }
        };
        // RFC-0159 P0.2: per-family install level (bulk spans go to the
        // bottom level; everything else L0, unchanged).
        let levels: Vec<u32> = files
            .iter()
            .map(|(t, _, _)| self.bulk_span_level(self.bulk_family_of_table(t), &imm))
            .collect();
        let pairs: Vec<_> = files.into_iter().map(|(t, num, _)| (t, num)).collect();
        if let Err(e) = self.install_ssts_at_levels(pairs, &levels) {
            self.imm = Some(imm);
            return Err(self.fence_io_err(e));
        }
        Ok(())
    }

    /// Rotate WAL only when mem, imm, **and** the off-lock flush pin are empty.
    ///
    /// After [`Self::prepare_flush_imm`] the only copy of acked keys may be the
    /// pin (and an in-flight SST). Truncating WAL here leaves a checkpoint or
    /// crash with nothing to replay.
    fn try_rotate_wal(&mut self) -> Result<()> {
        if crate::flush_kernel::wal_rotate_decision(self.wal_pin_state())
            == crate::flush_kernel::WalRotateAction::KeepWal
        {
            return Ok(());
        }
        // Edge-trigger: a drained pipeline with an empty current segment has
        // nothing to rotate. Without this every idle poll (the compat compact
        // worker tick during read-only phases) rewrites MANIFEST+CURRENT and
        // pays two fdatasync barriers per tick — 10k+ barriers per slipstream
        // guest run, ~42 s of flush traffic competing with the read legs.
        if self.wal.lock().position() == 0 {
            return Ok(());
        }
        self.rotate_wal_now()
    }

    /// Snapshot of every way acked keys can still depend on the WAL
    /// (input to `flush_kernel::wal_rotate_decision`).
    fn wal_pin_state(&self) -> crate::flush_kernel::WalPinState {
        crate::flush_kernel::WalPinState {
            mem_empty: self.mem.is_empty(),
            imm_present: self.imm.is_some(),
            pin_live: self.flush_read_pin.is_some(),
            parked_unflushed: !self.parked_unflushed.is_empty(),
            commit_inflight: self.commit_inflight.load(Ordering::Acquire) > 0
                || !self.unapplied.is_empty(),
        }
    }

    /// F2 guard: value-log/blob GC may only delete or replace pre-GC vlog
    /// sources when the covering WAL was **actually rotated** by the flush that
    /// precedes it. `flush` rotates best-effort — a writer parked in the
    /// off-lock fsync window (`commit_inflight > 0`) or staged-but-unflushed
    /// mem keeps acked records in the WAL, and replaying those after the GC
    /// round would shadow the remapped SSTs with stale pre-GC pointers (a lost
    /// sync-acked write). Refuse and let the caller retry when idle.
    ///
    /// While the caller holds the write lock no new records can be appended,
    /// so this predicate (the exact skip-condition of [`Self::try_rotate_wal`],
    /// evaluated after a completed flush) is equivalent to "the WAL was
    /// rotated".
    fn ensure_wal_rotated_for_gc(&self) -> Result<()> {
        if crate::flush_kernel::wal_rotate_decision(self.wal_pin_state())
            == crate::flush_kernel::WalRotateAction::KeepWal
        {
            return Err(CoreError::Internal(
                "vlog gc refused: wal not rotated (commits in flight or mem staged) — retry when idle"
                    .into(),
            ));
        }
        Ok(())
    }

    fn rotate_wal_now(&mut self) -> Result<()> {
        // SST + MANIFEST must be durable before the WAL that covers those
        // keys is discarded (G1). L0 flush skips file fsync; this is the pay
        // point.
        if let Err(e) = self.persist_manifest_durable() {
            return Err(self.fence_io_err(e));
        }
        // WAL truncate drops the rebuild source for the CHANGELOG cache.
        // Persist first when debounce is on. interval 0: skip on auto-flush
        // (RFC-0036) — F53 SST rebuild covers crash+reopen; explicit flush
        // / close still store.
        if self.changelog_interval > 0 {
            self.persist_changelog_best_effort();
        }
        let wal_path = self.dir.join(WAL_FILE_NAME);
        // F182: drain the old handle's pending async frame BEFORE
        // `create_on` truncates the inode. POSIX does not reset the old fd's
        // offset, so `close()` after the truncate would write the frame at
        // the pre-truncate offset — a sparse zero hole that makes reopen
        // fail-stop (`WalZeroHeader`) although L0+MANIFEST are intact.
        self.wal.lock().flush()?;
        let mut new = Wal::create_on(&self.env, &wal_path)?;
        // Barrier class is a DB-level contract: the rotated segment inherits
        // the strong-class flag (OpenOptions::wal_full_fsync).
        new.set_full_fsync(self.wal.lock().full_fsync());
        let old = std::mem::replace(&mut *self.wal.lock(), new);
        old.close()?;
        self.sync_dir_if_required(&self.dir)?;
        Ok(())
    }

    /// Compact the lowest non-empty level into the next (leveled / size-tier style).
    ///
    /// Flushes the MemTable first. Merges a **subset** of SSTs: all files at the
    /// chosen level N together with all files at N+1, writing one output file at
    /// N+1. Other levels are left untouched (not a whole-DB merge into one SST).
    /// Crash-safe: tmp → rename, then MANIFEST, then delete inputs.
    ///
    /// # Errors
    /// I/O while writing the compacted SST or deleting old files.
    pub fn compact(&mut self) -> Result<()> {
        self.compact_with(CompactOptions::default())
    }

    /// Settle with bounded leveled jobs ([`crate::leveling`]): drain L0 with
    /// overlap-closed L0→L1 merges, then push over-target levels down one
    /// oldest-file job at a time, until the shape is quiet. On a DB whose
    /// steady state already holds (hydrate drained as it wrote), this is a
    /// handful of small jobs — not a whole-database rewrite. A stacked
    /// (non-disjoint) level — a DB written before leveling — is repaired
    /// first with one whole-level rewrite per family.
    ///
    /// `PEDRA_LEVELED=0` selects the historical whole-level [`Self::compact`].
    ///
    /// # Errors
    /// SST / MANIFEST I/O.
    pub fn compact_leveled(&mut self) -> Result<()> {
        if !crate::leveling::leveled_enabled() {
            return self.compact_with(CompactOptions::default());
        }
        self.dump_level_diag("compact_leveled_start");
        self.repair_stacked_levels()?;
        // Across-job batching: only with a thread-shareable env (the seam)
        // and `parallel_jobs > 1`; the batch's disjoint-job writes then run
        // on scoped threads through `ParallelMerge::merge_jobs` while this
        // loop (under the write lock) waits — the lock discipline is
        // unchanged, the drain wall time shrinks.
        let jobs_k = match &self.parallel_merge {
            Some(_) => self.parallel_jobs.clamp(1, 8),
            None => 1,
        };
        // Safety valve only: every job strictly removes an L0 file or moves
        // one file out of an over-target level, so the loop converges.
        for _ in 0..100_000 {
            // L0→L1 jobs stay one-at-a-time: they absorb the newest flush
            // and the overlapping L1 slice, and two of them would share it.
            if let Some(job) = self.prepare_l0_compact(CompactOptions::default())? {
                let tables = job.write()?;
                self.install_prepared_l0_compact(job, tables)?;
                continue;
            }
            let batch = self.prepare_disjoint_pushdown_batch(jobs_k)?;
            match batch.len() {
                0 => {
                    self.dump_level_diag("compact_leveled_done");
                    return Ok(());
                }
                1 => {
                    let job = batch.into_iter().next().expect("len checked");
                    let tables = job.write()?;
                    self.install_prepared_l0_compact(job, tables)?;
                }
                n => {
                    // PEDRA_LEVEL_DIAG: batch wall (per-job lines only cover
                    // the single-job arms; the sum is this line's outputs).
                    let t0 = std::env::var_os("PEDRA_LEVEL_DIAG").map(|_| Instant::now());
                    let specs: Vec<ParallelJobSpec> =
                        batch.iter().map(PreparedL0Compact::job_spec).collect();
                    let Some(pm) = self.parallel_merge.clone() else {
                        // Unreachable (jobs_k > 1 requires the seam); stay
                        // correct anyway — sequential fallback.
                        for job in batch {
                            let tables = job.write()?;
                            self.install_prepared_l0_compact(job, tables)?;
                        }
                        continue;
                    };
                    let outputs = pm.merge_jobs(specs)?;
                    if let Some(t0) = t0 {
                        let total: usize = outputs.iter().map(Vec::len).sum();
                        println!(
                            "COMPDUR jobs={n} outputs={total} ms={}",
                            t0.elapsed().as_millis()
                        );
                    }
                    for (job, tables) in batch.into_iter().zip(outputs) {
                        self.install_prepared_l0_compact(job, tables)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// `PEDRA_LEVEL_DIAG=1`: per-level file count + on-disk bytes at a
    /// scheduling milestone (settle start/end, repair end) — the shape the
    /// read path faces, on the guest serial console.
    fn dump_level_diag(&self, tag: &str) {
        if std::env::var_os("PEDRA_LEVEL_DIAG").is_none() {
            return;
        }
        for level in 0..=MAX_LSM_LEVEL {
            let mut n = 0usize;
            let mut bytes = 0u64;
            for (t, &lvl) in self.ssts.iter().zip(self.sst_levels.iter()) {
                if lvl == level {
                    n += 1;
                    bytes += self.table_bytes(t);
                }
            }
            eprintln!("LEVELDIAG {tag} level={level} files={n} bytes={bytes}");
        }
    }

    /// One whole-level rewrite per stacked (non-disjoint) family level until
    /// every level is a disjoint sorted run set — the precondition for
    /// bounded overlap-sliced jobs (see [`crate::leveling`]).
    fn repair_stacked_levels(&mut self) -> Result<()> {
        while crate::leveling::leveled_enabled() {
            let mut target: Option<(u32, Vec<usize>)> = None;
            'search: for level in 1..=MAX_LSM_LEVEL {
                let families: Vec<String> = self
                    .ssts
                    .iter()
                    .zip(self.sst_levels.iter())
                    .filter(|(_, &lvl)| lvl == level)
                    .map(|(t, _)| self.compact_family_key(t).to_string())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                for cf in families {
                    let view = self.level_view(level, &cf);
                    if view.len() >= 2 && !crate::leveling::is_disjoint(&view) {
                        target = Some((level, view.iter().map(|f| f.idx).collect()));
                        break 'search;
                    }
                }
            }
            let Some((level, idxs)) = target else {
                return Ok(());
            };
            let inputs: Vec<SstTable> = idxs.iter().map(|&i| self.ssts[i].clone()).collect();
            let Some(job) =
                self.build_prepared(inputs, level, crate::merge::CompactGcOptions::default())?
            else {
                return Ok(());
            };
            let tables = job.write()?;
            self.install_prepared_l0_compact(job, tables)?;
        }
        Ok(())
    }

    /// Compact with version GC options (RFC-0009 P1.3).
    ///
    /// # Errors
    /// I/O while writing the compacted SST or deleting old files.
    pub fn compact_with(&mut self, options: CompactOptions) -> Result<()> {
        self.flush()?;
        crate::buggify_hooks::inject_checked(crate::buggify_hooks::sites::BEFORE_COMPACT_WRITE)?;
        match self.compact_with_ssts_only(options) {
            Ok(()) => Ok(()),
            Err(e) => Err(self.fence_io_err(e)),
        }
    }

    /// Snapshot-safe version GC piggybacked on compaction (open-items §2.1 option b).
    ///
    /// Uses the oldest open [`SnapshotPin`] as the Rocks-style GC floor. With no
    /// pins, reclaims like latest-only (watermark = last sequence). Does **not**
    /// change default auto-compact (F20 still preserves history for bare
    /// [`Snapshot`] tokens).
    ///
    /// # Errors
    /// I/O while flushing or rewriting SSTs.
    pub fn compact_reclaim(&mut self) -> Result<()> {
        // F211 (auto_gc_floor): cap the no-pin floor at the published
        // sequence — `last_sequence()` counts applied-but-unpublished writes.
        let oldest = crate::compact_kernel::gc_oldest_from_pin(
            self.oldest_pinned_sequence(),
            self.last_sequence(),
            self.visible_sequence(),
        );
        // F201: honor open OCC transactions (same floor rule as auto-GC).
        let oldest = match self.occ_registry_floor() {
            Some(occ) => oldest.min(occ),
            None => oldest,
        };
        self.compact_with(CompactOptions {
            gc: crate::merge::CompactGcOptions::for_oldest_snapshot(oldest),
            max_input_files: None,
        })
    }

    /// Horizon-aware full compaction (RFC-0046 P0.5): flush, archive every
    /// version below the current horizon floor (fail-closed — an archive
    /// error aborts before anything is dropped), then rewrite **all** SSTs
    /// under that floor into one file. This is the same rewrite the
    /// automatic dead-weight-doubling trigger performs on its own schedule,
    /// exposed for operators who want the aged bytes back now (e.g. after
    /// shrinking the window). No-op when the horizon is `All`; with
    /// `set_auto_reclaim(true)` it behaves like [`Self::compact_reclaim`].
    /// Ages the `last_horizon_reclaim` baseline so the automatic trigger
    /// does not immediately re-run.
    ///
    /// # Errors
    /// I/O while flushing, archiving, or rewriting SSTs.
    pub fn compact_horizon(&mut self) -> Result<()> {
        self.flush()?;
        let Some((floor, archive)) = self.auto_gc_floor() else {
            return Ok(());
        };
        if self.ssts.is_empty() {
            return Ok(());
        }
        if archive {
            self.archive_history_below(floor)?;
        }
        let input_idxs: Vec<usize> = (0..self.ssts.len()).collect();
        self.rewrite_ssts(
            input_idxs,
            MAX_LSM_LEVEL,
            CompactOptions {
                gc: crate::merge::CompactGcOptions::for_oldest_snapshot(floor),
                max_input_files: None,
            },
        )?;
        let mut after: u64 = 0;
        for t in &self.ssts {
            if let Ok(len) = self.env.metadata_len(t.path()) {
                after = after.saturating_add(len);
            }
        }
        self.last_horizon_reclaim = Some((floor, after));
        Ok(())
    }

    /// Collapse write-burst history for read-heavy control-plane prefixes (RFC-0019 P2.2).
    ///
    /// Flushes the memtable, then rewrites **all** SST files into one with
    /// [`CompactOptions::latest_only`] GC (newest version per key; lone tombstones
    /// dropped). Point lookups after a write storm touch fewer files.
    ///
    /// Prefer over plain [`Self::compact`] when the workload is now read-mostly and
    /// historical versions are not needed for open snapshots.
    ///
    /// # Errors
    /// I/O while flushing or rewriting SSTs.
    pub fn compact_for_reads(&mut self) -> Result<()> {
        self.flush()?;
        if self.ssts.is_empty() {
            return Ok(());
        }
        // Merge every live SST (any level) with latest-only GC into one file at Lmax.
        // Full-DB rewrite: bottommost — tombstones may be collapsed (F177).
        let mut merged: Vec<(InternalKey, Bytes)> = Vec::new();
        for t in &self.ssts {
            merged.extend(t.entries_cloned()?);
        }
        // F216: `latest_only` seeds the GC watermark from `last_sequence()`,
        // which counts applied-but-unpublished writes (write-group off-lock
        // window). In that window the watermark rises above the published
        // sequence and every visible-snapshot read fails `SnapshotTooOld`.
        // Use the `auto_gc_floor` discipline instead (F201/F211): pin/OCC
        // floor capped at the published sequence. `for_oldest_snapshot`
        // keeps every version newer than the floor and, with `bottommost`,
        // still collapses lone tombstones — equivalent to latest-only when
        // nothing is pinned or in flight.
        let pin_or_last = crate::compact_kernel::gc_oldest_from_pin(
            self.oldest_pinned_sequence(),
            self.last_sequence(),
            self.visible_sequence(),
        );
        let gc_floor = match self.occ_registry_floor() {
            Some(occ) => pin_or_last.min(occ),
            None => pin_or_last,
        };
        let mut gc = crate::merge::CompactGcOptions::for_oldest_snapshot(gc_floor);
        gc.bottommost = true;
        let merged = crate::merge::gc_compact_entries(merged, gc);

        let num = self.next_file_num;
        let final_path = self.dir.join(format!("{num:06}.sst"));
        let tmp_path = self.dir.join(format!("{num:06}.sst.tmp"));
        let new_table = write_sst_entries_on(&self.env, &tmp_path, &merged)?;
        self.env.rename(&tmp_path, &final_path)?;
        self.sync_dir_if_required(&self.dir)?;
        // In-place table kept: the rename does not change the bytes, and a
        // re-open here re-read + decompressed + decoded every entry of the
        // freshly written file.
        let new_table = new_table.with_path(final_path.clone());
        self.adopt_sst(&new_table);
        self.table_cache.insert(Arc::new(new_table.clone()));
        let prev_next = self.next_file_num;
        self.next_file_num = num + 1;

        let old_paths: Vec<PathBuf> = self.ssts.iter().map(|t| t.path().to_path_buf()).collect();
        let prev_tables = std::mem::replace(&mut self.ssts, vec![new_table]);
        let prev_levels = std::mem::replace(&mut self.sst_levels, vec![MAX_LSM_LEVEL]);
        // Same as every other inventory swap: rebuild `sst_order_newest`
        // before reads resume (stale indices panic `lookup`).
        self.note_sst_inventory_changed();

        if let Ok(len) = self.env.metadata_len(&final_path) {
            self.bytes_written_sst = self.bytes_written_sst.saturating_add(len);
        }
        // Watermark must be raised before MANIFEST so reopen recovers it.
        let prev_earliest = self.earliest_readable_seq;
        self.note_version_gc_watermark(crate::merge::CompactGcOptions::for_oldest_snapshot(
            gc_floor,
        ));
        if let Err(e) = self.persist_manifest() {
            // F194: same contract as every other inventory swap (F173 /
            // `L0CompactUndo`): Err leaves the pre-compact state — a later
            // persist must not commit a GC the caller saw fail.
            self.ssts = prev_tables;
            self.sst_levels = prev_levels;
            self.next_file_num = prev_next;
            self.earliest_readable_seq = prev_earliest;
            self.note_sst_inventory_changed();
            let _ = self.remove_db_file(&final_path);
            let _ = self.env.sync_dir(&self.dir);
            return Err(e);
        }

        for path in old_paths {
            if path != final_path {
                let _ = self.remove_db_file(&path);
            }
        }
        self.compact_count = self.compact_count.saturating_add(1);
        // latest_only rewrite — same auto-blob path as leveled compact.
        self.run_auto_blob_gc_best_effort();
        Ok(())
    }

    /// Compact SST inventory only (caller already flushed).
    ///
    /// # Errors
    /// I/O while writing the compacted SST or deleting old files.
    pub fn compact_ssts_only(&mut self) -> Result<()> {
        self.compact_with_ssts_only(CompactOptions::default())
    }

    /// Compact SST levels without flushing memtables first.
    ///
    /// # Errors
    /// I/O while writing the compacted SST or deleting old files.
    pub fn compact_with_ssts_only(&mut self, options: CompactOptions) -> Result<()> {
        if self.ssts.is_empty() {
            return Ok(());
        }
        // Pick lowest level that has files and can promote (N → N+1);
        // decided by the pure kernel (RFC-0056 P0.3).
        let lowest = (0..MAX_LSM_LEVEL).find(|&lvl| self.level_file_count(lvl) > 0);
        let files_at_max = self.level_file_count(MAX_LSM_LEVEL) > 0;
        match crate::compact_kernel::compact_pick(
            lowest,
            files_at_max,
            options.gc.requests_gc(),
            MAX_LSM_LEVEL,
        ) {
            crate::compact_kernel::CompactPlan::Merge { from, to } => {
                // Unreachable in practice (from < MAX ⇒ to = from + 1), kept
                // as the historical single-file no-op guard.
                if from == to && self.ssts.len() == 1 && !options.gc.requests_gc() {
                    return Ok(());
                }
                self.compact_levels(from, to, options)
            }
            crate::compact_kernel::CompactPlan::GcRewriteMax => {
                self.compact_levels(MAX_LSM_LEVEL, MAX_LSM_LEVEL, options)
            }
            crate::compact_kernel::CompactPlan::NoOp => Ok(()),
        }
    }

    /// Promote L0 files into one new L1 file. Existing L1+ SSTs are left
    /// untouched so a write burst does not rewrite the whole level (RFC-0036).
    /// Visibility is unchanged: every version stays in some file.
    fn compact_l0_into_l1(&mut self, options: CompactOptions) -> Result<()> {
        let mut families = Vec::new();
        for (t, &lvl) in self.ssts.iter().zip(self.sst_levels.iter()) {
            if lvl == 0 {
                let cf = self.compact_family_key(t).to_string();
                if !families.contains(&cf) {
                    families.push(cf);
                }
            }
        }
        for fam in families {
            let input_idxs: Vec<usize> = self
                .ssts
                .iter()
                .zip(self.sst_levels.iter())
                .enumerate()
                .filter(|(_, (t, &lvl))| lvl == 0 && self.compact_family_key(t) == fam)
                .map(|(i, _)| i)
                .collect();
            if !input_idxs.is_empty() {
                self.rewrite_ssts(input_idxs, 1, options)?;
            }
        }
        Ok(())
    }

    /// Snapshot current L0 tables and reserve an output file number.
    ///
    /// Inputs stay readable. Call [`PreparedL0Compact::write`] without this
    /// lock, then [`Self::install_prepared_l0_compact`].
    ///
    /// Leveled selection: when the family's L1 is a disjoint sorted run set,
    /// the job also absorbs the L1 slice overlapping the selected L0s, so L1
    /// never degenerates into stacked full-range runs (see
    /// [`crate::leveling`]). A stacked L1 (legacy DB) keeps the L0-only job.
    /// When the overlapping slice is larger than the L1 slice cap, a pushdown
    /// job ([`Self::prepare_pushdown_compact`]) is returned instead — it is
    /// the bounded way to shrink the slice before the next L0→L1 merge.
    ///
    /// # Errors
    /// None today (reservation cannot fail); `Result` for fence / I/O later.
    pub fn prepare_l0_compact(
        &mut self,
        options: CompactOptions,
    ) -> Result<Option<PreparedL0Compact<E>>> {
        self.ensure_not_fenced()?;
        let leveled = crate::leveling::leveled_enabled();
        let mut by_cf: BTreeMap<String, Vec<SstTable>> = BTreeMap::new();
        for (t, &lvl) in self.ssts.iter().zip(self.sst_levels.iter()) {
            if lvl == 0 {
                by_cf
                    .entry(self.compact_family_key(t).to_string())
                    .or_default()
                    .push(t.clone());
            }
        }
        let (cf, mut inputs) = by_cf
            .into_iter()
            .max_by_key(|(_, v)| v.len())
            .map(|(cf, v)| (cf, v))
            .unwrap_or_default();
        if inputs.is_empty() {
            return Ok(None);
        }
        // `ssts` is append-ordered, so the family vec is oldest-first; a
        // truncated prefix is the oldest N L0 files. Any subset is a valid
        // merge: newer L0 files stay live and shadow the output at read
        // time exactly as they shadowed the inputs. The caller's bound is
        // the contract (compat's 2-input ticks bound merge memory — the
        // v15 25M OOM); leveled job size is bounded separately by the L1
        // slice cap below.
        if let Some(max) = options.max_input_files.filter(|m| *m > 0) {
            inputs.truncate(max);
        }
        if leveled {
            let l0_view = self.level_view(0, &cf);
            let l1_view = self.level_view(1, &cf);
            if crate::leveling::is_disjoint(&l1_view) {
                if let Some((_l0_sel, slice)) = crate::leveling::pick_l0_to_l1(
                    &l0_view,
                    &l1_view,
                    options
                        .max_input_files
                        .filter(|m| *m > 0)
                        .unwrap_or(usize::MAX),
                ) {
                    let slice_tables: Vec<SstTable> =
                        slice.iter().map(|&i| self.ssts[i].clone()).collect();
                    let slice_bytes: u64 = slice_tables.iter().map(|t| self.table_bytes(t)).sum();
                    let cap = self.l1_target_bytes.saturating_mul(4);
                    if slice_bytes > cap {
                        // Overlapping L1 is too fat for one bounded job:
                        // shrink it by pushdown first (oldest chunks leave
                        // L1 entirely). Falls through to L0-only stacking
                        // only when nothing can push down.
                        if let Some(job) = self.prepare_pushdown_compact()? {
                            return Ok(Some(job));
                        }
                    } else {
                        inputs.extend(slice_tables);
                    }
                }
            }
        }
        self.build_prepared(inputs, 1, options.gc)
    }

    /// Next bounded pushdown job: the oldest file of the lowest level that
    /// exceeds its size target, merged with the overlapping files one level
    /// down. `None` when every level is within target (or its destination is
    /// a stacked, non-disjoint level — those need a repair rewrite first).
    ///
    /// # Errors
    /// None today (reservation cannot fail); `Result` for fence / I/O later.
    pub fn prepare_pushdown_compact(&mut self) -> Result<Option<PreparedL0Compact<E>>> {
        if !crate::leveling::leveled_enabled() || self.ssts.is_empty() {
            return Ok(None);
        }
        let families: Vec<String> = self
            .ssts
            .iter()
            .map(|t| self.compact_family_key(t).to_string())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        for level in 1..MAX_LSM_LEVEL {
            let target = crate::leveling::level_target_bytes(level, self.l1_target_bytes);
            for cf in &families {
                let src_view = self.level_view(level, cf);
                if src_view.is_empty() || crate::leveling::total_bytes(&src_view) <= target {
                    continue;
                }
                let dst_view = self.level_view(level + 1, cf);
                let Some((src_idx, slice)) = crate::leveling::pick_pushdown(&src_view, &dst_view)
                else {
                    continue;
                };
                let mut inputs: Vec<SstTable> = vec![self.ssts[src_idx].clone()];
                for i in slice {
                    inputs.push(self.ssts[i].clone());
                }
                return self.build_prepared(
                    inputs,
                    level + 1,
                    crate::merge::CompactGcOptions::default(),
                );
            }
        }
        Ok(None)
    }

    /// Up to `max_jobs` **pairwise key-disjoint** pushdown jobs from the
    /// first over-target (level, family) — same priority as
    /// [`Self::prepare_pushdown_compact`], but instead of only the oldest
    /// source file it walks the source view oldest-first and keeps every
    /// candidate whose *combined input hull* (source ∪ overlapping
    /// destination files) stays clear of every already-claimed hull. Hull
    /// disjointness gives both safety properties the batch needs: no
    /// shared input file (a wide destination file spanning two sources is
    /// absorbed by whichever job claims it first), and disjoint output
    /// ranges at the destination level, so the sequential installs
    /// commute exactly like today's one-at-a-time installs.
    /// `max_jobs <= 1` delegates to the single-job picker (unchanged
    /// behavior). Returns an empty vec when nothing can push down.
    fn prepare_disjoint_pushdown_batch(
        &mut self,
        max_jobs: usize,
    ) -> Result<Vec<PreparedL0Compact<E>>> {
        if max_jobs <= 1 || !crate::leveling::leveled_enabled() || self.ssts.is_empty() {
            return self
                .prepare_pushdown_compact()
                .map(|j| j.into_iter().collect());
        }
        let families: Vec<String> = self
            .ssts
            .iter()
            .map(|t| self.compact_family_key(t).to_string())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        for level in 1..MAX_LSM_LEVEL {
            let target = crate::leveling::level_target_bytes(level, self.l1_target_bytes);
            for cf in &families {
                let src_view = self.level_view(level, cf);
                if src_view.is_empty() || crate::leveling::total_bytes(&src_view) <= target {
                    continue;
                }
                let dst_view = self.level_view(level + 1, cf);
                // Same gate `pick_pushdown` applies: a stacked destination
                // needs a repair rewrite, not a batch.
                if !crate::leveling::is_disjoint(&dst_view) {
                    continue;
                }
                let mut jobs: Vec<PreparedL0Compact<E>> = Vec::new();
                let mut hulls: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
                for src in &src_view {
                    if jobs.len() >= max_jobs {
                        break;
                    }
                    let slice: Vec<&crate::leveling::LevelFile> = dst_view
                        .iter()
                        .filter(|f| f.overlaps(&src.lo, &src.hi))
                        .collect();
                    let mut lo = src.lo.clone();
                    let mut hi = src.hi.clone();
                    for f in &slice {
                        if f.lo < lo {
                            lo = f.lo.clone();
                        }
                        if f.hi > hi {
                            hi = f.hi.clone();
                        }
                    }
                    // Reject when the hull touches any claimed hull
                    // (shared boundary = overlap, matching `is_disjoint`).
                    if hulls.iter().any(|(l2, h2)| !(hi < *l2 || *h2 < lo)) {
                        continue;
                    }
                    let mut inputs: Vec<SstTable> = vec![self.ssts[src.idx].clone()];
                    for f in &slice {
                        inputs.push(self.ssts[f.idx].clone());
                    }
                    let Some(job) = self.build_prepared(
                        inputs,
                        level + 1,
                        crate::merge::CompactGcOptions::default(),
                    )?
                    else {
                        continue;
                    };
                    hulls.push((lo, hi));
                    jobs.push(job);
                }
                if !jobs.is_empty() {
                    return Ok(jobs);
                }
            }
        }
        Ok(Vec::new())
    }

    /// Reservation tail shared by every prepared leveled job: burn a chunk
    /// range wide enough for the whole output, snapshot dir/env/kit.
    fn build_prepared(
        &mut self,
        inputs: Vec<SstTable>,
        to_level: u32,
        gc: crate::merge::CompactGcOptions,
    ) -> Result<Option<PreparedL0Compact<E>>> {
        if inputs.is_empty() {
            return Ok(None);
        }
        let file_num = self.alloc_file_num();
        // The off-lock `write()` emits one file per split-target chunk
        // starting at `file_num`, so only the reserved range is safe: a
        // concurrent allocator between `write` and `install` would
        // otherwise land inside the range and its tmp→rename would
        // clobber a chunk path, leaving a live table reading another
        // table's bytes (v5 per-block CRC mismatch — guest 25M run #4).
        // No static bound exists in the writer's split currency (logical
        // entry bytes) once lz4 shrinks the inputs, so the writer is
        // CAPPED to this chunk budget instead: the last chunk may exceed
        // the split target, which is sizing advice, not correctness.
        let inputs_bytes: u64 = inputs
            .iter()
            .map(|t| self.env.metadata_len(t.path()).unwrap_or(0))
            .sum();
        // Margin for the parallel spans: each span's last chunk can run
        // past target and one extra chunk absorbs split-key skew.
        let span_margin = merge_span_count(&self.env, &inputs).saturating_sub(1) as u64;
        let chunk_budget =
            usize::try_from(inputs_bytes / self.compact_target_file_bytes.max(1) + 2 + span_margin)
                .unwrap_or(usize::MAX);
        for _ in 1..chunk_budget {
            self.alloc_file_num();
        }
        let kit = self
            .sst_source
            .as_ref()
            .map(|source| crate::cache::PayloadKit {
                source: Arc::clone(source),
                pool: Arc::clone(&self.sst_payload_pool),
            });
        Ok(Some(PreparedL0Compact {
            inputs,
            to_level,
            file_num,
            gc,
            dir: self.dir.clone(),
            env: self.env.clone(),
            sync: self.sync,
            split_target: self.compact_target_file_bytes,
            chunk_budget,
            kit,
            parallel: self.parallel_merge.clone(),
        }))
    }

    /// Install a key-space-parallel merge executor (host open path, where
    /// `E: Send + Sync + 'static` holds). Without one, prepared jobs merge
    /// sequentially.
    pub(crate) fn set_parallel_merge(&mut self, pm: Arc<dyn ParallelMerge>) {
        self.parallel_merge = Some(pm);
    }

    /// Override the across-job batch width (tests; the host open path sets
    /// it from `PEDRA_PARALLEL_JOBS`). Needs [`Self::set_parallel_merge`]
    /// to take effect.
    pub fn set_parallel_jobs(&mut self, k: usize) {
        self.parallel_jobs = k.clamp(1, 8);
    }

    /// Scheduling view of one family's files at `level` (inventory indices
    /// with user-key range and on-disk size).
    fn level_view(&self, level: u32, cf: &str) -> Vec<crate::leveling::LevelFile> {
        self.ssts
            .iter()
            .zip(self.sst_levels.iter())
            .enumerate()
            .filter(|(_, (t, &lvl))| lvl == level && self.compact_family_key(t) == cf)
            .map(|(i, (t, _))| crate::leveling::LevelFile {
                idx: i,
                lo: t.smallest_user_key().unwrap_or_default().to_vec(),
                hi: t.largest_user_key().unwrap_or_default().to_vec(),
                bytes: self.table_bytes(t),
            })
            .collect()
    }

    /// On-disk size of one live table (0 when the stat fails — a missing
    /// file reports as empty, the conservative direction for sizing).
    fn table_bytes(&self, t: &SstTable) -> u64 {
        self.env.metadata_len(t.path()).unwrap_or(0)
    }

    /// Publish a prepared L0→L1 SST. L0s flushed while `write` ran are kept.
    ///
    /// If every input path is already gone (another install won), the new file
    /// is deleted and this is a no-op (G2: no duplicate live versions).
    ///
    /// # Errors
    /// MANIFEST I/O. On error the new file is not installed; old L0s stay.
    pub fn install_prepared_l0_compact(
        &mut self,
        job: PreparedL0Compact<E>,
        new_tables: Vec<SstTable>,
    ) -> Result<()> {
        let Some(undo) = self.apply_prepared_l0_compact(job, new_tables) else {
            return Ok(());
        };
        let old_paths = undo.old_paths().to_vec();
        if let Err(e) = self.persist_manifest() {
            self.undo_prepared_l0_compact(undo);
            return Err(e);
        }
        for path in old_paths {
            let _ = self.remove_db_file(&path);
        }
        self.compact_count = self.compact_count.saturating_add(1);
        Ok(())
    }

    /// Merge SSTs at `from_level` and `to_level` into one SST per CF at `to_level`.
    fn compact_levels(
        &mut self,
        from_level: u32,
        to_level: u32,
        options: CompactOptions,
    ) -> Result<()> {
        let mut families: BTreeSet<String> = BTreeSet::new();
        for (t, &lvl) in self.ssts.iter().zip(self.sst_levels.iter()) {
            if lvl == from_level || lvl == to_level {
                families.insert(self.compact_family_key(t).to_string());
            }
        }
        for fam in families {
            let input_idxs: Vec<usize> = self
                .ssts
                .iter()
                .zip(self.sst_levels.iter())
                .enumerate()
                .filter(|(_, (t, &lvl))| {
                    (lvl == from_level || lvl == to_level) && self.compact_family_key(t) == fam
                })
                .map(|(i, _)| i)
                .collect();
            if input_idxs.is_empty() {
                continue;
            }
            // Single file at target, no GC → nothing to do.
            if input_idxs.len() == 1
                && self.sst_levels[input_idxs[0]] == to_level
                && !options.gc.requests_gc()
            {
                continue;
            }
            self.rewrite_ssts(input_idxs, to_level, options)?;
        }
        Ok(())
    }

    /// Compact only SSTs of `cf` (RFC-0065 P0.2). Mixed/legacy files (`cf` empty
    /// on disk) are left alone unless `cf` is itself empty.
    ///
    /// # Errors
    /// SST / MANIFEST I/O.
    pub fn compact_ssts_only_cf(&mut self, cf: &str) -> Result<()> {
        let input_idxs: Vec<usize> = self
            .ssts
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                crate::cf_kernel::compact_rewrites_sst_cf(t.cf(), cf)
                    && crate::cf_kernel::key_in_cf_family(
                        &crate::cf_kernel::encode_cf_key(t.cf(), &[], false),
                        cf,
                    )
            })
            .map(|(i, _)| i)
            .collect();
        if input_idxs.is_empty() {
            return Ok(());
        }
        if input_idxs.len() == 1 {
            return Ok(());
        }
        let all_l0 = input_idxs.iter().all(|&i| self.sst_levels[i] == 0);
        let to_level = if all_l0 {
            1
        } else {
            input_idxs
                .iter()
                .map(|&i| self.sst_levels[i])
                .max()
                .unwrap_or(1)
                .max(1)
        };
        self.rewrite_ssts(input_idxs, to_level, CompactOptions::default())
    }

    /// Snapshot of live SST files (name, level, CF, size, bounds).
    #[must_use]
    pub fn live_sst_meta(&self) -> Vec<SstLiveMeta> {
        self.ssts
            .iter()
            .zip(self.sst_levels.iter())
            .map(|(t, &level)| {
                let name = t
                    .path()
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let size = self.env.metadata_len(t.path()).unwrap_or(0);
                SstLiveMeta {
                    name,
                    level,
                    cf: t.cf().to_string(),
                    size,
                    start_key: t.smallest_user_key().unwrap_or(&[]).to_vec(),
                    end_key: t.largest_user_key().unwrap_or(&[]).to_vec(),
                    num_entries: t.len() as u64,
                }
            })
            .collect()
    }

    /// Rewrite `input_idxs` into chunked SSTs at `to_level` (split at
    /// [`REWRITE_CHUNK_TARGET_BYTES`] logical bytes); keep every other file.
    fn rewrite_ssts(
        &mut self,
        input_idxs: Vec<usize>,
        to_level: u32,
        options: CompactOptions,
    ) -> Result<()> {
        let num = self.next_file_num;
        // F177: tombstone dropping is only visibility-safe when this rewrite
        // covers **every** live SST — otherwise an older version in a file
        // outside the input resurrects once the tombstone is dropped. The
        // landing level is irrelevant: if the input is the whole DB, the
        // output is the whole DB. Partial rewrites keep tombstones.
        let bottommost = input_idxs.len() == self.ssts.len();
        let mut gc = options.gc;
        gc.bottommost = bottommost;
        let options = CompactOptions { gc, ..options };
        let tables: Vec<SstTable> = input_idxs.iter().map(|&i| self.ssts[i].clone()).collect();
        let cf = tables
            .first()
            .map(|t| t.cf().to_string())
            .unwrap_or_default();
        let kit = self
            .sst_source
            .as_ref()
            .map(|source| crate::cache::PayloadKit {
                source: Arc::clone(source),
                pool: Arc::clone(&self.sst_payload_pool),
            });
        // Whole-levels rewrites merge every file of two levels, so the
        // writer's per-chunk transient (chunk body Vec + bloom + the
        // post-write read-back) rides on top of the full live-set read
        // traffic. Cap the chunk at 64 MiB logical — RocksDB's own L1
        // target-file-size shape — so that transient stays small; a
        // caller-set smaller target wins.
        let rewrite_split = self
            .compact_target_file_bytes
            .min(self.rewrite_chunk_target_bytes);
        let new_tables: Vec<SstTable> = write_merged_tables(
            &self.env,
            &self.dir,
            num,
            &tables,
            options.gc,
            self.sync,
            rewrite_split,
            // Runs under the `&mut self` write lock and advances
            // `next_file_num` after the write, so no other allocator can
            // interleave: unlimited chunks are safe here.
            usize::MAX,
            kit.as_ref(),
        )?
        .into_iter()
        .map(|t| t.with_cf(cf.clone()))
        .collect();
        for t in &new_tables {
            self.table_cache.insert(Arc::new(t.clone()));
        }
        self.next_file_num = num + new_tables.len() as u64;

        let old_paths: Vec<PathBuf> = input_idxs
            .iter()
            .map(|&i| self.ssts[i].path().to_path_buf())
            .collect();

        // Keep SSTs not in the input set; append the new files at `to_level`.
        let mut keep_tables = Vec::new();
        let mut keep_levels = Vec::new();
        for (i, (t, &lvl)) in self.ssts.iter().zip(self.sst_levels.iter()).enumerate() {
            if !input_idxs.contains(&i) {
                keep_tables.push(t.clone());
                keep_levels.push(lvl);
            }
        }
        let new_paths: Vec<PathBuf> = new_tables.iter().map(|t| t.path().to_path_buf()).collect();
        for t in new_tables {
            keep_tables.push(t);
            keep_levels.push(to_level);
        }
        let prev_tables = std::mem::replace(&mut self.ssts, keep_tables);
        let prev_levels = std::mem::replace(&mut self.sst_levels, keep_levels);
        // F173: the durable state only changes once the MANIFEST install
        // below succeeds — snapshot everything mutated before it (except
        // `next_file_num`, whose number stays burned like the off-lock L0
        // path) so a failed install rolls the whole compact back instead of
        // leaving a raised GC watermark + swapped inventory that the next
        // manifest write (e.g. a later flush) would install durably.
        let prev_manifest_file_num = self.manifest_file_num;
        let prev_earliest = self.earliest_readable_seq;
        self.note_sst_inventory_changed();

        for p in &new_paths {
            if let Ok(len) = self.env.metadata_len(p) {
                self.bytes_written_sst = self.bytes_written_sst.saturating_add(len);
            }
        }
        // Raise GC watermark before MANIFEST install (durable across reopen).
        if options.gc.requests_gc() {
            self.note_version_gc_watermark(options.gc);
        }
        if let Err(e) = self.persist_manifest() {
            self.ssts = prev_tables;
            self.sst_levels = prev_levels;
            self.manifest_file_num = prev_manifest_file_num;
            self.earliest_readable_seq = prev_earliest;
            self.note_sst_inventory_changed();
            return Err(e);
        }

        for path in old_paths {
            if !new_paths.contains(&path) {
                let _ = self.remove_db_file(&path);
            }
        }
        self.compact_count = self.compact_count.saturating_add(1);
        if options.gc.keep_only_latest || options.gc.oldest_snapshot.is_some() {
            // Dead vlog pointers may have been dropped — maybe reclaim sealed blobs.
            self.run_auto_blob_gc_best_effort();
        }
        Ok(())
    }

    /// Rewrite `VALUES.vlog` keeping only records still referenced by mem/imm/SSTs
    /// (RFC-0016 P0.1 crash-safe GC).
    ///
    /// Two-phase install:
    /// 1. **Prepare** — stage `VALUES.vlog.new` + remapped SST files only (no `self` inventory /
    ///    `vlog_use_new` / mem / `self.vlog` mutation). Err ⇒ process state unchanged.
    /// 2. **Install** — MANIFEST with remapped SSTs + `vlog_use_new` (atomic CURRENT); on
    ///    MANIFEST Err full rollback. After MANIFEST Ok is the commit point: open replacement
    ///    vlog via [`Self::replace_vlog_handle`] (never `vlog = None` first), then remap mem.
    /// 3. **Promote** — rename `.new` → primary; clear flag; same handle-replace + fence rules.
    ///
    /// # Errors
    /// I/O, CRC, or durability fence. After a post-commit handle open failure the DB is
    /// [`Self::is_durability_fenced`] so writers stop; reopen rebuilds from MANIFEST.
    pub fn compact_vlog(&mut self) -> Result<VlogRewriteStats> {
        let stats = self.compact_vlog_stage_manifest()?;
        self.compact_vlog_promote()?;
        Ok(stats)
    }

    /// Prepare + MANIFEST install (`vlog_use_new`); no promote.
    ///
    /// Used by crash-recovery tests; production callers use [`Self::compact_vlog`].
    ///
    /// # Errors
    /// I/O, CRC, or durability fence.
    pub fn compact_vlog_stage_manifest(&mut self) -> Result<VlogRewriteStats> {
        self.ensure_not_fenced()?;
        // F3: a staged-but-unpromoted round (`vlog_use_new=true`, `.new` live)
        // must never be rewritten in place — `rewrite_live_to_new` replacing
        // the live staging file could leave a truncated `.new` that recovery
        // trusts under `vlog_use_new` (or bricks the open). Promote first so
        // the rewrite only ever replaces a non-live staging file.
        if self.vlog_use_new {
            self.compact_vlog_promote()?;
        }
        // Durable empty WAL of unflushed pointers that would break after remap.
        self.flush()?;
        // F2: `flush` rotates best-effort; refuse the round unless it really
        // rotated (stale pre-GC pointers in an un-rotated WAL shadow the
        // remapped SSTs after crash replay).
        self.ensure_wal_rotated_for_gc()?;

        if self.vlog.is_none() {
            let main = self.dir.join(VLOG_FILE_NAME);
            if !self.env.exists(&main) {
                return Ok(VlogRewriteStats {
                    bytes_before: 0,
                    bytes_after: 0,
                    live_records: 0,
                });
            }
            // Open without clearing a live handle (there is none).
            self.replace_vlog_handle(self.vlog_use_new)?;
        }

        let prepared = self.prepare_vlog_gc()?;
        self.install_vlog_gc(prepared)
    }

    /// Stage 4: promote `.new` → primary and clear MANIFEST `vlog_use_new`.
    ///
    /// # Errors
    /// I/O or durability fence.
    pub fn compact_vlog_promote(&mut self) -> Result<()> {
        self.ensure_not_fenced()?;
        // Promote on disk, then swap handle without clearing first.
        match ValueLog::promote_new_and_reopen(&self.env, &self.dir) {
            Ok(new_log) => {
                if self.blob_active > 0 {
                    match ValueLog::open_blob(&self.env, &self.dir, self.blob_active) {
                        Ok(blob) => {
                            self.vlog = Some(Mutex::new(blob));
                        }
                        Err(e) => {
                            self.vlog = Some(Mutex::new(new_log));
                            self.fence_durability(&e, FenceClass::of_core(&e));
                            return Err(e);
                        }
                    }
                } else {
                    self.vlog = Some(Mutex::new(new_log));
                }
            }
            Err(e) => {
                // Rename may or may not have completed; never leave vlog=None.
                let _ = self.replace_vlog_handle(self.vlog_use_new);
                self.fence_durability(&e, FenceClass::of_core(&e));
                return Err(e);
            }
        }
        self.vlog_use_new = false;
        if let Err(e) = self.persist_manifest() {
            // Primary is live; flag false in memory. Fence so callers reopen.
            self.fence_durability(&e, FenceClass::of_core(&e));
            return Err(e);
        }
        self.vlog_gc_count = self.vlog_gc_count.saturating_add(1);
        Ok(())
    }

    /// Sealed blob files ranked by discardable ratio (highest first).
    ///
    /// Skips the active append generation (must rotate before GC). File 0
    /// (`VALUES.vlog`) is included when present and not the sole active path.
    ///
    /// # Errors
    /// I/O or CRC while sampling live pointers.
    pub fn blob_gc_candidates(&self) -> Result<Vec<BlobGcCandidate>> {
        let nums = vlog::list_blob_nums(&self.env, &self.dir);
        let mut out = Vec::new();
        for file_num in nums {
            if file_num == self.blob_active && file_num != 0 {
                // Active append gen: report but mark active (auto GC will skip).
                let path = vlog::blob_path(&self.dir, file_num);
                let bytes = self.env.metadata_len(&path).unwrap_or(0);
                out.push(BlobGcCandidate {
                    file_num,
                    bytes,
                    live_bytes: 0,
                    live_records: 0,
                    dead_ratio: 0.0,
                    is_active: true,
                });
                continue;
            }
            if file_num == 0 && self.blob_active == 0 {
                // Single-file mode: compact_vlog is the hammer; still report ratio.
            }
            let path = if file_num == 0 {
                self.dir.join(VLOG_FILE_NAME)
            } else {
                vlog::blob_path(&self.dir, file_num)
            };
            let bytes = self.env.metadata_len(&path).unwrap_or(0);
            let live = self.collect_vlog_live_for_file(file_num)?;
            let live_bytes: u64 = live.iter().map(|(_, b)| b.len() as u64).sum();
            let live_records = live.len() as u64;
            let dead_ratio = if bytes == 0 {
                0.0
            } else {
                1.0 - (live_bytes as f64 / bytes as f64)
            };
            out.push(BlobGcCandidate {
                file_num,
                bytes,
                live_bytes,
                live_records,
                dead_ratio,
                is_active: file_num == self.blob_active,
            });
        }
        out.sort_by(|a, b| {
            b.dead_ratio
                .partial_cmp(&a.dead_ratio)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.bytes.cmp(&a.bytes))
        });
        Ok(out)
    }

    /// GC the sealed blob with highest dead ratio ≥ `min_dead_ratio` (RFC-0029 P1.1).
    ///
    /// Default θ in the RFC is **0.5**. Returns `Ok(None)` when no sealed file
    /// qualifies (nothing to do). Operator can still call [`Self::compact_blob`]
    /// with an explicit id.
    ///
    /// # Errors
    /// Same as [`Self::compact_blob`].
    pub fn compact_blob_auto(
        &mut self,
        min_dead_ratio: f64,
    ) -> Result<Option<(u32, VlogRewriteStats)>> {
        self.ensure_not_fenced()?;
        let min = min_dead_ratio.clamp(0.0, 1.0);
        let pick = self.blob_gc_candidates()?.into_iter().find(|c| {
            crate::vlog_gc_kernel::blob_gc_action(c.is_active, c.bytes)
                == crate::vlog_gc_kernel::BlobGcAction::Rewrite
                && c.dead_ratio + f64::EPSILON >= min
        });
        let Some(c) = pick else {
            return Ok(None);
        };
        let st = self.compact_blob(c.file_num)?;
        Ok(Some((c.file_num, st)))
    }

    /// GC one sealed blob generation (RFC-0029 P0.2).
    ///
    /// Rewrites live records of `file_num` into a new blob, remaps only SSTs that
    /// mention that generation, then deletes the old file. Refuses the **active**
    /// append file (rotate first, or use [`Self::compact_vlog`] for file 0).
    ///
    /// # Errors
    /// I/O, CRC, active-file refuse, or durability fence.
    pub fn compact_blob(&mut self, file_num: u32) -> Result<VlogRewriteStats> {
        self.ensure_not_fenced()?;
        if file_num == 0 {
            return self.compact_vlog();
        }
        if file_num == self.blob_active {
            return Err(CoreError::Internal(
                "compact_blob refuses the active append generation (rotate first)".into(),
            ));
        }
        self.flush()?;
        // F2: the sealed blob is deleted after the remap — its WAL pointers
        // must be gone (rotated), not merely shadowed by newer mem state.
        self.ensure_wal_rotated_for_gc()?;
        let live = self.collect_vlog_live_for_file(file_num)?;
        let src = vlog::blob_path(&self.dir, file_num);
        let bytes_before = self.env.metadata_len(&src).unwrap_or(0);
        let dest_num = vlog::list_blob_nums(&self.env, &self.dir)
            .last()
            .copied()
            .unwrap_or(self.blob_active)
            .saturating_add(1)
            .max(self.blob_active.saturating_add(1));
        let (stats, remap) = ValueLog::<E::File>::rewrite_live_to_blob(
            &self.env,
            &self.dir,
            dest_num,
            &live,
            bytes_before,
        )?;
        let prepared = match self.prepare_remapped_ssts_blob(file_num, &remap) {
            Ok(p) => p,
            Err(e) => {
                let _ = self.remove_db_file(&vlog::blob_path(&self.dir, dest_num));
                return Err(e);
            }
        };
        let old_paths = prepared.old_paths;
        let next_file_num = prepared.next_file_num;
        let new_tables = prepared.tables;
        let new_levels = prepared.levels;
        let staged_bytes = prepared.bytes_written;

        let prev_ssts = std::mem::replace(&mut self.ssts, new_tables);
        let prev_levels = std::mem::replace(&mut self.sst_levels, new_levels);
        let prev_next = self.next_file_num;
        self.next_file_num = next_file_num;
        self.note_sst_inventory_changed();

        if let Err(e) = self.persist_manifest() {
            self.ssts = prev_ssts;
            self.sst_levels = prev_levels;
            self.next_file_num = prev_next;
            self.note_sst_inventory_changed();
            let _ = self.remove_db_file(&vlog::blob_path(&self.dir, dest_num));
            return Err(e);
        }

        let remap_fn = |stored: &Bytes| vlog::remap_stored_blob(stored, file_num, &remap);
        self.mem.map_values(remap_fn);
        if let Some(ref mut imm) = self.imm {
            imm.map_values(remap_fn);
        }
        self.bytes_written_sst = self.bytes_written_sst.saturating_add(staged_bytes);
        for t in &self.ssts {
            self.table_cache.insert(Arc::new(t.clone()));
        }
        for path in old_paths {
            let _ = self.remove_db_file(&path);
        }
        let _ = self.remove_db_file(&src);
        let _ = self.env.sync_dir(&self.dir);
        self.vlog_gc_count = self.vlog_gc_count.saturating_add(1);
        Ok(stats)
    }

    fn collect_vlog_live_for_file(&self, file_num: u32) -> Result<Vec<(u64, Bytes)>> {
        let mut meta: std::collections::BTreeMap<u64, (u32, u32)> =
            std::collections::BTreeMap::new();
        let mut consider = |stored: &Bytes| {
            if let Some(ptr) = vlog::decode_vlog_ptr(stored.as_ref()) {
                if ptr.file_num == file_num {
                    meta.entry(ptr.offset).or_insert((ptr.len, ptr.crc));
                }
            }
        };
        for (_, v) in self.mem.iter_internal() {
            consider(v);
        }
        if let Some(ref imm) = self.imm {
            for (_, v) in imm.iter_internal() {
                consider(v);
            }
        }
        for t in &self.ssts {
            for (_, v) in t.entries_cloned()? {
                consider(&v);
            }
        }
        let Some(ref handle) = self.vlog else {
            return Ok(Vec::new());
        };
        let guard = handle.lock();
        let mut live = Vec::with_capacity(meta.len());
        for (off, (len, crc)) in meta {
            let ptr = vlog::VlogPtr {
                file_num,
                offset: off,
                len,
                crc,
            };
            live.push((
                off,
                guard.read_ptr_on(&self.env, &self.dir, ptr, self.vlog_use_new)?,
            ));
        }
        Ok(live)
    }

    fn prepare_remapped_ssts_blob<S: std::hash::BuildHasher>(
        &self,
        file_num: u32,
        remap: &std::collections::HashMap<u64, Bytes, S>,
    ) -> Result<PreparedVlogSsts> {
        let mut next_file_num = self.next_file_num;
        let mut new_tables = Vec::with_capacity(self.ssts.len());
        let mut new_levels = Vec::with_capacity(self.sst_levels.len());
        let mut old_paths = Vec::new();
        let mut staged_paths = Vec::new();
        let mut bytes_written = 0u64;
        let remap_one = |stored: &Bytes| vlog::remap_stored_blob(stored, file_num, remap);

        for (idx, table) in self.ssts.iter().enumerate() {
            let mentions = table.entries_cloned()?.iter().any(|(_, v)| {
                vlog::decode_vlog_ptr(v.as_ref()).is_some_and(|p| p.file_num == file_num)
            });
            let level = self.sst_levels.get(idx).copied().unwrap_or(0);
            if !mentions {
                new_tables.push(table.clone());
                new_levels.push(level);
                continue;
            }
            let num = next_file_num;
            next_file_num = next_file_num.saturating_add(1);
            let dest = VersionSet::sst_path(&self.dir, num);
            let tmp = dest.with_extension("sst.tmp");
            staged_paths.push(tmp.clone());
            staged_paths.push(dest.clone());
            let entries: Vec<(InternalKey, Bytes)> = table
                .entries_cloned()?
                .into_iter()
                .map(|(k, v)| (k, remap_one(&v)))
                .collect();
            match write_sst_entries_on(&self.env, &tmp, &entries) {
                Ok(_) => {}
                Err(e) => {
                    for p in &staged_paths {
                        let _ = self.remove_db_file(p);
                    }
                    return Err(e);
                }
            }
            if let Err(e) = self.env.rename(&tmp, &dest) {
                for p in &staged_paths {
                    let _ = self.remove_db_file(p);
                }
                return Err(CoreError::Io(e));
            }
            let written = self.env.metadata_len(&dest).unwrap_or(0);
            bytes_written = bytes_written.saturating_add(written);
            match SstTable::open_on(&self.env, dest) {
                Ok(t) => {
                    old_paths.push(table.path().to_path_buf());
                    new_tables.push(t.with_cf(table.cf().to_string()));
                    new_levels.push(level);
                }
                Err(e) => {
                    for p in &staged_paths {
                        let _ = self.remove_db_file(p);
                    }
                    return Err(e);
                }
            }
        }
        Ok(PreparedVlogSsts {
            tables: new_tables,
            levels: new_levels,
            old_paths,
            next_file_num,
            bytes_written,
        })
    }

    /// Open (or replace) the value-log handle for `use_new` without ever assigning
    /// `self.vlog = None` first.
    ///
    /// On open failure: keep any existing handle (never None-gap), set
    /// [`Self::durability_fenced`] so writers stop (post-commit mismatch is unsafe
    /// to keep serving), and return the I/O error.
    ///
    /// # Errors
    /// I/O opening the log.
    fn replace_vlog_handle(&mut self, use_new: bool) -> Result<()> {
        self.vlog_sync_pending()?;
        // File-0 GC must not steal the append handle off a numbered blob.
        let opened = if self.blob_active > 0 {
            ValueLog::open_blob(&self.env, &self.dir, self.blob_active)
        } else {
            ValueLog::open_with_flag(&self.env, &self.dir, use_new)
        };
        match opened {
            Ok(log) => {
                self.vlog = Some(Mutex::new(log));
                Ok(())
            }
            Err(e) => {
                // Retry once; still never clear the old handle first.
                let retry = if self.blob_active > 0 {
                    ValueLog::open_blob(&self.env, &self.dir, self.blob_active)
                } else {
                    ValueLog::open_with_flag(&self.env, &self.dir, use_new)
                };
                if let Ok(log) = retry {
                    self.vlog = Some(Mutex::new(log));
                    return Ok(());
                }
                // Leave prior handle in place if any; fence so puts stop.
                self.fence_durability(&e, FenceClass::of_core(&e));
                Err(e)
            }
        }
    }

    /// Prepare phase: stage `.new` + remapped SST files; **no** mutation of inventory /
    /// `vlog_use_new` / mem / `self.vlog`.
    fn prepare_vlog_gc(&self) -> Result<VlogGcPrepared> {
        let live = self.collect_vlog_live_payloads()?;
        let (stats, remap) = ValueLog::<E::File>::rewrite_live_to_new(&self.env, &self.dir, &live)?;
        let prepared = match self.prepare_remapped_ssts(&remap) {
            Ok(p) => p,
            Err(e) => {
                let _ = self.remove_db_file(&self.dir.join(crate::vlog::VLOG_NEW_NAME));
                return Err(e);
            }
        };
        Ok(VlogGcPrepared {
            stats,
            remap,
            ssts: prepared,
        })
    }

    /// Install phase: MANIFEST commit then handle swap + mem remap.
    fn install_vlog_gc(&mut self, prepared: VlogGcPrepared) -> Result<VlogRewriteStats> {
        let VlogGcPrepared { stats, remap, ssts } = prepared;
        let old_paths = ssts.old_paths;
        let next_file_num = ssts.next_file_num;
        let new_tables = ssts.tables;
        let new_levels = ssts.levels;
        let staged_bytes = ssts.bytes_written;

        let prev_ssts = std::mem::replace(&mut self.ssts, new_tables);
        let prev_levels = std::mem::replace(&mut self.sst_levels, new_levels);
        let prev_next = self.next_file_num;
        self.next_file_num = next_file_num;
        self.vlog_use_new = true;
        self.note_sst_inventory_changed();

        if let Err(e) = self.persist_manifest() {
            self.ssts = prev_ssts;
            self.sst_levels = prev_levels;
            self.next_file_num = prev_next;
            self.vlog_use_new = false;
            self.note_sst_inventory_changed();
            let _ = self.remove_db_file(&self.dir.join(crate::vlog::VLOG_NEW_NAME));
            return Err(e);
        }

        // Commit point: MANIFEST durable with remapped SSTs + use_new.
        // Open .new handle BEFORE remapping mem so a failed open does not leave
        // remapped mem + old handle. Never clear vlog first.
        if let Err(e) = self.replace_vlog_handle(true) {
            // Inventory is committed; must not serve mismatched handle.
            self.fence_durability(&e, FenceClass::of_core(&e));
            return Err(e);
        }

        let remap_fn = |stored: &Bytes| vlog::remap_stored_value(stored, &remap);
        self.mem.map_values(remap_fn);
        if let Some(ref mut imm) = self.imm {
            imm.map_values(remap_fn);
        }
        self.bytes_written_sst = self.bytes_written_sst.saturating_add(staged_bytes);
        for t in &self.ssts {
            self.adopt_sst(t);
            self.table_cache.insert(Arc::new(t.clone()));
        }
        for path in old_paths {
            let _ = self.remove_db_file(&path);
        }
        Ok(stats)
    }

    /// Collect distinct live vlog records (offset → payload) from mem/imm/SSTs.
    fn collect_vlog_live_payloads(&self) -> Result<Vec<(u64, Bytes)>> {
        let mut meta: std::collections::BTreeMap<u64, (u32, u32)> =
            std::collections::BTreeMap::new();
        let mut consider = |stored: &Bytes| {
            if let Some((off, len, crc)) = vlog::decode_vlog_ref(stored.as_ref()) {
                meta.entry(off).or_insert((len, crc));
            }
        };
        for (_, v) in self.mem.iter_internal() {
            consider(v);
        }
        if let Some(ref imm) = self.imm {
            for (_, v) in imm.iter_internal() {
                consider(v);
            }
        }
        for t in &self.ssts {
            for (_, v) in t.entries_cloned()? {
                consider(&v);
            }
        }
        let Some(ref vlog) = self.vlog else {
            return Ok(Vec::new());
        };
        let guard = vlog.lock();
        let mut live = Vec::with_capacity(meta.len());
        for (off, (len, crc)) in meta {
            let data = guard.read_ptr_on(
                &self.env,
                &self.dir,
                vlog::VlogPtr {
                    file_num: 0,
                    offset: off,
                    len,
                    crc,
                },
                self.vlog_use_new,
            )?;
            live.push((off, data));
        }
        Ok(live)
    }

    /// Prepare remapped SST files on disk without mutating [`Self::ssts`].
    ///
    /// On success returns the new inventory; on failure leaves `self` unchanged
    /// (except best-effort cleanup of this attempt's tmp/final staged SST paths).
    fn prepare_remapped_ssts<S: std::hash::BuildHasher>(
        &self,
        remap: &std::collections::HashMap<u64, Bytes, S>,
    ) -> Result<PreparedVlogSsts> {
        let mut next_file_num = self.next_file_num;
        let mut new_tables = Vec::with_capacity(self.ssts.len());
        let mut new_levels = Vec::with_capacity(self.sst_levels.len());
        let mut old_paths = Vec::new();
        let mut staged_paths = Vec::new();
        let mut bytes_written = 0u64;

        let cleanup_staged = |env: &E, paths: &[PathBuf]| {
            for p in paths {
                let _ = env.remove_file(p);
            }
        };

        for (table, &level) in self.ssts.iter().zip(self.sst_levels.iter()) {
            let entries = table.entries_cloned()?;
            let needs = entries.iter().any(|(_, v)| {
                vlog::decode_vlog_ref(v.as_ref())
                    .is_some_and(|(off, _, _)| remap.contains_key(&off))
            });
            if !needs {
                new_tables.push(table.clone());
                new_levels.push(level);
                continue;
            }
            let remapped: Vec<(InternalKey, Bytes)> = entries
                .into_iter()
                .map(|(k, v)| (k, vlog::remap_stored_value(&v, remap)))
                .collect();
            let num = next_file_num;
            next_file_num = num + 1;
            let final_path = self.dir.join(format!("{num:06}.sst"));
            let tmp_path = self.dir.join(format!("{num:06}.sst.tmp"));
            match write_sst_entries_on(&self.env, &tmp_path, &remapped) {
                Ok(t) => {
                    drop(t);
                    if let Err(e) = self.env.rename(&tmp_path, &final_path) {
                        let _ = self.remove_db_file(&tmp_path);
                        cleanup_staged(&self.env, &staged_paths);
                        return Err(e.into());
                    }
                    if let Err(e) = self.sync_dir_if_required(&self.dir) {
                        let _ = self.remove_db_file(&final_path);
                        cleanup_staged(&self.env, &staged_paths);
                        return Err(e);
                    }
                    match SstTable::open_on(&self.env, &final_path) {
                        Ok(new_table) => {
                            if let Ok(len) = self.env.metadata_len(&final_path) {
                                bytes_written = bytes_written.saturating_add(len);
                            }
                            staged_paths.push(final_path.clone());
                            old_paths.push(table.path().to_path_buf());
                            new_tables.push(new_table.with_cf(table.cf().to_string()));
                            new_levels.push(level);
                        }
                        Err(e) => {
                            let _ = self.remove_db_file(&final_path);
                            cleanup_staged(&self.env, &staged_paths);
                            return Err(e);
                        }
                    }
                }
                Err(e) => {
                    let _ = self.remove_db_file(&tmp_path);
                    cleanup_staged(&self.env, &staged_paths);
                    return Err(e);
                }
            }
        }

        Ok(PreparedVlogSsts {
            tables: new_tables,
            levels: new_levels,
            next_file_num,
            old_paths,
            bytes_written,
        })
    }

    /// Begin a multi-key write transaction (single-writer: exclusive `&mut self`).
    pub fn begin(&mut self) -> Transaction<'_, E> {
        Transaction::new(self)
    }

    /// Put `key → value` (auto-commit, one sequence).
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        self.put_with(key, value, WriteOptions::default())?;
        Ok(())
    }

    /// Put and return the assigned commit sequence (RFC-0019 P0.2 layer pin).
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn put_with_seq(
        &mut self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<SequenceNumber> {
        self.put_with(key, value, WriteOptions::default())
    }

    /// Put with explicit [`WriteOptions`]; returns the commit sequence of the write.
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn put_with(
        &mut self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
        opts: WriteOptions,
    ) -> Result<SequenceNumber> {
        crate::buggify_hooks::inject_checked(crate::buggify_hooks::sites::AFTER_MEM_INSERT)?;
        crate::buggify_hooks::inject_checked(crate::buggify_hooks::sites::AFTER_WAL_APPEND)?;
        self.apply_batch_with([BatchOp::put(key, value)], opts)
    }

    /// Put only if `key` has no live value (RFC-0019 CAS / LWT substitute).
    ///
    /// # Errors
    /// [`CoreError::CasMismatch`] if the key already exists; WAL I/O otherwise.
    pub fn put_if_absent(
        &mut self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<SequenceNumber> {
        self.put_if_absent_with(key, value, WriteOptions::default())
    }

    /// [`put_if_absent`](Self::put_if_absent) with [`WriteOptions`].
    ///
    /// # Errors
    /// [`CoreError::CasMismatch`] or WAL I/O.
    pub fn put_if_absent_with(
        &mut self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
        opts: WriteOptions,
    ) -> Result<SequenceNumber> {
        self.ensure_not_fenced()?;
        let k = key.as_ref();
        if self.get(k).is_some() {
            return Err(CoreError::CasMismatch);
        }
        self.put_with(k, value, opts)
    }

    /// Put `value` only if the live value equals `expected` (RFC-0019 CAS).
    ///
    /// # Errors
    /// [`CoreError::CasMismatch`] if missing or different; WAL I/O otherwise.
    pub fn put_if_eq(
        &mut self,
        key: impl AsRef<[u8]>,
        expected: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<SequenceNumber> {
        self.put_if_eq_with(key, expected, value, WriteOptions::default())
    }

    /// [`put_if_eq`](Self::put_if_eq) with [`WriteOptions`].
    ///
    /// # Errors
    /// [`CoreError::CasMismatch`] or WAL I/O.
    pub fn put_if_eq_with(
        &mut self,
        key: impl AsRef<[u8]>,
        expected: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
        opts: WriteOptions,
    ) -> Result<SequenceNumber> {
        self.ensure_not_fenced()?;
        let k = key.as_ref();
        match self.get(k) {
            Some(cur) if cur.as_ref() == expected.as_ref() => self.put_with(k, value, opts),
            _ => Err(CoreError::CasMismatch),
        }
    }

    /// Alias for [`put_if_eq`](Self::put_if_eq) (compare-and-swap).
    ///
    /// # Errors
    /// Same as [`put_if_eq`](Self::put_if_eq).
    pub fn compare_and_swap(
        &mut self,
        key: impl AsRef<[u8]>,
        expected: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<SequenceNumber> {
        self.put_if_eq(key, expected, value)
    }

    /// Delete `key` (auto-commit tombstone).
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn delete(&mut self, key: impl AsRef<[u8]>) -> Result<()> {
        self.delete_with(key, WriteOptions::default())?;
        Ok(())
    }

    /// Delete and return the tombstone sequence (RFC-0019 P0.2).
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn delete_with_seq(&mut self, key: impl AsRef<[u8]>) -> Result<SequenceNumber> {
        self.delete_with(key, WriteOptions::default())
    }

    /// Delete with explicit [`WriteOptions`]; returns the commit sequence.
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn delete_with(
        &mut self,
        key: impl AsRef<[u8]>,
        opts: WriteOptions,
    ) -> Result<SequenceNumber> {
        self.apply_batch_with([BatchOp::delete(key)], opts)
    }

    /// Range-delete `[start, end)` (end exclusive). Keys outside remain.
    ///
    /// Implemented as a range tombstone in the WAL/MemTable; compaction with
    /// [`CompactOptions::latest_only`] drops covered keys.
    ///
    /// # Errors
    /// WAL I/O, sequence exhaustion, or `start >= end`.
    pub fn delete_range(&mut self, start: impl AsRef<[u8]>, end: impl AsRef<[u8]>) -> Result<()> {
        self.delete_range_with(start, end, WriteOptions::default())
    }

    /// [`delete_range`](Self::delete_range) with [`WriteOptions`].
    ///
    /// # Errors
    /// WAL I/O, sequence exhaustion, or invalid bounds.
    pub fn delete_range_with(
        &mut self,
        start: impl AsRef<[u8]>,
        end: impl AsRef<[u8]>,
        opts: WriteOptions,
    ) -> Result<()> {
        let s = start.as_ref();
        let e = end.as_ref();
        if s >= e {
            return Err(CoreError::Internal(
                "delete_range requires start < end".into(),
            ));
        }
        self.apply_batch_with([BatchOp::delete_range(s, e)], opts)?;
        Ok(())
    }

    /// Point lookups for many keys at the latest snapshot (RFC-0019 P1.1).
    ///
    /// Order matches `keys`; each entry is the same as [`Self::get`] for that
    /// key — including its fail-stop-on-corruption contract (F1).
    ///
    /// # Panics
    /// If any stored value is a vlog reference whose payload fails CRC/I-O.
    #[must_use]
    pub fn multi_get(&self, keys: &[impl AsRef<[u8]>]) -> Vec<Option<Bytes>> {
        keys.iter().map(|k| self.get(k.as_ref())).collect()
    }

    /// [`multi_get`](Self::multi_get) at an explicit [`Snapshot`].
    #[must_use]
    /// Multi-get at an explicit snapshot.
    ///
    /// Each key answers exactly as [`Self::get_at`] — including the
    /// below-watermark tier read and LSM fallback (RFC-0046 P2.1/P2.3):
    /// one per-key loop, one visibility contract.
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`] if `snap` is below the version-GC
    /// watermark and history for a requested key cannot be covered by the
    /// retained tier or a surviving LSM version.
    pub fn multi_get_at(
        &self,
        snap: Snapshot,
        keys: &[impl AsRef<[u8]>],
    ) -> Result<Vec<Option<Bytes>>> {
        let mut out = Vec::with_capacity(keys.len());
        for k in keys {
            out.push(self.get_at(snap, k.as_ref())?);
        }
        Ok(out)
    }

    /// Changes with `from_seq < sequence <= to_seq` (RFC-0019 change feed).
    ///
    /// # Errors
    /// [`CoreError::CorruptValue`] when a live entry's vlog payload fails
    /// CRC/I-O during a lazy rebuild (F1: corruption is an error, never a
    /// raw pointer served as the user value).
    /// [`CoreError::SnapshotTooOld`] when the window starts below the
    /// retention watermark (RFC-0046): versions below it — including lone
    /// tombstones — may have been GC'd out of every source, so a partial
    /// `Ok` would silently drop events. Catch up from `earliest_readable`
    /// (or the last sequence a previous feed call returned) instead.
    pub fn changes(
        &self,
        from_seq: SequenceNumber,
        to_seq: SequenceNumber,
    ) -> Result<Vec<ChangeEntry>> {
        let first = from_seq.saturating_add(1);
        if first < self.earliest_readable_seq {
            return Err(CoreError::SnapshotTooOld {
                requested: first,
                earliest: self.earliest_readable_seq,
            });
        }
        let to = to_seq.min(self.last_sequence());
        let entries = if self.feed_is_lazy() {
            self.lazy_feed_entries()?
                .into_iter()
                .filter(|e| e.sequence > from_seq && e.sequence <= to)
                .collect::<Vec<_>>()
        } else {
            self.change_log.changes_in(from_seq, to)
        };
        self.resolve_feed_entries(entries)
    }

    /// All durable changes with `sequence > from_seq` (tail / watch catch-up).
    ///
    /// Below the retention watermark (RFC-0046) this returns what survived
    /// GC — fine for the last-write-wins seeding callers use it for (the
    /// newest version of every key always survives GC, and a dropped lone
    /// tombstone leaves the key absent, which is the same final state),
    /// NOT exact event history. For an exact windowed feed use
    /// [`Self::changes`], which fails `SnapshotTooOld` below the watermark.
    ///
    /// Fail-stop on a corrupt value log (F1): the `Vec` shape cannot express
    /// the error and a swallowed resolve would serve the raw VLG pointer as
    /// the user value. Use [`Self::changes`] for an error-shaped read.
    ///
    /// # Panics
    /// If a live entry's vlog payload fails CRC/I/O.
    #[must_use]
    pub fn changes_after(&self, from_seq: SequenceNumber) -> Vec<ChangeEntry> {
        let from = from_seq.min(self.last_sequence());
        let entries = if self.feed_is_lazy() {
            match self.lazy_feed_entries() {
                Ok(e) => e,
                Err(e) => fail_stop_corrupt_value("changes_after feed rebuild", &e),
            }
            .into_iter()
            .filter(|e| e.sequence > from)
            .collect::<Vec<_>>()
        } else {
            self.change_log.changes_after(from)
        };
        match self.resolve_feed_entries(entries) {
            Ok(e) => e,
            // F1 contract (doc above): never serve the raw pointer; fail-stop.
            Err(e) => fail_stop_corrupt_value("changes_after feed resolve", &e),
        }
    }

    /// F190: cached feed entries are stored-form values (vlog pointer or the
    /// F188 inline escape) — resolve to user values at the read boundary so
    /// every feed path agrees with `collect_feed_from_live` (which resolves).
    /// Range-delete entries carry the exclusive end key, not a value: as-is.
    ///
    /// # Errors
    /// [`CoreError::CorruptValue`] when a spilled payload fails CRC/I/O.
    fn resolve_feed_entries(&self, entries: Vec<ChangeEntry>) -> Result<Vec<ChangeEntry>> {
        let mut out = Vec::with_capacity(entries.len());
        for mut e in entries {
            if e.kind == ChangeKind::Put {
                e.value = self.resolve_stored_value(e.value)?;
            }
            out.push(e);
        }
        Ok(out)
    }

    /// `changelog_interval == 0`: do not grow an in-memory ChangeEntry vec on
    /// every write (RFC-0039 P0.3 / RFC-0041 P1.1). Watchers rebuild last-per-key
    /// from mem+SST; flush/close still persist.
    fn feed_is_lazy(&self) -> bool {
        self.changelog_interval == 0
    }

    /// Live entry count the lazy CHANGELOG rebuild would materialize
    /// (mem + imm + every SST). Same view `collect_feed_from_live` walks.
    fn live_entry_estimate(&self) -> u64 {
        let mem = self.mem.len() as u64;
        let imm = self.imm.as_ref().map_or(0, |m| m.len() as u64);
        let ssts: u64 = self.ssts.iter().map(|t| t.len() as u64).sum();
        mem + imm + ssts
    }

    /// Full WAL history when the log is still live; last-per-key after rotate.
    ///
    /// # Errors
    /// [`CoreError::CorruptValue`] via [`Self::collect_feed_from_live`].
    fn lazy_feed_entries(&self) -> Result<Vec<ChangeEntry>> {
        let from_wal = self.collect_feed_from_wal();
        // F183: the WAL tail only covers writes since the last rotate. Keys
        // already flushed to SST + persisted CHANGELOG were silently dropped
        // by the WAL-first short-circuit (`put A; flush; put B` → feed `[B]`).
        // Union instead: flush-time last-per-key cache + newer WAL ops, in
        // sequence order.
        if !self.change_log.is_empty() {
            let mut out = self.change_log.changes_after(0);
            if from_wal.is_empty() {
                return Ok(out);
            }
            // Per-key cutoff, not `out.last().sequence`. The cache is
            // last-per-key sorted by seq, so the global max is some other
            // key's latest — WAL ops for a key whose cached latest is older
            // (snapshot wipe Delete, then export Put) were dropped, and
            // watchers/oracles saw a delete while `get` served the restore
            // (F-found World seed 502514).
            let mut cached_latest = std::collections::BTreeMap::<bytes::Bytes, u64>::new();
            for e in &out {
                cached_latest
                    .entry(e.key.clone())
                    .and_modify(|s| *s = (*s).max(e.sequence))
                    .or_insert(e.sequence);
            }
            out.extend(
                from_wal
                    .into_iter()
                    .filter(|e| e.sequence > cached_latest.get(&e.key).copied().unwrap_or(0)),
            );
            out.sort_by_key(|e| e.sequence);
            return Ok(out);
        }
        if !from_wal.is_empty() {
            return Ok(from_wal);
        }
        self.collect_feed_from_live()
    }

    fn collect_feed_from_wal(&self) -> Vec<ChangeEntry> {
        let path = self.dir.join(WAL_FILE_NAME);
        if !self.env.exists(&path) {
            return Vec::new();
        }
        let Ok((records, _, _resync)) =
            crate::wal::Wal::<E::File>::recover_span_on(&self.env, &path)
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for raw in records {
            let Ok(rec) = WriteRecord::decode(&raw) else {
                break;
            };
            for op in rec.ops {
                out.push(ChangeEntry::from_write_op(&op));
            }
        }
        out
    }

    /// # Errors
    /// [`CoreError::CorruptValue`] when a live entry's vlog payload fails
    /// CRC/I-O (F1: never serve the raw pointer as the user value).
    fn collect_feed_from_live(&self) -> Result<Vec<ChangeEntry>> {
        let mut latest: BTreeMap<Bytes, (InternalKey, Bytes)> = BTreeMap::new();
        let consider = |map: &mut BTreeMap<Bytes, (InternalKey, Bytes)>,
                        ik: InternalKey,
                        v: Bytes| match map.get(&ik.user_key) {
            Some((old, _)) if old.sequence >= ik.sequence => {}
            _ => {
                map.insert(ik.user_key.clone(), (ik, v));
            }
        };
        for (ik, v) in self.mem.iter_internal() {
            consider(&mut latest, ik.clone(), v.clone());
        }
        if let Some(ref imm) = self.imm {
            for (ik, v) in imm.iter_internal() {
                consider(&mut latest, ik.clone(), v.clone());
            }
        }
        for sst in &self.ssts {
            // Streaming keeps L0 lazy (`entries_cloned` filled the
            // materialize cache on every explicit flush and broke the
            // lazy-input invariant of streaming L0 compact).
            let mut stream = sst.iter_internal_streaming();
            loop {
                match stream.next_entry() {
                    Ok(Some((ik, v))) => consider(&mut latest, ik, v),
                    Ok(None) => break,
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            table = %sst.path().display(),
                            "CHANGELOG feed rebuild: corrupt SST skipped (feed rebuilt on open)"
                        );
                        break;
                    }
                }
            }
        }
        let mut out = Vec::with_capacity(latest.len());
        for (ik, v) in latest.into_values() {
            let value = self.resolve_stored_value(v.clone())?;
            out.push(ChangeEntry {
                sequence: ik.sequence,
                key: ik.user_key,
                kind: match ik.kind {
                    ValueType::Value => ChangeKind::Put,
                    ValueType::Deletion => ChangeKind::Delete,
                    ValueType::RangeDeletion => ChangeKind::DeleteRange,
                },
                value,
            });
        }
        // Hardening (B2, latent): last-per-key comes out in user-key order,
        // not sequence order. Every current caller re-sorts via
        // `replace_sorted`, but the feed contract is non-decreasing sequence
        // (watcher cursors) — enforce it at the source.
        out.sort_by_key(|e| e.sequence);
        Ok(out)
    }

    /// When CHANGELOG is missing after flush (WAL already truncated), rebuild a
    /// last-per-key feed from MemTable ∪ SSTs so fold/journal are not empty.
    fn maybe_rebuild_feed_from_live(&mut self) {
        let feed_empty = self.change_log.max_sequence().unwrap_or(0) == 0;
        if !changelog_needs_sst_rebuild(feed_empty, self.last_sequence()) {
            return;
        }
        // Same rebuild budget as the flush path: above it the cache stays
        // empty and `lazy_feed_entries` rebuilds on demand (WAL / live).
        if !crate::changelog_kernel::changelog_rebuild_within_budget(
            self.live_entry_estimate(),
            self.changelog_rebuild_budget_entries,
        ) {
            return;
        }
        // Best-effort cache rebuild: a corrupt payload keeps the (stale but
        // WAL-covered) feed as-is instead of failing the flush.
        let entries = match self.collect_feed_from_live() {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "CHANGELOG rebuild skipped: value-log read failed");
                return;
            }
        };
        if entries.is_empty() {
            return;
        }
        self.change_log.replace_sorted(entries);
        self.persist_changelog_best_effort();
    }

    /// Apply an ordered multi-op batch atomically (one WAL record, no OCC).
    ///
    /// For Raft/log apply and bulk import: sequences are assigned in order;
    /// either the whole batch is durable on success or none of it is visible
    /// after recovery.
    ///
    /// Returns the sequence of the last op in the batch (or current
    /// `last_sequence` if the batch is empty).
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn apply_batch(
        &mut self,
        ops: impl IntoIterator<Item = BatchOp>,
    ) -> Result<SequenceNumber> {
        self.apply_batch_with(ops, WriteOptions::default())
    }

    /// [`apply_batch`](Self::apply_batch) with [`WriteOptions`].
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn apply_batch_with(
        &mut self,
        batch: impl IntoIterator<Item = BatchOp>,
        durability: WriteOptions,
    ) -> Result<SequenceNumber> {
        let batch: Vec<BatchOp> = batch.into_iter().collect();
        if !self.write_admission_idle() {
            let families = self.batch_families(&batch);
            self.ensure_write_admitted_for(&families)?;
        }
        self.observe_bulk_batch(&batch);
        // Assign sequences only for this attempt; roll back `next_seq` if WAL fails
        // so a failed multi-op does not burn sequence space (TX denser / mid-commit).
        let seq_checkpoint = self.next_seq;
        let mut records = Vec::new();
        for op in batch {
            let seq = match self.alloc_seq() {
                Ok(s) => s,
                Err(e) => {
                    self.next_seq = seq_checkpoint;
                    return Err(e);
                }
            };
            match op {
                BatchOp::Put { key, value } => {
                    self.bytes_ingested = self.bytes_ingested.saturating_add(value.len() as u64);
                    let stored = match self.maybe_spill_large_value(value) {
                        Ok(v) => v,
                        Err(e) => {
                            self.next_seq = seq_checkpoint;
                            return Err(e);
                        }
                    };
                    records.push(WriteOp::put(seq, key, stored));
                }
                BatchOp::Delete { key } => {
                    records.push(WriteOp::delete(seq, key));
                }
                BatchOp::DeleteRange { start, end } => {
                    records.push(WriteOp::delete_range(seq, start, end));
                }
            }
        }
        if records.is_empty() {
            return Ok(self.last_sequence());
        }
        match self.commit_ops_with(records, durability) {
            Ok(()) => {
                // F18: the write is already durable (WAL fsync under sync=true). Auto-flush
                // is a background space concern — failing it must not surface as "put/commit
                // failed" or clients will retry and the operator loses the success signal.
                self.maybe_auto_flush_best_effort();
                Ok(self.last_sequence())
            }
            Err(e) => {
                self.next_seq = seq_checkpoint;
                Err(e)
            }
        }
    }

    /// Flush WAL according to open options (for tests / graceful shutdown).
    ///
    /// After a series of `WriteOptions::no_sync()` writes, call this to make
    /// them durable (group fsync).
    ///
    /// # Errors
    /// I/O from fsync, or [`CoreError::DurabilityFenced`].
    pub fn sync(&mut self) -> Result<()> {
        self.ensure_not_fenced()?;
        self.wal.lock().sync_data()
    }

    /// The first fence's report, if this Db was ever durability-fenced
    /// (RFC-0047 P1.1). Present even after [`Self::recover_from_fence`].
    #[must_use]
    pub fn fence_report(&self) -> Option<&FenceReport> {
        self.fence_report.as_ref()
    }

    /// RFC-0047 P1.1: assisted close+replay+reopen after a durability
    /// fence. The kernel stays fail-closed (writes refused); this is the
    /// one-call evacuation: reopen rebuilds from WAL/MANIFEST and the
    /// typed [`FenceRecovery`] says which in-flight sequences were
    /// uncertain and whether the replay proved them lost (G5: the report
    /// never pretends to know).
    ///
    /// `Ok(None)` = not fenced (no-op, nothing touched). On `Err` this Db
    /// stays unusable (lock released, writes refused) — drop it.
    ///
    /// Shared answer caches (point/count/epoch, published watermark — the
    /// ones a [`crate::concurrent::ConcurrentDb`] caches) are carried over:
    /// cleared and re-adopted so the reopened Db invalidates them going
    /// forward (never a stale hit after lost writes).
    ///
    /// # Errors
    /// Reopen I/O (old shell already closed and lock released).
    pub fn recover_from_fence(&mut self) -> Result<Option<FenceRecovery>> {
        let Some(fence) = self.fence_report.clone() else {
            return Ok(None);
        };
        let dir = self.dir.clone();
        let opts = self.open_opts;
        let env = self.env.clone();
        let point_cache = Arc::clone(&self.point_cache);
        let count_cache = Arc::clone(&self.count_cache);
        let read_cache_epoch = Arc::clone(&self.read_cache_epoch);
        let point_tls_epoch = Arc::clone(&self.point_tls_epoch);
        let key_gen = Arc::clone(&self.key_gen);
        let published_seq = Arc::clone(&self.published_seq);
        // Detach the old shell: persist what we can, release the dir lock
        // (Drop then has nothing left to release). Do NOT flush the WAL —
        // the uncertain tail is exactly what the replay must adjudicate.
        self.persist_changelog_best_effort();
        self.release_lock()?;
        let mut db = match Db::open_with_env(&dir, opts, env) {
            Ok(db) => db,
            Err(e) => {
                // Shell stays fenced with the lock released: writes keep
                // refusing; the host must drop this Db.
                return Err(e);
            }
        };
        // Adopt the carried handles into the reopened Db: caches cleared
        // (async-acked values the replay may prove undurable must not be
        // hits), epoch bumped (TLS invalidation), published watermark kept
        // — then re-published up to the replayed durable state so a
        // resumed default read observes exactly what a fresh reopen would.
        point_cache.clear();
        count_cache.clear();
        read_cache_epoch.fetch_add(1, Ordering::Release);
        point_tls_epoch.fetch_add(1, Ordering::Release);
        db.point_cache = point_cache;
        db.count_cache = count_cache;
        db.read_cache_epoch = read_cache_epoch;
        db.point_tls_epoch = point_tls_epoch;
        db.key_gen = key_gen;
        db.published_seq = published_seq;
        let replayed_through = db.last_sequence();
        db.publish_sequence(replayed_through);
        let lost_writes = fence.uncertain_from <= fence.uncertain_through
            && replayed_through < fence.uncertain_through;
        *self = db;
        Ok(Some(FenceRecovery {
            fence,
            replayed_through,
            lost_writes,
        }))
    }

    /// Close the WAL and release the directory lock via [`Env`] when held.
    ///
    /// Prefer this over bare `drop` so unlock is fault-injectable (RFC-0015 H3).
    ///
    /// # Errors
    /// I/O from WAL flush or lock release.
    pub fn close(mut self) -> Result<()> {
        // RFC-0159 P0.3: the open bulk tail is disableWAL-class for
        // *crashes* only — a clean close must not drop acked puts. Drain
        // runs to SST + MANIFEST (same as `flush`) before the CHANGELOG
        // persist, whose RAM guard skips while runs are open.
        if let Some(persist) = self.flush_all_bulk_runs()? {
            persist.write()?;
        }
        // RFC-0031: close is a persist point for the CHANGELOG cache.
        self.persist_changelog_best_effort();
        self.vlog_sync_pending()?;
        self.release_lock()?;
        // Flush in place — `Db` implements `Drop` (Env unlock), so we cannot move `wal`.
        self.wal.lock().flush()
    }

    /// Lookup visible version at `snapshot` across mem + imm + SSTs.
    ///
    /// Merges point versions and range tombstones across all layers so a range
    /// delete in a newer layer correctly hides older puts.
    pub(crate) fn lookup(&self, key: &[u8], snapshot: SequenceNumber) -> Lookup {
        if self.bulk_runs.is_empty()
            && self.parked_bulk.is_empty()
            && self.bulk_encoding.is_none()
            && self.mem.is_empty()
            && self.imm.is_none()
            && self.flush_read_pin.is_none()
            && self.parked_unflushed.is_empty()
        {
            return self.lookup_sst_packed(key, snapshot);
        }
        let fam = self.bulk_family_of_key(key);
        if let Some(run) = self.bulk_runs.get(fam) {
            match run.lookup(key, snapshot) {
                Lookup::NotFound => {}
                other => return other,
            }
        }
        for (f, run) in &self.parked_bulk {
            if f == fam {
                match run.lookup(key, snapshot) {
                    Lookup::NotFound => {}
                    other => return other,
                }
            }
        }
        if let Some((f, run)) = &self.bulk_encoding {
            if f == fam {
                match run.lookup(key, snapshot) {
                    Lookup::NotFound => {}
                    other => return other,
                }
            }
        }
        let mut best_point_seq: Option<SequenceNumber> = None;
        let mut best_point: Lookup = Lookup::NotFound;
        let mut range_tombs = Vec::new();

        for table in self.mem_layers() {
            if table.is_empty() && !table.has_range_tombstones() {
                continue;
            }
            Self::scan_mem_for_lookup(
                table,
                key,
                snapshot,
                &mut best_point_seq,
                &mut best_point,
                &mut range_tombs,
            );
            if let Some(seq) = best_point_seq {
                // Newest mem layer with a point wins (single-writer). Skip SST.
                self.get_mem_hit.fetch_add(1, Ordering::Relaxed);
                return match best_point {
                    Lookup::Found(_)
                        if !crate::merge::visible_at(
                            crate::key::ValueType::Value,
                            range_deleted(key, seq, &range_tombs),
                        ) =>
                    {
                        Lookup::Deleted
                    }
                    other => other,
                };
            }
        }
        self.get_sst_fallback.fetch_add(1, Ordering::Relaxed);
        // Newest file with a point wins (L0 before L1). Older files cannot
        // hide a newer point; a newer tombstone is seen first.
        // Encoded-block seek: CRC-verify + decompress the one candidate block
        // and copy out only the winning value (the decoded-block cache
        // thrashed at random-key scale). Block faults fail-stop — a corrupt
        // block must never read as a miss.
        let mut seek_scratch: Option<crate::sst::PointSeekScratch> = None;
        // One probe per table: range-prune, then seek the single candidate
        // block. The bounds span every entry's user key (deletion markers
        // included), so a key outside them has no point version here.
        // Without the prune, a get walks every chunk's bloom — ~95 disjoint
        // chunks after leveled settle measured ~10 µs/get of pure candidate
        // checking (25M guest).
        let ssts = &self.ssts;
        // RFC-0161: never route point get through `point_at_cached`.
        // That path `decode_block`s every entry in the 4 KiB block
        // (`InternalKey` + `Bytes` each) and mutex-inserts into the
        // byte-budgeted `BlockCache`. lookup_100 draws 100 fresh keys
        // every Criterion iter — the insert never hits. Guest v73b
        // (Slipstream `set_block_cache` 1 GiB ⇒ `budget_bytes()>0`):
        // get_loop 5.25–5.30 ms vs v71 seeking 3.868 ms. `RAW_BLOCKS`
        // (512 × 4 KiB TLS) already covers get_hit repeats on the
        // encoded miss path. Default `BlockCache::new(8192)` has
        // `budget_bytes==0` and was already seeking.
        let mut probe = |table: &SstTable| -> Option<(SequenceNumber, Lookup)> {
            self.lookup_sst_considered.fetch_add(1, Ordering::Relaxed);
            if !table.key_may_match(key) {
                return None;
            }
            let scratch = seek_scratch.get_or_insert_with(take_tls_point_seek_scratch);
            match table.point_at_seeking(key, snapshot, scratch) {
                Ok(Some((seq, look))) => Some((seq, look)),
                Ok(None) => None,
                Err(e) => fail_stop_corrupt_block(table.path(), &e),
            }
        };
        // Levels ascend (L0 newest → L1+), newest-first inside a level —
        // the same order as the flat walk. Disjoint runs bisect to the one
        // candidate table; every other run shape keeps the linear walk.
        'runs: for run in &self.sst_runs {
            // Range tombstones always flow: a tombstone's end key lives in
            // its value, so the table bounds do not cover its span. A whole
            // run at once is behavior-preserving: `range_deleted` hides only
            // strictly newer points, so tombstones from tables older than
            // the run's winner stay inert.
            if run.any_range_tombstones {
                for &sst_i in &run.tables_newest_first {
                    ssts[sst_i].collect_range_tombstones(snapshot, &mut range_tombs);
                }
            }
            if let (Some(by_lo), Some(plos), Some(phis)) =
                (&run.sorted_by_lo, &run.packed_lo, &run.packed_hi)
            {
                let p = plos.partition_point_gt(key);
                if p == 0 {
                    continue;
                }
                if run.disjoint_by_lo.is_some() {
                    if phis.lo(p - 1) >= key {
                        if let Some((seq, look)) = probe(&ssts[by_lo[p - 1]]) {
                            best_point_seq = Some(seq);
                            best_point = look;
                            break 'runs;
                        }
                    }
                } else {
                    // Overlapping files that share a user key must probe
                    // newest-first (a newer delete hides an older put).
                    for &sst_i in &run.tables_newest_first {
                        if let Some(pos) = by_lo.iter().position(|&i| i == sst_i) {
                            if pos >= p || phis.lo(pos) < key {
                                continue;
                            }
                        }
                        if let Some((seq, look)) = probe(&ssts[sst_i]) {
                            best_point_seq = Some(seq);
                            best_point = look;
                            break 'runs;
                        }
                    }
                }
            } else {
                for &sst_i in &run.tables_newest_first {
                    if let Some((seq, look)) = probe(&ssts[sst_i]) {
                        best_point_seq = Some(seq);
                        best_point = look;
                        break 'runs;
                    }
                }
            }
        }
        if let Some(scratch) = seek_scratch {
            put_tls_point_seek_scratch(scratch);
        }

        match best_point {
            Lookup::Found(v) => {
                let seq = best_point_seq.unwrap_or(0);
                if crate::merge::visible_at(
                    crate::key::ValueType::Value,
                    range_deleted(key, seq, &range_tombs),
                ) {
                    Lookup::Found(v)
                } else {
                    Lookup::Deleted
                }
            }
            Lookup::Deleted => Lookup::Deleted,
            Lookup::NotFound => {
                if range_deleted(key, 0, &range_tombs) {
                    Lookup::Deleted
                } else {
                    Lookup::NotFound
                }
            }
        }
    }

    fn lookup_sst_packed(&self, key: &[u8], snapshot: SequenceNumber) -> Lookup {
        {
            let g = self.sst_envelope.read();
            if g.iter()
                .all(|(lo, hi)| key < lo.as_ref() || key > hi.as_ref())
            {
                return Lookup::NotFound;
            }
        }
        self.get_sst_fallback.fetch_add(1, Ordering::Relaxed);
        let mut seek_scratch: Option<crate::sst::PointSeekScratch> = None;
        let ssts = &self.ssts;
        let mut best_point_seq: Option<SequenceNumber> = None;
        let mut best_point = Lookup::NotFound;
        let mut range_tombs = Vec::new();
        let mut probe = |table: &SstTable| -> Option<(SequenceNumber, Lookup)> {
            self.lookup_sst_considered.fetch_add(1, Ordering::Relaxed);
            if !table.key_may_match(key) {
                return None;
            }
            let scratch = seek_scratch.get_or_insert_with(take_tls_point_seek_scratch);
            match table.point_at_seeking(key, snapshot, scratch) {
                Ok(Some((seq, look))) => Some((seq, look)),
                Ok(None) => None,
                Err(e) => fail_stop_corrupt_block(table.path(), &e),
            }
        };
        'runs: for run in &self.sst_runs {
            if run.any_range_tombstones {
                for &sst_i in &run.tables_newest_first {
                    ssts[sst_i].collect_range_tombstones(snapshot, &mut range_tombs);
                }
            }
            if let (Some(by_lo), Some(plos), Some(phis)) =
                (&run.sorted_by_lo, &run.packed_lo, &run.packed_hi)
            {
                let p = plos.partition_point_gt(key);
                if p == 0 {
                    continue;
                }
                if run.disjoint_by_lo.is_some() {
                    if phis.lo(p - 1) >= key {
                        if let Some((seq, look)) = probe(&ssts[by_lo[p - 1]]) {
                            best_point_seq = Some(seq);
                            best_point = look;
                            break 'runs;
                        }
                    }
                } else {
                    // Overlapping files that share a user key must probe
                    // newest-first (a newer delete hides an older put).
                    for &sst_i in &run.tables_newest_first {
                        if let Some(pos) = by_lo.iter().position(|&i| i == sst_i) {
                            if pos >= p || phis.lo(pos) < key {
                                continue;
                            }
                        }
                        if let Some((seq, look)) = probe(&ssts[sst_i]) {
                            best_point_seq = Some(seq);
                            best_point = look;
                            break 'runs;
                        }
                    }
                }
            } else {
                for &sst_i in &run.tables_newest_first {
                    if let Some((seq, look)) = probe(&ssts[sst_i]) {
                        best_point_seq = Some(seq);
                        best_point = look;
                        break 'runs;
                    }
                }
            }
        }
        if let Some(scratch) = seek_scratch {
            put_tls_point_seek_scratch(scratch);
        }
        match best_point {
            Lookup::Found(v) => {
                let seq = best_point_seq.unwrap_or(0);
                if crate::merge::visible_at(
                    crate::key::ValueType::Value,
                    range_deleted(key, seq, &range_tombs),
                ) {
                    Lookup::Found(v)
                } else {
                    Lookup::Deleted
                }
            }
            Lookup::Deleted => Lookup::Deleted,
            Lookup::NotFound => {
                if range_deleted(key, 0, &range_tombs) {
                    Lookup::Deleted
                } else {
                    Lookup::NotFound
                }
            }
        }
    }

    fn scan_mem_for_lookup(
        table: &MemTable,
        key: &[u8],
        snapshot: SequenceNumber,
        best_point_seq: &mut Option<SequenceNumber>,
        best_point: &mut Lookup,
        range_tombs: &mut Vec<crate::merge::RangeTombstone>,
    ) {
        // BTree seek — do not walk the whole memtable on every point get
        // (RFC-0032: ycsb_c / layered lookup).
        if table.has_range_tombstones() {
            table.collect_range_tombstones(snapshot, range_tombs);
        }
        if let Some((seq, look)) = table.get_entry(key, snapshot) {
            if best_point_seq.is_none_or(|s| seq > s) {
                *best_point_seq = Some(seq);
                *best_point = look;
            }
        }
    }

    pub(crate) fn alloc_seq(&mut self) -> Result<SequenceNumber> {
        let seq = self.next_seq;
        if seq > MAX_SEQUENCE_NUMBER {
            return Err(CoreError::Internal(
                "sequence number space exhausted".into(),
            ));
        }
        self.next_seq = seq + 1;
        Ok(seq)
    }

    /// Peek next sequence without allocating (TX sequence checkpoint).
    #[must_use]
    pub(crate) fn next_seq_peek(&self) -> SequenceNumber {
        self.next_seq
    }

    /// Restore sequence counter after a failed multi-op commit (no WAL durable).
    pub(crate) fn restore_next_seq(&mut self, seq: SequenceNumber) {
        self.next_seq = seq;
    }

    pub(crate) fn commit_ops_with(
        &mut self,
        records: Vec<WriteOp>,
        durability: WriteOptions,
    ) -> Result<()> {
        self.ensure_not_fenced()?;
        // Append then sync: if either fails, caller rolls back sequence; mem not applied.
        // RFC-0015 H1: if append OK and required sync fails, fence so later fsyncs
        // cannot silently publish an unacked prefix while in-process mem diverges.
        // RFC-0040: encode into WAL scratch (one payload memcpy), then move ops to mem.
        let do_sync = durability.sync.unwrap_or(self.sync);
        self.vlog_prepare_wal(do_sync)?;
        // WAL is O_APPEND: a torn write_all leaves bytes at EOF. Fence on
        // append failure the same as sync failure.
        let n = {
            let r = self.wal.lock().append_write_ops(&records);
            match r {
                Ok(n) => n,
                Err(e) => {
                    self.fence_durability(&e, FenceClass::of_core(&e));
                    return Err(e);
                }
            }
        };
        self.bytes_written_wal = self.bytes_written_wal.saturating_add(n);
        if do_sync {
            let sync_err = self.wal.lock().sync_data().err();
            if let Some(e) = sync_err {
                self.fence_durability(&e, FenceClass::of_core(&e));
                return Err(e);
            }
            self.note_wal_sync();
        }
        // In-memory change feed after durable WAL. CHANGELOG on disk is a cache:
        // never gate commit success on a second fsync/rename (RFC-0019) — reopen
        // rebuilds missing entries from WAL. Always apply mem once WAL is durable
        // so get and feed stay aligned and sequences are not rolled back.
        // Bytes::clone is a refcount — payload is not memcpy'd again.
        // interval=0: do not grow a million-entry Vec on the apply path
        // (RFC-0041 P1.1); changes() rebuilds last-per-key from live tables.
        if !self.feed_is_lazy() {
            self.change_log
                .extend(records.iter().map(ChangeEntry::from_write_op));
        }
        if do_sync {
            // RFC-0031: debounce the cache store. WAL is already durable.
            self.maybe_persist_changelog_after_durable_commit();
        }
        self.apply_ops_to_mem(records);
        Ok(())
    }

    /// Assign sequences + spill large values for a batch (no WAL yet).
    ///
    /// On error, sequence counter is restored.
    pub(crate) fn prepare_write_ops(
        &mut self,
        batch: impl IntoIterator<Item = BatchOp>,
    ) -> Result<(Vec<WriteOp>, SequenceNumber)> {
        self.prepare_write_ops_spill(batch, true)
    }

    /// Assign sequences. `spill` rewrites large values into the vlog (G1).
    /// Async coluna A (`commit_async_ops`) keeps the payload in the WAL —
    /// same class as Rocks `sync=false` (RFC-0149 P2.1 blob).
    pub(crate) fn prepare_write_ops_spill(
        &mut self,
        batch: impl IntoIterator<Item = BatchOp>,
        spill: bool,
    ) -> Result<(Vec<WriteOp>, SequenceNumber)> {
        self.ensure_not_fenced()?;
        let seq_checkpoint = self.next_seq;
        let batch = batch.into_iter();
        let mut records = Vec::with_capacity(batch.size_hint().0);
        for op in batch {
            let seq = match self.alloc_seq() {
                Ok(s) => s,
                Err(e) => {
                    self.next_seq = seq_checkpoint;
                    return Err(e);
                }
            };
            match op {
                BatchOp::Put { key, value } => {
                    self.bytes_ingested = self.bytes_ingested.saturating_add(value.len() as u64);
                    let stored = if spill {
                        match self.maybe_spill_large_value(value) {
                            Ok(v) => v,
                            Err(e) => {
                                self.next_seq = seq_checkpoint;
                                return Err(e);
                            }
                        }
                    } else {
                        escape_inline_value(value)
                    };
                    records.push(WriteOp::put(seq, key, stored));
                }
                BatchOp::Delete { key } => {
                    records.push(WriteOp::delete(seq, key));
                }
                BatchOp::DeleteRange { start, end } => {
                    records.push(WriteOp::delete_range(seq, start, end));
                }
            }
        }
        if records.is_empty() {
            return Ok((records, self.last_sequence()));
        }
        crate::batch::share_consecutive_equal_values(&mut records);
        let last = records.last().map_or(self.last_sequence(), |o| o.sequence);
        Ok((records, last))
    }

    /// Single-op form of [`Self::prepare_write_ops_spill`] (RFC-0154 P1.6).
    fn prepare_one_spill(&mut self, op: BatchOp, spill: bool) -> Result<(WriteOp, SequenceNumber)> {
        self.ensure_not_fenced()?;
        let seq_checkpoint = self.next_seq;
        let seq = match self.alloc_seq() {
            Ok(s) => s,
            Err(e) => {
                self.next_seq = seq_checkpoint;
                return Err(e);
            }
        };
        let rec = match op {
            BatchOp::Put { key, value } => {
                self.bytes_ingested = self.bytes_ingested.saturating_add(value.len() as u64);
                let stored = if spill {
                    match self.maybe_spill_large_value(value) {
                        Ok(v) => v,
                        Err(e) => {
                            self.next_seq = seq_checkpoint;
                            return Err(e);
                        }
                    }
                } else {
                    escape_inline_value(value)
                };
                WriteOp::put(seq, key, stored)
            }
            BatchOp::Delete { key } => WriteOp::delete(seq, key),
            BatchOp::DeleteRange { start, end } => WriteOp::delete_range(seq, start, end),
        };
        Ok((rec, seq))
    }

    /// One WAL `fdatasync` for a group of already-appended records.
    pub(crate) fn wal_sync_group(&mut self) -> Result<()> {
        self.ensure_not_fenced()?;
        let sync_err = self.wal.lock().sync_data().err();
        if let Some(e) = sync_err {
            self.fence_durability(&e, FenceClass::of_core(&e));
            return Err(e);
        }
        self.note_wal_sync();
        Ok(())
    }

    /// Apply prepared ops to the memtable after durable WAL.
    pub(crate) fn apply_ops_to_mem(&mut self, ops: Vec<WriteOp>) {
        self.note_dirty_points(&ops);
        apply_ops_owned(&mut self.mem, ops);
        self.publish_sequence(self.last_sequence());
    }

    /// Async commit: encode WAL and `write()` it before `Ok` — same
    /// process-crash class as RocksDB default (`sync=false`,
    /// `manual_wal_flush=false` flushes per record). No `fdatasync`
    /// (that is G1), no write-group.
    pub(crate) fn commit_async_ops(&mut self, batch: Vec<BatchOp>) -> Result<SequenceNumber> {
        if !self.write_admission_idle() {
            let families = self.batch_families(&batch);
            self.ensure_write_admitted_for(&families)?;
        }
        self.observe_bulk_batch(&batch);
        self.flush_dead_bulk_runs()?;
        let (ladder, bulk_puts) = if self.bulk_route_enabled {
            let mut ladder = Vec::new();
            let mut bulk_puts = Vec::new();
            for op in batch {
                match op {
                    BatchOp::Put { key, value } => {
                        let fam = self.bulk_family_of_key(key.as_ref());
                        if self.bulk_latch.is_latched(fam) {
                            bulk_puts.push((key, value));
                        } else {
                            ladder.push(BatchOp::Put { key, value });
                        }
                    }
                    other => ladder.push(other),
                }
            }
            (ladder, bulk_puts)
        } else {
            (batch, Vec::new())
        };
        if !bulk_puts.is_empty() {
            let fam = self.bulk_family_of_key(bulk_puts[0].0.as_ref()).to_string();
            if !self.bulk_runs.contains_key(&fam) {
                self.absorb_mem_family_into_run(&fam)?;
            }
            let n = bulk_puts.len();
            let mut keys = Vec::with_capacity(n);
            let mut vals = Vec::with_capacity(n);
            for (k, v) in bulk_puts {
                keys.push(k);
                vals.push(v);
            }
            self.bulk_append_puts(&fam, keys, vals)?;
        }
        if ladder.is_empty() {
            let seq = self.last_sequence();
            self.publish_sequence(seq);
            return Ok(seq);
        }
        let batch = ladder;
        let st = self.phase_stats.clone();
        let t0 = st.as_ref().map(|_| Instant::now());
        let (ops, seq) = self.prepare_write_ops_spill(batch, false)?;
        if let (Some(st), Some(t0)) = (st.as_ref(), t0) {
            st.prepare_ns
                .fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        self.vlog_prepare_wal(false)?;
        {
            let t1 = st.as_ref().map(|_| Instant::now());
            let mut w = self.wal.lock();
            let sl = ops.as_slice();
            w.encode_write_op_batches(&[sl])?;
            w.write_pending_frame()?;
            if let (Some(st), Some(t1)) = (st.as_ref(), t1) {
                st.wal_ns
                    .fetch_add(t1.elapsed().as_nanos() as u64, Ordering::Relaxed);
            }
        }
        // F213: feed the non-lazy change log on the async path too — the
        // write is visible via `get` once published; `commit_ops_with`
        // extends regardless of sync, and a later durable commit would
        // otherwise persist a CHANGELOG that never contains this event.
        if !self.feed_is_lazy() {
            self.change_log
                .extend(ops.iter().map(ChangeEntry::from_write_op));
        }
        let t2 = st.as_ref().map(|_| Instant::now());
        self.note_dirty_points(&ops);
        apply_ops_owned(&mut self.mem, ops);
        if let (Some(st), Some(t2)) = (st.as_ref(), t2) {
            st.mem_ns
                .fetch_add(t2.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        let t3 = st.as_ref().map(|_| Instant::now());
        self.publish_sequence(seq);
        if let (Some(st), Some(t3)) = (st.as_ref(), t3) {
            st.publish_ns
                .fetch_add(t3.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        let t4 = st.as_ref().map(|_| Instant::now());
        self.maybe_auto_flush_best_effort();
        if let (Some(st), Some(t4)) = (st.as_ref(), t4) {
            st.flush_check_ns
                .fetch_add(t4.elapsed().as_nanos() as u64, Ordering::Relaxed);
            st.commits.fetch_add(1, Ordering::Relaxed);
        }
        Ok(seq)
    }

    /// Lone-async 1-op put/delete: no `Vec<BatchOp>` / `Vec<WriteOp>`
    /// (RFC-0154 P1.6). Same WAL bytes as [`Self::commit_async_ops`].
    pub(crate) fn commit_async_one(&mut self, batch: BatchOp) -> Result<SequenceNumber> {
        if !self.write_admission_idle() {
            let families = self.batch_families(std::slice::from_ref(&batch));
            self.ensure_write_admitted_for(&families)?;
        }
        self.observe_bulk_op(&batch);
        let st = self.phase_stats.clone();
        let t0 = st.as_ref().map(|_| Instant::now());
        let (op, seq) = self.prepare_one_spill(batch, false)?;
        if let (Some(st), Some(t0)) = (st.as_ref(), t0) {
            st.prepare_ns
                .fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        self.vlog_prepare_wal(false)?;
        {
            let t1 = st.as_ref().map(|_| Instant::now());
            let mut w = self.wal.lock();
            w.encode_write_op_batches(&[std::slice::from_ref(&op)])?;
            w.write_pending_frame()?;
            if let (Some(st), Some(t1)) = (st.as_ref(), t1) {
                st.wal_ns
                    .fetch_add(t1.elapsed().as_nanos() as u64, Ordering::Relaxed);
            }
        }
        if !self.feed_is_lazy() {
            self.change_log
                .extend(std::iter::once(ChangeEntry::from_write_op(&op)));
        }
        let t2 = st.as_ref().map(|_| Instant::now());
        self.note_dirty_points(std::slice::from_ref(&op));
        apply_ops_owned(&mut self.mem, std::iter::once(op));
        if let (Some(st), Some(t2)) = (st.as_ref(), t2) {
            st.mem_ns
                .fetch_add(t2.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        let t3 = st.as_ref().map(|_| Instant::now());
        self.publish_sequence(seq);
        if let (Some(st), Some(t3)) = (st.as_ref(), t3) {
            st.publish_ns
                .fetch_add(t3.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        let t4 = st.as_ref().map(|_| Instant::now());
        self.maybe_auto_flush_best_effort();
        if let (Some(st), Some(t4)) = (st.as_ref(), t4) {
            st.flush_check_ns
                .fetch_add(t4.elapsed().as_nanos() as u64, Ordering::Relaxed);
            st.commits.fetch_add(1, Ordering::Relaxed);
        }
        Ok(seq)
    }

    /// Sequential G1 client, split like the group path (RFC-0045 P2.1):
    /// [`Self::lone_encode_commit`] under the write lock (admission,
    /// prepare, vlog flush, WAL encode — no fd), `fdatasync` OFF the lock,
    /// then [`Self::lone_publish_commit`] on the second write-lock hold
    /// (publish gate, mem apply, publish). Skips [`GroupInFlight`] —
    /// multi-writer leaders stay on [`Self::group_start`].
    ///
    /// Returns `(commit seq, fdatasync ns)` — the fd sample feeds the
    /// write-group EMA that bounds the catch-up wait (RFC-0042 P1.1);
    /// `0` when no WAL append happened (nothing to sample).
    pub(crate) fn lone_encode_commit(
        &mut self,
        ops: Vec<BatchOp>,
    ) -> Result<(SequenceNumber, Option<LoneInFlight>)> {
        if ops.is_empty() {
            return Ok((self.last_sequence(), None));
        }
        if !self.write_admission_idle() {
            let families = self.batch_families(&ops);
            self.ensure_write_admitted_for(&families)?;
        }
        self.observe_bulk_batch(&ops);
        let (records, seq) = self.prepare_write_ops(ops)?;
        if records.is_empty() {
            return Ok((seq, None));
        }
        self.vlog_prepare_wal(true)?;
        let n = self
            .wal
            .lock()
            .encode_write_op_batches(&[records.as_slice()])?;
        self.bytes_written_wal = self.bytes_written_wal.saturating_add(n);
        // RFC-0045 P2.1: OCC must see these encoded-but-unapplied ops while
        // the fd runs off the write lock (same staging as group leaders).
        self.stage_unapplied_ops(&records);
        Ok((seq, Some(LoneInFlight { records, seq })))
    }

    /// Second write-lock hold of the lone G1 path. `fd` carries the WAL I/O
    /// result (and duration) of the off-lock `fdatasync`; Ok only after the
    /// publish gate — same kernel decision as the group path (RFC-0071).
    pub(crate) fn lone_publish_commit(
        &mut self,
        lone: LoneInFlight,
        fd: Result<u64>,
    ) -> Result<(SequenceNumber, u64)> {
        let LoneInFlight { records, seq } = lone;
        // Same entry contract as `group_apply`: drop the staged ops on both
        // outcomes — a leftover stage would pin `wal_pin_state().commit_inflight`
        // forever and refuse every later WAL rotation.
        self.unstage_unapplied_ops(&records);
        if !crate::group_commit_kernel::may_publish_group(fd.is_ok()) {
            let e = fd.err().expect("publish refused iff WAL I/O failed");
            self.fence_durability(&e, FenceClass::of_core(&e));
            return Err(e);
        }
        let fd_ns = fd.expect("io ok ⇒ fd duration present");
        self.note_wal_sync();
        if !self.feed_is_lazy() {
            self.change_log
                .extend(records.iter().map(ChangeEntry::from_write_op));
        }
        self.maybe_persist_changelog_after_durable_commit();
        self.note_dirty_points(&records);
        apply_ops_owned(&mut self.mem, records);
        self.publish_sequence(seq);
        // Same as `commit_ops_with` / `commit_async_ops`. P1.1 lone_sync
        // skipped this; 1c G1 then never auto-flushed (imm never staged,
        // write_buffer_size was a no-op for the sequential host).
        self.maybe_auto_flush_best_effort();
        Ok((seq, fd_ns))
    }
}

/// WAL-encoded lone commit parked between its encode (first write-lock
/// hold) and its mem-apply/publish (second hold) — the RFC-0045 P2.1
/// off-lock `fdatasync` window, same shape as a group leader's.
pub(crate) struct LoneInFlight {
    records: Vec<WriteOp>,
    seq: SequenceNumber,
}

impl<E: Env> Db<E> {
    /// Shared WAL handle for off-lock `fdatasync` (ConcurrentDb group
    /// leader and lone path).
    pub(crate) fn wal_arc(&self) -> Arc<Mutex<Wal<E::File>>> {
        Arc::clone(&self.wal)
    }

    pub(crate) fn begin_commit(&self) {
        self.commit_inflight.fetch_add(1, Ordering::Release);
    }

    pub(crate) fn end_commit(&self) {
        self.commit_inflight.fetch_sub(1, Ordering::Release);
    }

    /// Remember WAL-encoded ops so OCC sees them while memtable apply waits
    /// for off-lock `fdatasync` (RFC-0045 P2.1).
    pub(crate) fn stage_unapplied(&mut self, g: &GroupInFlight) {
        for (_, ops, _) in &g.appended {
            self.stage_unapplied_ops(ops);
        }
    }

    /// Lone-path variant of [`Self::stage_unapplied`] (no `GroupInFlight`).
    pub(crate) fn stage_unapplied_ops(&mut self, ops: &[WriteOp]) {
        for op in ops {
            self.unapplied.push(UnappliedOp {
                seq: op.sequence,
                kind: op.kind,
                key: op.key.clone(),
                end: if op.kind == ValueType::RangeDeletion {
                    op.value.clone()
                } else {
                    Bytes::new()
                },
            });
        }
    }

    /// Drop staged ops for `g` (apply finished, or the group's fsync fenced).
    pub(crate) fn unstage_unapplied(&mut self, g: &GroupInFlight) {
        if self.unapplied.is_empty() {
            return;
        }
        for (_, ops, _) in &g.appended {
            self.unstage_unapplied_ops(ops);
        }
    }

    /// Lone-path variant of [`Self::unstage_unapplied`]: drop the staged
    /// ops covered by one batch's sequence range.
    pub(crate) fn unstage_unapplied_ops(&mut self, ops: &[WriteOp]) {
        if self.unapplied.is_empty() {
            return;
        }
        let mut lo = u64::MAX;
        let mut hi = 0u64;
        let mut any = false;
        for op in ops {
            any = true;
            lo = lo.min(op.sequence);
            hi = hi.max(op.sequence);
        }
        if !any {
            return;
        }
        self.unapplied.retain(|u| u.seq < lo || u.seq > hi);
    }

    /// WAL appends whose `fdatasync`/mem-apply has not finished.
    #[must_use]
    pub fn commit_inflight(&self) -> usize {
        self.commit_inflight.load(Ordering::Acquire)
    }

    /// Shared handle to [`Self::commit_inflight`] for lock-free observation
    /// (RFC-0042 P1.1: a commit must be provably in flight even while its
    /// off-lock `fdatasync` window is open).
    #[must_use]
    pub(crate) fn commit_inflight_handle(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.commit_inflight)
    }

    pub(crate) fn fence_durability(&mut self, io_error: impl std::fmt::Display, class: FenceClass) {
        if self.fence_report.is_none() {
            let published = self.published_seq.load(Ordering::Acquire);
            self.fence_report = Some(FenceReport {
                io_error: io_error.to_string(),
                class,
                uncertain_from: published.saturating_add(1),
                uncertain_through: self.last_sequence(),
            });
        }
        self.durability_fenced = true;
    }

    /// Fence then return `e` (explicit flush / compact I/O — RFC-0050 P0.3).
    fn fence_io_err(&mut self, e: CoreError) -> CoreError {
        self.fence_durability(&e, FenceClass::of_core(&e));
        e
    }

    pub(crate) fn note_wal_sync(&self) {
        self.wal_sync_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Rocks-style group commit: many client batches, one fsync if any requires sync.
    ///
    /// For each input `(ops, do_sync)` returns the corresponding `Result` (last sequence
    /// of that batch on success). Empty batches yield `Ok(last_sequence)` without I/O.
    ///
    /// A failed WAL sync after appends fences the `Db` and fails every batch that
    /// was appended in this group (mem not applied).
    pub fn group_commit(
        &mut self,
        batches: Vec<(Vec<BatchOp>, bool)>,
    ) -> Vec<Result<SequenceNumber>> {
        match self.group_start(batches) {
            Ok(g) => self.group_finish(g),
            Err(results) => results,
        }
    }

    /// Prepare + WAL append (no fsync). Caller may [`Self::group_absorb`] more
    /// members that arrived during this work, then [`Self::group_finish`].
    pub(crate) fn group_start(
        &mut self,
        batches: Vec<(Vec<BatchOp>, bool)>,
    ) -> std::result::Result<GroupInFlight, Vec<Result<SequenceNumber>>> {
        let n = batches.len();
        let mut g = GroupInFlight {
            results: (0..n).map(|_| None).collect(),
            pending: Vec::new(),
            appended: Vec::new(),
            any_sync: false,
            next_i: n,
            failed: false,
        };
        if n == 0 {
            return Ok(g);
        }
        if let Err(results) = self.group_admit(n) {
            return Err(results);
        }
        self.group_prepare(&mut g, batches, 0);
        if let Err(e) = self.vlog_prepare_wal(g.any_sync) {
            let msg = e.to_string();
            return Err((0..n)
                .map(|_| Err(CoreError::Internal(format!("vlog flush failed: {msg}"))))
                .collect());
        }
        self.group_append_ops(&mut g);
        Ok(g)
    }

    /// Append members that queued after [`Self::group_start`] (no extra wait).
    pub(crate) fn group_absorb(
        &mut self,
        g: &mut GroupInFlight,
        batches: Vec<(Vec<BatchOp>, bool)>,
    ) {
        if g.failed || batches.is_empty() {
            return;
        }
        let base = g.next_i;
        g.next_i = base.saturating_add(batches.len());
        g.results
            .resize_with(g.next_i, || None::<Result<SequenceNumber>>);
        self.group_prepare(g, batches, base);
        if let Err(e) = self.vlog_prepare_wal(g.any_sync) {
            let msg = e.to_string();
            g.failed = true;
            for (i, _, _) in &g.pending {
                g.results[*i] = Some(Err(CoreError::Internal(format!(
                    "vlog flush failed: {msg}"
                ))));
            }
            g.pending.clear();
            return;
        }
        self.group_append_ops(g);
    }

    fn group_admit(&mut self, n: usize) -> std::result::Result<(), Vec<Result<SequenceNumber>>> {
        match self.ensure_write_admitted() {
            Ok(()) => Ok(()),
            Err(CoreError::WriteStall { l0_files, limit }) => Err((0..n)
                .map(|_| Err(CoreError::WriteStall { l0_files, limit }))
                .collect()),
            Err(CoreError::WriteStallMem { mem_bytes, limit }) => Err((0..n)
                .map(|_| Err(CoreError::WriteStallMem { mem_bytes, limit }))
                .collect()),
            Err(e) => {
                let msg = e.to_string();
                Err((0..n)
                    .map(|_| Err(CoreError::Internal(msg.clone())))
                    .collect())
            }
        }
    }

    fn group_prepare(
        &mut self,
        g: &mut GroupInFlight,
        batches: Vec<(Vec<BatchOp>, bool)>,
        index_base: usize,
    ) {
        for (off, (ops, do_sync)) in batches.into_iter().enumerate() {
            let i = index_base + off;
            if ops.is_empty() {
                g.results[i] = Some(Ok(self.last_sequence()));
                continue;
            }
            self.observe_bulk_batch(&ops);
            match self.prepare_write_ops(ops) {
                Ok((write_ops, last_seq)) => {
                    if do_sync {
                        g.any_sync = true;
                    }
                    g.pending.push((i, write_ops, last_seq));
                }
                Err(e) => g.results[i] = Some(Err(e)),
            }
        }
    }

    /// Encode [`GroupInFlight::pending`] into the WAL frame (no `write` syscall).
    fn group_append_ops(&mut self, g: &mut GroupInFlight) {
        if g.failed || g.pending.is_empty() {
            return;
        }
        if let Err(e) = self.ensure_not_fenced() {
            let msg = e.to_string();
            for (i, _, _) in &g.pending {
                g.results[*i] = Some(Err(CoreError::Internal(format!(
                    "group wal append failed: {msg}"
                ))));
            }
            g.failed = true;
            g.pending.clear();
            return;
        }
        let refs: Vec<&[crate::batch::WriteOp]> =
            g.pending.iter().map(|(_, ops, _)| ops.as_slice()).collect();
        match self.wal.lock().encode_write_op_batches(&refs) {
            Ok(n) => {
                self.bytes_written_wal = self.bytes_written_wal.saturating_add(n);
                g.appended.extend(g.pending.drain(..));
            }
            Err(e) => {
                let msg = e.to_string();
                for (i, _, _) in &g.pending {
                    g.results[*i] = Some(Err(CoreError::Internal(format!(
                        "group wal append failed: {msg}"
                    ))));
                }
                g.failed = true;
                g.pending.clear();
            }
        }
    }

    pub(crate) fn group_finish(&mut self, g: GroupInFlight) -> Vec<Result<SequenceNumber>> {
        if let Err(e) = self.vlog_prepare_wal(g.needs_sync()) {
            self.fence_durability(&e, FenceClass::of_core(&e));
            return g.fail_sync(e);
        }
        // One WAL lock: `sync_data` already `write()`s the pending frame
        // then `fdatasync`s. Split write-then-sync was two mutex hops on
        // the 1c G1 raftlog batch (RFC-0062 P1.1 p11h).
        if g.needs_sync() {
            if let Err(e) = self.wal_sync_group() {
                return g.fail_sync(e);
            }
        } else {
            let write_err = self.wal.lock().write_pending_frame().err();
            if let Some(e) = write_err {
                self.fence_durability(&e, FenceClass::of_core(&e));
                return g.fail_sync(e);
            }
        }
        let pub_seq = g.max_appended_seq();
        let results = self.group_apply(g);
        self.publish_sequence(pub_seq);
        results
    }

    /// Mem apply + feed after WAL is durable. No fsync (RFC-0041: leader may
    /// have `fdatasync`'d off the write lock). RFC-0045 P2.1: ConcurrentDb
    /// calls this on the second write-lock hold, after the off-lock fd.
    pub(crate) fn group_apply(&mut self, g: GroupInFlight) -> Vec<Result<SequenceNumber>> {
        self.unstage_unapplied(&g);
        let GroupInFlight {
            mut results,
            appended,
            any_sync,
            failed,
            ..
        } = g;
        if failed {
            for (i, _, _) in &appended {
                if results[*i].is_none() {
                    results[*i] = Some(Err(CoreError::Internal("group wal append failed".into())));
                }
            }
            return finish_group_results(results);
        }
        if appended.is_empty() {
            return finish_group_results(results);
        }

        if !self.feed_is_lazy() {
            let mut feed_batch: Vec<ChangeEntry> = Vec::new();
            for (_, write_ops, _) in &appended {
                for op in write_ops {
                    feed_batch.push(ChangeEntry::from_write_op(op));
                }
            }
            self.change_log.extend(feed_batch);
        }
        if any_sync {
            self.maybe_persist_changelog_after_durable_commit();
        }

        for (i, write_ops, last_seq) in appended {
            self.note_dirty_points(&write_ops);
            apply_ops_owned(&mut self.mem, write_ops);
            results[i] = Some(Ok(last_seq));
        }
        // Caches bump on [`Self::publish_sequence`] after WAL is durable so
        // a failed fd cannot leave a stale miss for an unpublished key.
        self.maybe_auto_flush_best_effort();
        finish_group_results(results)
    }
}

/// In-flight group commit (prepare / encode / append / apply).
pub(crate) struct GroupInFlight {
    results: Vec<Option<Result<SequenceNumber>>>,
    /// Seq-assigned ops not yet WAL-encoded (still under the Db write lock;
    /// RFC-0045 P1.1 prepare-off-lock was a negative).
    pending: Vec<(usize, Vec<WriteOp>, SequenceNumber)>,
    appended: Vec<(usize, Vec<WriteOp>, SequenceNumber)>,
    any_sync: bool,
    next_i: usize,
    failed: bool,
}

impl GroupInFlight {
    pub(crate) fn needs_sync(&self) -> bool {
        self.any_sync && !self.failed && !self.appended.is_empty()
    }

    pub(crate) fn max_appended_seq(&self) -> SequenceNumber {
        // RFC-0057 P2.1: the fence watermark is the group-commit kernel's
        // decision — one publish sequence for the whole group.
        let mut seqs = Vec::with_capacity(self.appended.len());
        for (_, _, seq) in &self.appended {
            seqs.push(*seq);
        }
        crate::group_commit_kernel::fence_publish_seq(&seqs)
    }

    pub(crate) fn fail_sync(mut self, e: impl std::fmt::Display) -> Vec<Result<SequenceNumber>> {
        let msg = e.to_string();
        for (i, _, _) in &self.appended {
            self.results[*i] = Some(Err(CoreError::Internal(format!(
                "group wal sync failed: {msg}"
            ))));
        }
        finish_group_results(self.results)
    }

    /// ConcurrentDb off-lock I/O failure (write or `fdatasync`). Prefix is
    /// part of the RFC-0051 PCT oracle (`starts_with("group wal write/sync failed")`).
    pub(crate) fn fail_io(mut self, e: impl std::fmt::Display) -> Vec<Result<SequenceNumber>> {
        let msg = format!("group wal write/sync failed: {e}");
        for (i, _, _) in &self.appended {
            self.results[*i] = Some(Err(CoreError::Internal(msg.clone())));
        }
        finish_group_results(self.results)
    }

    #[cfg(feature = "pct")]
    pub(crate) fn collect_appended_seqs(&self, out: &mut Vec<u64>) {
        for (_, _, seq) in &self.appended {
            out.push(*seq);
        }
    }
}

fn finish_group_results(
    results: Vec<Option<Result<SequenceNumber>>>,
) -> Vec<Result<SequenceNumber>> {
    results
        .into_iter()
        .map(|r| {
            r.unwrap_or(Err(CoreError::Internal(
                "group commit missing result".into(),
            )))
        })
        .collect()
}

impl<E: Env> Drop for Db<E> {
    fn drop(&mut self) {
        // PEDRA_WRITE_PHASE_STATS: one summary line at teardown so a bench
        // run can attribute hydrate wall time to the commit phases
        // (RFC-0159 P1.1). No-op when the env was unset at open.
        if let Some(st) = &self.phase_stats {
            let ms = |v: &AtomicU64| v.load(Ordering::Relaxed) as f64 / 1e6;
            println!(
                "WRITEPHASE commits={} prepare_ms={:.1} wal_ms={:.1} mem_ms={:.1} \
                 publish_ms={:.1} flush_check_ms={:.1} lock_wait_ms={:.1}",
                st.commits.load(Ordering::Relaxed),
                ms(&st.prepare_ns),
                ms(&st.wal_ns),
                ms(&st.mem_ns),
                ms(&st.publish_ns),
                ms(&st.flush_check_ns),
                ms(&st.lock_wait_ns),
            );
        }
        // Prefer Env unlock so FailingEnv can observe release; Drop of DirLock
        // is std best-effort only if release already ran or Env fails here.
        if let Some(mut lock) = self.dir_lock.take() {
            let _ = lock.release(&self.env);
        }
    }
}

impl<E: Env> Db<E> {
    fn ensure_not_fenced(&self) -> Result<()> {
        if self.durability_fenced {
            Err(CoreError::DurabilityFenced)
        } else {
            Ok(())
        }
    }

    /// When open options require durability, fsync the directory (propagate errors).
    fn sync_dir_if_required(&self, dir: &Path) -> Result<()> {
        if self.sync {
            self.env.sync_dir(dir)?;
        }
        Ok(())
    }

    /// Primary unlock path through Env (RFC-0015 H3).
    fn release_lock(&mut self) -> Result<()> {
        if let Some(mut lock) = self.dir_lock.take() {
            lock.release(&self.env)?;
        }
        Ok(())
    }

    /// Refuse writes when L0 or mem is at/above stall limits (open-items §2.3).
    pub(crate) fn ensure_write_admitted(&mut self) -> Result<()> {
        self.ensure_write_admitted_for(&[])
    }

    /// Per-CF stall (RFC-0065 P1.2). Empty `families` = global (kernel / mixed group).
    pub(crate) fn ensure_write_admitted_for(&mut self, families: &[String]) -> Result<()> {
        let per_cf = !self.physical_cfs.is_empty() && !families.is_empty();
        // Mem bound first: flush is the natural drain for mem pressure.
        if let Some(limit) = self.write_stall_mem_bytes {
            let mut mem_bytes = if per_cf {
                families
                    .iter()
                    .map(|f| self.mem.approx_memory_usage_cf(f))
                    .max()
                    .unwrap_or(0)
            } else {
                self.mem.approx_memory_usage()
            };
            if mem_bytes >= limit {
                if self.write_stall_drain {
                    if per_cf {
                        for f in families {
                            let _ = self.flush_cf(f);
                        }
                    } else {
                        let _ = self.flush();
                    }
                    mem_bytes = if per_cf {
                        families
                            .iter()
                            .map(|f| self.mem.approx_memory_usage_cf(f))
                            .max()
                            .unwrap_or(0)
                    } else {
                        self.mem.approx_memory_usage()
                    };
                }
                if mem_bytes >= limit {
                    self.write_stall_count = self.write_stall_count.saturating_add(1);
                    return Err(CoreError::WriteStallMem { mem_bytes, limit });
                }
            }
        }

        let l0_of = |db: &Self, fam: Option<&str>| -> usize {
            match fam {
                Some(f) if !db.physical_cfs.is_empty() => db.level_file_count_cf(f),
                _ => db.level_file_count(0),
            }
        };

        // Soft pressure (b): drain once when L0 is elevated, then continue to hard check.
        if let Some(soft) = self.write_pressure_l0 {
            let hit = if per_cf {
                families.iter().any(|f| l0_of(self, Some(f)) >= soft)
            } else {
                l0_of(self, None) >= soft
            };
            if hit {
                self.drain_l0_once();
                self.write_pressure_count = self.write_pressure_count.saturating_add(1);
            }
        }

        let Some(limit) = self.write_stall_l0 else {
            return Ok(());
        };
        let mut l0 = if per_cf {
            families
                .iter()
                .map(|f| l0_of(self, Some(f)))
                .max()
                .unwrap_or(0)
        } else {
            l0_of(self, None)
        };
        if l0 < limit {
            return Ok(());
        }
        if self.write_stall_drain {
            // One honest self-help pass — no sleep, no unbounded loop.
            self.drain_l0_once();
            l0 = if per_cf {
                families
                    .iter()
                    .map(|f| l0_of(self, Some(f)))
                    .max()
                    .unwrap_or(0)
            } else {
                l0_of(self, None)
            };
            if l0 < limit {
                return Ok(());
            }
        }
        self.write_stall_count = self.write_stall_count.saturating_add(1);
        Err(CoreError::WriteStall {
            l0_files: l0,
            limit,
        })
    }

    pub(crate) fn maybe_auto_flush(&mut self) -> Result<()> {
        if !self.physical_cfs.is_empty() {
            // Hot path: integer compare, not a walk of every memtable key.
            // `cf_families()` scans tail+map (O(entries)) — with CFs registered
            // every 1c put paid that (ycsb_a 2M→0.6M qps, RFC-0149).
            let mem = self.mem.approx_memory_usage();
            let global_under = self.auto_flush_bytes.map_or(true, |lim| mem < lim);
            let cf_under = self.cf_write_buffer.values().all(|&n| n == 0 || mem < n);
            if global_under && cf_under {
                return Ok(());
            }
            let n = self.physical_cfs.len();
            for i in 0..n {
                let fam = self.physical_cfs[i].as_str();
                let Some(limit) = self.write_buffer_for(fam) else {
                    continue;
                };
                if self.mem.approx_memory_usage_cf(fam) < limit {
                    continue;
                }
                let fam = self.physical_cfs[i].clone();
                if self.defer_auto_compact {
                    let taken = self.mem.take_family(&fam);
                    if !taken.is_empty() {
                        self.push_parked_unflushed(taken);
                    }
                } else {
                    self.flush_cf(&fam)?;
                }
            }
            return Ok(());
        }
        let Some(limit) = self.auto_flush_bytes else {
            return Ok(());
        };
        if self.mem.approx_memory_usage() >= limit {
            if self.defer_auto_compact {
                // Leave the table in `imm` for the host worker. Do not call
                // `prepare_flush_imm` here — that takes the table out and
                // `has_imm` goes false (291k mem / 0 SST in the P2.1 attempt).
                let _ = self.stage_flush_imm()?;
                return Ok(());
            }
            self.auto_flush_mem()?;
        }
        Ok(())
    }

    /// Auto-flush: same SST/WAL path as [`Self::flush`] but does not rewrite
    /// the CHANGELOG cache when `changelog_interval == 0` (RFC-0036).
    fn auto_flush_mem(&mut self) -> Result<()> {
        self.ensure_not_fenced()?;
        if self.imm.is_some() {
            self.flush_imm_to_l0()?;
        }
        if self.mem.is_empty() {
            self.try_rotate_wal()?;
            return Ok(());
        }
        self.imm = Some(std::mem::replace(&mut self.mem, MemTable::new()));
        self.flush_imm_to_l0()?;
        self.finish_flush_pipeline()
    }

    /// Like [`maybe_auto_flush`] but never fails the caller (F18).
    ///
    /// On flush error the MemTable still holds the data; a later explicit
    /// [`Db::flush`] or successful auto-flush can retry.
    pub(crate) fn maybe_auto_flush_best_effort(&mut self) {
        let _ = self.maybe_auto_flush();
    }

    fn maybe_auto_compact(&mut self) -> Result<()> {
        let count_hit = self
            .auto_compact_sst_count
            .is_some_and(|limit| self.ssts.len() >= limit);
        let l0_hit = self.level_file_count(0) >= L0_COMPACTION_TRIGGER;
        let bytes_hit = if let Some(limit) = self.auto_compact_sst_bytes {
            let mut total = 0u64;
            for t in &self.ssts {
                if let Ok(len) = self.env.metadata_len(t.path()) {
                    total = total.saturating_add(len);
                }
            }
            total >= limit
        } else {
            false
        };
        if l0_hit {
            // Bounded work: L0 → one new L1. Do not absorb the existing L1
            // (that rewrite grew with the DB and dominated apply/raftlog).
            // RFC-0046 P0.2: horizon-derived GC archives first; an archive
            // failure keeps the history-preserving merge (never drop
            // unarchived). `auto_reclaim` floors skip the archive (reclaim
            // keeps nothing — Rocks storage profile, RFC-0047 divergence 4).
            let opts = match self.auto_gc_floor() {
                Some((floor, true)) => {
                    if let Err(e) = self.archive_history_below(floor) {
                        self.last_auto_compact_error = Some(e.to_string());
                        CompactOptions::default()
                    } else {
                        CompactOptions {
                            gc: crate::merge::CompactGcOptions::for_oldest_snapshot(floor),
                            max_input_files: None,
                        }
                    }
                }
                Some((floor, false)) => CompactOptions {
                    gc: crate::merge::CompactGcOptions::for_oldest_snapshot(floor),
                    max_input_files: None,
                },
                None => CompactOptions::default(),
            };
            self.compact_l0_into_l1(opts)?;
            self.last_auto_compact_error = None;
        } else if count_hit || bytes_hit {
            // RFC-0046 P0.2 (same fail-closed archive rule as the L0 path).
            match self.auto_gc_floor() {
                Some((floor, true)) => {
                    if let Err(e) = self.archive_history_below(floor) {
                        self.last_auto_compact_error = Some(e.to_string());
                        self.compact_with(CompactOptions::default())?;
                    } else {
                        self.compact_with_ssts_only(CompactOptions {
                            gc: crate::merge::CompactGcOptions::for_oldest_snapshot(floor),
                            max_input_files: None,
                        })?;
                    }
                }
                Some((floor, false)) => {
                    self.compact_with_ssts_only(CompactOptions {
                        gc: crate::merge::CompactGcOptions::for_oldest_snapshot(floor),
                        max_input_files: None,
                    })?;
                }
                None => {
                    self.compact_with(CompactOptions::default())?;
                }
            }
            self.last_auto_compact_error = None;
        }
        // RFC-0046 P0.5: dead-weight-doubling full rewrite. The horizon
        // floor always lags the versions `compact_l0_into_l1` just merged
        // (they age out only after reaching L1) and the L0 path never
        // absorbs old levels, so disk grows without bound — the `window`
        // profile measured byte-identical to `All` (rfc0046-sizing). When
        // the floor advanced past the last full reclaim AND total SST bytes
        // at least doubled since then, rewrite ALL SSTs under the horizon
        // floor. Archive first, GC fail-closed (an archive error keeps the
        // data and the trigger retries on the next flush).
        if !self.auto_reclaim && !self.ssts.is_empty() {
            if let Some((floor, true)) = self.auto_gc_floor() {
                let mut total: u64 = 0;
                for t in &self.ssts {
                    if let Ok(len) = self.env.metadata_len(t.path()) {
                        total = total.saturating_add(len);
                    }
                }
                let fire = match self.last_horizon_reclaim {
                    None => false,
                    Some((last_floor, last_bytes)) => {
                        floor > last_floor && total >= last_bytes.saturating_mul(2)
                    }
                };
                if fire {
                    if let Err(e) = self.archive_history_below(floor) {
                        self.last_auto_compact_error = Some(e.to_string());
                    } else {
                        let input_idxs: Vec<usize> = (0..self.ssts.len()).collect();
                        self.rewrite_ssts(
                            input_idxs,
                            MAX_LSM_LEVEL,
                            CompactOptions {
                                gc: crate::merge::CompactGcOptions::for_oldest_snapshot(floor),
                                max_input_files: None,
                            },
                        )?;
                        let mut after: u64 = 0;
                        for t in &self.ssts {
                            if let Ok(len) = self.env.metadata_len(t.path()) {
                                after = after.saturating_add(len);
                            }
                        }
                        tracing::info!(
                            floor,
                            before_bytes = total,
                            after_bytes = after,
                            "horizon-driven full rewrite (RFC-0046 P0.5)"
                        );
                        self.last_horizon_reclaim = Some((floor, after));
                        self.last_auto_compact_error = None;
                    }
                } else if self.last_horizon_reclaim.is_none() {
                    // First observation: record the baseline, don't rewrite.
                    self.last_horizon_reclaim = Some((floor, total));
                }
            }
        }
        Ok(())
    }

    /// Run auto-compact after flush; record failures without failing the flush.
    fn run_auto_compact_best_effort(&mut self) {
        if self.defer_auto_compact {
            return;
        }
        if let Err(e) = self.maybe_auto_compact() {
            self.auto_compact_failures = self.auto_compact_failures.saturating_add(1);
            self.last_auto_compact_error = Some(e.to_string());
            tracing::warn!(
                error = %e,
                failures = self.auto_compact_failures,
                "auto-compact after flush failed (flush still Ok)"
            );
        }
    }

    /// Best-effort sealed-blob GC when [`Self::set_auto_blob_gc_min_ratio`] is set.
    fn run_auto_blob_gc_best_effort(&mut self) {
        let Some(theta) = self.auto_blob_gc_min_ratio else {
            return;
        };
        // Cheap gate: multi-blob mode with only the active gen → nothing sealed.
        // (Single-file `VALUES.vlog` / file 0 still runs — `compact_blob_auto` may
        // rewrite it via `compact_vlog`.)
        if self.blob_active > 0 {
            let nums = vlog::list_blob_nums(&self.env, &self.dir);
            if !nums.iter().any(|&n| n != self.blob_active) {
                return;
            }
        }
        match self.compact_blob_auto(theta) {
            Ok(Some((num, st))) => {
                tracing::info!(
                    file = num,
                    before = st.bytes_before,
                    after = st.bytes_after,
                    theta,
                    "auto blob GC rewrote sealed generation"
                );
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    theta,
                    "auto blob GC after flush/compact failed (caller still Ok)"
                );
            }
        }
    }

    /// Write MANIFEST + CURRENT for the live SST set (with levels).
    ///
    /// `fdatasync`s any L0 that was written without sync first so CURRENT
    /// never points at a torn file (RFC-0041).
    fn persist_manifest(&mut self) -> Result<()> {
        let fsync_result = self.fsync_unsynced_ssts();
        let sst_durable = fsync_result.is_ok() && self.unsynced_ssts.is_empty();
        if !crate::flush_kernel::may_publish_manifest(sst_durable) {
            // Kernel is the write gate: AS-IS always-true would fall through
            // and publish CURRENT naming an unsynced/torn SST.
            return fsync_result.and(Err(CoreError::Internal(
                "MANIFEST publish without durable SST".into(),
            )));
        }
        match self.take_manifest_persist()?.write() {
            Ok(()) => Ok(()),
            // F196: CURRENT already names the new MANIFEST — the version on
            // disk IS the new one (unsynced). Undoing here would put memory
            // behind disk and delete files the manifest references; fence
            // and treat the persist as landed (promote/fence shape).
            Err(CoreError::ManifestCommittedUnsynced { ref source, .. }) => {
                self.fence_durability_post_commit(source);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Public wrapper: SST `fdatasync` + MANIFEST before WAL rotate / checkpoint.
    ///
    /// # Errors
    /// SST / MANIFEST I/O.
    pub fn persist_manifest_durable(&mut self) -> Result<()> {
        self.persist_manifest()
    }

    /// `fdatasync` L0 files that were written without sync (RFC-0041).
    ///
    /// # Errors
    /// Env I/O.
    pub fn fsync_unsynced_ssts(&mut self) -> Result<()> {
        let paths = std::mem::take(&mut self.unsynced_ssts);
        if let Err(e) = Self::fsync_sst_paths(&self.env, &self.dir, &paths, self.sync) {
            self.unsynced_ssts.extend(paths);
            return Err(e);
        }
        Ok(())
    }

    /// Take the unsynced L0 list so the caller can `fdatasync` off the write lock.
    pub fn take_unsynced_ssts(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.unsynced_ssts)
    }

    /// Put unsynced L0 paths back after a failed off-lock `fdatasync`.
    pub fn restore_unsynced_ssts(&mut self, paths: Vec<PathBuf>) {
        self.unsynced_ssts.extend(paths);
    }

    /// How many L0 files still need a durability `fdatasync`.
    #[must_use]
    pub fn unsynced_sst_count(&self) -> usize {
        self.unsynced_ssts.len()
    }

    /// `fdatasync` these SST paths (no `Db` lock). Used by the host worker.
    ///
    /// # Errors
    /// Env I/O.
    pub fn fsync_sst_paths(env: &E, dir: &Path, paths: &[PathBuf], sync_dir: bool) -> Result<()> {
        for path in paths {
            if !env.exists(path) {
                continue;
            }
            let mut f = env.open_read(path)?;
            f.sync_data()?;
        }
        if sync_dir && !paths.is_empty() {
            env.sync_dir(dir)?;
        }
        Ok(())
    }

    fn version_set_now(&self) -> Result<VersionSet> {
        let mut nums = Vec::with_capacity(self.ssts.len());
        for table in &self.ssts {
            let name = table
                .path()
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| CoreError::Internal("sst path missing file name".into()))?;
            let num = manifest::parse_sst_name(name)
                .ok_or_else(|| CoreError::Internal(format!("bad sst name {name}")))?;
            nums.push(num);
        }
        debug_assert_eq!(nums.len(), self.sst_levels.len());
        let mut vs = VersionSet {
            next_file_num: self.next_file_num,
            sst_file_nums: nums,
            sst_levels: self.sst_levels.clone(),
            manifest_file_num: self.manifest_file_num,
            vlog_use_new: self.vlog_use_new,
            earliest_readable_seq: self.earliest_readable_seq,
            sst_cfs: self.ssts.iter().map(|t| t.cf().to_string()).collect(),
        };
        vs.normalize_levels();
        Ok(vs)
    }

    /// Reserve the next MANIFEST number and snapshot the job so the caller can
    /// `fsync` MANIFEST/`CURRENT` **without** the Db write lock (RFC-0041 P1.1).
    ///
    /// WAL rotate must wait until [`ManifestPersist::write`] succeeds.
    pub fn take_manifest_persist(&mut self) -> Result<ManifestPersist<E>> {
        let mut vs = self.version_set_now()?;
        vs.manifest_file_num = vs.manifest_file_num.saturating_add(1).max(1);
        self.manifest_file_num = vs.manifest_file_num;
        let epoch = self
            .manifest_epoch
            .checked_add(1)
            .expect("manifest epoch overflow after 2^64 persists");
        self.manifest_epoch = epoch;
        Ok(ManifestPersist {
            env: self.env.clone(),
            dir: self.dir.clone(),
            vs,
            sync: self.sync,
            epoch,
            gate: Arc::clone(&self.manifest_write_gate),
        })
    }

    /// Push a flushed L0 SST into the in-memory inventory (no MANIFEST I/O).
    pub fn apply_l0_install(&mut self, table: SstTable, file_num: u64) -> L0InstallUndo {
        self.apply_l0_installs(vec![(table, file_num)])
    }

    /// Push flushed L0 SSTs into the in-memory inventory (no MANIFEST I/O).
    pub fn apply_l0_installs(&mut self, files: Vec<(SstTable, u64)>) -> L0InstallUndo {
        self.apply_sst_installs(files, &[])
    }

    /// Level-explicit in-memory install (RFC-0159 P0.2 bulk chunks land at
    /// `MAX_LSM_LEVEL`; missing entries default to L0).
    pub fn apply_sst_installs(
        &mut self,
        files: Vec<(SstTable, u64)>,
        levels: &[u32],
    ) -> L0InstallUndo {
        let undo = L0InstallUndo {
            prev_next: self.next_file_num,
            prev_manifest: self.manifest_file_num,
            n: files.len(),
        };
        for (i, (table, file_num)) in files.into_iter().enumerate() {
            self.adopt_sst(&table);
            self.note_sst_bytes_written(table.path());
            self.table_cache.insert(Arc::new(table.clone()));
            if self.next_file_num <= file_num {
                self.next_file_num = file_num.saturating_add(1);
            }
            self.unsynced_ssts.push(table.path().to_path_buf());
            let level = levels.get(i).copied().unwrap_or(0);
            self.ssts.push(table);
            self.sst_levels.push(level);
            // RFC-0159 P1.11: bottom-level bulk files must not pin the
            // whole image (100M OOM). Streaming v6 is empty at write;
            // a latched span that flushed through write_imm_l0 still
            // carries a resident body — drop it so first get promotes.
            if level == MAX_LSM_LEVEL && self.bulk_route_enabled && self.sst_source.is_some() {
                if let Some(t) = self.ssts.last() {
                    t.release_resident();
                }
            }
        }
        self.note_sst_inventory_changed();
        undo
    }

    /// Undo [`Self::apply_l0_install`] after a failed off-lock MANIFEST persist.
    pub fn undo_l0_install(&mut self, undo: L0InstallUndo) {
        for _ in 0..undo.n {
            if let Some(t) = self.ssts.last() {
                let p = t.path().to_path_buf();
                self.unsynced_ssts.retain(|x| x != &p);
            }
            let _ = self.ssts.pop();
            let _ = self.sst_levels.pop();
        }
        self.next_file_num = undo.prev_next;
        self.manifest_file_num = undo.prev_manifest;
        self.note_sst_inventory_changed();
    }

    /// In-memory half of [`Self::install_prepared_l0_compact`] (no MANIFEST I/O).
    ///
    /// Returns `None` when another install already dropped the inputs (no-op).
    pub fn apply_prepared_l0_compact(
        &mut self,
        job: PreparedL0Compact<E>,
        new_tables: Vec<SstTable>,
    ) -> Option<L0CompactUndo> {
        let input_paths: Vec<PathBuf> = job.input_paths();
        // Every input must still be live. With leveled jobs the inputs can
        // mix levels, and a concurrent install may have taken only *some* of
        // them: this job's outputs merge the data of inputs that are gone,
        // so installing them would duplicate live versions (G2). Another
        // install's outputs already cover the gone inputs — skip entirely.
        let still_live = input_paths
            .iter()
            .all(|p| self.ssts.iter().any(|t| t.path() == p.as_path()));
        if !still_live {
            for t in &new_tables {
                let _ = self.remove_db_file(t.path());
            }
            return None;
        }
        let old_paths = input_paths;
        let mut keep_tables = Vec::new();
        let mut keep_levels = Vec::new();
        for (t, &lvl) in self.ssts.iter().zip(self.sst_levels.iter()) {
            if old_paths.iter().any(|p| t.path() == p.as_path()) {
                continue;
            }
            keep_tables.push(t.clone());
            keep_levels.push(lvl);
        }
        for t in &new_tables {
            self.adopt_sst(t);
            self.note_sst_bytes_written(t.path());
            self.table_cache.insert(Arc::new(t.clone()));
            keep_tables.push(t.clone());
            keep_levels.push(job.to_level);
        }
        // The prepared job reserved exactly one file number; a split output
        // consumed `job.file_num ..= job.file_num + n - 1`, so burn the rest.
        let want = job.file_num.saturating_add(new_tables.len() as u64);
        if self.next_file_num < want {
            self.next_file_num = want;
        }
        let prev_tables = std::mem::replace(&mut self.ssts, keep_tables);
        let prev_levels = std::mem::replace(&mut self.sst_levels, keep_levels);
        let prev_manifest = self.manifest_file_num;
        let prev_earliest = self.earliest_readable_seq;
        if job.gc.requests_gc() {
            self.note_version_gc_watermark(job.gc);
        }
        self.unsynced_ssts
            .retain(|p| !old_paths.iter().any(|o| o == p));
        self.note_sst_inventory_changed();
        Some(L0CompactUndo {
            prev_tables,
            prev_levels,
            prev_manifest,
            prev_earliest,
            old_paths,
        })
    }

    /// Undo [`Self::apply_prepared_l0_compact`] after a failed MANIFEST persist.
    pub fn undo_prepared_l0_compact(&mut self, undo: L0CompactUndo) {
        self.ssts = undo.prev_tables;
        self.sst_levels = undo.prev_levels;
        self.manifest_file_num = undo.prev_manifest;
        // F173: the GC watermark was raised before the failed install too —
        // restore it or live snapshots die with `SnapshotTooOld` and the next
        // manifest write installs the failed GC durably.
        self.earliest_readable_seq = undo.prev_earliest;
        self.note_sst_inventory_changed();
    }

    /// Env handle (host compact deletes retired L0s after off-lock persist).
    #[must_use]
    pub fn env(&self) -> &E {
        &self.env
    }

    /// Count a successful L0→L1 install (off-lock persist path).
    pub fn note_l0_compact(&mut self) {
        self.compact_count = self.compact_count.saturating_add(1);
    }
}

/// Off-lock MANIFEST/`CURRENT` write (RFC-0041 P1.1).
pub struct ManifestPersist<E: Env> {
    env: E,
    dir: PathBuf,
    vs: VersionSet,
    sync: bool,
    /// Snapshot epoch (F187): `write` no-ops when a newer epoch already won.
    epoch: u64,
    /// Highest epoch written to disk, shared with `Db`.
    gate: Arc<Mutex<u64>>,
}

impl<E: Env> ManifestPersist<E> {
    /// `fsync` MANIFEST + CURRENT. Does not touch `Db`.
    ///
    /// F187: a snapshot taken before a newer successful persist is stale —
    /// writing it would regress `CURRENT` (and `manifest::store` deletes the
    /// newer MANIFEST). Stale snapshots no-op; the watermark advances only on
    /// success, so a failed newer write leaves older snapshots eligible.
    ///
    /// # Errors
    /// Env I/O.
    pub fn write(self) -> Result<()> {
        let mut written = self.gate.lock();
        if self.epoch <= *written {
            return Ok(());
        }
        let res = manifest::store(&self.env, &self.dir, &self.vs, self.sync);
        // F196: committed-unsynced landed on disk (CURRENT swung) — the
        // epoch gate must advance so an older snapshot cannot overwrite it.
        let committed =
            res.is_ok() || matches!(res, Err(CoreError::ManifestCommittedUnsynced { .. }));
        if committed {
            *written = self.epoch;
        }
        res
    }
}

/// Rollback token for [`Db::apply_l0_install`].
pub struct L0InstallUndo {
    prev_next: u64,
    prev_manifest: u64,
    n: usize,
}

/// Rollback token for [`Db::apply_prepared_l0_compact`].
pub struct L0CompactUndo {
    prev_tables: Vec<SstTable>,
    prev_levels: Vec<u32>,
    prev_manifest: u64,
    prev_earliest: SequenceNumber,
    old_paths: Vec<PathBuf>,
}

impl L0CompactUndo {
    /// SST paths replaced by the compact (delete only after MANIFEST is durable).
    #[must_use]
    pub fn old_paths(&self) -> &[PathBuf] {
        &self.old_paths
    }
}

fn write_checkpoint_meta(env: &impl Env, dest: &Path, meta: &CheckpointMeta) -> Result<()> {
    let path = dest.join(CHECKPOINT_META_FILE);
    let mut body = Vec::new();
    // PDBCKP02: last_sequence + sst_count + earliest_readable_seq.
    body.extend_from_slice(b"PDBCKP02");
    body.extend_from_slice(&meta.last_sequence.to_le_bytes());
    body.extend_from_slice(&(meta.sst_count as u64).to_le_bytes());
    body.extend_from_slice(&meta.earliest_readable_seq.to_le_bytes());
    let crc = crc32c::crc32c(&body);
    body.extend_from_slice(&crc.to_le_bytes());
    let mut f = env.create(&path)?;
    f.write_all(&body)?;
    f.sync_all()?;
    Ok(())
}

/// Read [`CHECKPOINT_META_FILE`] written by [`Db::create_checkpoint`].
///
/// Accepts **PDBCKP02** (with watermark) and legacy **PDBCKP01** (`earliest=0`).
///
/// # Errors
/// Missing/corrupt meta or I/O.
pub fn read_checkpoint_meta(env: &impl Env, dir: impl AsRef<Path>) -> Result<CheckpointMeta> {
    let path = dir.as_ref().join(CHECKPOINT_META_FILE);
    if !env.exists(&path) {
        return Err(CoreError::Internal(format!(
            "missing {CHECKPOINT_META_FILE} in {}",
            dir.as_ref().display()
        )));
    }
    let mut f = env.open_read(&path)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    if buf.len() < 8 + 8 + 8 + 4 {
        return Err(CoreError::Internal("checkpoint meta too short".into()));
    }
    let (payload, crc_bytes) = buf.split_at(buf.len() - 4);
    let crc_arr: [u8; 4] = crc_bytes
        .try_into()
        .map_err(|_| CoreError::Internal("checkpoint meta CRC truncated".into()))?;
    let stored = u32::from_le_bytes(crc_arr);
    let computed = crc32c::crc32c(payload);
    if !crate::wal::crc::crc_match_ok(stored, computed) {
        return Err(CoreError::Internal(format!(
            "checkpoint meta CRC mismatch: stored {stored:#x} computed {computed:#x}"
        )));
    }
    if payload.len() < 8 {
        return Err(CoreError::Internal("checkpoint meta too short".into()));
    }
    let magic = &payload[0..8];
    let seq_arr: [u8; 8] = payload[8..16]
        .try_into()
        .map_err(|_| CoreError::Internal("checkpoint meta seq truncated".into()))?;
    let last_sequence = u64::from_le_bytes(seq_arr);
    let count_arr: [u8; 8] = payload[16..24]
        .try_into()
        .map_err(|_| CoreError::Internal("checkpoint meta count truncated".into()))?;
    let sst_count_u64 = u64::from_le_bytes(count_arr);
    let sst_count = usize::try_from(sst_count_u64).map_err(|_| {
        CoreError::Internal(format!(
            "checkpoint sst_count {sst_count_u64} does not fit usize"
        ))
    })?;
    let earliest_readable_seq = if magic == b"PDBCKP02" {
        if payload.len() < 32 {
            return Err(CoreError::Internal(
                "checkpoint meta v2 truncated (missing earliest_readable)".into(),
            ));
        }
        let ear_arr: [u8; 8] = payload[24..32]
            .try_into()
            .map_err(|_| CoreError::Internal("checkpoint meta earliest truncated".into()))?;
        u64::from_le_bytes(ear_arr)
    } else if magic == b"PDBCKP01" {
        if payload.len() != 24 {
            return Err(CoreError::Internal(
                "checkpoint meta v1 trailing garbage".into(),
            ));
        }
        0
    } else {
        return Err(CoreError::Internal("bad checkpoint meta magic".into()));
    };
    Ok(CheckpointMeta {
        last_sequence,
        sst_count,
        earliest_readable_seq,
    })
}

/// Copy a checkpoint directory (or live DB dir file set) into an empty `dest`.
///
/// Used by ops restore; does not open the DB. Caller should [`Db::open`] after.
///
/// # Errors
/// I/O or non-empty dest.
pub fn copy_db_directory(
    env: &impl Env,
    src: impl AsRef<Path>,
    dest: impl AsRef<Path>,
) -> Result<()> {
    let src = src.as_ref();
    let dest = dest.as_ref();
    if env.exists(dest) {
        let names = env.read_dir_names(dest)?;
        if !names.is_empty() {
            return Err(CoreError::Internal(format!(
                "copy dest not empty: {}",
                dest.display()
            )));
        }
    } else {
        env.create_dir_all(dest)?;
    }
    for name in env.read_dir_names(src)? {
        if name == crate::lock::LOCK_FILE {
            continue; // never copy live LOCK
        }
        let from = src.join(&name);
        let to = dest.join(&name);
        // Skip nested dirs for base layout (checkpoints are flat; unknown
        // dirs re-materialize on open). `history` is the exception: it is
        // the only copy of archived versions after horizon GC (F175).
        // `metadata_len` succeeds on directories on some platforms, so test
        // dir-ness explicitly.
        if env.is_dir(&from).unwrap_or(false) {
            if name == "history" {
                let dest_hist = dest.join(&name);
                env.create_dir_all(&dest_hist)?;
                for seg in env.read_dir_names(&from)? {
                    env.copy_file(&from.join(&seg), &dest_hist.join(&seg))?;
                }
            }
            continue;
        }
        if env.metadata_len(&from).is_ok() {
            env.copy_file(&from, &to)?;
        }
    }
    Ok(())
}

/// Count-cache key buffer: inline for the small windows real queries use,
/// heap fallback for pathological bounds. Avoids a malloc per scan op.
pub(crate) enum CountKeyBuf {
    Inline { buf: [u8; 64], len: usize },
    Heap(Vec<u8>),
}

impl CountKeyBuf {
    pub(crate) fn from_slice(s: &[u8]) -> Self {
        if s.len() <= 64 {
            let mut buf = [0u8; 64];
            buf[..s.len()].copy_from_slice(s);
            Self::Inline { buf, len: s.len() }
        } else {
            Self::Heap(s.to_vec())
        }
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        match self {
            Self::Inline { buf, len } => &buf[..*len],
            Self::Heap(v) => v,
        }
    }
}

impl AsRef<[u8]> for CountKeyBuf {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

/// InternalKey order (user asc, seq desc, kind desc) as a bare predicate.
fn internal_less(a: &InternalKey, b: &InternalKey) -> bool {
    a < b
}

/// One layer's borrowed stream for [`Db::count_visible`] (RFC-0037 P1.3).
///
/// `head()` is the next yieldable entry after filtering (kind, snapshot,
/// window); `step_user` advances past every version of one user key.
enum CountCursor<'a> {
    Mem(MemCountCursor<'a>),
    Sst(SstCountCursor<'a>),
}

impl CountCursor<'_> {
    fn head(&self) -> Option<&InternalKey> {
        match self {
            Self::Mem(c) => c.head(),
            Self::Sst(c) => c.head(),
        }
    }

    /// Advance past the current head's user key without `Bytes::clone`.
    fn step_current_user(&mut self) {
        match self {
            Self::Mem(c) => c.step_current(),
            Self::Sst(c) => c.step_current(),
        }
    }
}

/// Memtable cursor over the bounded user window (versions newest-first).
struct MemCountCursor<'a> {
    it: MemCountIter<'a>,
    head: Option<&'a InternalKey>,
    snapshot: SequenceNumber,
}

enum MemCountIter<'a> {
    /// Common path: concrete BTree range or map+tail merge (no `dyn`).
    Range(crate::memtable::MemInternalIter<'a>),
    /// Range tombstones whose start sits outside the window: walk the
    /// full table with a concrete filter (RFC-0039 P2.1: no `Box<dyn>`).
    Filter(MemCountFilter<'a>),
}

/// Concrete in-range filter over [`crate::memtable::MemInternalIter`].
struct MemCountFilter<'a> {
    inner: crate::memtable::MemInternalIter<'a>,
    start: Bound<&'a [u8]>,
    end: Bound<&'a [u8]>,
}

impl<'a> Iterator for MemCountIter<'a> {
    type Item = (&'a InternalKey, &'a Bytes);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Range(it) => it.next(),
            Self::Filter(it) => loop {
                let (k, v) = it.inner.next()?;
                if crate::merge::user_key_in_range(k.user_key.as_ref(), it.start, it.end) {
                    return Some((k, v));
                }
            },
        }
    }
}

impl<'a> MemCountCursor<'a> {
    fn new(
        table: &'a MemTable,
        start: Bound<&'a [u8]>,
        end: Bound<&'a [u8]>,
        snapshot: SequenceNumber,
    ) -> Self {
        // Same two branches as the owned memtable stream: tombstone-bearing
        // tables iterate everything (tombstone starts may precede the
        // window), bounded range otherwise.
        let it = if table.has_range_tombstones() {
            MemCountIter::Filter(MemCountFilter {
                inner: table.iter_internal_iter(Bound::Unbounded, Bound::Unbounded),
                start,
                end,
            })
        } else {
            MemCountIter::Range(table.iter_internal_iter_at(start, end, snapshot))
        };
        let mut c = Self {
            it,
            head: None,
            snapshot,
        };
        c.settle();
        c
    }

    fn settle(&mut self) {
        self.head = self
            .it
            .by_ref()
            .find(|(k, _)| k.kind != ValueType::RangeDeletion && k.sequence <= self.snapshot)
            .map(|(k, _)| k);
    }

    fn head(&self) -> Option<&InternalKey> {
        self.head
    }

    fn step_user(&mut self, user: &[u8]) {
        // RFC-0054 P1.3: fast group skip — popping one version at a time
        // made each scanned hot user cost O(its versions) after apply
        // filled the shared memtable (Filter keeps the old loop).
        if let MemCountIter::Range(it) = &mut self.it {
            it.step_user(user);
            self.head = None;
            self.settle();
            return;
        }
        while self.head.is_some_and(|h| h.user_key.as_ref() == user) {
            self.head = None;
            self.settle();
        }
    }

    fn step_current(&mut self) {
        let Some(h) = self.head else {
            return;
        };
        let user = h.user_key.as_ref();
        self.step_user(user);
    }
}

/// Concrete SST block loader (RFC-0040: no `Box<dyn>` per scan).
struct SstBlockLoad<'a> {
    cache: &'a crate::cache::BlockCache,
    table: &'a crate::sst::SstTable,
}

impl SstBlockLoad<'_> {
    fn load(&self, bi: usize) -> Option<std::sync::Arc<Vec<(InternalKey, Bytes)>>> {
        Some(self.cache.get_or_insert_with(self.table.path(), bi, || {
            self.table.decode_block(bi).unwrap_or_default()
        }))
    }
}

/// SST cursor: walks only overlapping blocks (block cache) with the same
/// filtering as `SstRangeIter`, minus the owned-key clone per yield.
struct SstCountCursor<'a> {
    current: Option<std::sync::Arc<Vec<(InternalKey, Bytes)>>>,
    idx: usize,
    blocks: std::vec::IntoIter<usize>,
    load: SstBlockLoad<'a>,
    start: Bound<&'a [u8]>,
    end: Bound<&'a [u8]>,
    snapshot: SequenceNumber,
    exhausted: bool,
}

impl<'a> SstCountCursor<'a> {
    fn new(
        table: &'a crate::sst::SstTable,
        start: Bound<&'a [u8]>,
        end: Bound<&'a [u8]>,
        snapshot: SequenceNumber,
        cache: &'a crate::cache::BlockCache,
    ) -> Self {
        let mut c = Self {
            current: None,
            idx: 0,
            blocks: if table.is_lazy() {
                table.blocks_overlapping_range(start, end).into_iter()
            } else {
                Vec::new().into_iter()
            },
            load: SstBlockLoad { cache, table },
            start,
            end,
            snapshot,
            exhausted: false,
        };
        if !table.is_lazy() {
            // Eager tables: one synthetic "block" with the in-range leftover.
            let leftover: Vec<_> = table
                .entries_cloned()
                .unwrap_or_else(|e| {
                    panic!(
                        "pedradb: corrupt SST in {} on count: {e}",
                        table.path().display()
                    )
                })
                .into_iter()
                .filter(|(k, _)| {
                    k.kind != ValueType::RangeDeletion
                        && crate::merge::user_key_in_range(k.user_key.as_ref(), start, end)
                })
                .collect();
            c.current = Some(std::sync::Arc::new(leftover));
            c.idx = 0;
        }
        c.settle();
        c
    }

    fn settle(&mut self) {
        loop {
            if let Some(ref block) = self.current {
                while self.idx < block.len() {
                    let k = &block[self.idx].0;
                    let uk = k.user_key.as_ref();
                    let past_end = match self.end {
                        Bound::Unbounded => false,
                        Bound::Included(e) => uk > e,
                        Bound::Excluded(e) => uk >= e,
                    };
                    if past_end {
                        self.exhausted = true;
                        self.current = None;
                        self.blocks = Vec::new().into_iter();
                        return;
                    }
                    let before_start = match self.start {
                        Bound::Unbounded => false,
                        Bound::Included(s) => uk < s,
                        Bound::Excluded(s) => uk <= s,
                    };
                    let skip = before_start
                        || k.kind == ValueType::RangeDeletion
                        || k.sequence > self.snapshot;
                    if skip {
                        self.idx += 1;
                        continue;
                    }
                    return; // head is block[self.idx]
                }
            }
            let Some(bi) = self.blocks.next() else {
                self.exhausted = true;
                return;
            };
            self.current = self.load.load(bi);
            self.idx = match (&self.current, self.start) {
                (Some(block), Bound::Included(s)) => {
                    block.partition_point(|(k, _)| k.user_key.as_ref() < s)
                }
                (Some(block), Bound::Excluded(s)) => {
                    block.partition_point(|(k, _)| k.user_key.as_ref() <= s)
                }
                _ => 0,
            };
        }
    }

    fn head(&self) -> Option<&InternalKey> {
        if self.exhausted {
            return None;
        }
        self.current
            .as_ref()
            .and_then(|b| b.get(self.idx))
            .map(|(k, _)| k)
    }

    fn step_user(&mut self, user: &[u8]) {
        while self.head().is_some_and(|h| h.user_key.as_ref() == user) {
            self.idx += 1;
            self.settle();
        }
    }

    fn step_current(&mut self) {
        let Some(h) = self.head() else {
            return;
        };
        // SST block may be dropped on settle — copy the user key off the
        // Arc before advancing (RFC-0039 P2.1: not `Bytes::clone`).
        let buf = CountKeyBuf::from_slice(h.user_key.as_ref());
        self.step_user(buf.as_ref());
    }
}

pub(crate) fn count_cache_key(
    start: Bound<&[u8]>,
    end: Bound<&[u8]>,
    limit: Option<usize>,
) -> CountKeyBuf {
    let mut inline = [0u8; 64];
    let mut heap: Option<Vec<u8>> = None;
    let mut len = 0usize;
    let mut push = |bytes: &[u8]| {
        if let Some(h) = heap.as_mut() {
            h.extend_from_slice(bytes);
        } else if len + bytes.len() <= inline.len() {
            inline[len..len + bytes.len()].copy_from_slice(bytes);
            len += bytes.len();
        } else {
            let mut h = Vec::with_capacity(64);
            h.extend_from_slice(&inline[..len]);
            h.extend_from_slice(bytes);
            heap = Some(h);
        }
    };
    let push_bound = |push: &mut dyn FnMut(&[u8]), b: Bound<&[u8]>| match b {
        Bound::Unbounded => push(&[0]),
        Bound::Included(s) => {
            push(&[1]);
            push(&(s.len() as u32).to_le_bytes());
            push(s);
        }
        Bound::Excluded(s) => {
            push(&[2]);
            push(&(s.len() as u32).to_le_bytes());
            push(s);
        }
    };
    let mut p = |b: &[u8]| push(b);
    push_bound(&mut p, start);
    push_bound(&mut p, end);
    p(&limit.map(|n| n as u64).unwrap_or(u64::MAX).to_le_bytes());
    match heap {
        Some(h) => CountKeyBuf::Heap(h),
        None => CountKeyBuf::Inline { buf: inline, len },
    }
}

fn apply_record(mem: &mut MemTable, rec: &WriteRecord) {
    for op in &rec.ops {
        match op.kind {
            ValueType::Value => {
                mem.put(op.key.clone(), op.sequence, op.value.clone());
            }
            ValueType::Deletion => {
                mem.delete(op.key.clone(), op.sequence);
            }
            ValueType::RangeDeletion => {
                mem.delete_range(op.key.clone(), op.value.clone(), op.sequence);
            }
        }
    }
}

/// RFC-0040: move `WriteOp` Bytes into the memtable (no extra payload memcpy).
fn apply_ops_owned(mem: &mut MemTable, ops: impl IntoIterator<Item = WriteOp>) {
    mem.insert_many(ops.into_iter().map(|op| {
        (
            crate::key::InternalKey::new(op.key, op.sequence, op.kind),
            op.value,
        )
    }));
}

/// Repeat puts of the same slice (kvrocks SET / blob) share one `Bytes`.
fn intern_bytes(v: &[u8]) -> Bytes {
    thread_local! {
        static LAST: std::cell::RefCell<Bytes> = const { std::cell::RefCell::new(Bytes::new()) };
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

/// Staged SST inventory for value-log GC (not yet installed in MANIFEST).
struct PreparedVlogSsts {
    tables: Vec<SstTable>,
    levels: Vec<u32>,
    next_file_num: u64,
    old_paths: Vec<PathBuf>,
    bytes_written: u64,
}

/// Prepare-phase output for value-log GC (staged files only; no `Db` mutation).
struct VlogGcPrepared {
    stats: VlogRewriteStats,
    remap: std::collections::HashMap<u64, Bytes>,
    ssts: PreparedVlogSsts,
}

/// Recover SST tables from MANIFEST when present, else directory scan (legacy).
///
/// Recovered SST inventory: tables, levels, next file num, manifest num,
/// `vlog_use_new`, max seq, earliest_readable_seq.
type RecoveredSsts = (
    Vec<SstTable>,
    Vec<u32>,
    u64,
    u64,
    bool,
    SequenceNumber,
    SequenceNumber,
);

/// Returns `(tables, levels, next_file_num, manifest_file_num, vlog_use_new, max_sequence, earliest_readable)`.
///
/// Every decision routes through `manifest_kernel` (RFC-0056 P0.1): the
/// observation (`manifest::load` + listed-SST existence) is gathered here,
/// the reopen action is decided by the pure kernel.
fn recover_ssts<E: Env>(
    env: &E,
    dir: &Path,
    sync: bool,
    table_cache: &TableCache,
) -> Result<RecoveredSsts> {
    use crate::manifest_kernel::{
        first_install_action, sst_recover_action, FirstInstallAction, FirstInstallOutcome,
        ListedSst, ManifestObs, SstRecoverAction,
    };

    // Gather the two observations; the kernel decides.
    let loaded = manifest::load(env, dir);
    let (obs, listed, missing_num) = match &loaded {
        Ok(None) => (ManifestObs::Absent, ListedSst::AllPresent, None),
        Ok(Some(vs)) => {
            let missing = vs
                .sst_file_nums
                .iter()
                .copied()
                .find(|num| !env.exists(&VersionSet::sst_path(dir, *num)));
            match missing {
                Some(num) => (ManifestObs::Inventory, ListedSst::Missing(num), Some(num)),
                None => (ManifestObs::Inventory, ListedSst::AllPresent, None),
            }
        }
        // Corrupt CURRENT/MANIFEST (and any other load failure — also
        // fail-closed) refuse below via the kernel; the original error is
        // what the caller sees.
        Err(_) => (ManifestObs::Corrupt, ListedSst::AllPresent, None),
    };

    match sst_recover_action(obs, listed) {
        SstRecoverAction::ServeInventory => {
            let mut vs = match loaded {
                Ok(Some(vs)) => vs,
                // Kernel contract: ServeInventory implies a loaded inventory.
                _ => unreachable!("ServeInventory requires a decoded MANIFEST"),
            };
            vs.normalize_levels();
            // Drop SST files not listed (mid-compact / failed flush orphans).
            manifest::gc_orphan_ssts(env, dir, &vs.sst_file_nums)?;
            let mut max_seq = 0;
            let mut tables = Vec::with_capacity(vs.sst_file_nums.len());
            let mut levels = Vec::with_capacity(vs.sst_file_nums.len());
            for (i, num) in vs.sst_file_nums.iter().enumerate() {
                let path = VersionSet::sst_path(dir, *num);
                if !env.exists(&path) {
                    return Err(CoreError::CorruptManifest(format!(
                        "MANIFEST lists missing SST {num:06}.sst"
                    )));
                }
                let t = table_cache.get_or_open(env, &path)?;
                max_seq = max_seq.max(t.max_sequence());
                let mut table = (*t).clone();
                if let Some(cf) = vs.sst_cfs.get(i).filter(|s| !s.is_empty()) {
                    table = table.with_cf(cf.clone());
                }
                tables.push(table);
                levels.push(vs.sst_levels.get(i).copied().unwrap_or(0));
            }
            Ok((
                tables,
                levels,
                vs.next_file_num,
                vs.manifest_file_num,
                vs.vlog_use_new,
                max_seq,
                vs.earliest_readable_seq,
            ))
        }
        SstRecoverAction::ScanAndInstall => {
            // Kernel contract: ScanAndInstall implies absent inventory.
            debug_assert!(loaded.is_ok() && loaded.as_ref().unwrap().is_none());
            // Legacy / first open: scan directory, then write initial MANIFEST.
            let (tables, next_file_num, max_seq) = load_ssts_scan(env, dir, table_cache)?;
            let levels = vec![0u32; tables.len()];
            let mut vs = VersionSet {
                next_file_num,
                sst_file_nums: tables
                    .iter()
                    .filter_map(|t| {
                        t.path()
                            .file_name()
                            .and_then(|n| n.to_str())
                            .and_then(manifest::parse_sst_name)
                    })
                    .collect(),
                sst_levels: levels.clone(),
                manifest_file_num: 0,
                vlog_use_new: false,
                earliest_readable_seq: 0,
                sst_cfs: tables.iter().map(|t| t.cf().to_string()).collect(),
            };
            // Always install so subsequent opens use inventory (even if empty).
            // F196: committed-unsynced during first open = the inventory IS
            // committed (nothing acked is at risk yet); open proceeds.
            let installed = match manifest::install_next(env, dir, &mut vs, sync) {
                Ok(()) => FirstInstallOutcome::Committed,
                Err(CoreError::ManifestCommittedUnsynced { .. }) => {
                    FirstInstallOutcome::CommittedUnsynced
                }
                Err(e) => return Err(e),
            };
            if first_install_action(installed) == FirstInstallAction::RefuseOpen {
                return Err(CoreError::CorruptManifest(
                    "first MANIFEST install failed".into(),
                ));
            }
            Ok((
                tables,
                levels,
                next_file_num,
                vs.manifest_file_num,
                false,
                max_seq,
                0,
            ))
        }
        SstRecoverAction::RefuseOpen => match (loaded, missing_num) {
            (Err(e), _) => Err(e),
            (Ok(_), Some(num)) => Err(CoreError::CorruptManifest(format!(
                "MANIFEST lists missing SST {num:06}.sst"
            ))),
            // Kernel contract: RefuseOpen implies damage or a missing listed
            // SST; the defensive arm keeps the open fail-closed either way.
            (Ok(_), None) => Err(CoreError::CorruptManifest(
                "SST inventory damaged at reopen".into(),
            )),
        },
    }
}

/// Load `NNNNNN.sst` files ascending; return tables, next file num, max sequence.
/// `table_cache` supplies the payload kit (RFC-0042 v18): each opened table is
/// attached + registered so a bounded open stays within budget during the scan.
fn load_ssts_scan<E: Env>(
    env: &E,
    dir: &Path,
    table_cache: &TableCache,
) -> Result<(Vec<SstTable>, u64, SequenceNumber)> {
    let kit = table_cache.payload_kit();
    let mut files: Vec<(u64, PathBuf)> = Vec::new();
    if env.exists(dir) {
        for name in env.read_dir_names(dir)? {
            if let Some(num) = manifest::parse_sst_name(&name) {
                files.push((num, dir.join(name)));
            }
        }
    }
    files.sort_by_key(|(n, _)| *n);
    let next_file_num = files.last().map_or(1, |(n, _)| n + 1);
    let mut max_seq = 0;
    let mut tables = Vec::with_capacity(files.len());
    for (_, path) in files {
        let t = SstTable::open_on(env, path)?;
        if let Some(kit) = &kit {
            t.attach_payload_kit(&kit.source, &kit.pool);
        }
        max_seq = max_seq.max(t.max_sequence());
        tables.push(t);
    }
    Ok((tables, next_file_num, max_seq))
}

/// Whole-levels rewrite chunk target (logical bytes). RocksDB's default L1
/// target file size; small enough that the chunked writer's per-chunk
/// transient (chunk body + bloom + read-back) stays small when a rewrite
/// covers an entire level (guest 25M settle: 48 × ~230 MB L1 files).
const REWRITE_CHUNK_TARGET_BYTES: u64 = 64 * 1024 * 1024;

/// Current process RSS in KiB for `PEDRA_REWRITE_DIAG` (Linux `/proc`,
/// `ps` elsewhere). `None` when unavailable.
fn rewrite_diag_rss_kib() -> Option<u64> {
    if cfg!(target_os = "linux") {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("VmRSS:"))
                    .and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse().ok()))
            })
    } else {
        let out = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    }
}

/// Release allocator-free pages to the OS after each rewrite chunk. glibc
/// arenas pin freed small chunks next to retained ones (per-block index
/// keys), so a whole-levels rewrite's RSS creeps even though nothing is
/// retained — the 6M local repro (macOS) is flat across the job while the
/// 25M guest climb was monotonic. The glibc FFI lives in `pedradb-posix`
/// (core is `forbid(unsafe_code)`); no-op elsewhere.
fn rewrite_trim_allocator() {
    pedradb_posix::trim_process_heap();
}

/// Merge `tables` into `{file_num:06}.sst` (RFC-0037 streaming when `!gc.requests_gc()`).
/// Approximate on-disk footprint of one merged entry (key + value + entry
/// overhead) — the chunking size proxy for [`write_merged_tables`].
fn merged_entry_bytes(ikey: &InternalKey, value: &Bytes) -> u64 {
    (ikey.user_key.len() + value.len() + 24) as u64
}

/// Finalize one merged chunk file: rename the `.tmp`, fsync the dir, open.
fn finish_merged_chunk_on(
    env: &impl Env,
    dir: &Path,
    file_num: u64,
    do_sync_dir: bool,
    table: SstTable,
) -> Result<SstTable> {
    let final_path = dir.join(format!("{file_num:06}.sst"));
    let tmp_path = dir.join(format!("{file_num:06}.sst.tmp"));
    env.rename(&tmp_path, &final_path)?;
    if do_sync_dir {
        let _ = env.sync_dir(dir);
    }
    // The writer's in-place table is the truth for the bytes just written;
    // a re-open here re-read + decompressed + decoded every entry of every
    // compaction chunk (the caller-side half of the read-back v21p removed
    // from the writer — the per-job drain rate stayed ~110 MiB/s because
    // of exactly this call). Recovery opens still verify fully.
    Ok(table.with_path(final_path))
}

/// How many key-space spans a merge job should be split into (Rocks-shaped
/// subcompactions). 1 = sequential. Gated on total input size: small jobs pay
/// more in thread setup and straggler skew than they gain. `PEDRA_MERGE_SPANS`
/// overrides (A/B on the guest without a rebuild).
fn merge_span_count(env: &impl Env, tables: &[SstTable]) -> usize {
    if let Ok(v) = std::env::var("PEDRA_MERGE_SPANS") {
        if let Ok(n) = v.trim().parse::<usize>() {
            return n.max(1);
        }
    }
    const PARALLEL_MIN_INPUT_BYTES: u64 = 96 * 1024 * 1024;
    let total: u64 = tables
        .iter()
        .map(|t| env.metadata_len(t.path()).unwrap_or(0))
        .sum();
    if total < PARALLEL_MIN_INPUT_BYTES {
        return 1;
    }
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(8)
}

/// `parts - 1` user-key split points sampled from the inputs' in-memory block
/// indexes (no data read). Consecutive splits define half-open spans
/// `[split_i, split_{i+1})`; a user key's whole version run stays inside one
/// span, so per-user-key GC decisions remain complete per span.
fn sample_span_splits(tables: &[SstTable], parts: usize) -> Vec<Vec<u8>> {
    let mut keys: Vec<&[u8]> = Vec::new();
    for t in tables {
        keys.extend(t.block_first_user_keys());
    }
    keys.sort_unstable();
    keys.dedup();
    if keys.len() < parts {
        return Vec::new();
    }
    (1..parts)
        .map(|i| {
            let at = (keys.len() * i) / parts;
            keys[at.min(keys.len() - 1)].to_vec()
        })
        .collect()
}

/// Merge `tables` into one **or more** SSTs bounded to user keys in `[lo, hi)`
/// (`None` = unbounded on that side), split at `split_target` logical bytes.
/// Splits fall between user keys — output files hold disjoint contiguous key
/// ranges at the same level. `file_alloc` hands out the next output file
/// number (reserved by the caller); `span_budget` is a hard cap on this
/// span's chunk count: when the split target would exceed it, the current
/// chunk simply grows past target (sizing is advisory; correctness is that
/// the writer never touches numbers beyond the reserved range).
///
/// GC rewrites stream too: `GcMergeSource` applies the same retention
/// decisions per user-key run, so no input table is ever materialized.
#[allow(clippy::too_many_arguments)]
fn write_merged_tables_span(
    env: &impl Env,
    dir: &Path,
    tables: &[SstTable],
    gc: crate::merge::CompactGcOptions,
    do_sync_dir: bool,
    split_target: u64,
    span_budget: usize,
    kit: Option<&crate::cache::PayloadKit>,
    lo: Option<&[u8]>,
    hi: Option<&[u8]>,
    file_alloc: &mut dyn FnMut() -> u64,
    span_tag: usize,
) -> Result<Vec<SstTable>> {
    let bloom_hint: usize = tables.iter().map(SstTable::len).sum();
    let mut out: Vec<SstTable> = Vec::new();
    if tables.is_empty() {
        return Ok(out);
    }
    let streams: Vec<_> = tables
        .iter()
        .map(|t| t.iter_internal_between(lo, hi))
        .collect();
    let merge = crate::merge::KwayInternalMerge::from_streams(streams)?;
    // GC rewrites stream too: `GcMergeSource` applies the same retention
    // decisions per user-key run, so no input table is ever materialized
    // (the old batch path held every decoded input table plus a BTreeMap
    // copy — at bulk-load scale that tripled the merge footprint).
    let mut source: Box<dyn crate::merge::CompactSource> = if gc.requests_gc() {
        Box::new(crate::merge::GcMergeSource::new(merge, gc))
    } else {
        Box::new(merge)
    };
    let mut peeked: Option<Result<(InternalKey, Bytes)>> = None;
    let mut stream_ended = false;
    let mut last_user: Option<Bytes> = None;
    // PEDRA_REWRITE_DIAG: one line per finished chunk (guest 25M settle
    // OOM hunts — RSS trajectory of the whole-levels rewrite).
    let rewrite_diag = std::env::var_os("PEDRA_REWRITE_DIAG").is_some();
    let rewrite_started = std::time::Instant::now();
    while peeked.is_some() || !stream_ended {
        let mut acc = 0u64;
        let mut closed = false;
        let mut entries = std::iter::from_fn(|| {
            if closed {
                return None;
            }
            let entry = match peeked.take() {
                Some(e) => e,
                None => match source.next_entry() {
                    Ok(Some(e)) => Ok(e),
                    Ok(None) => {
                        stream_ended = true;
                        closed = true;
                        return None;
                    }
                    Err(e) => {
                        closed = true;
                        return Some(Err(e));
                    }
                },
            };
            let ok_entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    closed = true;
                    return Some(Err(e));
                }
            };
            if crate::compact_kernel::compact_should_split_at(acc, split_target)
                && out.len() + 1 < span_budget
                && last_user
                    .as_ref()
                    .is_none_or(|u| u.as_ref() != ok_entry.0.user_key.as_ref())
            {
                // Target reached, the user key changed, and one more chunk
                // still fits this span's share of the reserved range —
                // start a new file with this entry. A same-user version run
                // never splits: it stays in one file. Past the span budget
                // the split is skipped and the chunk runs long instead.
                peeked = Some(Ok(ok_entry));
                closed = true;
                return None;
            }
            acc = acc.saturating_add(merged_entry_bytes(&ok_entry.0, &ok_entry.1));
            last_user = Some(ok_entry.0.user_key.clone());
            Some(Ok(ok_entry))
        });
        let file_num = file_alloc();
        let tmp_path = dir.join(format!("{file_num:06}.sst.tmp"));
        let written = crate::sst::write_sst_try_sorted_on(env, &tmp_path, &mut entries, bloom_hint);
        drop(entries);
        let written = match written {
            Ok(table) => table,
            Err(e) => {
                let _ = env.remove_file(&tmp_path);
                return Err(e);
            }
        };
        let chunk = finish_merged_chunk_on(env, dir, file_num, do_sync_dir, written)?;
        // Register the chunk's resident body the moment it exists: this
        // Vec accumulates every chunk of the job, and a freshly opened
        // chunk holds its whole file body in RAM. Unregistered payloads
        // are invisible to the pool and can never be evicted — a
        // whole-levels rewrite then holds its entire output resident
        // (the 25M settle OOM at ~6 chunks). Idempotent with the
        // install-time `adopt_sst`.
        if let Some(kit) = kit {
            chunk.attach_payload_kit(&kit.source, &kit.pool);
        }
        out.push(chunk);
        rewrite_trim_allocator();
        if rewrite_diag {
            eprintln!(
                "REWRITEDIAG span={span_tag} chunk={file_num:06} out={} pool_b={} rss_kib={} t={:.1}s",
                out.len(),
                kit.map(|k| k.pool.resident_bytes()).unwrap_or(0),
                rewrite_diag_rss_kib().unwrap_or(0),
                rewrite_started.elapsed().as_secs_f32()
            );
        }
    }
    Ok(out)
}

/// Merge `tables` into one **or more** SSTs split at `split_target` logical
/// bytes (sequential; bounded jobs and small inputs). `chunk_budget` caps the
/// chunk count (the reserved file-number range).
fn write_merged_tables(
    env: &impl Env,
    dir: &Path,
    first_file_num: u64,
    tables: &[SstTable],
    gc: crate::merge::CompactGcOptions,
    do_sync_dir: bool,
    split_target: u64,
    chunk_budget: usize,
    kit: Option<&crate::cache::PayloadKit>,
) -> Result<Vec<SstTable>> {
    let mut next = first_file_num;
    let mut alloc = || {
        let n = next;
        next += 1;
        n
    };
    write_merged_tables_span(
        env,
        dir,
        tables,
        gc,
        do_sync_dir,
        split_target,
        chunk_budget,
        kit,
        None,
        None,
        &mut alloc,
        0,
    )
}

/// [`write_merged_tables`] with key-space parallelism when the inputs are
/// large ([`merge_span_count`]): Rocks-shaped subcompactions. File numbers
/// come from one shared atomic over the reserved range, so spans interleave
/// but never collide and stay gapless.
fn write_merged_tables_parallel<E: Env + Sync>(
    env: &E,
    dir: &Path,
    first_file_num: u64,
    tables: &[SstTable],
    gc: crate::merge::CompactGcOptions,
    do_sync_dir: bool,
    split_target: u64,
    chunk_budget: usize,
    kit: Option<&crate::cache::PayloadKit>,
) -> Result<Vec<SstTable>> {
    let parts = merge_span_count(env, tables).min(chunk_budget.max(1));
    if parts <= 1 {
        return write_merged_tables(
            env,
            dir,
            first_file_num,
            tables,
            gc,
            do_sync_dir,
            split_target,
            chunk_budget,
            kit,
        );
    }
    let splits = sample_span_splits(tables, parts);
    let parts = splits.len() + 1;
    let span_budget = (chunk_budget / parts).max(1);
    let next_file_num = std::sync::atomic::AtomicU64::new(first_file_num);
    let alloc_next = &next_file_num;
    // Half-open spans: [None, s0), [s0, s1), ... [s_{n-1}, None).
    let mut lo: Option<Vec<u8>> = None;
    let results: Vec<Result<Vec<SstTable>>> = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(parts);
        for (i, hi) in splits
            .iter()
            .map(Some)
            .chain(std::iter::once(None))
            .enumerate()
        {
            let span_lo = lo.clone();
            let hi = hi.cloned();
            let span_hi = hi.clone();
            let handle = scope.spawn(move || {
                let mut alloc = || alloc_next.fetch_add(1, Ordering::Relaxed);
                write_merged_tables_span(
                    env,
                    dir,
                    tables,
                    gc,
                    do_sync_dir,
                    split_target,
                    span_budget,
                    kit,
                    span_lo.as_deref(),
                    span_hi.as_deref(),
                    &mut alloc,
                    i,
                )
            });
            handles.push(handle);
            lo = hi;
        }
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(CoreError::Internal("merge span panicked".into())))
            })
            .collect()
    });
    let mut out: Vec<SstTable> = Vec::new();
    for r in results {
        out.extend(r?);
    }
    let consumed = next_file_num.into_inner() - first_file_num;
    debug_assert_eq!(
        consumed,
        out.len() as u64,
        "span file numbers must be gapless"
    );
    // Spans are key-disjoint; present them in key order for a tidy inventory.
    out.sort_by(|a, b| {
        a.smallest_user_key()
            .unwrap_or(&[])
            .cmp(b.smallest_user_key().unwrap_or(&[]))
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// RFC-0162 P0.2: BSTAGE line fields for both callers (inline vs
    /// worker) and the off-by-default gate (env unset ⇒ timing off).
    #[test]
    fn rfc0162_bstage_line_format_and_default_off() {
        assert_eq!(
            bulk_stage_line(7, 1234, 56, 1_048_576, 67_108_864, "inline", true),
            "BSTAGE idx=7 epoch_ms=1234 t_ms=56 entries=1048576 bytes=67108864 \
             caller=inline sync=true"
        );
        assert_eq!(
            bulk_stage_line(8, 2000, 78, 2, 1024, "worker", false),
            "BSTAGE idx=8 epoch_ms=2000 t_ms=78 entries=2 bytes=1024 \
             caller=worker sync=false"
        );
        // Skipped when the diagnostic shell exports the var for the run.
        if std::env::var_os("PEDRA_BULK_STAGE_TIMING").is_none() {
            assert!(
                !bulk_stage_timing_on(),
                "BSTAGE must be off when PEDRA_BULK_STAGE_TIMING is unset"
            );
        }
    }

    /// RFC-0162 P1.1 test env: records per-file `sync_data` and every
    /// `Env::advise` (`FenceEnv` pattern — `pedradb-sim` is not a core dep).
    #[derive(Clone)]
    struct BulkProbeEnv {
        inner: StdEnv,
        syncs: std::sync::Arc<std::sync::Mutex<Vec<std::path::PathBuf>>>,
        advises: std::sync::Arc<std::sync::Mutex<Vec<(std::path::PathBuf, u64, u64, AdviseKind)>>>,
    }

    impl BulkProbeEnv {
        fn new() -> Self {
            Self {
                inner: StdEnv,
                syncs: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
                advises: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            }
        }
    }

    struct BulkProbeFile {
        inner: <StdEnv as Env>::File,
        path: std::path::PathBuf,
        syncs: std::sync::Arc<std::sync::Mutex<Vec<std::path::PathBuf>>>,
    }

    impl std::io::Read for BulkProbeFile {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read(buf)
        }
    }
    impl std::io::Write for BulkProbeFile {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.inner.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }
    impl std::io::Seek for BulkProbeFile {
        fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(pos)
        }
    }
    impl EnvFile for BulkProbeFile {
        fn sync_data(&mut self) -> std::io::Result<()> {
            self.syncs.lock().unwrap().push(self.path.clone());
            self.inner.sync_data()
        }
        fn sync_all(&mut self) -> std::io::Result<()> {
            self.inner.sync_all()
        }
        fn set_len(&mut self, len: u64) -> std::io::Result<()> {
            self.inner.set_len(len)
        }
        fn len(&mut self) -> std::io::Result<u64> {
            self.inner.len()
        }
    }

    impl Env for BulkProbeEnv {
        type File = BulkProbeFile;
        fn create_dir_all(&self, path: &std::path::Path) -> std::io::Result<()> {
            self.inner.create_dir_all(path)
        }
        fn create(&self, path: &std::path::Path) -> std::io::Result<Self::File> {
            Ok(BulkProbeFile {
                inner: self.inner.create(path)?,
                path: path.to_path_buf(),
                syncs: self.syncs.clone(),
            })
        }
        fn open_append(&self, path: &std::path::Path) -> std::io::Result<Self::File> {
            Ok(BulkProbeFile {
                inner: self.inner.open_append(path)?,
                path: path.to_path_buf(),
                syncs: self.syncs.clone(),
            })
        }
        fn open_read(&self, path: &std::path::Path) -> std::io::Result<Self::File> {
            Ok(BulkProbeFile {
                inner: self.inner.open_read(path)?,
                path: path.to_path_buf(),
                syncs: self.syncs.clone(),
            })
        }
        fn sync_dir(&self, path: &std::path::Path) -> std::io::Result<()> {
            self.inner.sync_dir(path)
        }
        fn read_dir_names(&self, path: &std::path::Path) -> std::io::Result<Vec<String>> {
            self.inner.read_dir_names(path)
        }
        fn remove_file(&self, path: &std::path::Path) -> std::io::Result<()> {
            self.inner.remove_file(path)
        }
        fn rename(&self, from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
            self.inner.rename(from, to)
        }
        fn exists(&self, path: &std::path::Path) -> bool {
            self.inner.exists(path)
        }
        fn metadata_len(&self, path: &std::path::Path) -> std::io::Result<u64> {
            self.inner.metadata_len(path)
        }
        fn advise(
            &self,
            path: &std::path::Path,
            offset: u64,
            len: u64,
            kind: AdviseKind,
        ) -> std::io::Result<()> {
            self.advises
                .lock()
                .unwrap()
                .push((path.to_path_buf(), offset, len, kind));
            self.inner.advise(path, offset, len, kind)
        }
    }

    /// RFC-0162 P1.1: chunk install is a durability + page-cache hygiene
    /// point — `fdatasync` runs on the tmp SST even with DB `sync=false`,
    /// and the final path gets exactly one whole-file `DONTNEED` after the
    /// rename. Fails if the old `if sync` gate or the missing advise return.
    #[test]
    fn rfc0162_bulk_chunk_install_syncs_and_dontneeds_even_with_sync_off() {
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        let env = BulkProbeEnv::new();
        let mut run = crate::bulk_run::BulkRun::default();
        for i in 0..100u32 {
            run.push(
                Bytes::from(format!("key{i:08}")),
                Bytes::from_static(b"val08bytes"),
                u64::from(i) + 1,
            );
        }
        let (table, num) =
            Db::write_bulk_run_sst(&env, &dir, 7, &run, "default", false, "worker").unwrap();
        let tmp = dir.join("000007.sst.tmp");
        let final_p = dir.join("000007.sst");
        assert_eq!(num, 7);
        assert!(final_p.exists(), "chunk must be renamed into place");
        assert!(!tmp.exists(), "no tmp residue");
        assert_eq!(table.cf(), "default");
        let syncs = env.syncs.lock().unwrap().clone();
        assert!(
            syncs.iter().any(|p| p == &tmp),
            "chunk install must fdatasync the tmp SST even with sync=false, got {syncs:?}"
        );
        assert_eq!(
            env.advises.lock().unwrap().clone(),
            vec![(final_p, 0, 0, AdviseKind::DontNeed)],
            "exactly one whole-file DONTNEED on the final path after rename"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0157 P1.4 — db.rs stage 1: golden-fingerprint characterization
    /// of the CURRENT open/recovery path (nothing extracted;
    /// `db_rs_extracted` stays false). Client-observable fingerprint over
    /// three phases: (1) live full-range scan with a mixed layout
    /// (flushed prefix + WAL tail), (2) clean close + reopen recovery,
    /// (3) WAL-tail put then kill WITHOUT close (`mem::forget` — no
    /// flush; same-PID reopen steals the LOCK) + reopen recovery. Double
    /// replay into fresh dirs must be identical and equal the pinned
    /// golden literal — any behavior change on the open/recovery path
    /// fails here, protecting the stage-2 extraction.
    #[test]
    fn rfc0157_db_open_recovery_golden() {
        fn hexify(b: &[u8]) -> String {
            b.iter().map(|x| format!("{x:02x}")).collect()
        }
        fn scan_all(db: &Db) -> String {
            db.scan(Bound::Unbounded, Bound::Unbounded)
                .map(|kv| format!("{}={}", hexify(&kv.key), hexify(&kv.value)))
                .collect::<Vec<_>>()
                .join(";")
        }
        fn run_scenario() -> String {
            let dir = temp_dir();
            let mut db = Db::open(&dir).unwrap();
            db.put(b"golden/a", b"A1").unwrap();
            db.put(b"golden/b", b"B2").unwrap();
            db.flush().unwrap(); // prefix becomes SST-resident
            db.put(b"golden/c", b"C3").unwrap(); // WAL tail
            let live = scan_all(&db);
            db.close().unwrap();
            let mut db2 = Db::open(&dir).unwrap();
            let after_close = scan_all(&db2);
            db2.put(b"golden/d", b"D4").unwrap(); // tail before kill
            std::mem::forget(db2); // kill without close (no flush, LOCK stolen back)
            let db3 = Db::open(&dir).unwrap();
            let after_kill = scan_all(&db3);
            db3.close().unwrap();
            let _ = fs::remove_dir_all(&dir);
            format!("{live}|{after_close}|{after_kill}")
        }
        let a = run_scenario();
        let b = run_scenario();
        assert_eq!(
            a, b,
            "db.rs open/recovery scenario must replay deterministically"
        );
        eprintln!("RFC0157_GOLDEN={a}");
        assert_eq!(
            a,
            concat!(
                "676f6c64656e2f61=4131;676f6c64656e2f62=4232;676f6c64656e2f63=4333|",
                "676f6c64656e2f61=4131;676f6c64656e2f62=4232;676f6c64656e2f63=4333|",
                "676f6c64656e2f61=4131;676f6c64656e2f62=4232;676f6c64656e2f63=4333;676f6c64656e2f64=4434"
            ),
            "golden fingerprint moved — open/recovery behavior changed (stage-2 protection)"
        );
    }

    /// RFC-0078 P0: live StdEnv put (real fdatasync before Ok) then a
    /// media-proof claim is refused. AS-IS would admit after fsync Ok.
    #[test]
    fn claim_media_durable_refused_after_fsync_ok() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"fsync-lie/k", b"fsync-lie/v").unwrap();
        assert_eq!(db.get(b"fsync-lie/k").as_deref(), Some(&b"fsync-lie/v"[..]));
        assert!(
            !db.claim_media_durable(),
            "fsync Ok must not round to a media theorem"
        );
        assert!(
            crate::group_commit_kernel::media_durable_admitted_as_is(true),
            "AS-IS dente: fsync Ok would claim the drive"
        );
        assert!(!crate::group_commit_kernel::media_durable_admitted(true));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Catalog three-teeth plant. Direct `fsync_ok_is_not_media_proof` /
    /// `claim_media_durable_refused_after_fsync_ok` are **not** this tooth.
    #[test]
    fn media_durable_admitted_on_live_db_is_not_ok() {
        assert!(!crate::group_commit_kernel::media_durable_admitted(true));
        assert!(
            crate::group_commit_kernel::media_durable_admitted_as_is(true),
            "AS-IS dente: fsync Ok is rounded to a media theorem"
        );
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                exclusive: true,
                ..OpenOptions::default()
            },
        )
        .unwrap();
        db.put(b"media/k", b"media/v").unwrap();
        assert_eq!(db.get(b"media/k").as_deref(), Some(&b"media/v"[..]));
        assert!(
            !db.claim_media_durable(),
            "live Db after fdatasync Ok must refuse a media-proof claim"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Catalog three-teeth plant. Direct `zero_glue_is_a_trajectory` is
    /// **not** this tooth.
    #[test]
    fn zero_glue_admitted_on_live_db_is_not_ok() {
        assert!(!crate::sst::zero_glue_admitted());
        assert!(
            crate::sst::zero_glue_admitted_as_is(),
            "AS-IS dente: extracting sst_crc_fate looks like glue is gone"
        );
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                exclusive: true,
                ..OpenOptions::default()
            },
        )
        .unwrap();
        db.put(b"glue/k", b"glue/v").unwrap();
        assert_eq!(db.get(b"glue/k").as_deref(), Some(&b"glue/v"[..]));
        assert!(
            !db.claim_zero_glue(),
            "live Db after put must refuse a zero-glue claim"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0050 P0.1 / P0.7: product surface is FailClosed | PointInTime.
    /// Adding skip-any (or any third variant) fails this exhaustive match.
    #[test]
    fn wal_recovery_exactly_two_modes() {
        fn classify(m: WalRecovery) -> u8 {
            match m {
                WalRecovery::FailClosed => 0,
                WalRecovery::PointInTime => 1,
            }
        }
        assert_eq!(classify(WalRecovery::FailClosed), 0);
        assert_eq!(classify(WalRecovery::PointInTime), 1);
        assert_eq!(WalRecovery::default(), WalRecovery::FailClosed);
        assert_ne!(WalRecovery::FailClosed, WalRecovery::PointInTime);
    }

    /// RFC-0062 P0.4: 16-insert batch on empty caches skips dirty-key clones
    /// but the keys must still be readable (memtable, not a stale miss).
    #[test]
    fn sixteen_insert_batch_readable_without_dirty_clones() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        let ops: Vec<BatchOp> = (0..16u32)
            .map(|i| BatchOp::Put {
                key: Bytes::from(format!("raftlog/{i:08}")),
                value: Bytes::from_static(b"xxxxxxxxxxxxxxxx"),
            })
            .collect();
        db.apply_batch(ops).unwrap();
        for i in 0..16u32 {
            let k = format!("raftlog/{i:08}");
            assert_eq!(
                db.get(k.as_bytes()).as_deref(),
                Some(b"xxxxxxxxxxxxxxxx".as_ref()),
                "key {k}"
            );
        }
        db.put(b"raftlog/00000007", b"new").unwrap();
        assert_eq!(
            db.get(b"raftlog/00000007").as_deref(),
            Some(b"new".as_ref()),
            "overwrite after 16-batch must not serve the interned value"
        );
        // Warm the point cache, then a 16-batch that overwrites: dirty
        // clones must run (caches non-empty) and the get must not stick.
        let _ = db.get(b"warm");
        db.put(b"warm", b"old").unwrap();
        assert_eq!(db.get(b"warm").as_deref(), Some(b"old".as_ref()));
        let mut ops: Vec<BatchOp> = (0..15u32)
            .map(|i| BatchOp::Put {
                key: Bytes::from(format!("n/{i:02}")),
                value: Bytes::from_static(b"x"),
            })
            .collect();
        ops.push(BatchOp::Put {
            key: Bytes::from_static(b"warm"),
            value: Bytes::from_static(b"new"),
        });
        db.apply_batch(ops).unwrap();
        assert_eq!(
            db.get(b"warm").as_deref(),
            Some(b"new".as_ref()),
            "16-batch overwrite of a cached key must invalidate"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0045 P2.1: staged-but-unapplied WAL ops are visible to OCC and
    /// invisible to default `get` (publish has not happened).
    #[test]
    fn unapplied_ops_are_visible_to_occ_not_get() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"keep", b"1").unwrap();
        let snap = db.visible_sequence();
        assert!(!db.key_has_write_after(b"k", snap));
        db.unapplied.push(UnappliedOp {
            seq: snap + 1,
            kind: ValueType::Value,
            key: Bytes::from_static(b"k"),
            end: Bytes::new(),
        });
        db.unapplied.push(UnappliedOp {
            seq: snap + 2,
            kind: ValueType::RangeDeletion,
            key: Bytes::from_static(b"r"),
            end: Bytes::from_static(b"t"),
        });
        assert!(db.key_has_write_after(b"k", snap));
        assert!(db.key_has_write_after(b"s", snap), "range tomb covers s");
        assert!(!db.key_has_write_after(b"a", snap));
        assert_eq!(db.get(b"k"), None, "unapplied must not publish");
        assert_eq!(db.get(b"keep").as_deref(), Some(&b"1"[..]));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0036 addendum: `OpenOptions::wal_full_fsync` routes every WAL
    /// barrier to [`crate::env::EnvFile::sync_data_strong`] (Darwin
    /// `F_FULLFSYNC` class — the CMake-RocksDB `sync=true` class); the
    /// default stays `sync_data` (`fdatasync` class, the `librocksdb-sys`
    /// peer class). Counted at the env seam so the contract is proven, not
    /// assumed.
    #[test]
    fn wal_full_fsync_switches_barrier_class() {
        use crate::env::{Env, EnvFile, StdEnv};
        use std::cell::Cell;
        use std::io::{self, Read, Seek, SeekFrom, Write};
        use std::path::Path;
        use std::rc::Rc;

        // RFC-0036 addendum v2: the product default IS the strong class.
        assert!(
            OpenOptions::default().wal_full_fsync,
            "default must be strongest data barrier (F_FULLFSYNC on Darwin)"
        );

        #[derive(Default)]
        struct Counts {
            normal: Cell<u64>,
            strong: Cell<u64>,
        }

        #[derive(Clone)]
        struct CountingEnv {
            inner: StdEnv,
            counts: Rc<Counts>,
        }
        struct CountingFile {
            inner: <StdEnv as Env>::File,
            counts: Rc<Counts>,
            /// Only barriers on the WAL file count (SST/tmp publish syncs are
            /// a different contract).
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
                    self.counts.normal.set(self.counts.normal.get() + 1);
                }
                self.inner.sync_data()
            }
            fn sync_data_strong(&mut self) -> io::Result<()> {
                if self.is_wal {
                    self.counts.strong.set(self.counts.strong.get() + 1);
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

        for full in [false, true] {
            let dir = temp_dir();
            let env = CountingEnv {
                inner: StdEnv,
                counts: Rc::default(),
            };
            let mut opts = OpenOptions::default();
            opts.sync = true;
            opts.wal_full_fsync = full;
            let mut db = Db::open_with_env(&dir, opts, env.clone()).unwrap();
            db.put(b"k", b"v").unwrap();
            db.close().unwrap();
            let (normal, strong) = (env.counts.normal.get(), env.counts.strong.get());
            if full {
                assert!(strong > 0, "flag on: WAL barrier must be strong");
                assert_eq!(normal, 0, "flag on: no weak-class WAL barrier may run");
            } else {
                assert!(normal > 0, "flag off: WAL barrier must be fdatasync-class");
                assert_eq!(strong, 0, "flag off: strong class must stay untouched");
            }
            let _ = fs::remove_dir_all(&dir);
        }
    }

    /// RFC-0062 P1.1: blob-on + G1 must not `fdatasync` `VALUES.vlog` on
    /// small puts (parity bench default min_blob=4096, ycsb payload 100 B).
    /// A spilled value still takes one strong vlog barrier before Ok.
    #[test]
    fn g1_small_put_does_not_fsync_empty_vlog() {
        use crate::env::{Env, EnvFile, StdEnv};
        use std::cell::Cell;
        use std::io::{self, Read, Seek, SeekFrom, Write};
        use std::path::Path;
        use std::rc::Rc;

        #[derive(Default)]
        struct Counts {
            wal_strong: Cell<u64>,
            vlog_strong: Cell<u64>,
            vlog_all: Cell<u64>,
        }
        #[derive(Clone)]
        struct CountingEnv {
            inner: StdEnv,
            counts: Rc<Counts>,
        }
        struct CountingFile {
            inner: <StdEnv as Env>::File,
            counts: Rc<Counts>,
            kind: u8, // 1 = WAL, 2 = vlog
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
                self.inner.sync_data()
            }
            fn sync_data_strong(&mut self) -> io::Result<()> {
                match self.kind {
                    1 => self.counts.wal_strong.set(self.counts.wal_strong.get() + 1),
                    2 => self
                        .counts
                        .vlog_strong
                        .set(self.counts.vlog_strong.get() + 1),
                    _ => {}
                }
                self.inner.sync_data_strong()
            }
            fn sync_all(&mut self) -> io::Result<()> {
                if self.kind == 2 {
                    self.counts.vlog_all.set(self.counts.vlog_all.get() + 1);
                }
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
                    kind: file_kind(path),
                })
            }
            fn open_append(&self, path: &Path) -> io::Result<Self::File> {
                Ok(CountingFile {
                    inner: self.inner.open_append(path)?,
                    counts: Rc::clone(&self.counts),
                    kind: file_kind(path),
                })
            }
            fn open_read(&self, path: &Path) -> io::Result<Self::File> {
                Ok(CountingFile {
                    inner: self.inner.open_read(path)?,
                    counts: Rc::clone(&self.counts),
                    kind: file_kind(path),
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
        fn file_kind(path: &Path) -> u8 {
            match path.file_name().and_then(|n| n.to_str()) {
                Some(WAL_FILE_NAME) => 1,
                Some(VLOG_FILE_NAME) => 2,
                _ => 0,
            }
        }

        let dir = temp_dir();
        let env = CountingEnv {
            inner: StdEnv,
            counts: Rc::default(),
        };
        let mut opts = vlog_opts();
        opts.sync = true;
        opts.large_value_threshold = Some(512);
        let mut db = Db::open_with_env(&dir, opts, env.clone()).unwrap();
        let vlog_at_open = env.counts.vlog_strong.get() + env.counts.vlog_all.get();
        for i in 0..8u8 {
            db.put([b'k', i], vec![b'v'; 32]).unwrap();
        }
        assert_eq!(
            env.counts.wal_strong.get(),
            8,
            "each G1 put pays one WAL barrier"
        );
        assert_eq!(
            env.counts.vlog_strong.get() + env.counts.vlog_all.get(),
            vlog_at_open,
            "small puts must not fsync VALUES.vlog"
        );
        db.put(b"big", vec![b'B'; 1024]).unwrap();
        assert_eq!(
            env.counts.vlog_strong.get(),
            1,
            "spill still barriers the vlog once before Ok"
        );
        db.put(b"k9", vec![b'v'; 32]).unwrap();
        assert_eq!(
            env.counts.vlog_strong.get(),
            1,
            "small put after spill must not re-fsync a durable vlog"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// F188 regression (TX path): values must reach the stored form through
    /// the same escape/spill contract as `apply_batch`. A staged value whose
    /// first byte is the `0x01` inline-escape marker used to be stored raw,
    /// and every later read stripped that byte (length kept) — silent
    /// corruption, one byte past the value (broke all dcs meta round-trips).
    #[test]
    fn tx_value_escape_marker_round_trips() {
        let dir = temp_dir();
        let mut db = Db::open_with(&dir, OpenOptions::default()).unwrap();
        assert!(db.get(b"k").is_none());
        db.put(b"k", b"v").unwrap();
        assert_eq!(db.get(b"k"), Some(bytes::Bytes::from_static(b"v")));
        assert!(db.get(b"t").is_none());
        let mut tx = db.begin();
        tx.put(b"t", b"tv").expect("tx put");
        tx.commit().expect("tx commit");
        assert_eq!(db.get(b"t"), Some(bytes::Bytes::from_static(b"tv")));
        // dcs meta shape: 24 B starting with 0x01 (the escape marker).
        let meta: bytes::Bytes = Bytes::from(vec![
            1u8, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        db.put(b"m", meta.clone()).unwrap();
        assert_eq!(
            db.get(b"m").unwrap(),
            meta,
            "put path mangled the 0x01-leading value"
        );
        assert!(db.get(b"mt").is_none());
        let mut tx = db.begin();
        tx.put(b"mt", meta.clone()).expect("tx put meta");
        tx.commit().expect("tx commit meta");
        assert_eq!(
            db.get(b"mt").expect("tx-path meta read"),
            meta,
            "tx path mangled the 0x01-leading value"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    fn temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pedradb-db-test-{n}-{i}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    /// RFC-0159 P1.3: the host-worker stage threshold takes the max of the
    /// global auto-flush cap and per-CF buffers — a CF buffer above the
    /// global cap must not be cut down to it (bench: 64 MiB global vs
    /// 256 MiB data CF staged 64 MiB chunks), and a smaller CF buffer
    /// never lowers the global stage point.
    #[test]
    fn auto_flush_threshold_takes_max_of_global_and_cf_buffers() {
        let dir = temp_dir();
        let mut db = Db::<StdEnv>::open_with(
            &dir,
            OpenOptions {
                sync: false,
                auto_flush_bytes: Some(1024),
                ..OpenOptions::default()
            },
        )
        .unwrap();
        assert_eq!(db.auto_flush_threshold(), Some(1024));
        db.set_cf_write_buffer("data", 16 * 1024);
        assert_eq!(db.auto_flush_threshold(), Some(16 * 1024));
        db.set_cf_write_buffer("meta", 512);
        assert_eq!(db.auto_flush_threshold(), Some(16 * 1024));
        db.set_cf_write_buffer("data", 0); // removal falls back to the rest
        assert_eq!(db.auto_flush_threshold(), Some(1024));
        // RFC-0159 P1.3 sweep knob: `PEDRA_STAGE_MAX_BYTES` clamps down
        // for chunk-size sweeps (single test — the env is process-global,
        // a separate test would race this one's asserts).
        std::env::set_var("PEDRA_STAGE_MAX_BYTES", "512");
        assert_eq!(db.auto_flush_threshold(), Some(512));
        std::env::set_var("PEDRA_STAGE_MAX_BYTES", "0");
        assert_eq!(db.auto_flush_threshold(), Some(1024));
        std::env::set_var("PEDRA_STAGE_MAX_BYTES", "not-a-number");
        assert_eq!(db.auto_flush_threshold(), Some(1024));
        std::env::set_var("PEDRA_STAGE_MAX_BYTES", "999999999");
        assert_eq!(
            db.auto_flush_threshold(),
            Some(1024),
            "clamp never raises the threshold"
        );
        std::env::remove_var("PEDRA_STAGE_MAX_BYTES");
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0042 v18: a bounded open keeps SST payloads within budget and
    /// still serves identical reads across eviction (flush installs adopt
    /// tables into the pool; eviction pushes blocks to file reads).
    #[test]
    fn bounded_open_serves_reads_after_pool_eviction() {
        let dir = temp_dir();
        let mut opts = OpenOptions {
            sync: false,
            auto_flush_bytes: Some(4 * 1024 * 1024),
            ..OpenOptions::default()
        };
        opts.sst_payload_budget_bytes = Some(1); // evict everything, always
        let mut db = Db::<StdEnv>::open_with_env_bounded(&dir, opts, StdEnv).unwrap();
        for round in 0..3u32 {
            for i in 0..200u32 {
                let k = format!("r{round}-key-{i:04}").into_bytes();
                let v = vec![(i % 199) as u8; 120];
                db.put(&k, &v).unwrap();
            }
            db.flush().unwrap();
        }
        assert!(
            db.ssts.len() >= 3,
            "want several tables, got {}",
            db.ssts.len()
        );
        assert!(
            db.sst_payload_pool().resident_bytes() <= 1,
            "pool must hold (almost) nothing at budget 1"
        );
        for round in 0..3u32 {
            for i in 0..200u32 {
                let k = format!("r{round}-key-{i:04}").into_bytes();
                let want = vec![(i % 199) as u8; 120];
                assert_eq!(db.get(&k).expect("read after eviction"), want);
            }
        }
        // Bounded reopen: recovery itself must stay within budget.
        drop(db);
        let mut opts2 = OpenOptions {
            sync: false,
            ..OpenOptions::default()
        };
        opts2.sst_payload_budget_bytes = Some(1);
        let db2 = Db::<StdEnv>::open_with_env_bounded(&dir, opts2, StdEnv).unwrap();
        assert!(
            db2.sst_payload_pool().resident_bytes() <= 1,
            "recovery must evict during reopen, not after"
        );
        assert_eq!(
            db2.get("r1-key-0150".as_bytes()).expect("reopen read"),
            vec![(150 % 199) as u8; 120]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    fn vlog_opts() -> OpenOptions {
        OpenOptions {
            wal_full_fsync: true,
            history: Default::default(),
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: Some(512),
            wal_recovery: WalRecovery::FailClosed,
            sst_payload_budget_bytes: None,
        }
    }

    fn sync_opts() -> OpenOptions {
        OpenOptions {
            wal_full_fsync: true,
            history: Default::default(),
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
            wal_recovery: WalRecovery::FailClosed,
            sst_payload_budget_bytes: None,
        }
    }

    /// RFC-0047 P0.2: same as [`sync_opts`] but with the Rocks-shaped
    /// drop-in recovery profile (compat default).
    fn pit_opts() -> OpenOptions {
        OpenOptions {
            wal_full_fsync: true,
            wal_recovery: WalRecovery::PointInTime,
            ..sync_opts()
        }
    }

    /// RFC-0083 P0: production put writes a WAL record; lie only on the
    /// stored CRC bytes (payload intact). FailClosed `Db::open` is `Crc`.
    /// AS-IS `crc_match_ok` would replay the intact payload and serve `k`.
    #[test]
    fn crc_mismatch_on_live_wal_open_is_not_ok() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any WAL crc would match"
        );
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"k", b"v").unwrap();
            db.close().unwrap();
        }
        let wal = dir.join(WAL_FILE_NAME);
        let mut bytes = fs::read(&wal).unwrap();
        assert!(
            bytes.len() >= crate::wal::format::HEADER_SIZE,
            "production WAL must hold a record header"
        );
        bytes[0] ^= 0xff;
        fs::write(&wal, &bytes).unwrap();
        let err = match Db::open(&dir) {
            Ok(db) => {
                let served = db.get(b"k");
                let _ = db.close();
                let _ = fs::remove_dir_all(&dir);
                panic!("WAL CRC-field lie must not open; AS-IS would serve k={served:?}");
            }
            Err(e) => e,
        };
        let _ = fs::remove_dir_all(&dir);
        assert!(
            matches!(err, CoreError::Crc { .. }),
            "must fail on crc_match_ok, not a payload parse; got {err}"
        );
    }

    /// RFC-0083 P2.1: WAL-open `crc_match_ok` is not a CRC32C collision theorem.
    #[test]
    fn wal_open_crc_collision_axiom_remains() {
        assert!(!crate::wal::crc::crc_collision_admitted());
        assert!(
            crate::wal::crc::crc_collision_admitted_as_is(),
            "AS-IS dente: matching CRC looks collision-free"
        );
        assert!(
            crate::wal::crc::crc_match_ok(1, 1),
            "equal u32s still match; that is not R-crc"
        );
        let residuals = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/formal/residuals.json");
        let text = std::fs::read_to_string(&residuals).expect("residuals.json");
        assert!(
            text.contains("\"id\": \"R-crc\""),
            "R-crc must stay in the residual catalog"
        );
        assert!(
            text.contains("\"R-crc\""),
            "never_floor must still list R-crc"
        );
    }

    /// RFC-0083 P2.2: PointInTime still reports+serves the WAL prefix (RFC-0047).
    /// Same CRC-field lie as P0, on the *second* record: FailClosed refuses;
    /// PIT opens, serves k00, drops k01, publishes a `crc` report.
    #[test]
    fn wal_crc_field_lie_point_in_time_serves_prefix() {
        use crate::wal::reopen_kernel::{
            reopen_outcome, reopen_outcome_as_is_silent, ReopenDamage, ReopenOutcome,
        };
        assert_eq!(
            reopen_outcome(ReopenDamage::Crc, true, false),
            ReopenOutcome::ServePrefixReport
        );
        assert_eq!(
            reopen_outcome(ReopenDamage::Crc, false, false),
            ReopenOutcome::RefuseOpen
        );
        assert_eq!(
            reopen_outcome_as_is_silent(ReopenDamage::Crc, true, false),
            ReopenOutcome::ServeAll,
            "AS-IS dente: PIT CRC lie would look like a clean open"
        );
        let dir = temp_dir();
        let wal = dir.join(WAL_FILE_NAME);
        {
            let mut db = Db::open_with(&dir, sync_opts()).unwrap();
            db.put(b"k00", b"v0").unwrap();
            db.put(b"k01", b"v1").unwrap();
            db.close().unwrap();
        }
        let mut bytes = fs::read(&wal).unwrap();
        let rec_len =
            |buf: &[u8], h: usize| 7 + u16::from_le_bytes([buf[h + 4], buf[h + 5]]) as usize;
        let h1 = rec_len(&bytes, 0);
        assert!(h1 + crate::wal::format::HEADER_SIZE <= bytes.len());
        bytes[h1] ^= 0xff;
        fs::write(&wal, &bytes).unwrap();
        match Db::open_with(&dir, sync_opts()) {
            Ok(db) => {
                let _ = db.close();
                let _ = fs::remove_dir_all(&dir);
                panic!("FailClosed must still refuse a CRC-field lie");
            }
            Err(CoreError::Crc { .. }) => {}
            Err(e) => {
                let _ = fs::remove_dir_all(&dir);
                panic!("FailClosed must be CoreError::Crc, got {e}");
            }
        }
        let db = Db::open_with(&dir, pit_opts()).expect("PointInTime must open");
        assert_eq!(db.get(b"k00").as_deref(), Some(b"v0".as_ref()));
        assert_eq!(db.get(b"k01"), None, "damaged record is not served");
        let report = db
            .last_recovery_report()
            .expect("PointInTime must report the CRC discard");
        assert_eq!(report.kind, "crc");
        assert!(report.discarded_bytes > 0);
        let _ = db.close();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0077 P1.2: production put+flush writes an SST; a flipped payload
    /// must not `Db::open` / serve the key. AS-IS `sst_crc_fate` would strip.
    #[test]
    fn crc_mismatch_on_live_sst_db_open_is_not_ok() {
        assert_eq!(
            crate::sst::sst_crc_fate(1, 2, 100),
            crate::sst::SstCrcFate::Reject
        );
        assert_eq!(
            crate::sst::sst_crc_fate_as_is(1, 2, 100),
            crate::sst::SstCrcFate::StripTrailer,
            "AS-IS dente: flipped SST would open as a table"
        );
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"k", b"sst-db-open-0077").unwrap();
            db.flush().unwrap();
            db.close().unwrap();
        }
        let sst = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| {
                p.extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s == "sst")
            })
            .expect("flush must write an SST");
        let mut bytes = fs::read(&sst).unwrap();
        assert!(
            bytes.len() >= crate::sst::SST_LEGACY_NO_CRC_MAX,
            "modern SST must not take the tiny-legacy path"
        );
        let pos = bytes.len() / 2;
        assert!(pos + 4 < bytes.len(), "flip must not be the CRC trailer");
        bytes[pos] ^= 0xff;
        fs::write(&sst, &bytes).unwrap();
        let err = match Db::open(&dir) {
            Ok(db) => {
                let served = db.get(b"k");
                let _ = db.close();
                let _ = fs::remove_dir_all(&dir);
                panic!("flipped SST must not open; AS-IS would serve k={served:?}");
            }
            Err(e) => e,
        };
        let msg = err.to_string();
        let _ = fs::remove_dir_all(&dir);
        assert!(
            msg.contains("CRC mismatch"),
            "must fail on SST CRC, not serve the flipped key; got {msg}"
        );
    }

    /// RFC-0081 P1.2: production large put spills to VALUES.vlog; a flipped
    /// payload is Err on `get_at` (F1), never the flipped blob. `get` fail-stops
    /// via the same `resolve_stored_value`. AS-IS `crc_match_ok` would serve it.
    #[test]
    fn crc_mismatch_on_live_db_get_large_value_is_not_ok() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any vlog crc would match"
        );
        let dir = temp_dir();
        let payload = vec![b'L'; 800];
        {
            let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
            db.put(b"k", &payload).unwrap();
            assert_eq!(db.get(b"k").as_deref(), Some(payload.as_slice()));
            db.close().unwrap();
        }
        let path = dir.join(VLOG_FILE_NAME);
        let mut bytes = fs::read(&path).unwrap();
        assert!(
            bytes.len() > 8 + 8,
            "large put must have spilled a vlog record"
        );
        // Magic (8) + len/crc header (8): first payload byte.
        let pos = 16;
        assert!(pos < bytes.len());
        bytes[pos] ^= 0xff;
        fs::write(&path, &bytes).unwrap();
        let db = Db::open_with(&dir, vlog_opts()).unwrap();
        let err = db
            .get_at(db.snapshot(), b"k")
            .expect_err("flipped large value must not be Ok");
        let msg = err.to_string();
        let _ = db.close();
        let _ = fs::remove_dir_all(&dir);
        assert!(
            msg.to_ascii_lowercase().contains("crc"),
            "must fail on crc_match_ok, not serve the flipped blob; got {msg}"
        );
    }

    /// RFC-0038 D: a mid-WAL CRC flip is fail-stop (unchanged), journaled,
    /// and the Nth recorded event escalates — then a repaired/replaced WAL
    /// opens normally (evacuation path stays open).
    #[test]
    fn wal_crc_corruption_journals_then_escalates_then_recovers() {
        let dir = temp_dir();
        let wal = dir.join(WAL_FILE_NAME);
        {
            let mut db = Db::open_with(&dir, sync_opts()).unwrap();
            for i in 0..8 {
                db.put(format!("k{i:02}").as_bytes(), &[7u8; 120]).unwrap();
            }
        }
        assert!(wal.exists());

        // Isolated bitflip in a payload region (header is 7 bytes).
        let mut bytes = fs::read(&wal).unwrap();
        bytes[30] ^= 0xFF;
        fs::write(&wal, &bytes).unwrap();

        for attempt in 1..crate::corrupt::CORRUPTION_ESCALATION_EVENTS {
            let err = match Db::open_with(&dir, sync_opts()) {
                Ok(_) => panic!("attempt {attempt}: corrupted WAL must not open"),
                Err(e) => e,
            };
            assert!(
                matches!(err, CoreError::Crc { .. }),
                "attempt {attempt}: {err:?}"
            );
        }
        assert_eq!(
            fs::read_to_string(dir.join(crate::corrupt::CORRUPTLOG_NAME))
                .unwrap()
                .lines()
                .count() as u32,
            crate::corrupt::CORRUPTION_ESCALATION_EVENTS - 1
        );

        // Nth event escalates with the journal count.
        let err = match Db::open_with(&dir, sync_opts()) {
            Ok(_) => panic!("escalation must refuse open"),
            Err(e) => e,
        };
        match err {
            CoreError::CorruptionEscalated { events, limit } => {
                assert_eq!(events, crate::corrupt::CORRUPTION_ESCALATION_EVENTS);
                assert_eq!(limit, crate::corrupt::CORRUPTION_ESCALATION_EVENTS);
            }
            other => panic!("expected escalation, got {other:?}"),
        }
        let journal = fs::read_to_string(dir.join(crate::corrupt::CORRUPTLOG_NAME)).unwrap();
        assert!(journal.lines().all(|l| l.contains("\tcrc\t")));

        // Escalation never bricks a clean directory: replace the WAL, open fine.
        fs::remove_file(&wal).unwrap();
        let mut db = Db::open_with(&dir, sync_opts()).unwrap();
        db.put(b"after", b"repair").unwrap();
        assert_eq!(db.get(b"after").as_deref(), Some(&b"repair"[..]));
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0038 D: routine torn tails (crash mid-append) never journal —
    /// they auto-recover and must not count toward escalation. The WAL is
    /// truncated to the last good record so later appends never replay the
    /// torn region.
    #[test]
    fn torn_tail_does_not_journal() {
        let dir = temp_dir();
        let wal = dir.join(WAL_FILE_NAME);
        {
            let mut db = Db::open_with(&dir, sync_opts()).unwrap();
            for i in 0..8 {
                db.put(format!("k{i:02}").as_bytes(), &[7u8; 120]).unwrap();
            }
        }
        // Tear the tail mid-record (resyncable).
        let len = fs::metadata(&wal).unwrap().len() as usize;
        let mut bytes = fs::read(&wal).unwrap();
        bytes.truncate(len - 5);
        fs::write(&wal, &bytes).unwrap();

        let torn_len = fs::metadata(&wal).unwrap().len();
        let mut db = Db::open_with(&dir, sync_opts()).unwrap();
        assert!(db.get(b"k00").is_some(), "prefix survives torn tail");
        assert!(!dir.join(crate::corrupt::CORRUPTLOG_NAME).exists());
        // The torn region must be cut: WAL shrinks to the last good record.
        assert!(
            fs::metadata(&wal).unwrap().len() < torn_len,
            "WAL must be truncated to last good offset"
        );
        // Writing after recovery, then re-opening, must stay clean — the
        // damaged tail is gone, not buried under new records. k07 was the
        // torn record: dropped (never distinguishable from an unacked write),
        // so the durable prefix is k00..=k06.
        db.put(b"after", b"recovered").unwrap();
        drop(db);
        let db = Db::open_with(&dir, sync_opts()).unwrap();
        assert_eq!(db.get(b"after").as_deref(), Some(b"recovered".as_ref()));
        assert_eq!(db.get(b"k06").as_deref(), Some(&[7u8; 120][..]));
        assert_eq!(db.get(b"k07"), None);
        assert!(!dir.join(crate::corrupt::CORRUPTLOG_NAME).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0047 P0.2: in PointInTime mode a mid-WAL CRC flip recovers the
    /// decoded prefix, reports the discarded suffix (never silently), cuts
    /// the WAL at the last good record, and still journals the event. The
    /// recovery is terminal: a fail-closed reopen of the same directory is
    /// clean.
    #[test]
    fn point_in_time_recovers_prefix_and_reports() {
        use crate::wal::recover_choose::{apply_recover_choice, RecoverChoice};

        let dir = temp_dir();
        let wal = dir.join(WAL_FILE_NAME);
        {
            let mut db = Db::open_with(&dir, sync_opts()).unwrap();
            for i in 0..8 {
                db.put(format!("k{i:02}").as_bytes(), &[7u8; 120]).unwrap();
            }
        }
        // FlipCrc at physical record 3 (middle of the WAL) via the
        // recover_choose injector — the sweep harness drives the real reader.
        let mut bytes = fs::read(&wal).unwrap();
        assert!(apply_recover_choice(
            &mut bytes,
            RecoverChoice::FlipCrc { index: 3 }
        ));
        fs::write(&wal, &bytes).unwrap();

        let db = Db::open_with(&dir, pit_opts()).unwrap();
        for i in 0..3 {
            assert_eq!(
                db.get(format!("k{i:02}").as_bytes()).as_deref(),
                Some(&[7u8; 120][..])
            );
        }
        for i in 3..8 {
            assert_eq!(
                db.get(format!("k{i:02}").as_bytes()),
                None,
                "k{i:02} is in the discarded suffix"
            );
        }
        let report = db
            .last_recovery_report()
            .expect("PointInTime must report the discard");
        assert_eq!(report.kind, "crc");
        assert!(report.good_through_offset > 0);
        assert_eq!(
            report.corrupt_offset, report.good_through_offset,
            "prefix ends exactly where the corrupt record began"
        );
        assert!(report.discarded_bytes > 0);
        let journal = fs::read_to_string(dir.join(crate::corrupt::CORRUPTLOG_NAME)).unwrap();
        assert_eq!(journal.lines().count(), 1);
        assert!(journal.lines().all(|l| l.contains("\tcrc\t")));
        let good_through = report.good_through_offset;
        drop(db);
        // The corrupt suffix is physically cut: reopen fail-closed, clean.
        let wal_len = fs::metadata(&wal).unwrap().len();
        assert_eq!(wal_len, good_through);
        let db = Db::open_with(&dir, sync_opts()).unwrap();
        assert!(db.last_recovery_report().is_none());
        assert_eq!(db.get(b"k02").as_deref(), Some(&[7u8; 120][..]));
        let _ = fs::remove_dir_all(&dir);
    }

    /// F171: a length bitrot mid-log is resyncable — the walk re-anchors on
    /// later records, so PointInTime serves prefix + re-anchored suffix while
    /// REPORTING the skipped region (kind `resync`), and fail-closed reopens
    /// keep refusing until the damage is repaired (it sits mid-log; the
    /// tail-cut that heals CRC suffixes cannot heal it).
    #[test]
    fn point_in_time_reports_resync_reanchor() {
        let dir = temp_dir();
        let wal = dir.join(WAL_FILE_NAME);
        {
            let mut db = Db::open_with(&dir, sync_opts()).unwrap();
            for i in 0..8 {
                db.put(format!("k{i:02}").as_bytes(), &[7u8; 120]).unwrap();
            }
        }
        // Oversize k02's length (high byte |= 0xff): exceeds max payload →
        // resyncable LengthCorrupt, and the walk re-anchors on k03.
        let mut bytes = fs::read(&wal).unwrap();
        {
            // Full non-fragmented records: walk two headers by their lengths.
            let rec_len =
                |buf: &[u8], h: usize| 7 + u16::from_le_bytes([buf[h + 4], buf[h + 5]]) as usize;
            let h1 = rec_len(&bytes, 0);
            let k02 = h1 + rec_len(&bytes, h1);
            assert!(k02 + 5 < bytes.len());
            bytes[k02 + 5] ^= 0xff;
        }
        fs::write(&wal, &bytes).unwrap();

        let db = Db::open_with(&dir, pit_opts()).unwrap();
        assert_eq!(db.get(b"k00").as_deref(), Some(&[7u8; 120][..]));
        assert_eq!(db.get(b"k01").as_deref(), Some(&[7u8; 120][..]));
        assert_eq!(db.get(b"k02"), None, "damaged record is dropped");
        for i in 3..8 {
            assert_eq!(
                db.get(format!("k{i:02}").as_bytes()).as_deref(),
                Some(&[7u8; 120][..]),
                "k{i:02} re-anchored after the walk"
            );
        }
        let report = db
            .last_recovery_report()
            .expect("PointInTime must report the resync");
        assert_eq!(report.kind, "resync");
        assert!(report.corrupt_offset > 0);
        let journal = fs::read_to_string(dir.join(crate::corrupt::CORRUPTLOG_NAME)).unwrap();
        assert!(journal.lines().all(|l| l.contains("\tresync\t")));
        drop(db);
        // RFC-0048 P1.1: the PointInTime open rewrote the WAL from the
        // recovered records (the damage sat mid-log; a tail-cut could not
        // remove it) — a fail-closed reopen is now clean, keeps the
        // re-anchored suffix and still drops the damaged record.
        let db = Db::open_with(&dir, sync_opts()).unwrap();
        assert!(db.last_recovery_report().is_none());
        assert_eq!(db.get(b"k00").as_deref(), Some(&[7u8; 120][..]));
        assert_eq!(db.get(b"k02"), None);
        assert_eq!(db.get(b"k07").as_deref(), Some(&[7u8; 120][..]));
        let _ = fs::remove_dir_all(&dir);
    }

    /// F170/P1.2: a zero header mid-block at a fresh alignment is corruption
    /// (the writer only pads `< HEADER_SIZE` zero bytes). Fail-closed opens
    /// journal it (RFC-0038 escalation counts it) and refuse; PointInTime
    /// serves the decoded prefix, reports the discard and heals by tail-cut.
    #[test]
    fn zero_header_journals_and_pit_reports() {
        let dir = temp_dir();
        let wal = dir.join(WAL_FILE_NAME);
        {
            let mut db = Db::open_with(&dir, sync_opts()).unwrap();
            for i in 0..8 {
                db.put(format!("k{i:02}").as_bytes(), &[7u8; 120]).unwrap();
            }
        }
        let mut bytes = fs::read(&wal).unwrap();
        {
            let rec_len =
                |buf: &[u8], h: usize| 7 + u16::from_le_bytes([buf[h + 4], buf[h + 5]]) as usize;
            let h1 = rec_len(&bytes, 0);
            let k02 = h1 + rec_len(&bytes, h1);
            // Zero k02's CRC-irrelevant length+type bytes: later records in
            // the same block stay intact (non-zero tail after the header).
            assert!(k02 + 6 < bytes.len());
            bytes[k02 + 4] = 0;
            bytes[k02 + 5] = 0;
            bytes[k02 + 6] = 0;
        }
        fs::write(&wal, &bytes).unwrap();

        // Fail-closed: refuse + journal with the exact corruption kind.
        let err = match Db::open_with(&dir, sync_opts()) {
            Err(e) => e,
            Ok(_) => panic!("fail-closed open must refuse a zero header mid-block"),
        };
        assert!(
            matches!(err, CoreError::WalZeroHeader { .. }),
            "got {err:?}"
        );
        let journal = fs::read_to_string(dir.join(crate::corrupt::CORRUPTLOG_NAME)).unwrap();
        assert!(
            journal.lines().any(|l| l.contains("\tzero_header\t")),
            "journal must record the zero_header event: {journal}"
        );

        // PointInTime: serve the prefix, report the discard, heal by cut.
        let db = Db::open_with(&dir, pit_opts()).unwrap();
        assert_eq!(db.get(b"k00").as_deref(), Some(&[7u8; 120][..]));
        assert_eq!(db.get(b"k01").as_deref(), Some(&[7u8; 120][..]));
        for i in 2..8 {
            assert_eq!(
                db.get(format!("k{i:02}").as_bytes()),
                None,
                "k{i:02} discarded"
            );
        }
        let report = db.last_recovery_report().expect("PointInTime must report");
        assert_eq!(report.kind, "zero_header");
        drop(db);
        let db = Db::open_with(&dir, sync_opts()).unwrap();
        assert!(db.last_recovery_report().is_none());
        assert_eq!(db.get(b"k00").as_deref(), Some(&[7u8; 120][..]));
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0047 P0.2: CORRUPTLOG escalation (RFC-0038 D) refuses the open in
    /// *every* mode — PointInTime never buys its way past the 3rd event. The
    /// WAL self-heals on each PointInTime recovery (suffix cut), so the
    /// corruption must be re-injected per attempt (persistent bitrot).
    #[test]
    fn escalation_refuses_in_every_mode() {
        use crate::wal::recover_choose::{apply_recover_choice, RecoverChoice};

        let dir = temp_dir();
        let wal = dir.join(WAL_FILE_NAME);
        {
            let mut db = Db::open_with(&dir, sync_opts()).unwrap();
            for i in 0..12 {
                db.put(format!("k{i:02}").as_bytes(), &[7u8; 120]).unwrap();
            }
        }
        // Each successful PointInTime open cuts the corrupt suffix, so flip a
        // middle record of whatever survives: 12 -> 8 -> 4 records, then the
        // 3rd journaled event must escalate even in PointInTime mode.
        for (attempt, index) in [8usize, 4].iter().enumerate() {
            let mut bytes = fs::read(&wal).unwrap();
            assert!(apply_recover_choice(
                &mut bytes,
                RecoverChoice::FlipCrc { index: *index }
            ));
            fs::write(&wal, &bytes).unwrap();
            let db = match Db::open_with(&dir, pit_opts()) {
                Ok(db) => db,
                Err(e) => panic!(
                    "attempt {}: PointInTime must recover the prefix, got {e:?}",
                    attempt + 1
                ),
            };
            assert!(db.last_recovery_report().is_some());
            let journal = fs::read_to_string(dir.join(crate::corrupt::CORRUPTLOG_NAME)).unwrap();
            assert_eq!(journal.lines().count() as u32, attempt as u32 + 1);
            drop(db);
        }
        let mut bytes = fs::read(&wal).unwrap();
        assert!(apply_recover_choice(
            &mut bytes,
            RecoverChoice::FlipCrc { index: 1 }
        ));
        fs::write(&wal, &bytes).unwrap();
        let err = match Db::open_with(&dir, pit_opts()) {
            Ok(_) => panic!("escalation must refuse open even in PointInTime mode"),
            Err(e) => e,
        };
        match err {
            CoreError::CorruptionEscalated { events, limit } => {
                assert_eq!(events, crate::corrupt::CORRUPTION_ESCALATION_EVENTS);
                assert_eq!(limit, crate::corrupt::CORRUPTION_ESCALATION_EVENTS);
            }
            other => panic!("expected escalation, got {other:?}"),
        }
        let journal = fs::read_to_string(dir.join(crate::corrupt::CORRUPTLOG_NAME)).unwrap();
        assert!(journal.lines().all(|l| l.contains("\tcrc\t")));
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0047 P0.2: a routine torn tail (crash mid-append) is unchanged by
    /// PointInTime mode — auto-recovered, not journaled, and *not* a
    /// RecoveryReport (a tear is a crash artifact, not a corruption event;
    /// reporting it would be crying wolf).
    #[test]
    fn torn_tail_unchanged_prefix_only() {
        let dir = temp_dir();
        let wal = dir.join(WAL_FILE_NAME);
        {
            let mut db = Db::open_with(&dir, sync_opts()).unwrap();
            for i in 0..8 {
                db.put(format!("k{i:02}").as_bytes(), &[7u8; 120]).unwrap();
            }
        }
        let len = fs::metadata(&wal).unwrap().len() as usize;
        let mut bytes = fs::read(&wal).unwrap();
        bytes.truncate(len - 5);
        fs::write(&wal, &bytes).unwrap();

        let mut db = Db::open_with(&dir, pit_opts()).unwrap();
        for i in 0..7 {
            assert_eq!(
                db.get(format!("k{i:02}").as_bytes()).as_deref(),
                Some(&[7u8; 120][..])
            );
        }
        assert_eq!(db.get(b"k07"), None, "torn record is dropped");
        assert!(
            db.last_recovery_report().is_none(),
            "routine torn tail is not a reported corruption event"
        );
        assert!(!dir.join(crate::corrupt::CORRUPTLOG_NAME).exists());
        db.put(b"after", b"recovered").unwrap();
        drop(db);
        let db = Db::open_with(&dir, sync_opts()).unwrap();
        assert_eq!(db.get(b"after").as_deref(), Some(b"recovered".as_ref()));
        assert!(!dir.join(crate::corrupt::CORRUPTLOG_NAME).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0014 P2.2: large values spill to VALUES.vlog; reopen resolves.
    #[test]
    fn large_value_vlog_put_get_reopen() {
        let dir = temp_dir();
        let big = vec![0xABu8; 4096];
        {
            let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
            db.put(b"small", b"ok").unwrap();
            db.put(b"huge", &big).unwrap();
            // Stored value should be a compact vlog pointer, not 4KiB.
            // Public get returns resolved payload.
            assert_eq!(db.get(b"huge").as_deref(), Some(big.as_slice()));
            assert_eq!(db.get(b"small").as_deref(), Some(b"ok".as_ref()));
            db.flush().unwrap();
            db.close().unwrap();
        }
        assert!(
            dir.join(crate::vlog::VLOG_FILE_NAME).exists(),
            "VALUES.vlog must exist after large put"
        );
        let db = Db::open_with(&dir, vlog_opts()).unwrap();
        assert_eq!(db.get(b"huge").as_deref(), Some(big.as_slice()));
        assert_eq!(db.get(b"small").as_deref(), Some(b"ok".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Checkpoint must copy VALUES.vlog so large keys remain readable.
    #[test]
    fn large_value_survives_checkpoint() {
        let dir = temp_dir();
        let ckpt = temp_dir();
        let big = vec![0xCDu8; 2048];
        {
            let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
            db.put(b"huge", &big).unwrap();
            db.flush().unwrap();
            db.create_checkpoint(&ckpt).unwrap();
            db.close().unwrap();
        }
        assert!(
            ckpt.join(crate::vlog::VLOG_FILE_NAME).exists(),
            "checkpoint must include VALUES.vlog"
        );
        let db = Db::open_with(&ckpt, vlog_opts()).unwrap();
        assert_eq!(
            db.get(b"huge").as_deref(),
            Some(big.as_slice()),
            "checkpoint open must resolve large value"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ckpt);
    }

    /// scan must yield resolved payloads, not raw VLG1 pointers.
    #[test]
    fn large_value_scan_resolves_payload() {
        use std::ops::Bound;
        let dir = temp_dir();
        let big = vec![0xEFu8; 3000];
        let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
        db.put(b"a", &big).unwrap();
        db.put(b"b", b"tiny").unwrap();
        let scanned: Vec<_> = db
            .scan(Bound::Unbounded, Bound::Unbounded)
            .map(|kv| (kv.key, kv.value))
            .collect();
        let a = scanned.iter().find(|(k, _)| k.as_ref() == b"a").unwrap();
        assert_eq!(a.1.as_ref(), big.as_slice());
        assert_eq!(a.1.len(), 3000, "scan must not return 20-byte VLG1 pointer");
        let b = scanned.iter().find(|(k, _)| k.as_ref() == b"b").unwrap();
        assert_eq!(b.1.as_ref(), b"tiny");
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Value-resolved block-cache slots must be reused as-is: resolving is
    /// not idempotent (F188 strips one escape byte), so a resolved slot that
    /// ever flowed back through a resolve would corrupt 0x01-prefixed
    /// values. A key-only scan reads the same block under the raw id and
    /// must not disturb the resolved form.
    #[test]
    fn scan_resolved_blocks_reused_verbatim_across_repeated_scans() {
        use std::ops::Bound;
        let dir = temp_dir();
        // vlog-spilled value whose resolved bytes start with the escape byte.
        let mut big = vec![0xABu8; 3000];
        big[0] = INLINE_ESCAPE;
        let mut small = b"inline".to_vec();
        small.insert(0, INLINE_ESCAPE);
        let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
        db.put(b"k-big", &big).unwrap();
        db.put(b"k-small", &small).unwrap();
        db.put(b"k-plain", b"v").unwrap();
        db.flush().unwrap();
        let snap = db.last_sequence();

        let collect = |db: &Db<StdEnv>| -> Vec<(Bytes, Bytes)> {
            db.scan(Bound::Unbounded, Bound::Unbounded)
                .map(|kv| (kv.key, kv.value))
                .collect()
        };
        let first = collect(&db);
        // Key-only pass over the same blocks (raw slots, same cache).
        assert_eq!(
            db.count_in_range(snap, Bound::Unbounded, Bound::Unbounded, None)
                .unwrap(),
            3
        );
        // Repeat full scans: resolved slots are hits and must be verbatim.
        for round in 0..3 {
            let again = collect(&db);
            assert_eq!(again, first, "scan round {round} diverged");
        }
        let big_got = first.iter().find(|(k, _)| k.as_ref() == b"k-big").unwrap();
        assert_eq!(big_got.1.as_ref(), big.as_slice(), "spilled value verbatim");
        let small_got = first
            .iter()
            .find(|(k, _)| k.as_ref() == b"k-small")
            .unwrap();
        assert_eq!(
            small_got.1.as_ref(),
            small.as_slice(),
            "escape-prefixed inline verbatim"
        );
        assert_eq!(db.get(b"k-big").as_deref(), Some(big.as_slice()));
        assert_eq!(db.get(b"k-small").as_deref(), Some(small.as_slice()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Mid-GC after MANIFEST remap (`vlog_use_new`) but before promote: reopen correct.
    #[test]
    fn compact_vlog_mid_gc_after_manifest_reopen_correct() {
        let dir = temp_dir();
        let big = vec![0xABu8; 2048];
        let big2 = vec![0xCDu8; 2048];
        {
            let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
            db.put(b"keep", &big).unwrap();
            db.put(b"drop", &big2).unwrap();
            db.flush().unwrap();
            db.delete(b"drop").unwrap();
            db.flush().unwrap();
            db.compact_with(CompactOptions::latest_only()).unwrap();
            // Stage through MANIFEST; do not promote (simulates crash).
            let st = db.compact_vlog_stage_manifest().unwrap();
            assert!(st.live_records >= 1);
            assert!(
                dir.join(crate::vlog::VLOG_NEW_NAME).exists(),
                "staged .new must exist after MANIFEST stage"
            );
            assert!(db.vlog_use_new, "MANIFEST flag set before promote");
            assert_eq!(db.get(b"keep").as_deref(), Some(big.as_slice()));
            assert_eq!(db.get(b"drop"), None);
            // Process kill without promote.
            std::mem::forget(db);
        }
        let db = Db::open_with(&dir, vlog_opts()).unwrap();
        assert_eq!(
            db.get(b"keep").as_deref(),
            Some(big.as_slice()),
            "reopen mid-GC (use_new) must resolve remapped large value"
        );
        assert_eq!(db.get(b"drop"), None);
        // Finish promote on recovered handle.
        let mut db = db;
        db.compact_vlog_promote().unwrap();
        assert!(!db.vlog_use_new);
        assert_eq!(db.get(b"keep").as_deref(), Some(big.as_slice()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Mid-GC after writing `.new` but before MANIFEST remap: reopen uses old vlog.
    #[test]
    fn compact_vlog_mid_gc_before_manifest_keeps_old_offsets() {
        use crate::env::StdEnv;
        let dir = temp_dir();
        let big = vec![0x11u8; 1500];
        {
            let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
            db.put(b"k", &big).unwrap();
            db.flush().unwrap();
            assert_eq!(db.get(b"k").as_deref(), Some(big.as_slice()));
            // Only stage the rewritten log file — leave SSTs/MANIFEST on old offsets.
            let live = db.collect_vlog_live_payloads().unwrap();
            let (_st, _remap) =
                ValueLog::<std::fs::File>::rewrite_live_to_new(&StdEnv, &dir, &live).unwrap();
            assert!(dir.join(crate::vlog::VLOG_NEW_NAME).exists());
            assert!(!db.vlog_use_new);
            std::mem::forget(db);
        }
        let db = Db::open_with(&dir, vlog_opts()).unwrap();
        assert_eq!(
            db.get(b"k").as_deref(),
            Some(big.as_slice()),
            "reopen before MANIFEST must use primary vlog + old SST offsets"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0016 P0.1: overwrite/delete large values → `compact_vlog` shrinks file; reopen ok.
    #[test]
    fn compact_vlog_reclaims_after_overwrite_and_delete() {
        let dir = temp_dir();
        let v1 = vec![0x11u8; 2048];
        let v2 = vec![0x22u8; 2048];
        let v3 = vec![0x33u8; 2048];
        {
            let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
            db.put(b"k1", &v1).unwrap();
            db.put(b"k2", &v2).unwrap();
            db.put(b"k3", &v3).unwrap();
            db.flush().unwrap();
            // Overwrite k1, delete k2 — leaves garbage in vlog until SST drops
            // old versions, then compact_vlog reclaims unreferenced payloads.
            db.put(b"k1", &v3).unwrap();
            db.delete(b"k2").unwrap();
            db.flush().unwrap();
            db.compact_with(CompactOptions::latest_only()).unwrap();
            let before = db.stats().vlog_bytes;
            assert!(before > 0);
            let stats = db.compact_vlog().unwrap();
            assert!(
                stats.bytes_after < stats.bytes_before,
                "GC must shrink: before={} after={}",
                stats.bytes_before,
                stats.bytes_after
            );
            assert!(stats.live_records >= 2, "k1 live + k3 live");
            let after = db.stats().vlog_bytes;
            assert!(after < before, "vlog_bytes {after} should be < {before}");
            assert_eq!(db.get(b"k1").as_deref(), Some(v3.as_slice()));
            assert_eq!(db.get(b"k2"), None);
            assert_eq!(db.get(b"k3").as_deref(), Some(v3.as_slice()));
            assert!(db.stats().vlog_gc_count >= 1);
            db.close().unwrap();
        }
        let db = Db::open_with(&dir, vlog_opts()).unwrap();
        assert_eq!(db.get(b"k1").as_deref(), Some(v3.as_slice()));
        assert_eq!(db.get(b"k2"), None);
        assert_eq!(db.get(b"k3").as_deref(), Some(v3.as_slice()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn blob_rotate_reopen() {
        let dir = temp_dir();
        let payload = vec![0xABu8; 2000];
        {
            let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
            db.set_vlog_rotate_bytes(Some(4_096));
            for i in 0..8u8 {
                db.put(&[b'k', i], &payload).unwrap();
            }
            db.flush().unwrap();
            let nums = db.blob_file_nums();
            assert!(
                nums.len() >= 2,
                "expected rotation, blobs={nums:?} line={}",
                db.stats().vlog_line()
            );
            assert!(db.blob_active() >= 1);
            for i in 0..8u8 {
                assert_eq!(db.get(&[b'k', i]).as_deref(), Some(payload.as_slice()));
            }
            db.close().unwrap();
        }
        let db = Db::open_with(&dir, vlog_opts()).unwrap();
        assert!(db.blob_file_nums().len() >= 2);
        for i in 0..8u8 {
            assert_eq!(
                db.get(&[b'k', i]).as_deref(),
                Some(payload.as_slice()),
                "key k{i} after reopen"
            );
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Rotation after some VLG1 spills must keep file-0 reads.
    #[test]
    fn blob_rotate_keeps_legacy_vlg1() {
        let dir = temp_dir();
        let v1 = vec![0xABu8; 2000];
        let v2 = vec![0xCDu8; 2000];
        {
            let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
            db.put(b"old", &v1).unwrap();
            db.set_vlog_rotate_bytes(Some(4_096));
            for i in 0..6u8 {
                db.put(&[b'n', i], &v2).unwrap();
            }
            db.flush().unwrap();
            assert_eq!(db.get(b"old").as_deref(), Some(v1.as_slice()));
            for i in 0..6u8 {
                assert_eq!(db.get(&[b'n', i]).as_deref(), Some(v2.as_slice()));
            }
            db.close().unwrap();
        }
        let db = Db::open_with(&dir, vlog_opts()).unwrap();
        assert_eq!(db.get(b"old").as_deref(), Some(v1.as_slice()));
        assert_eq!(db.get(&[b'n', 0]).as_deref(), Some(v2.as_slice()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_blob_drops_dead_only() {
        let dir = temp_dir();
        let v1 = vec![0x11u8; 1800];
        let v2 = vec![0x22u8; 1800];
        let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
        db.set_vlog_rotate_bytes(Some(3_500));
        db.put(b"a", &v1).unwrap();
        db.put(b"b", &v1).unwrap();
        db.flush().unwrap();
        db.put(b"a", &v2).unwrap();
        db.put(b"c", &v2).unwrap();
        db.flush().unwrap();
        db.compact_with(CompactOptions::latest_only()).unwrap();
        let sealed = db
            .blob_file_nums()
            .into_iter()
            .find(|n| *n != db.blob_active())
            .expect("sealed blob");
        let before = db.stats().vlog_bytes;
        let st = db.compact_blob(sealed).unwrap();
        assert!(st.bytes_after <= st.bytes_before);
        assert!(!vlog::blob_path(&dir, sealed).exists() || st.live_records == 0);
        assert_eq!(db.get(b"a").as_deref(), Some(v2.as_slice()));
        assert_eq!(db.get(b"b").as_deref(), Some(v1.as_slice()));
        assert_eq!(db.get(b"c").as_deref(), Some(v2.as_slice()));
        assert!(db.stats().vlog_bytes <= before);
        db.close().unwrap();
        let db = Db::open_with(&dir, vlog_opts()).unwrap();
        assert_eq!(db.get(b"a").as_deref(), Some(v2.as_slice()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_blob_auto_picks_worst_ratio() {
        let dir = temp_dir();
        let v1 = vec![0xAAu8; 1800];
        let v2 = vec![0xBBu8; 1800];
        let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
        db.set_vlog_rotate_bytes(Some(3_500));
        db.put(b"a", &v1).unwrap();
        db.put(b"b", &v1).unwrap();
        db.flush().unwrap();
        // Overwrite creates dead space in the sealed gen after rotate.
        db.put(b"a", &v2).unwrap();
        db.put(b"c", &v2).unwrap();
        db.flush().unwrap();
        db.compact_with(CompactOptions::latest_only()).unwrap();
        let cands = db.blob_gc_candidates().unwrap();
        assert!(
            cands.iter().any(|c| !c.is_active && c.dead_ratio > 0.0),
            "expected sealed dead space: {cands:?}"
        );
        // θ = 0.0 → any sealed with bytes
        let got = db.compact_blob_auto(0.0).unwrap();
        assert!(got.is_some(), "auto GC should pick a sealed file");
        let (num, st) = got.unwrap();
        assert_ne!(num, db.blob_active());
        assert!(st.bytes_after <= st.bytes_before);
        assert_eq!(db.get(b"a").as_deref(), Some(v2.as_slice()));
        assert_eq!(db.get(b"b").as_deref(), Some(v1.as_slice()));
        // High θ → nothing left dirty enough
        let none = db.compact_blob_auto(0.99).unwrap();
        assert!(none.is_none() || none.as_ref().map(|(n, _)| *n) != Some(num));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn auto_blob_gc_runs_after_latest_only() {
        let dir = temp_dir();
        let v1 = vec![0x11u8; 1800];
        let v2 = vec![0x22u8; 1800];
        let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
        db.set_vlog_rotate_bytes(Some(3_500));
        db.set_auto_blob_gc_min_ratio(Some(0.0));
        assert_eq!(db.auto_blob_gc_min_ratio(), Some(0.0));
        db.put(b"a", &v1).unwrap();
        db.put(b"b", &v1).unwrap();
        db.flush().unwrap();
        db.put(b"a", &v2).unwrap();
        db.put(b"c", &v2).unwrap();
        db.flush().unwrap();
        let sealed_before: Vec<u32> = db
            .blob_file_nums()
            .into_iter()
            .filter(|n| *n != db.blob_active())
            .collect();
        assert!(!sealed_before.is_empty());
        // latest_only drops dead SST pointers → auto GC should rewrite/drop sealed.
        db.compact_with(CompactOptions::latest_only()).unwrap();
        assert_eq!(db.get(b"a").as_deref(), Some(v2.as_slice()));
        assert_eq!(db.get(b"b").as_deref(), Some(v1.as_slice()));
        // At least one sealed gen should have been GC'd (file gone or fewer sealed).
        let sealed_after: Vec<u32> = db
            .blob_file_nums()
            .into_iter()
            .filter(|n| *n != db.blob_active())
            .collect();
        assert!(
            db.stats().vlog_gc_count >= 1 || sealed_after.len() < sealed_before.len(),
            "auto blob GC should run after latest_only: before={sealed_before:?} after={sealed_after:?} gc={}",
            db.stats().vlog_gc_count
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_prefetch_same_visible_kvs() {
        let dir = temp_dir();
        let payload = vec![0xCDu8; 1500];
        let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
        db.set_vlog_rotate_bytes(Some(8_192));
        for i in 0..6u8 {
            db.put(&[b'p', i], &payload).unwrap();
        }
        db.flush().unwrap();
        let scanned: Vec<_> = db
            .scan(Bound::Unbounded, Bound::Unbounded)
            .map(|kv| (kv.key.to_vec(), kv.value.to_vec()))
            .collect();
        assert_eq!(scanned.len(), 6);
        for i in 0..6u8 {
            let got = db.get(&[b'p', i]).unwrap();
            let from_scan = scanned
                .iter()
                .find(|(k, _)| k.as_slice() == [b'p', i])
                .map(|(_, v)| v.as_slice());
            assert_eq!(from_scan, Some(got.as_ref()));
        }
        assert!(
            db.stats().scan_prefetch_hits > 0,
            "prefetch should fire on vlog scan: {}",
            db.stats().vlog_line()
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0029 P2.2: measure scan wall for prefetch N ∈ {1,2,4,8,16} (not magic 32).
    /// Default N=4 remains correct; this records relative costs for the RFC.
    #[test]
    fn scan_prefetch_n_window_measure() {
        use std::time::Instant;
        let dir = temp_dir();
        let payload = vec![0xEFu8; 2048];
        let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
        db.set_vlog_rotate_bytes(Some(16_384));
        let n_keys = 48u8;
        for i in 0..n_keys {
            db.put(&[b's', i], &payload).unwrap();
        }
        db.flush().unwrap();
        let mut rows = Vec::new();
        for &n in &[1usize, 2, 4, 8, 16] {
            db.set_scan_prefetch(n);
            assert_eq!(db.scan_prefetch(), n.min(64));
            // Warm once.
            let _ = db
                .scan(Bound::Unbounded, Bound::Unbounded)
                .map(|kv| kv.value.len())
                .sum::<usize>();
            let t0 = Instant::now();
            let mut rounds = 0u32;
            let mut total_vals = 0usize;
            while t0.elapsed().as_millis() < 80 {
                total_vals = db
                    .scan(Bound::Unbounded, Bound::Unbounded)
                    .map(|kv| kv.value.len())
                    .sum();
                rounds = rounds.saturating_add(1);
            }
            let wall_ms = t0.elapsed().as_secs_f64() * 1000.0;
            let ms_per_scan = wall_ms / f64::from(rounds.max(1));
            rows.push((n, rounds, ms_per_scan, total_vals));
            assert_eq!(total_vals, usize::from(n_keys) * payload.len());
        }
        // Prefer the N with lowest ms/scan among measured; default 4 must not be worst by ≫2×.
        let best = rows
            .iter()
            .min_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap();
        let n4 = rows.iter().find(|r| r.0 == 4).unwrap();
        assert!(
            n4.2 <= best.2 * 2.5 + 0.5,
            "default N=4 should stay competitive: rows={rows:?} best={best:?}"
        );
        // Persist measurement for the RFC (best-effort).
        let out =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../findings/rfc0029-prefetch-n");
        let _ = std::fs::create_dir_all(&out);
        let mut body = String::from(
            "{\n  \"bench\": \"scan_prefetch_n_window\",\n  \"keys\": 48,\n  \"value_bytes\": 2048,\n  \"rows\": [\n",
        );
        for (i, (n, rounds, ms, _)) in rows.iter().enumerate() {
            if i > 0 {
                body.push_str(",\n");
            }
            body.push_str(&format!(
                "    {{\"n\":{n},\"rounds\":{rounds},\"ms_per_scan\":{ms:.4}}}"
            ));
        }
        body.push_str(&format!(
            "\n  ],\n  \"best_n\": {},\n  \"default_n\": 4,\n  \"note\": \"single-threaded Env reads; lab laptop\"\n}}\n",
            best.0
        ));
        let _ = std::fs::write(out.join("stdout.json"), body);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_blob_crash_after_new_file_keeps_reads() {
        let dir = temp_dir();
        let payload = vec![0x99u8; 1600];
        let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
        db.set_vlog_rotate_bytes(Some(3_200));
        db.put(b"x", &payload).unwrap();
        db.put(b"y", &payload).unwrap();
        db.flush().unwrap();
        db.put(b"z", &payload).unwrap();
        db.flush().unwrap();
        let sealed = db
            .blob_file_nums()
            .into_iter()
            .find(|n| *n != db.blob_active());
        if let Some(n) = sealed {
            let live = db.collect_vlog_live_for_file(n).unwrap();
            let dest = n.saturating_add(10);
            let _ = ValueLog::<std::fs::File>::rewrite_live_to_blob(
                &crate::env::StdEnv,
                &dir,
                dest,
                &live,
                1,
            )
            .unwrap();
            assert!(vlog::blob_path(&dir, dest).exists());
            assert!(vlog::blob_path(&dir, n).exists());
        }
        assert_eq!(db.get(b"x").as_deref(), Some(payload.as_slice()));
        assert_eq!(db.get(b"y").as_deref(), Some(payload.as_slice()));
        db.close().unwrap();
        let db = Db::open_with(&dir, vlog_opts()).unwrap();
        assert_eq!(db.get(b"x").as_deref(), Some(payload.as_slice()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0016 P0.3: amp / durability metrics move under load.
    #[test]
    fn stats_amp_and_vlog_metrics() {
        let dir = temp_dir();
        let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
        let big = vec![0xAAu8; 1024];
        db.put(b"a", &big).unwrap();
        db.put(b"b", b"small").unwrap();
        let s = db.stats();
        assert!(s.bytes_ingested >= 1024 + 5);
        assert!(s.bytes_written_wal > 0);
        assert!(s.wal_sync_count >= 1);
        assert!(s.vlog_bytes > 0);
        assert!(s.vlog_live_bytes >= 1024);
        assert_eq!(s.vlog_live_records, 1);
        let big2 = vec![0xBBu8; 1024];
        db.put(b"a", &big2).unwrap();
        db.flush().unwrap();
        db.compact_with(CompactOptions::latest_only()).unwrap();
        let s2 = db.stats();
        assert!(
            s2.vlog_bytes > s2.vlog_live_bytes,
            "after latest_only, old vlog record is unreferenced: {}",
            s2.vlog_line()
        );
        assert!(s2.vlog_live_ratio() < 1.0);
        assert!(db.stats().bytes_written_sst > 0);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// F167: an SST whose point keys all precede the scan window still carries a
    /// range tombstone whose span reaches into the window. The whole-file
    /// fast-reject in `entries_in_user_range` used only `smallest/largest` point
    /// keys, so the tombstone was skipped and covered keys scanned as live while
    /// point `get` correctly returned `None`.
    #[test]
    fn scan_applies_range_tombstone_from_earlier_file() {
        let dir = temp_dir();
        {
            let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
            // File 1: point `k-e` at seq 1.
            db.put(b"k-e", b"old").unwrap();
            db.flush().unwrap();
            // File 2: only entry is the range tombstone [k-b, k-f) at seq 2
            // (start key k-b < k-e < end key k-f; the end key lives in the value,
            // so the file's largest point key is k-b).
            db.delete_range(b"k-b", b"k-f").unwrap();
            db.flush().unwrap();

            assert_eq!(db.get(b"k-e"), None, "point path applies the tombstone");

            let scanned: Vec<_> = db
                .range_limited(
                    std::ops::Bound::Included(&b"k-e"[..]),
                    std::ops::Bound::Included(&b"k-g"[..]),
                    None,
                )
                .into_iter()
                .map(|(k, _)| k)
                .collect();
            assert!(
                scanned.is_empty(),
                "scan must apply the earlier-file tombstone, got {scanned:?}"
            );
            db.close().unwrap();
        }
        // Same disagreement after reopen (lazy tables take the same path).
        let db = Db::open_with(&dir, vlog_opts()).unwrap();
        assert_eq!(db.get(b"k-e"), None);
        let scanned: Vec<_> = db
            .range_limited(
                std::ops::Bound::Included(&b"k-e"[..]),
                std::ops::Bound::Included(&b"k-g"[..]),
                None,
            )
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert!(scanned.is_empty(), "post-reopen scan got {scanned:?}");
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Tombstone in mem over SST put must survive flush.
    #[test]
    fn flush_preserves_delete_over_sst_put() {
        let dir = temp_dir();
        let big = vec![0xABu8; 400];
        {
            let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
            db.put(b"k", &big).unwrap();
            db.flush().unwrap();
            assert_eq!(db.get(b"k").as_deref(), Some(big.as_slice()));
            db.delete(b"k").unwrap();
            assert_eq!(db.get(b"k"), None, "mem tombstone hides SST put");
            db.flush().unwrap();
            assert_eq!(db.get(b"k"), None, "after flush tombstone must remain");
            db.close().unwrap();
        }
        let db = Db::open_with(&dir, vlog_opts()).unwrap();
        assert_eq!(db.get(b"k"), None, "reopen after delete+flush");
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0016 P0.4: fixed-seed soak — model vs Db, `silent_wrong` = 0.
    ///
    /// Exercises put/get/delete/flush/compact/`compact_vlog` (and multi-block SST
    /// point lookup after F29). Avoids `CompactOptions::latest_only` which can
    /// drop tombstones while older levels still hold puts.
    #[test]
    fn soak_fixed_seed_silent_wrong_zero() {
        use crate::rng::{Rng, SeedRng};
        use std::collections::HashMap;

        let dir = temp_dir();
        let rng = SeedRng::new(0x00C0_FFEE);
        let mut model: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
        let mut silent_wrong = 0u64;
        {
            let mut db = Db::open_with(
                &dir,
                OpenOptions {
                    wal_full_fsync: true,
                    history: Default::default(),
                    wal_recovery: Default::default(),
                    sync: true,
                    auto_flush_bytes: Some(8 * 1024),
                    auto_compact_sst_count: Some(4),
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: Some(256),
                    sst_payload_budget_bytes: None,
                },
            )
            .unwrap();

            for step in 0..400u64 {
                let op = rng.next_u64() % 100;
                let k = format!("k{:04}", rng.next_u64() % 40);
                let key = k.as_bytes();
                if op < 55 {
                    let sz = 8 + (rng.next_u64() % 512) as usize;
                    let mut val = vec![0u8; sz];
                    for b in &mut val {
                        *b = (rng.next_u64() & 0xff) as u8;
                    }
                    db.put(key, &val).unwrap();
                    model.insert(key.to_vec(), val);
                } else if op < 75 {
                    db.delete(key).unwrap();
                    model.remove(key);
                } else if op < 88 {
                    let got = db.get(key);
                    let expect = model.get(key).map(Vec::as_slice);
                    if got.as_deref() != expect {
                        silent_wrong += 1;
                    }
                } else if op < 94 {
                    db.flush().unwrap();
                } else if op < 97 {
                    let _ = db.compact();
                } else {
                    let _ = db.compact_vlog();
                }
                if step % 50 == 49 {
                    for i in 0..40u64 {
                        let kk = format!("k{i:04}");
                        let got = db.get(kk.as_bytes());
                        let expect = model.get(kk.as_bytes()).map(Vec::as_slice);
                        if got.as_deref() != expect {
                            silent_wrong += 1;
                        }
                    }
                }
            }
            for i in 0..40u64 {
                let kk = format!("k{i:04}");
                let got = db.get(kk.as_bytes());
                let expect = model.get(kk.as_bytes()).map(Vec::as_slice);
                if got.as_deref() != expect {
                    silent_wrong += 1;
                }
            }
            db.close().unwrap();
        }
        {
            let db = Db::open_with(
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
                    large_value_threshold: Some(256),
                    sst_payload_budget_bytes: None,
                },
            )
            .unwrap();
            for i in 0..40u64 {
                let kk = format!("k{i:04}");
                let got = db.get(kk.as_bytes());
                let expect = model.get(kk.as_bytes()).map(Vec::as_slice);
                if got.as_deref() != expect {
                    silent_wrong += 1;
                }
            }
            db.close().unwrap();
        }
        assert_eq!(silent_wrong, 0, "soak must not return silent wrong values");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn put_get_delete() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"k", b"v1").unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(b"v1".as_ref()));
        db.put(b"k", b"v2").unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(b"v2".as_ref()));
        db.delete(b"k").unwrap();
        assert_eq!(db.get(b"k"), None);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reopen_recovers_puts() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"a", b"1").unwrap();
            db.put(b"b", b"2").unwrap();
            db.close().unwrap();
        }
        {
            let db = Db::open(&dir).unwrap();
            assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
            assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
            assert_eq!(db.last_sequence(), 2);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reopen_recovers_delete() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"k", b"v").unwrap();
            db.delete(b"k").unwrap();
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"k"), None);
        assert_eq!(db.last_sequence(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_after_reopen_continues_sequence() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"a", b"1").unwrap();
            db.close().unwrap();
        }
        {
            let mut db = Db::open(&dir).unwrap();
            assert_eq!(db.last_sequence(), 1);
            db.put(b"b", b"2").unwrap();
            assert_eq!(db.last_sequence(), 2);
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn async_mode_still_recovers_after_clean_close() {
        let dir = temp_dir();
        {
            let mut db = Db::open_with(
                &dir,
                OpenOptions {
                    wal_full_fsync: true,
                    history: Default::default(),
                    wal_recovery: Default::default(),
                    sync: false,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                    sst_payload_budget_bytes: None,
                },
            )
            .unwrap();
            db.put(b"x", b"y").unwrap();
            db.sync().unwrap();
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"x").as_deref(), Some(b"y".as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn crash_after_sync_put_reopen_recovers() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"durable", b"yes").unwrap();
            std::mem::forget(db);
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"durable").as_deref(), Some(b"yes".as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0056 P1.1 end-to-end exercise of the theorem-link
    /// (`verus/dictionary_link.rs`): acked put → flush (SST + MANIFEST +
    /// WAL rotate via `flush_kernel`) → acked tail put (mem/WAL only) →
    /// crash → reopen (inventory via `manifest_kernel`, tail via WAL
    /// recover, `reopen_outcome` ServeAll) → both acked keys visible.
    #[test]
    fn crash_after_flush_and_tail_put_recovers_both_paths() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"flushed", b"F").unwrap();
            db.flush().unwrap();
            assert!(db.sst_count() >= 1, "flush wrote the SST");
            db.put(b"tail", b"T").unwrap();
            std::mem::forget(db);
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(
            db.get(b"flushed").as_deref(),
            Some(b"F".as_ref()),
            "acked key on the SST-inventory path"
        );
        assert_eq!(
            db.get(b"tail").as_deref(),
            Some(b"T".as_ref()),
            "acked key on the WAL-replay path"
        );
        assert!(db.sst_count() >= 1, "reopen served the committed inventory");
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn crash_after_multi_key_commit_reopen_recovers_both() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            let mut tx = db.begin();
            tx.put(b"row", b"R").unwrap();
            tx.put(b"idx", b"I").unwrap();
            tx.commit().unwrap();
            std::mem::forget(db);
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"row").as_deref(), Some(b"R".as_ref()));
        assert_eq!(db.get(b"idx").as_deref(), Some(b"I".as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Multi-key [`apply_batch`](Db::apply_batch) Ok → crash → full batch present (all-or-nothing WAL record).
    #[test]
    fn crash_after_apply_batch_reopen_recovers_all_ops() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.apply_batch([
                BatchOp::put(b"a", b"1"),
                BatchOp::put(b"b", b"2"),
                BatchOp::put(b"c", b"3"),
                BatchOp::delete(b"missing"),
            ])
            .unwrap();
            std::mem::forget(db);
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        assert_eq!(db.get(b"c").as_deref(), Some(b"3".as_ref()));
        assert_eq!(db.get(b"missing"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Staged multi-key TX never partially visible after crash (no commit = no WAL).
    #[test]
    fn uncommitted_multi_key_tx_leaves_no_half_after_crash() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"base", b"0").unwrap();
            let mut tx = db.begin();
            tx.put(b"half1", b"1").unwrap();
            tx.put(b"half2", b"2").unwrap();
            tx.put(b"half3", b"3").unwrap();
            // Drop tx without commit; process-kill style forget of db.
            std::mem::forget(tx);
            std::mem::forget(db);
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"base").as_deref(), Some(b"0".as_ref()));
        assert_eq!(db.get(b"half1"), None);
        assert_eq!(db.get(b"half2"), None);
        assert_eq!(db.get(b"half3"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn uncommitted_tx_not_on_disk_after_crash() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"keep", b"1").unwrap();
            let mut tx = db.begin();
            tx.put(b"ghost", b"2").unwrap();
            std::mem::forget(tx);
            std::mem::forget(db);
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"keep").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"ghost"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Denser schedule: put/delete/batch/tx/flush/compact + crash reopen — no silent wrong vs model.
    #[test]
    fn denser_fault_schedule_no_silent_wrong_vs_model() {
        let dir = temp_dir();
        let mut model: std::collections::BTreeMap<Vec<u8>, Vec<u8>> =
            std::collections::BTreeMap::new();
        {
            let mut db = Db::open_with(
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
            )
            .unwrap();
            for i in 0..40u32 {
                let k = format!("k{i:03}").into_bytes();
                let v = format!("v{i}").into_bytes();
                db.put(&k, &v).unwrap();
                model.insert(k, v);
            }
            // Deletes + overwrite.
            for i in 0..10u32 {
                let k = format!("k{i:03}").into_bytes();
                db.delete(&k).unwrap();
                model.remove(&k);
            }
            for i in 10..20u32 {
                let k = format!("k{i:03}").into_bytes();
                let v = b"ov".to_vec();
                db.put(&k, &v).unwrap();
                model.insert(k, v);
            }
            // Multi-key TX.
            {
                let mut tx = db.begin();
                tx.put(b"tx-a", b"A").unwrap();
                tx.put(b"tx-b", b"B").unwrap();
                tx.commit().unwrap();
            }
            model.insert(b"tx-a".to_vec(), b"A".to_vec());
            model.insert(b"tx-b".to_vec(), b"B".to_vec());
            // Batch.
            db.apply_batch([BatchOp::put(b"batch1", b"1"), BatchOp::put(b"batch2", b"2")])
                .unwrap();
            model.insert(b"batch1".to_vec(), b"1".to_vec());
            model.insert(b"batch2".to_vec(), b"2".to_vec());
            db.flush().unwrap();
            db.compact().unwrap();
            // Process kill after durable work.
            std::mem::forget(db);
        }
        let db = Db::open(&dir).unwrap();
        for (k, v) in &model {
            assert_eq!(
                db.get(k).as_deref(),
                Some(v.as_slice()),
                "silent wrong/missing for key {}",
                String::from_utf8_lossy(k)
            );
        }
        // No invented keys outside model (spot-check deleted).
        assert_eq!(db.get(b"k000"), None);
        assert_eq!(db.get(b"k009"), None);
        // Corruption fail-closed: flip SST and reopen or verify.
        db.close().unwrap();
        if let Some(sst) = fs::read_dir(&dir)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "sst"))
        {
            let mut bytes = fs::read(&sst).unwrap();
            if bytes.len() > 32 {
                let mid = bytes.len() / 2;
                bytes[mid] ^= 0xff;
                fs::write(&sst, &bytes).unwrap();
                match Db::open(&dir) {
                    Err(_) => { /* fail-closed on open */ }
                    Ok(corrupted) => {
                        assert!(
                            corrupted.verify_checksums().is_err(),
                            "verify must fail closed on SST bitflip"
                        );
                        corrupted.close().unwrap();
                    }
                }
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn flush_then_get_from_sst() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"a", b"1").unwrap();
        db.put(b"b", b"2").unwrap();
        assert_eq!(db.sst_count(), 0);
        db.flush().unwrap();
        assert_eq!(db.sst_count(), 1);
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn flush_reopen_loads_sst_without_wal_data() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"a", b"1").unwrap();
            db.put(b"b", b"2").unwrap();
            db.flush().unwrap();
            db.close().unwrap();
        }
        // WAL should be empty; data only in SST
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.sst_count(), 1);
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        assert_eq!(db.last_sequence(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn flush_then_overwrite_and_reopen() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"k", b"old").unwrap();
            db.flush().unwrap();
            db.put(b"k", b"new").unwrap();
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(b"new".as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn flush_then_delete_hides_sst_value() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"k", b"v").unwrap();
        db.flush().unwrap();
        db.delete(b"k").unwrap();
        assert_eq!(db.get(b"k"), None);
        db.close().unwrap();
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"k"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_flushes_reopen() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"a", b"1").unwrap();
            db.flush().unwrap();
            db.put(b"b", b"2").unwrap();
            db.flush().unwrap();
            assert_eq!(db.sst_count(), 2);
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.sst_count(), 2);
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tx_reads_from_sst_after_flush() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"row", b"R").unwrap();
        db.flush().unwrap();
        {
            let mut tx = db.begin();
            assert_eq!(tx.get(b"row").unwrap().as_deref(), Some(b"R".as_ref()));
            tx.put(b"idx", b"I").unwrap();
            tx.commit().unwrap();
        }
        assert_eq!(db.get(b"idx").as_deref(), Some(b"I".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn range_after_flush_ordered_mvcc() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"a", b"1").unwrap();
        db.put(b"b", b"2").unwrap();
        db.put(b"c", b"3").unwrap();
        db.put(b"d", b"4").unwrap();
        db.flush().unwrap();
        db.put(b"b", b"2b").unwrap(); // newer in mem
        db.delete(b"c").unwrap();

        let mid = db.range_limited(Bound::Included(b"b"), Bound::Excluded(b"d"), None);
        assert_eq!(mid.len(), 1);
        assert_eq!(mid[0].0.as_ref(), b"b");
        assert_eq!(mid[0].1.as_ref(), b"2b");

        let all: Vec<_> = db
            .range_limited(Bound::Unbounded, Bound::Unbounded, None)
            .into_iter()
            .map(|(k, _)| k.to_vec())
            .collect();
        assert_eq!(all, vec![b"a".to_vec(), b"b".to_vec(), b"d".to_vec()]);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_merges_ssts_preserves_live_keys() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"a", b"1").unwrap();
            db.flush().unwrap();
            db.put(b"b", b"2").unwrap();
            db.flush().unwrap();
            db.put(b"a", b"1b").unwrap();
            db.flush().unwrap();
            assert!(db.sst_count() >= 2);
            let before: Vec<_> = db.range_limited(Bound::Unbounded, Bound::Unbounded, None);
            db.compact().unwrap();
            assert_eq!(db.sst_count(), 1);
            let after: Vec<_> = db.range_limited(Bound::Unbounded, Bound::Unbounded, None);
            assert_eq!(before, after);
            assert_eq!(db.get(b"a").as_deref(), Some(b"1b".as_ref()));
            assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.sst_count(), 1);
        assert_eq!(db.get(b"a").as_deref(), Some(b"1b".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        let keys: Vec<_> = db
            .range_limited(Bound::Unbounded, Bound::Unbounded, None)
            .into_iter()
            .map(|(k, _)| k.to_vec())
            .collect();
        assert_eq!(keys, vec![b"a".to_vec(), b"b".to_vec()]);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Auto-compact at L0 trigger promotes L0 only — the existing L1 file
    /// is not rewritten (RFC-0036 apply/raftlog tail).
    #[test]
    fn auto_compact_l0_leaves_existing_l1() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.bulk_route_enabled = false; // ladder mechanics; bulk would install at MAX
        for i in 0..L0_COMPACTION_TRIGGER {
            db.put([b'a', i as u8], [b'1', i as u8]).unwrap();
            db.flush().unwrap();
        }
        assert_eq!(db.level_file_count(0), 0, "L0 should have been promoted");
        assert!(db.level_file_count(1) >= 1);
        let first_l1 = db
            .ssts
            .iter()
            .zip(db.sst_levels.iter())
            .find(|(_, &lvl)| lvl == 1)
            .map(|(t, _)| t.path().to_path_buf())
            .expect("L1 file");
        for i in 0..L0_COMPACTION_TRIGGER {
            db.put([b'b', i as u8], [b'2', i as u8]).unwrap();
            db.flush().unwrap();
        }
        assert_eq!(db.level_file_count(0), 0);
        assert_eq!(db.level_file_count(1), 2, "old L1 plus one new L1");
        assert!(
            db.ssts.iter().any(|t| t.path() == first_l1.as_path()),
            "first L1 must survive the second L0 compact"
        );
        assert_eq!(db.get(&[b'a', 0]).as_deref(), Some([b'1', 0].as_slice()));
        assert_eq!(
            db.get(&[b'b', (L0_COMPACTION_TRIGGER - 1) as u8])
                .as_deref(),
            Some([b'2', (L0_COMPACTION_TRIGGER - 1) as u8].as_slice())
        );
        db.close().unwrap();
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(&[b'a', 0]).as_deref(), Some([b'1', 0].as_slice()));
        assert_eq!(db.get(&[b'b', 0]).as_deref(), Some([b'2', 0].as_slice()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Default L0 compact (no GC) keeps every version, including tombstones.
    /// Streaming k-way must match `gc_compact_entries` on the concatenated inputs.
    #[test]
    fn compact_l0_streaming_matches_concat_and_keeps_tombstones() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"a", b"a1").unwrap();
        db.put(b"b", b"b1").unwrap();
        db.put(b"c", b"c1").unwrap();
        db.flush().unwrap();
        db.put(b"b", b"b2").unwrap();
        db.delete(b"a").unwrap();
        db.put(b"d", b"d1").unwrap();
        db.flush().unwrap();
        assert_eq!(db.level_file_count(0), 2);
        assert!(
            db.ssts
                .iter()
                .zip(db.sst_levels.iter())
                .filter(|(_, &lvl)| lvl == 0)
                .all(|(t, _)| t.is_lazy() && !t.materialize_cache_filled()),
            "L0 inputs must stay unmaterialized before compact"
        );

        let mut concat = Vec::new();
        for (t, &lvl) in db.ssts.iter().zip(db.sst_levels.iter()) {
            if lvl == 0 {
                concat.extend(t.entries_cloned().unwrap());
            }
        }
        let expected =
            crate::merge::gc_compact_entries(concat, crate::merge::CompactGcOptions::default());

        db.compact_l0_into_l1(CompactOptions::default()).unwrap();
        assert_eq!(db.level_file_count(0), 0);
        assert_eq!(db.level_file_count(1), 1);
        let l1 = db
            .ssts
            .iter()
            .zip(db.sst_levels.iter())
            .find(|(_, &lvl)| lvl == 1)
            .map(|(t, _)| t)
            .expect("L1");
        assert_eq!(l1.entries_cloned().unwrap(), expected);
        assert_eq!(db.get(b"a"), None);
        assert_eq!(db.get(b"b").as_deref(), Some(b"b2".as_slice()));
        assert_eq!(db.get(b"c").as_deref(), Some(b"c1".as_slice()));
        assert_eq!(db.get(b"d").as_deref(), Some(b"d1".as_slice()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// GC compaction must stream: no input table's decoded-entries cache may
    /// be filled (2026-08-31 25M OOM: the batch GC path materialized every
    /// input via `entries_cloned`, holding 1.15M cached entries live
    /// mid-hydrate). Clones share the cache Arc, so the assert sees exactly
    /// what the compaction touched.
    #[test]
    fn compact_l0_with_gc_leaves_inputs_unmaterialized() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        for round in 0..2u64 {
            db.put(&round.to_be_bytes(), b"v").unwrap();
            db.put(b"a", format!("a{round}").as_bytes()).unwrap();
            db.delete(b"b").unwrap();
            db.put(b"b", b"gone").unwrap();
            db.flush().unwrap();
        }
        assert_eq!(db.level_file_count(0), 2);
        let inputs: Vec<SstTable> = db
            .ssts
            .iter()
            .zip(db.sst_levels.iter())
            .filter(|(_, &lvl)| lvl == 0)
            .map(|(t, _)| t.clone())
            .collect();
        assert!(
            inputs.iter().all(|t| !t.materialize_cache_filled()),
            "L0 inputs must start unmaterialized"
        );

        let gc = crate::merge::CompactGcOptions::for_oldest_snapshot(6);
        db.compact_l0_into_l1(CompactOptions {
            gc,
            ..CompactOptions::default()
        })
        .unwrap();

        assert_eq!(db.level_file_count(0), 0);
        assert!(
            inputs.iter().all(|t| !t.materialize_cache_filled()),
            "GC compact must not materialize its input tables"
        );
        assert_eq!(db.get(b"a").as_deref(), Some(b"a1".as_slice()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"gone".as_slice()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0065 P0.1: mixed memtable flush emits one SST per CF.
    #[test]
    fn flush_splits_sst_per_cf_family() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.set_physical_cfs(vec!["default".into(), "lock".into(), "write".into()]);
        db.set_defer_auto_compact(true);
        db.put(b"lock\0k", b"L").unwrap();
        db.put(b"default\0k", b"D").unwrap();
        db.put(b"write\0k", b"W").unwrap();
        db.flush().unwrap();
        let meta = db.live_sst_meta();
        let cfs: Vec<_> = {
            let mut v: Vec<_> = meta.iter().map(|m| m.cf.as_str()).collect();
            v.sort_unstable();
            v
        };
        assert_eq!(cfs, vec!["default", "lock", "write"], "meta={meta:?}");
        db.close().unwrap();
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"lock\0k").as_deref(), Some(b"L".as_ref()));
        assert_eq!(db.get(b"default\0k").as_deref(), Some(b"D".as_ref()));
        assert_eq!(db.get(b"write\0k").as_deref(), Some(b"W".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0065 P0.1: prefix-era mixed SST still opens; get of both families works.
    #[test]
    fn prefix_era_mixed_sst_opens() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.set_defer_auto_compact(true);
        db.put(b"lock\0k", b"L").unwrap();
        db.put(b"default\0k", b"D").unwrap();
        let imm = db.prepare_flush_imm().unwrap().expect("imm");
        let num = db.alloc_file_num();
        let (env, path, sync) = db.l0_write_ctx();
        let (table, n, _) = Db::write_imm_l0_file(&env, &path, sync, &imm, num).unwrap();
        assert!(
            table.cf().is_empty(),
            "mixed bounds must tag empty CF, got {:?}",
            table.cf()
        );
        db.install_l0_sst(table, n).unwrap();
        db.persist_manifest_durable().unwrap();
        db.close().unwrap();
        let mut db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"lock\0k").as_deref(), Some(b"L".as_ref()));
        assert_eq!(db.get(b"default\0k").as_deref(), Some(b"D".as_ref()));
        let before: Vec<_> = db
            .live_sst_meta()
            .into_iter()
            .map(|m| (m.name, m.size, m.cf))
            .collect();
        db.compact_ssts_only_cf("lock").unwrap();
        let after: Vec<_> = db
            .live_sst_meta()
            .into_iter()
            .map(|m| (m.name, m.size, m.cf))
            .collect();
        assert_eq!(
            before, after,
            "mixed SST must stay; compact lock is a no-op"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0065 P0.2: compact of lock does not rewrite default SSTs.
    #[test]
    fn compact_cf_leaves_other_family_ssts() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.set_physical_cfs(vec!["default".into(), "lock".into()]);
        db.set_defer_auto_compact(true);
        db.put(b"lock\0a", b"1").unwrap();
        db.put(b"default\0a", b"1").unwrap();
        db.flush().unwrap();
        db.put(b"lock\0b", b"2").unwrap();
        db.put(b"default\0b", b"2").unwrap();
        db.flush().unwrap();
        let default_before: Vec<_> = db
            .live_sst_meta()
            .into_iter()
            .filter(|m| m.cf == "default")
            .map(|m| (m.name, m.size, m.level, m.num_entries))
            .collect();
        assert!(
            default_before.len() >= 2,
            "need ≥2 default SSTs, got {default_before:?}"
        );
        db.compact_ssts_only_cf("lock").unwrap();
        let default_after: Vec<_> = db
            .live_sst_meta()
            .into_iter()
            .filter(|m| m.cf == "default")
            .map(|m| (m.name, m.size, m.level, m.num_entries))
            .collect();
        assert_eq!(default_before, default_after);
        let lock_after: Vec<_> = db
            .live_sst_meta()
            .into_iter()
            .filter(|m| m.cf == "lock")
            .collect();
        assert_eq!(lock_after.len(), 1, "lock compacted to one file");
        assert_eq!(db.get(b"lock\0a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"default\0a").as_deref(), Some(b"1".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0065 P1.1: default auto-flush does not dump lock keys to SST.
    #[test]
    fn auto_flush_default_does_not_flush_lock() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                auto_flush_bytes: Some(64 * 1024),
                ..OpenOptions::default()
            },
        )
        .unwrap();
        db.set_physical_cfs(vec!["default".into(), "lock".into()]);
        db.set_cf_write_buffer("default", 400);
        db.set_cf_write_buffer("lock", 64 * 1024);
        db.put(b"lock\0k", b"L").unwrap();
        for i in 0..20u8 {
            let k = [b'd', b'e', b'f', b'a', b'u', b'l', b't', 0, b'k', i];
            db.put(&k, &[b'x'; 32]).unwrap();
        }
        let lock_sst = db
            .live_sst_meta()
            .into_iter()
            .filter(|m| m.cf == "lock")
            .count();
        let default_sst = db
            .live_sst_meta()
            .into_iter()
            .filter(|m| m.cf == "default")
            .count();
        assert_eq!(lock_sst, 0, "lock must stay in mem");
        assert!(default_sst >= 1, "default should have auto-flushed");
        assert_eq!(db.get(b"lock\0k").as_deref(), Some(b"L".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0149: auto-flush with physical CFs must not scan every memtable
    /// key per put. 20k puts under a 64 MiB cap stay in mem and the per-put
    /// cost must not grow with the number of resident keys. The check is a
    /// early-window/late-window ratio, not a wall-clock bound: suite load
    /// slows both windows equally, but the "cf_families-per-put" regression
    /// (rescanning every key on every put) makes the late window ~10x
    /// slower per put than the early one.
    #[test]
    fn maybe_auto_flush_physical_cf_is_not_linear_in_keys() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                auto_flush_bytes: Some(64 * 1024 * 1024),
                sync: false,
                ..OpenOptions::default()
            },
        )
        .unwrap();
        db.set_physical_cfs(vec!["default".into(), "lock".into()]);
        fn put_i(db: &mut Db, i: u32) {
            let mut k = [0u8; 10];
            k[..7].copy_from_slice(b"default");
            k[7] = 0;
            k[8] = (i >> 8) as u8;
            k[9] = i as u8;
            db.put(&k, b"v").unwrap();
        }
        // Early window: memtable holds 0..2k keys.
        let t0 = std::time::Instant::now();
        for i in 0..2_000u32 {
            put_i(&mut db, i);
        }
        let early = t0.elapsed();
        for i in 2_000..18_000u32 {
            put_i(&mut db, i);
        }
        // Late window: memtable holds ~18k keys.
        let t1 = std::time::Instant::now();
        for i in 18_000..20_000u32 {
            put_i(&mut db, i);
        }
        let late = t1.elapsed();
        let growth = late.as_secs_f64() / early.as_secs_f64();
        assert!(
            growth < 6.0,
            "per-put cost grows with memtable size (cf_families-per-put is back): \
             late {late:?} vs early {early:?} over 2000 puts each"
        );
        assert!(
            db.live_sst_meta().is_empty(),
            "64 MiB cap must not flush 20k tiny keys"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0149 P2.1: stall-off (parity default) still writes physical-CF
    /// keys; stall-on still refuses using the CF of the batch, not a String
    /// copy per put.
    #[test]
    fn physical_cf_idle_admission_still_stalls_when_armed() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                auto_flush_bytes: None,
                sync: false,
                ..OpenOptions::default()
            },
        )
        .unwrap();
        db.set_physical_cfs(vec!["default".into(), "lock".into()]);
        let mut k = [0u8; 9];
        k[..7].copy_from_slice(b"default");
        k[7] = 0;
        k[8] = b'a';
        db.put(&k, b"v").unwrap();
        db.set_write_stall_mem_bytes(Some(8));
        let err = db.put(&k, &vec![b'x'; 64]).unwrap_err();
        assert!(
            matches!(err, CoreError::WriteStallMem { .. }),
            "expected WriteStallMem, got {err:?}"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0065 P1.3: flush_cf(lock) does not create a default SST; WAL recovers both.
    #[test]
    fn flush_cf_lock_does_not_create_default_sst() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.set_physical_cfs(vec!["default".into(), "lock".into()]);
            db.set_defer_auto_compact(true);
            db.put(b"lock\0k", b"L").unwrap();
            db.put(b"default\0k", b"D").unwrap();
            let default_before = db
                .live_sst_meta()
                .into_iter()
                .filter(|m| m.cf == "default")
                .count();
            db.flush_cf("lock").unwrap();
            let default_after = db
                .live_sst_meta()
                .into_iter()
                .filter(|m| m.cf == "default")
                .count();
            assert_eq!(default_before, default_after);
            assert!(
                db.live_sst_meta().iter().any(|m| m.cf == "lock"),
                "lock SST missing"
            );
            std::mem::forget(db);
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"lock\0k").as_deref(), Some(b"L".as_ref()));
        assert_eq!(db.get(b"default\0k").as_deref(), Some(b"D".as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0065 P1.2: default L0 at the stall limit does not block lock puts.
    #[test]
    fn default_l0_stall_does_not_block_lock() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.set_physical_cfs(vec!["default".into(), "lock".into()]);
        db.set_defer_auto_compact(true);
        db.set_write_stall_l0(Some(2));
        db.put(b"default\0a", b"1").unwrap();
        db.flush().unwrap();
        db.put(b"default\0b", b"2").unwrap();
        db.flush().unwrap();
        assert!(db.level_file_count_cf("default") >= 2);
        let stalled = db.put(b"default\0c", b"3");
        assert!(
            matches!(stalled, Err(CoreError::WriteStall { .. })),
            "default put should stall, got {stalled:?}"
        );
        db.put(b"lock\0k", b"L").unwrap();
        assert_eq!(db.get(b"lock\0k").as_deref(), Some(b"L".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0065 P0.3: one WriteBatch / one WAL covers lock+default+write;
    /// crash after Ok recovers all three (not a 3-DB / 3-WAL design).
    #[test]
    fn multi_cf_batch_crash_recovers_all_or_nothing() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.apply_batch([
                BatchOp::put(b"lock\0k", b"L"),
                BatchOp::put(b"default\0k", b"D"),
                BatchOp::put(b"write\0k", b"W"),
            ])
            .unwrap();
            std::mem::forget(db);
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"lock\0k").as_deref(), Some(b"L".as_ref()));
        assert_eq!(db.get(b"default\0k").as_deref(), Some(b"D".as_ref()));
        assert_eq!(db.get(b"write\0k").as_deref(), Some(b"W".as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    /// `CompactOptions::max_input_files` bounds one prepared job to the
    /// oldest N L0 files; the rest stay live at L0 and shadow the merged
    /// output exactly as they shadowed the inputs (host worker merges in
    /// bounded slices instead of every L0 at once).
    #[test]
    fn prepare_l0_compact_respects_max_input_files() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.set_defer_auto_compact(true);
        for round in 0..5u8 {
            for i in 0..4u8 {
                db.put([b'k', i], [b'v', round, i]).unwrap();
            }
            db.flush().unwrap();
        }
        assert_eq!(db.level_file_count(0), 5);
        let job = db
            .prepare_l0_compact(CompactOptions {
                max_input_files: Some(2),
                ..CompactOptions::default()
            })
            .unwrap()
            .expect("L0 job");
        let tables = job.write().unwrap();
        db.install_prepared_l0_compact(job, tables).unwrap();
        assert_eq!(db.level_file_count(0), 3, "only two oldest L0s merged");
        assert!(db.level_file_count(1) >= 1);
        // Newest version wins across the slice boundary.
        assert_eq!(db.get(&[b'k', 0]).as_deref(), Some(&[b'v', 4, 0][..]));
        db.close().unwrap();
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(&[b'k', 0]).as_deref(), Some(&[b'v', 4, 0][..]));
        let _ = fs::remove_dir_all(&dir);
    }

    /// v21c regression: the off-lock L0 compact writes one file per split
    /// chunk starting at the number `prepare` reserved; `prepare` must
    /// reserve every number the job can emit. Otherwise a concurrent
    /// allocator between `write` and `install` lands inside the chunk
    /// range and clobbers a chunk path (guest 25M run #4 died in settle
    /// with `SST block CRC mismatch in .../000018.sst`).
    #[test]
    fn prepare_l0_compact_reserves_whole_chunk_range() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.set_defer_auto_compact(true);
        db.set_compact_target_file_bytes(4 * 1024);
        for round in 0..4u8 {
            for i in 0..64u8 {
                let mut val = vec![0xA5u8; 96];
                val[0] = round;
                val[1] = i;
                db.put([b'k', i], val).unwrap();
            }
            db.flush().unwrap();
        }
        let job = db
            .prepare_l0_compact(CompactOptions::default())
            .unwrap()
            .expect("L0 job");
        let tables = job.write().unwrap();
        assert!(tables.len() >= 2, "fixture must split into chunks");
        let chunk_num = |t: &SstTable| -> u64 {
            t.path()
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse().ok())
                .expect("numeric sst name")
        };
        let used: Vec<u64> = tables.iter().map(chunk_num).collect();
        assert!(
            used.iter().all(|n| *n >= job.file_num),
            "chunks must start at the reserved number: {used:?} vs {}",
            job.file_num
        );
        // The next allocator must clear the whole chunk range.
        let next = db.alloc_file_num();
        let max_used = *used.iter().max().unwrap();
        assert!(
            next > max_used,
            "unreserved chunk number inside the range: next={next}, max used={max_used}"
        );
        db.install_prepared_l0_compact(job, tables).unwrap();
        assert!(db.get(&[b'k', 0]).is_some(), "store reads after install");
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Across-job batching (v21o): `prepare_disjoint_pushdown_batch` must
    /// return pairwise key-disjoint jobs (shared-boundary = overlap), and a
    /// full `compact_leveled` drain through the real `ParallelMergeEnv`
    /// executor (scoped threads) must leave every key readable, every level
    /// a disjoint run set, and a reopen-able MANIFEST.
    #[test]
    fn parallel_jobs_batch_disjoint_and_correct() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.bulk_route_enabled = false; // ladder mechanics; bulk would install at MAX
        db.set_defer_auto_compact(true);
        db.set_compact_target_file_bytes(64 * 1024);
        // Resting shape: L1 target 8 KiB pushes each round's file down to
        // L2, while L2's 10× fan-out target holds all rounds (~5 KiB each).
        db.l1_target_bytes = 8 * 1024;
        let mut expect = std::collections::BTreeMap::new();
        for round in 0..8u8 {
            for i in 0..32u8 {
                let key = [round, i];
                let val = vec![0xA5u8; 64];
                db.put(key, val.clone()).unwrap();
                expect.insert(key.to_vec(), val);
            }
            db.flush().unwrap();
            // No seam installed yet: this drains one job at a time, so the
            // fixture shape is the pre-v21o sequential behavior.
            db.compact_leveled().unwrap();
        }
        // Squeeze L2 over target so pushdowns are available.
        db.l1_target_bytes = 1024;
        let batch = db.prepare_disjoint_pushdown_batch(4).unwrap();
        assert_eq!(batch.len(), 4, "fixture must fill the requested width");
        let hulls: Vec<(Vec<u8>, Vec<u8>)> = batch
            .iter()
            .map(|j| {
                let lo = j
                    .inputs
                    .iter()
                    .map(|t| t.smallest_user_key().unwrap_or(&[]).to_vec())
                    .min()
                    .unwrap();
                let hi = j
                    .inputs
                    .iter()
                    .map(|t| t.largest_user_key().unwrap_or(&[]).to_vec())
                    .max()
                    .unwrap();
                (lo, hi)
            })
            .collect();
        for a in 0..hulls.len() {
            for b in (a + 1)..hulls.len() {
                let (loa, hia) = &hulls[a];
                let (lob, hib) = &hulls[b];
                assert!(
                    hia < lob || hib < loa,
                    "job hulls must be disjoint: {hulls:?}"
                );
            }
        }
        // Dropping a prepared batch burns only file numbers; inputs stay
        // live. Now drain through the real parallel executor.
        db.set_parallel_merge(Arc::new(ParallelMergeEnv::new(StdEnv)));
        db.set_parallel_jobs(4);
        db.compact_leveled().unwrap();
        let scan: Vec<(Vec<u8>, Vec<u8>)> = db
            .scan(std::ops::Bound::Unbounded, std::ops::Bound::Unbounded)
            .map(|kv| (kv.key.to_vec(), kv.value.to_vec()))
            .collect();
        let want: Vec<(Vec<u8>, Vec<u8>)> = expect.into_iter().collect();
        assert_eq!(scan, want, "every key survives the parallel drain");
        for level in 1..=MAX_LSM_LEVEL {
            let view = db.level_view(level, "");
            assert!(
                crate::leveling::is_disjoint(&view),
                "level {level} stacked after parallel drain"
            );
        }
        db.close().unwrap();
        let reopened = Db::open(&dir).unwrap();
        let rescan: Vec<(Vec<u8>, Vec<u8>)> = reopened
            .scan(std::ops::Bound::Unbounded, std::ops::Bound::Unbounded)
            .map(|kv| (kv.key.to_vec(), kv.value.to_vec()))
            .collect();
        assert_eq!(rescan, want, "manifest recovers the batched installs");
        drop(reopened);
        let _ = fs::remove_dir_all(&dir);
    }

    /// LevelRunStream oracle (scan heap-width cut): a level holding many
    /// strictly disjoint tables must scan identically to the per-file merge
    /// it replaces — full range, windows that cut file boundaries, `limit`,
    /// newest-wins overwrites, a point delete, and a range tombstone — while
    /// the overlapping L0 pair keeps the per-file stream path.
    #[test]
    fn scan_grouped_disjoint_level_matches_btree() {
        fn check(
            db: &Db,
            expect: &std::collections::BTreeMap<Vec<u8>, Vec<u8>>,
            start: Bound<&[u8]>,
            end: Bound<&[u8]>,
            limit: Option<usize>,
        ) {
            let snap = db.visible_sequence();
            let got: Vec<(Vec<u8>, Vec<u8>)> = db
                .scan_at(snap, start, end, limit)
                .map(|kv| (kv.key.to_vec(), kv.value.to_vec()))
                .collect();
            let mut want: Vec<(Vec<u8>, Vec<u8>)> = expect
                .range::<[u8], _>((start, end))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            if let Some(n) = limit {
                want.truncate(n);
            }
            assert_eq!(got, want, "scan {start:?}..{end:?} limit {limit:?}");
        }
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.bulk_route_enabled = false; // ladder mechanics; bulk would install at MAX
        db.set_defer_auto_compact(true);
        db.set_compact_target_file_bytes(64 * 1024);
        // Same resting shape as `parallel_jobs_batch_disjoint_and_correct`:
        // each round's L0 file is pushed to L2, which keeps every round.
        db.l1_target_bytes = 8 * 1024;
        let mut expect = std::collections::BTreeMap::new();
        for round in 0..8u8 {
            for i in 0..64u8 {
                let key = [round, i];
                let val = vec![0xA5u8; 48];
                db.put(key, val.clone()).unwrap();
                expect.insert(key.to_vec(), val);
            }
            db.flush().unwrap();
            db.compact_leveled().unwrap();
        }
        let grouped = db
            .sst_runs
            .iter()
            .filter(|r| r.disjoint_by_lo.is_some())
            .map(|r| r.tables_newest_first.len())
            .max()
            .unwrap_or(0);
        assert!(
            grouped >= 3,
            "fixture must build a grouped disjoint run (>=3 files), got {grouped}"
        );
        // Overlapping L0 pair: same key hull in both files keeps that run on
        // the per-file stream path (newest file wins the shared keys).
        for (i, v) in [(0u8, 1u8), (1, 2)] {
            db.put([0u8, 200 + i], vec![v; 16]).unwrap();
            expect.insert([0, 200 + i].to_vec(), vec![v; 16]);
            db.put([2u8, 10], vec![v; 16]).unwrap();
            expect.insert([2, 10].to_vec(), vec![v; 16]);
            db.flush().unwrap();
        }
        // Live memtable rows merged over both stream shapes, a point delete,
        // and a range tombstone spanning L2 file boundaries.
        db.put([3, 7], b"mem-newest".to_vec()).unwrap();
        expect.insert([3, 7].to_vec(), b"mem-newest".to_vec());
        db.delete([5, 10]).unwrap();
        expect.remove(&[5u8, 10].to_vec()[..]);
        db.delete_range([2u8, 100], [4u8, 20]).unwrap();
        let covered: Vec<Vec<u8>> = expect
            .range([2u8, 100].to_vec()..[4u8, 20].to_vec())
            .map(|(k, _)| k.clone())
            .collect();
        for k in covered {
            expect.remove(&k);
        }
        check(&db, &expect, Bound::Unbounded, Bound::Unbounded, None);
        check(
            &db,
            &expect,
            Bound::Included(&[2u8, 50]),
            Bound::Excluded(&[5u8, 200]),
            None,
        );
        check(
            &db,
            &expect,
            Bound::Included(&[2u8, 50]),
            Bound::Included(&[3u8, 7]),
            None,
        );
        check(&db, &expect, Bound::Unbounded, Bound::Unbounded, Some(7));
        check(
            &db,
            &expect,
            Bound::Excluded(&[0u8, 5]),
            Bound::Unbounded,
            Some(200),
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// v21d regression: rewrite chunks (whole-levels `compact` /
    /// `rewrite_ssts`) enter the live inventory pool-registered. Each
    /// freshly opened chunk holds its whole file body resident, and the
    /// writer accumulates every chunk of the job before returning, so an
    /// unregistered chunk payload can never be evicted — the 25M guest
    /// settle OOM'd at ~6 resident chunks (guest run #5).
    #[test]
    fn compact_rewrite_chunks_are_pool_evictable() {
        let dir = temp_dir();
        let mut opts = OpenOptions {
            sync: false,
            auto_flush_bytes: Some(4 * 1024 * 1024),
            ..OpenOptions::default()
        };
        opts.sst_payload_budget_bytes = Some(1); // evict everything, always
        let mut db = Db::<StdEnv>::open_with_env_bounded(&dir, opts, StdEnv).unwrap();
        db.set_defer_auto_compact(true);
        for round in 0..3u32 {
            for i in 0..200u32 {
                let k = format!("r{round}-key-{i:04}").into_bytes();
                let v = vec![(i % 199) as u8; 120];
                db.put(&k, &v).unwrap();
            }
            db.flush().unwrap();
        }
        assert!(db.ssts.len() >= 3, "want several L0 tables");
        // Whole-levels rewrite: L0(+L1) of the family through
        // `rewrite_ssts`, the same path settle's `compact()` takes.
        db.compact_ssts_only().unwrap();
        let resident = db.ssts.iter().filter(|t| t.payload_resident()).count();
        assert_eq!(
            resident, 0,
            "rewrite chunks must be pool-evictable; {resident} live payloads resident at budget 1"
        );
        // Evicted chunk reads re-read blocks from file and must be
        // identical (fail-closed CRC path).
        for round in 0..3u32 {
            for i in 0..200u32 {
                let k = format!("r{round}-key-{i:04}").into_bytes();
                let want = vec![(i % 199) as u8; 120];
                assert_eq!(db.get(&k).expect("read after rewrite"), want);
            }
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// v21e regression: whole-levels rewrites split at
    /// `rewrite_chunk_target_bytes` even when `compact_target_file_bytes`
    /// is set enormous — the chunked writer's per-chunk transient (chunk
    /// body + bloom + read-back) must not scale with the configured
    /// compact target during a full-level merge (guest 25M settle
    /// peak-OOM'd with 256 MiB chunks on a 3.9 GB box).
    #[test]
    fn rewrite_caps_chunk_size_for_whole_level_merges() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.bulk_route_enabled = false; // ladder mechanics; bulk would install at MAX
        db.set_defer_auto_compact(true);
        // Without the cap this merge would emit ONE file.
        db.set_compact_target_file_bytes(u64::MAX / 2);
        db.rewrite_chunk_target_bytes = 8 * 1024;
        for round in 0..2u32 {
            for i in 0..200u32 {
                let k = format!("r{round:02}-key-{i:06}").into_bytes();
                let v = vec![0xA5u8; 300];
                db.put(&k, &v).unwrap();
            }
            db.flush().unwrap();
        }
        db.compact_ssts_only().unwrap();
        assert!(
            db.ssts.len() >= 2,
            "rewrite must split at its chunk cap, got {} files",
            db.ssts.len()
        );
        assert!(db.sst_levels.iter().all(|&lvl| lvl == 1));
        assert!(db.get(b"r00-key-000000").is_some());
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepare_write_install_l0_keeps_inputs_until_install() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.set_defer_auto_compact(true);
        for i in 0..L0_COMPACTION_TRIGGER {
            db.put([b'a', i as u8], [b'1', i as u8]).unwrap();
            db.flush().unwrap();
        }
        assert_eq!(db.level_file_count(0), L0_COMPACTION_TRIGGER);
        let job = db
            .prepare_l0_compact(CompactOptions::default())
            .unwrap()
            .expect("L0 job");
        assert_eq!(
            db.level_file_count(0),
            L0_COMPACTION_TRIGGER,
            "prepare must not hide L0"
        );
        db.put(b"zz", b"live").unwrap();
        db.flush().unwrap();
        let extra_l0 = db.level_file_count(0);
        assert!(
            extra_l0 > L0_COMPACTION_TRIGGER,
            "flush during write stays L0"
        );
        let tables = job.write().unwrap();
        assert_eq!(db.level_file_count(0), extra_l0);
        db.install_prepared_l0_compact(job, tables).unwrap();
        assert_eq!(db.level_file_count(0), 1, "L0 flushed during write kept");
        assert!(db.level_file_count(1) >= 1);
        assert_eq!(db.get(&[b'a', 0]).as_deref(), Some([b'1', 0].as_slice()));
        assert_eq!(db.get(b"zz").as_deref(), Some(b"live".as_slice()));
        db.close().unwrap();
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(&[b'a', 0]).as_deref(), Some([b'1', 0].as_slice()));
        assert_eq!(db.get(b"zz").as_deref(), Some(b"live".as_slice()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// The merged compaction output must split at `compact_target_file_bytes`
    /// (the SST writer buffers one output file in memory — the unbounded
    /// single-file merge OOMed a 4 GiB guest at settle). Every key stays
    /// readable, the split survives reopen, and a same-user version run is
    /// never divided across files.
    #[test]
    fn compact_splits_merged_output_at_target_file_bytes() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.set_compact_target_file_bytes(4 * 1024);
        // 100 distinct keys x ~134 B ≈ 13 KiB — must split at the 4 KiB target.
        for i in 0..100u32 {
            db.put(format!("split/{i:04}").into_bytes(), vec![b'v'; 100])
                .unwrap();
        }
        // One user key with a 40-version run (~5 KiB): the run extends past
        // the target and must stay inside a single output file.
        for _ in 0..40u32 {
            db.put(b"split/zzzz", vec![b'r'; 100]).unwrap();
        }
        db.flush().unwrap();
        assert_eq!(db.level_file_count(0), 1);
        db.compact().unwrap();
        assert_eq!(db.level_file_count(0), 0, "compact drains L0");
        let l1 = db.level_file_count(1);
        assert!(
            l1 >= 2,
            "merged output must split at the 4 KiB target, got {l1} L1 files"
        );
        for i in 0..100u32 {
            let k = format!("split/{i:04}").into_bytes();
            assert_eq!(db.get(&k).as_deref().map(|v| v.len()), Some(100), "key {i}");
        }
        assert_eq!(
            db.get(b"split/zzzz").as_deref().map(|v| v.len()),
            Some(100),
            "latest version of the run"
        );
        db.close().unwrap();
        let db = Db::open(&dir).unwrap();
        assert!(
            db.level_file_count(1) >= 2,
            "split inventory survives reopen"
        );
        assert_eq!(db.get(b"split/0042").as_deref().map(|v| v.len()), Some(100));
        assert_eq!(db.get(b"split/zzzz").as_deref().map(|v| v.len()), Some(100));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stage_flush_imm_leaves_has_imm_for_worker() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
                sync: false,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
                sst_payload_budget_bytes: None,
            },
        )
        .unwrap();
        db.set_defer_auto_compact(true);
        for i in 0..32u8 {
            db.put([b'k', i], vec![i; 128]).unwrap();
        }
        assert!(db.stage_flush_imm().unwrap());
        assert!(db.has_imm(), "stage must leave the table in the imm slot");
        assert_eq!(db.get(&[b'k', 0]).as_deref(), Some([0u8; 128].as_slice()));
        let imm = db.prepare_flush_imm().unwrap().expect("take staged");
        assert!(!db.has_imm());
        let num = db.alloc_file_num();
        let (table, _, _) = db.write_memtable_to_l0_file_num(&imm, num).unwrap();
        db.install_l0_sst(table, num).unwrap();
        assert_eq!(db.sst_count(), 1);
        assert_eq!(db.get(&[b'k', 0]).as_deref(), Some([0u8; 128].as_slice()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn auto_flush_when_mem_exceeds_threshold() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
                sync: true,
                auto_flush_bytes: Some(200),
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
                sst_payload_budget_bytes: None,
            },
        )
        .unwrap();
        // Each put ~ key+value+8; a few large values should trip flush.
        for i in 0..20u8 {
            let val = vec![i; 64];
            db.put([b'k', i], &val).unwrap();
        }
        assert!(
            db.sst_count() >= 1,
            "expected auto-flush to create at least one SST"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_sync_then_sync_groups_durability() {
        let dir = temp_dir();
        {
            let mut db = Db::open_with(
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
            )
            .unwrap();
            for i in 0..10u8 {
                db.put_with([b'x', i], b"v", WriteOptions::no_sync())
                    .unwrap();
            }
            db.sync().unwrap();
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        for i in 0..10u8 {
            assert_eq!(db.get(&[b'x', i]).as_deref(), Some(b"v".as_ref()));
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// Async WAL (`sync=false`): Ok after `write()`, not `fdatasync`. Process
    /// crash must still recover (Rocks `WriteOptions.sync=false`). Power loss
    /// may lose recent acks.
    #[test]
    fn async_puts_are_in_wal_without_fsync() {
        let dir = temp_dir();
        {
            let mut db = Db::open_with(
                &dir,
                OpenOptions {
                    wal_full_fsync: true,
                    history: Default::default(),
                    wal_recovery: Default::default(),
                    sync: false,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                    sst_payload_budget_bytes: None,
                },
            )
            .unwrap();
            for i in 0..20u8 {
                db.put([b'a', i], b"xxxxxxxx").unwrap();
            }
            let wal = dir.join(WAL_FILE_NAME);
            let n = fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
            assert!(
                n > 0,
                "async Ok must have write()d the WAL (got CURRENT.log={n})"
            );
            // Drop without `sync()` — no fdatasync. Reopen must still see puts.
            drop(db);
        }
        let db = Db::open(&dir).unwrap();
        for i in 0..20u8 {
            assert_eq!(db.get(&[b'a', i]).as_deref(), Some(b"xxxxxxxx".as_ref()));
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn async_large_puts_recover_without_vlog_fsync() {
        let dir = temp_dir();
        let payload = vec![b'B'; 800];
        {
            let mut db = Db::open_with(
                &dir,
                OpenOptions {
                    wal_full_fsync: true,
                    history: Default::default(),
                    wal_recovery: Default::default(),
                    sync: false,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: Some(512),
                    sst_payload_budget_bytes: None,
                },
            )
            .unwrap();
            for i in 0..16u8 {
                db.put([b'k', i], &payload).unwrap();
            }
            for i in 0..16u8 {
                assert_eq!(db.get(&[b'k', i]).as_deref(), Some(payload.as_slice()));
            }
            drop(db);
        }
        let db = Db::open_with(
            &dir,
            OpenOptions {
                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
                sync: false,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: Some(512),
                sst_payload_budget_bytes: None,
            },
        )
        .unwrap();
        for i in 0..16u8 {
            assert_eq!(
                db.get(&[b'k', i]).as_deref(),
                Some(payload.as_slice()),
                "async large put must write() vlog before the WAL pointer"
            );
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0044 P1.1: interned payload + WAL v2 still recovers every key.
    #[test]
    fn interned_pipeline_batch_recovers_after_close() {
        let dir = temp_dir();
        let payload = vec![b'k'; 256];
        {
            let mut db = Db::open_with(
                &dir,
                OpenOptions {
                    wal_full_fsync: true,
                    history: Default::default(),
                    wal_recovery: Default::default(),
                    sync: false,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                    sst_payload_budget_bytes: None,
                },
            )
            .unwrap();
            let val = intern_bytes(&payload);
            let ops: Vec<_> = (0..32u8)
                .map(|i| BatchOp::Put {
                    key: Bytes::from(vec![b'p', i]),
                    value: val.clone(),
                })
                .collect();
            db.apply_batch_with(ops, WriteOptions::no_sync()).unwrap();
            db.sync().unwrap();
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        for i in 0..32u8 {
            assert_eq!(
                db.get(&[b'p', i]).as_deref(),
                Some(payload.as_slice()),
                "key p/{i}"
            );
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0044 P1.2: async 1-op interned puts coalesce into v2 records.
    #[test]
    fn async_interned_1ops_coalesce_and_recover() {
        let dir = temp_dir();
        let payload = vec![b'k'; 256];
        {
            let mut db = Db::open_with(
                &dir,
                OpenOptions {
                    wal_full_fsync: true,
                    history: Default::default(),
                    wal_recovery: Default::default(),
                    sync: false,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                    sst_payload_budget_bytes: None,
                },
            )
            .unwrap();
            for i in 0..40u8 {
                db.put_with([b'q', i], &payload, WriteOptions::no_sync())
                    .unwrap();
            }
            db.sync().unwrap();
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        for i in 0..40u8 {
            assert_eq!(
                db.get(&[b'q', i]).as_deref(),
                Some(payload.as_slice()),
                "key q/{i}"
            );
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn auto_compact_when_sst_count_reaches_threshold() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: Some(3),
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
                sst_payload_budget_bytes: None,
            },
        )
        .unwrap();
        for i in 0..3u8 {
            db.put([b'k', i], b"v").unwrap();
            db.flush().unwrap();
        }
        // Third flush should have triggered auto-compact → single SST.
        assert_eq!(
            db.sst_count(),
            1,
            "auto-compact should merge when count >= threshold"
        );
        for i in 0..3u8 {
            assert_eq!(db.get(&[b'k', i]).as_deref(), Some(b"v".as_ref()));
        }
        assert_eq!(db.stats().auto_compact_failures, 0);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0015 P2.2: auto-compact I/O fail after successful flush increments stats; flush stays Ok.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn auto_compact_fail_after_flush_records_stats() {
        use crate::env::{Env, EnvFile, StdEnv};
        use std::cell::Cell;
        use std::io::{self, Read, Seek, SeekFrom, Write};
        use std::path::Path;
        use std::rc::Rc;

        /// Allows `n` creates of paths ending in `.sst.tmp`, then fails further ones.
        /// Lets two flush SST publishes succeed; the auto-compact output SST create fails.
        #[derive(Clone)]
        struct FailSstTmpAfter {
            inner: StdEnv,
            remaining: Rc<Cell<u64>>,
        }

        struct F {
            inner: <StdEnv as Env>::File,
        }
        impl Read for F {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.inner.read(buf)
            }
        }
        impl Write for F {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.inner.write(buf)
            }
            fn flush(&mut self) -> io::Result<()> {
                self.inner.flush()
            }
        }
        impl Seek for F {
            fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
                self.inner.seek(pos)
            }
        }
        impl EnvFile for F {
            fn sync_data(&mut self) -> io::Result<()> {
                self.inner.sync_data()
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
        impl Env for FailSstTmpAfter {
            type File = F;
            fn create_dir_all(&self, path: &Path) -> io::Result<()> {
                self.inner.create_dir_all(path)
            }
            fn create(&self, path: &Path) -> io::Result<Self::File> {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.ends_with(".sst.tmp") {
                    let left = self.remaining.get();
                    if left == 0 {
                        return Err(io::Error::other("injected sst.tmp create fail (compact)"));
                    }
                    self.remaining.set(left - 1);
                }
                Ok(F {
                    inner: self.inner.create(path)?,
                })
            }
            fn open_append(&self, path: &Path) -> io::Result<Self::File> {
                Ok(F {
                    inner: self.inner.open_append(path)?,
                })
            }
            fn open_read(&self, path: &Path) -> io::Result<Self::File> {
                Ok(F {
                    inner: self.inner.open_read(path)?,
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
        // Two flush SST publishes OK; compact's third `.sst.tmp` fails.
        let env = FailSstTmpAfter {
            inner: StdEnv,
            remaining: Rc::new(Cell::new(2)),
        };
        let mut db = Db::open_with_env(
            &dir,
            OpenOptions {
                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: Some(2),
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
                sst_payload_budget_bytes: None,
            },
            env,
        )
        .unwrap();

        db.put(b"a", b"1").unwrap();
        db.flush().unwrap();
        assert_eq!(db.stats().auto_compact_failures, 0);

        db.put(b"b", b"2").unwrap();
        // Flush must succeed (F18); auto-compact after second SST fails via Env.
        db.flush().unwrap();
        let st = db.stats();
        assert!(
            st.auto_compact_failures >= 1,
            "expected auto-compact fail counter, got {}",
            st.auto_compact_failures
        );
        assert!(
            !st.last_auto_compact_error.is_empty(),
            "expected last_auto_compact_error to be set"
        );
        assert!(
            st.last_auto_compact_error.contains("injected")
                || st.last_auto_compact_error.contains("compact")
                || st.last_auto_compact_error.contains("io"),
            "unexpected error text: {}",
            st.last_auto_compact_error
        );
        // Acked data still readable; flush was Ok.
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// F20: auto-compact must preserve versions needed by held snapshots.
    #[test]
    fn auto_compact_preserves_snapshot_history() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: Some(2),
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
                sst_payload_budget_bytes: None,
            },
        )
        .unwrap();
        assert!(!db.auto_reclaim());
        db.put(b"k", b"old").unwrap();
        let snap = db.snapshot();
        db.flush().unwrap();
        db.put(b"k", b"new").unwrap();
        db.flush().unwrap(); // triggers auto-compact at count >= 2
        assert_eq!(db.sst_count(), 1);
        assert_eq!(db.get(b"k").as_deref(), Some(b"new".as_ref()));
        // Historical read at pre-overwrite snapshot must still see "old".
        assert_eq!(
            db.get_at(snap, b"k").unwrap().as_deref(),
            Some(b"old".as_ref()),
            "F20: auto-compact must not GC versions still visible at open snapshots"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// open-items §2.3: L0 write stall refuses puts until compact drains L0.
    #[test]
    fn write_stall_refuses_when_l0_at_limit() {
        let dir = temp_dir();
        let mut db = Db::open_with(
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
        )
        .unwrap();
        db.set_write_stall_l0(Some(2));
        assert_eq!(db.write_stall_l0(), Some(2));
        db.put(b"a", b"1").unwrap();
        db.flush().unwrap();
        db.put(b"b", b"2").unwrap();
        db.flush().unwrap();
        assert!(db.level_file_count(0) >= 2 || db.sst_count() >= 2);
        // Force L0 count check: after two flushes without compact we have ≥2 SSTs at L0.
        let l0 = db.level_file_count(0);
        if l0 < 2 {
            // Compact may have been skipped if only one level path — create more L0.
            for i in 0..3u8 {
                db.put([b'x', i], [b'v', i]).unwrap();
                db.flush().unwrap();
            }
        }
        assert!(
            db.level_file_count(0) >= 2,
            "need L0>=2 for stall, got {}",
            db.level_file_count(0)
        );
        let err = db.put(b"c", b"3").unwrap_err();
        assert!(
            matches!(err, CoreError::WriteStall { limit: 2, .. }),
            "expected WriteStall, got {err:?}"
        );
        assert!(db.write_stall_count() >= 1);
        assert!(db.stats().write_stall_count >= 1);
        // Drain L0 and write again.
        db.compact().unwrap();
        if db.level_file_count(0) >= 2 {
            db.compact().unwrap();
        }
        if db.level_file_count(0) < 2 {
            db.put(b"c", b"3").unwrap();
            assert_eq!(db.get(b"c").as_deref(), Some(b"3".as_ref()));
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Mem stall without drain: put fails when active mem exceeds limit.
    #[test]
    fn write_stall_mem_refuses_when_over_limit() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
                sync: true,
                auto_flush_bytes: None, // no auto flush — mem grows
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
                sst_payload_budget_bytes: None,
            },
        )
        .unwrap();
        // Tiny limit: one modest put should exceed after a few writes.
        db.set_write_stall_mem_bytes(Some(64));
        assert_eq!(db.write_stall_mem_bytes(), Some(64));
        let payload = vec![0xABu8; 40];
        db.put(b"a", &payload).unwrap(); // first write under/near limit
                                         // Keep putting until stall (no drain).
        let mut stalled = false;
        for i in 0..20u8 {
            match db.put([b'k', i], &payload) {
                Ok(()) => {}
                Err(CoreError::WriteStallMem { mem_bytes, limit }) => {
                    assert!(mem_bytes >= limit);
                    assert_eq!(limit, 64);
                    stalled = true;
                    break;
                }
                Err(e) => panic!("unexpected {e:?}"),
            }
        }
        assert!(stalled, "expected WriteStallMem");
        assert!(db.write_stall_count() >= 1);
        // Explicit flush clears mem; writes resume.
        db.flush().unwrap();
        db.put(b"after", b"ok").unwrap();
        assert_eq!(db.get(b"after").as_deref(), Some(b"ok".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Mem stall + drain: flush admits the write.
    #[test]
    fn write_stall_mem_drain_flushes() {
        let dir = temp_dir();
        let mut db = Db::open_with(
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
        )
        .unwrap();
        let payload = vec![0xCDu8; 80];
        db.put(b"seed", &payload).unwrap();
        // seed alone is large enough that mem is over a 64B stall limit.
        db.set_write_stall_mem_bytes(Some(64));
        db.set_write_stall_drain(true);
        let stalls_before = db.write_stall_count();
        db.put(b"ok", b"1").unwrap();
        assert_eq!(db.get(b"ok").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.write_stall_count(), stalls_before);
        assert!(db.sst_count() >= 1, "drain should have flushed");
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// enable_write_backpressure_defaults wires pressure + hard stall + drain.
    #[test]
    fn write_backpressure_defaults_preset() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.enable_write_backpressure_defaults();
        assert_eq!(db.write_pressure_l0(), Some(L0_COMPACTION_TRIGGER));
        assert_eq!(
            db.write_stall_l0(),
            Some(L0_COMPACTION_TRIGGER.saturating_mul(2))
        );
        assert!(db.write_stall_drain());
        // Empty DB admits writes under defaults.
        db.put(b"k", b"v").unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(b"v".as_ref()));
        assert_eq!(db.stats().l0_files, 0);
        db.flush().unwrap();
        assert_eq!(db.stats().l0_files, 1);
        assert!(db.stats().gc_line().contains("l0=1"));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Soft L0 pressure drains without refusing the write (§2.3 b).
    #[test]
    fn write_pressure_l0_drains_without_error() {
        let dir = temp_dir();
        let mut db = Db::open_with(
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
        )
        .unwrap();
        for i in 0..3u8 {
            db.put([b'k', i], [b'v', i]).unwrap();
            db.flush().unwrap();
        }
        let l0_before = db.level_file_count(0);
        assert!(l0_before >= 2, "need L0 pressure, got {l0_before}");
        db.set_write_pressure_l0(Some(2));
        assert_eq!(db.write_pressure_l0(), Some(2));
        let pressure_before = db.write_pressure_count();
        // Put under pressure: must succeed and record a pressure drain.
        db.put(b"under-pressure", b"1").unwrap();
        assert_eq!(db.get(b"under-pressure").as_deref(), Some(b"1".as_ref()));
        assert!(
            db.write_pressure_count() > pressure_before,
            "expected pressure drain counter bump"
        );
        assert_eq!(
            db.write_stall_count(),
            0,
            "soft pressure must not hard-stall"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Drain-before-stall: one compact attempt can admit the write without error.
    #[test]
    fn write_stall_drain_admits_after_compact() {
        let dir = temp_dir();
        let mut db = Db::open_with(
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
        )
        .unwrap();
        // Build L0 without stall, then enable drain+limit.
        for i in 0..3u8 {
            db.put([b'k', i], [b'v', i]).unwrap();
            db.flush().unwrap();
        }
        assert!(
            db.level_file_count(0) >= 2,
            "need L0>=2, got {}",
            db.level_file_count(0)
        );
        db.set_write_stall_l0(Some(2));
        db.set_write_stall_drain(true);
        assert!(db.write_stall_drain());
        let stalls_before = db.write_stall_count();
        // Drain path should compact L0 down and accept the put.
        db.put(b"ok", b"1").unwrap();
        assert_eq!(db.get(b"ok").as_deref(), Some(b"1".as_ref()));
        assert_eq!(
            db.write_stall_count(),
            stalls_before,
            "drain should avoid WriteStall when compact reduces L0"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Opt-in auto_reclaim: bare snap becomes SnapshotTooOld; pin is preserved.
    #[test]
    fn auto_reclaim_on_auto_compact() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: Some(2),
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
                sst_payload_budget_bytes: None,
            },
        )
        .unwrap();
        db.set_auto_reclaim(true);
        assert!(db.auto_reclaim());

        db.put(b"k", b"old").unwrap();
        let bare = db.snapshot();
        let pin = db.pin_snapshot();
        db.flush().unwrap();
        db.put(b"k", b"new").unwrap();
        db.flush().unwrap(); // auto-compact + reclaim with pin floor

        assert_eq!(db.get(b"k").as_deref(), Some(b"new".as_ref()));
        assert_eq!(
            db.get_at(pin.snapshot(), b"k").unwrap().as_deref(),
            Some(b"old".as_ref()),
            "pin must survive auto_reclaim"
        );
        // Watermark at pin seq → bare snap at same seq still ok; below fails.
        // After reclaim with pin at old, floor is pin.seq; bare equals pin so ok.
        assert_eq!(
            db.get_at(bare, b"k").unwrap().as_deref(),
            Some(b"old".as_ref())
        );

        db.release_snapshot_pin(pin);
        // Next reclaim without pins → watermark = last_seq; bare too old.
        db.put(b"k", b"newer").unwrap();
        db.flush().unwrap();
        db.put(b"x", b"1").unwrap();
        db.flush().unwrap();
        let err = db.get_at(bare, b"k").unwrap_err();
        assert!(
            matches!(err, CoreError::SnapshotTooOld { .. }),
            "bare snap after unpin+reclaim: {err:?}"
        );
        assert_eq!(db.get(b"k").as_deref(), Some(b"newer".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// F19: file numbers past 6 digits still flush + reopen.
    #[test]
    fn high_file_number_sst_round_trip() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        // Jump allocator into the 7-digit regime.
        db.next_file_num = 1_000_000;
        db.put(b"hi", b"there").unwrap();
        db.flush().unwrap();
        assert!(db
            .ssts
            .iter()
            .any(|t| t.path().file_name().unwrap() == "1000000.sst"));
        db.close().unwrap();
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"hi").as_deref(), Some(b"there".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_with_latest_only_drops_old_versions() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"k", b"old").unwrap();
        db.flush().unwrap();
        db.put(b"k", b"new").unwrap();
        db.flush().unwrap();
        db.put(b"gone", b"x").unwrap();
        db.flush().unwrap();
        db.delete(b"gone").unwrap();
        db.flush().unwrap();

        let before_entries: usize = db.ssts.iter().map(SstTable::len).sum();
        assert!(
            before_entries >= 3,
            "history should include old put + tombstone before GC"
        );

        db.compact_with(CompactOptions::latest_only()).unwrap();
        assert_eq!(db.sst_count(), 1);
        assert_eq!(db.get(b"k").as_deref(), Some(b"new".as_ref()));
        assert_eq!(db.get(b"gone"), None);
        // Only live value for k remains; tombstoned key dropped entirely.
        assert_eq!(db.ssts[0].len(), 1);
        db.close().unwrap();

        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(b"new".as_ref()));
        assert_eq!(db.get(b"gone"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    /// open-items §2.1: pin_snapshot + compact_reclaim preserves get_at for the pin.
    #[test]
    fn compact_reclaim_respects_snapshot_pin() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"k", b"old").unwrap();
        db.flush().unwrap();
        let pin = db.pin_snapshot();
        assert_eq!(db.snapshot_pin_count(), 1);
        assert_eq!(db.oldest_pinned_sequence(), Some(pin.sequence()));
        assert_eq!(
            db.get_at(pin.snapshot(), b"k").unwrap().as_deref(),
            Some(b"old".as_ref())
        );

        db.put(b"k", b"new").unwrap();
        db.flush().unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(b"new".as_ref()));

        // Reclaim must keep `old` while pin is open.
        db.compact_reclaim().unwrap();
        assert_eq!(
            db.get_at(pin.snapshot(), b"k").unwrap().as_deref(),
            Some(b"old".as_ref())
        );
        assert_eq!(db.get(b"k").as_deref(), Some(b"new".as_ref()));

        db.release_snapshot_pin(pin);
        assert_eq!(db.snapshot_pin_count(), 0);
        // No pins → reclaim drops superseded history (latest-only watermark).
        let floor_before = db.earliest_readable_sequence();
        db.compact_reclaim().unwrap();
        assert!(
            db.earliest_readable_sequence() >= floor_before && db.earliest_readable_sequence() > 0,
            "reclaim without pins must raise watermark"
        );
        assert_eq!(db.get(b"k").as_deref(), Some(b"new".as_ref()));
        // Bare old snapshot is fail-closed (open-items §2.1 (c)).
        let old_snap = Snapshot::at(1.min(db.earliest_readable_sequence().saturating_sub(1)));
        if old_snap.sequence() < db.earliest_readable_sequence() {
            let err = db.get_at(old_snap, b"k").unwrap_err();
            assert!(
                matches!(err, CoreError::SnapshotTooOld { .. }),
                "expected SnapshotTooOld, got {err:?}"
            );
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// open-items §2.1 (c): latest_only raises watermark; old get_at fails closed.
    #[test]
    fn snapshot_too_old_after_latest_only() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"k", b"old").unwrap();
        let old_seq = db.last_sequence();
        db.flush().unwrap();
        db.put(b"k", b"new").unwrap();
        db.flush().unwrap();
        let snap = Snapshot::at(old_seq);
        assert_eq!(
            db.get_at(snap, b"k").unwrap().as_deref(),
            Some(b"old".as_ref())
        );
        db.compact_with(CompactOptions::latest_only()).unwrap();
        assert!(db.earliest_readable_sequence() > 0);
        let floor = db.earliest_readable_sequence();
        assert_eq!(db.get(b"k").as_deref(), Some(b"new".as_ref()));
        let err = db.get_at(snap, b"k").unwrap_err();
        assert!(
            matches!(
                err,
                CoreError::SnapshotTooOld {
                    requested,
                    earliest
                } if requested == old_seq && earliest == floor
            ),
            "got {err:?}"
        );
        // Range path must not silently look empty.
        let err = db
            .range_at_limited(old_seq, Bound::Unbounded, Bound::Unbounded, None)
            .unwrap_err();
        assert!(
            matches!(err, CoreError::SnapshotTooOld { .. }),
            "range_at too old: {err:?}"
        );
        let err = db
            .try_scan_at(old_seq, Bound::Unbounded, Bound::Unbounded, None)
            .err()
            .expect("try_scan_at must fail closed");
        assert!(
            matches!(err, CoreError::SnapshotTooOld { .. }),
            "try_scan_at too old: {err:?}"
        );
        // Latest range still works.
        let live = db.range_limited(Bound::Unbounded, Bound::Unbounded, None);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].1.as_ref(), b"new");
        db.close().unwrap();

        // MANIFEST v4: watermark survives reopen.
        let db = Db::open(&dir).unwrap();
        assert_eq!(
            db.earliest_readable_sequence(),
            floor,
            "earliest_readable must load from MANIFEST"
        );
        let err = db.get_at(snap, b"k").unwrap_err();
        assert!(
            matches!(err, CoreError::SnapshotTooOld { .. }),
            "reopen still too-old: {err:?}"
        );
        assert_eq!(db.get(b"k").as_deref(), Some(b"new".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0019 P2.2: compact_for_reads flushes + collapses all SSTs with latest-only GC.
    #[test]
    fn rfc19_compact_for_reads_collapses_write_burst() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        // Write burst: many small flushes → multi-file multi-version inventory.
        for i in 0..12u8 {
            db.put(b"k", [b'v', i]).unwrap();
            db.flush().unwrap();
        }
        db.put(b"live", b"yes").unwrap();
        db.delete(b"gone-after").unwrap();
        db.put(b"gone-after", b"tmp").unwrap();
        db.delete(b"gone-after").unwrap();
        db.flush().unwrap();

        assert!(
            db.sst_count() >= 2,
            "precondition: multiple SSTs after burst"
        );
        let before = db.sst_count();
        db.compact_for_reads().unwrap();
        assert_eq!(db.sst_count(), 1, "must collapse to one SST");
        assert!(db.sst_count() < before || before == 1);
        assert_eq!(db.get(b"k").as_deref(), Some([b'v', 11].as_slice()));
        assert_eq!(db.get(b"live").as_deref(), Some(b"yes".as_ref()));
        assert_eq!(db.get(b"gone-after"), None);
        // Only live keys remain in the SST (no multi-version history for k).
        assert_eq!(db.ssts[0].len(), 2, "k + live only");

        db.close().unwrap();
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some([b'v', 11].as_slice()));
        assert_eq!(db.get(b"live").as_deref(), Some(b"yes".as_ref()));
        assert_eq!(db.get(b"gone-after"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_cleans_orphan_sst_tmp_files() {
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        // Simulate crash mid-compact: orphan temp left behind.
        let tmp = dir.join("000099.sst.tmp");
        fs::write(&tmp, b"partial-garbage").unwrap();
        assert!(tmp.exists());

        let db = Db::open(&dir).unwrap();
        assert!(!tmp.exists(), "open must remove *.sst.tmp orphans");
        assert_eq!(db.sst_count(), 0);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_writes_final_sst_not_tmp() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"a", b"1").unwrap();
        db.flush().unwrap();
        db.put(b"b", b"2").unwrap();
        db.flush().unwrap();
        db.compact().unwrap();

        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect();
        let is_final_sst = |n: &str| {
            let p = Path::new(n);
            p.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("sst"))
                && !n.ends_with(".sst.tmp")
        };
        assert!(
            names.iter().any(|n| is_final_sst(n)),
            "compact must leave a final .sst: {names:?}"
        );
        assert!(
            names.iter().all(|n| !n.ends_with(".sst.tmp")),
            "no leftover tmp after successful compact: {names:?}"
        );
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exclusive_lock_blocks_other_process() {
        // Cross-process only: same-PID re-open steals (crash-sim). Hold LOCK with a child.
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        #[cfg(unix)]
        {
            use std::process::{Command, Stdio};
            let mut child = Command::new("sleep")
                .arg("30")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            fs::write(
                dir.join(crate::lock::LOCK_FILE),
                format!("{}\n", child.id()),
            )
            .unwrap();
            match Db::open(&dir) {
                Err(CoreError::AlreadyOpen { .. }) => {}
                Ok(_) => {
                    let _ = child.kill();
                    panic!("open must fail while foreign process holds LOCK");
                }
                Err(other) => {
                    let _ = child.kill();
                    panic!("expected AlreadyOpen, got {other:?}");
                }
            }
            let _ = child.kill();
            let _ = child.wait();
            // Stale lock after child death: open steals.
            let db = Db::open(&dir).unwrap();
            db.close().unwrap();
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_process_reopen_after_forget_steals_lock() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"k", b"v").unwrap();
            std::mem::forget(db);
        }
        // Crash-sim: same PID steals LOCK and recovers.
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(b"v".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exclusive_false_skips_lock_file() {
        let dir = temp_dir();
        let opts = OpenOptions {
            wal_full_fsync: true,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: false,
            large_value_threshold: None,
            sst_payload_budget_bytes: None,
        };
        let db = Db::open_with(&dir, opts).unwrap();
        assert!(
            !dir.join(crate::lock::LOCK_FILE).exists(),
            "exclusive=false must not create LOCK"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn flush_writes_manifest_and_current() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"k", b"v").unwrap();
            db.flush().unwrap();
            db.close().unwrap();
        }
        assert!(dir.join(crate::manifest::CURRENT_FILE).exists());
        let current = fs::read_to_string(dir.join(crate::manifest::CURRENT_FILE)).unwrap();
        let man_name = current.lines().next().unwrap_or("").trim();
        assert!(
            man_name.starts_with(crate::manifest::MANIFEST_PREFIX),
            "CURRENT={current:?}"
        );
        let man_path = dir.join(man_name);
        assert!(man_path.exists(), "manifest file missing");

        let db = Db::open(&dir).unwrap();
        assert_eq!(db.sst_count(), 1);
        assert_eq!(db.get(b"k").as_deref(), Some(b"v".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// L0 install without WAL rotate: SST is not in MANIFEST. Crash that
    /// tears the file is recovered from WAL (G1).
    #[test]
    fn unsynced_l0_torn_sst_recovers_from_wal() {
        let dir = temp_dir();
        let sst_path;
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"a", b"1").unwrap();
            assert!(db.stage_flush_imm().unwrap());
            let imm = db.prepare_flush_imm().unwrap().expect("staged imm");
            let num = db.alloc_file_num();
            let (table, _, path) = db.write_memtable_to_l0_file_num(&imm, num).unwrap();
            sst_path = path;
            db.apply_l0_install(table, num);
            db.clear_flush_read_pin();
            db.put(b"c", b"3").unwrap();
            std::mem::forget(db);
        }
        assert!(sst_path.exists(), "L0 file was written");
        fs::write(&sst_path, b"torn").unwrap();
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()), "WAL replay");
        assert_eq!(db.get(b"c").as_deref(), Some(b"3".as_ref()));
        assert!(
            !sst_path.exists() || db.sst_count() == 0,
            "torn L0 must not be live inventory (orphan GC or unused)"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// After flush+rotate, SST+MANIFEST are enough: deleting the WAL must
    /// not lose acked keys (rotate fsync'd the L0 first).
    #[test]
    fn flush_rotate_makes_sst_sufficient_without_wal() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"k", b"v").unwrap();
            db.flush().unwrap();
            db.close().unwrap();
        }
        let wal = dir.join(WAL_FILE_NAME);
        if wal.exists() {
            fs::remove_file(&wal).unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(b"v".as_ref()));
        assert!(db.sst_count() >= 1);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// The compat compact worker polls `try_rotate_wal_if_idle` while the DB
    /// is idle; a drained pipeline with an empty current segment must not
    /// rewrite MANIFEST+CURRENT on every poll (10k+ fdatasync barriers per
    /// slipstream guest run during the read legs).
    #[test]
    fn idle_rotate_with_empty_segment_does_not_rewrite_manifest() {
        // `store()` renumbers MANIFEST-NNNNNN and drops older files, so the
        // durable oracle for "a persist happened" is the number CURRENT names.
        let current_manifest_num = |dir: &Path| -> u32 {
            let cur = fs::read_to_string(dir.join(crate::manifest::CURRENT_FILE)).unwrap();
            let name = cur.lines().next().unwrap();
            name.trim_start_matches(crate::manifest::MANIFEST_PREFIX)
                .parse()
                .unwrap_or(0)
        };
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"k", b"v").unwrap();
            db.flush().unwrap();
            // Flush pipeline rotated the non-empty WAL once.
            let after_flush = current_manifest_num(&dir);
            assert!(after_flush >= 1);
            for _ in 0..8 {
                db.try_rotate_wal_if_idle().unwrap();
            }
            assert_eq!(
                current_manifest_num(&dir),
                after_flush,
                "idle polls must not rewrite MANIFEST for an empty segment"
            );
            // A new append re-arms rotation exactly once.
            db.put(b"k2", b"v2").unwrap();
            db.flush().unwrap();
            let after_second_flush = current_manifest_num(&dir);
            assert!(after_second_flush > after_flush);
            for _ in 0..8 {
                db.try_rotate_wal_if_idle().unwrap();
            }
            assert_eq!(current_manifest_num(&dir), after_second_flush);
            db.close().unwrap();
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_gcs_orphan_sst_not_in_manifest() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"keep", b"1").unwrap();
            db.flush().unwrap();
            db.close().unwrap();
        }
        // Plant an orphan SST that is not in MANIFEST.
        let orphan = dir.join("009999.sst");
        fs::write(&orphan, b"not-a-real-sst").unwrap();
        assert!(orphan.exists());

        let db = Db::open(&dir).unwrap();
        assert!(!orphan.exists(), "orphan SST must be GC'd via MANIFEST");
        assert_eq!(db.get(b"keep").as_deref(), Some(b"1".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_batch_atomic_and_snapshot() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            let before = db.snapshot();
            assert_eq!(before.sequence(), 0);
            let last = db
                .apply_batch([
                    BatchOp::put(b"row", b"R"),
                    BatchOp::put(b"idx", b"I"),
                    BatchOp::delete(b"gone"),
                ])
                .unwrap();
            assert_eq!(last, 3);
            let snap = db.snapshot();
            assert_eq!(snap.sequence(), 3);
            assert_eq!(
                db.get_at(snap, b"row").unwrap().as_deref(),
                Some(b"R".as_ref())
            );
            assert_eq!(
                db.get_at(snap, b"idx").unwrap().as_deref(),
                Some(b"I".as_ref())
            );
            // Old snapshot still empty world
            assert_eq!(db.get_at(before, b"row").unwrap(), None);
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"row").as_deref(), Some(b"R".as_ref()));
        assert_eq!(db.get(b"idx").as_deref(), Some(b"I".as_ref()));
        assert_eq!(db.get(b"gone"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wal_recover_from_offset_on_real_file() {
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("export.log");
        let mut offsets = Vec::new();
        {
            let mut wal = Wal::create(&path).unwrap();
            offsets.push(0u64);
            wal.append_record(b"r0").unwrap();
            wal.sync_all().unwrap();
            let o1 = wal.stream_position().unwrap();
            offsets.push(o1);
            wal.append_record(b"r1").unwrap();
            wal.sync_all().unwrap();
            let o2 = wal.stream_position().unwrap();
            offsets.push(o2);
            wal.append_record(b"r2").unwrap();
            wal.sync_all().unwrap();
            wal.close().unwrap();
        }
        let from_o1 = Wal::recover_from_offset(&path, offsets[1]).unwrap();
        assert_eq!(from_o1, vec![b"r1".to_vec(), b"r2".to_vec()]);
        let from_o2 = Wal::recover_from_offset(&path, offsets[2]).unwrap();
        assert_eq!(from_o2, vec![b"r2".to_vec()]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn range_limited_pages_without_full_materialization() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        for i in 0..20u8 {
            let k = [b'k', i];
            db.put(k, b"v").unwrap();
        }
        db.flush().unwrap();
        let page = db.range_limited(Bound::Unbounded, Bound::Unbounded, Some(5));
        assert_eq!(page.len(), 5);
        assert_eq!(page[0].0.as_ref(), b"k\x00");
        assert_eq!(page[4].0.as_ref(), b"k\x04");
        // Bounded range still respects limit.
        let mid = db.range_limited(
            Bound::Included(b"k\x05".as_ref()),
            Bound::Excluded(b"k\x0f".as_ref()),
            Some(3),
        );
        assert_eq!(mid.len(), 3);
        assert_eq!(mid[0].0.as_ref(), b"k\x05");
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stats_and_verify_checksums_on_live_db() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"a", b"1").unwrap();
        db.put(b"b", b"2").unwrap();
        db.flush().unwrap();
        let s = db.stats();
        assert_eq!(s.last_sequence, 2);
        assert_eq!(s.sst_count, 1);
        assert!(s.sst_bytes > 0, "SST file must have on-disk size");
        assert!(s.sst_entries >= 2);
        db.verify_checksums().unwrap();
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn checkpoint_is_openable_and_preserves_keys() {
        let dir = temp_dir();
        let ckpt = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"row", b"R").unwrap();
            db.put(b"idx", b"I").unwrap();
            let meta = db.create_checkpoint(&ckpt).unwrap();
            assert_eq!(meta.last_sequence, 2);
            assert!(meta.sst_count >= 1);
            assert_eq!(meta.earliest_readable_seq, 0);
            assert!(
                ckpt.join(CHECKPOINT_META_FILE).exists(),
                "CHECKPOINT meta file must be written"
            );
            let disk = read_checkpoint_meta(&StdEnv, &ckpt).unwrap();
            assert_eq!(disk, meta);
            db.put(b"after", b"ckpt").unwrap();
            db.close().unwrap();
        }
        // Checkpoint is a full DB directory: open without exclusive steal issues
        // (source dir may still hold LOCK until drop; source is closed).
        let restored = Db::open(&ckpt).unwrap();
        assert_eq!(restored.get(b"row").as_deref(), Some(b"R".as_ref()));
        assert_eq!(restored.get(b"idx").as_deref(), Some(b"I".as_ref()));
        assert_eq!(
            restored.get(b"after"),
            None,
            "writes after checkpoint must not appear in checkpoint"
        );
        restored.verify_checksums().unwrap();
        restored.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ckpt);
    }

    /// RFC-0084 P0: production `create_checkpoint` writes CHECKPOINT;
    /// XOR only the trailer CRC (payload intact). `read_checkpoint_meta`
    /// is crc mismatch. AS-IS would return Ok meta.
    #[test]
    fn crc_mismatch_on_live_checkpoint_meta_is_not_ok() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any checkpoint crc would match"
        );
        let dir = temp_dir();
        let ckpt = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"k", b"v").unwrap();
            db.create_checkpoint(&ckpt).unwrap();
            db.close().unwrap();
        }
        let meta_path = ckpt.join(CHECKPOINT_META_FILE);
        let mut bytes = fs::read(&meta_path).unwrap();
        assert!(bytes.len() >= 12, "CHECKPOINT must have payload + trailer");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        fs::write(&meta_path, &bytes).unwrap();
        let err = match read_checkpoint_meta(&StdEnv, &ckpt) {
            Ok(meta) => {
                let _ = fs::remove_dir_all(&dir);
                let _ = fs::remove_dir_all(&ckpt);
                panic!("CHECKPOINT trailer lie must not load; AS-IS would serve {meta:?}");
            }
            Err(e) => e,
        };
        let msg = err.to_string();
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ckpt);
        assert!(
            msg.to_ascii_lowercase().contains("crc mismatch"),
            "must fail on crc_match_ok, not a payload parse; got {msg}"
        );
    }

    /// RFC-0084 P2.1: checkpoint `crc_match_ok` is not a CRC32C collision theorem.
    #[test]
    fn checkpoint_crc_collision_axiom_remains() {
        assert!(!crate::wal::crc::crc_collision_admitted());
        assert!(
            crate::wal::crc::crc_collision_admitted_as_is(),
            "AS-IS dente: matching CRC looks collision-free"
        );
        assert!(
            crate::wal::crc::crc_match_ok(1, 1),
            "equal u32s still match; that is not R-crc"
        );
        let residuals = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/formal/residuals.json");
        let text = std::fs::read_to_string(&residuals).expect("residuals.json");
        assert!(
            text.contains("\"id\": \"R-crc\""),
            "R-crc must stay in the residual catalog"
        );
        assert!(
            text.contains("\"R-crc\""),
            "never_floor must still list R-crc"
        );
    }

    /// PDBCKP02 carries earliest_readable; checkpoint open restores MANIFEST watermark.
    #[test]
    fn checkpoint_meta_records_gc_watermark() {
        let dir = temp_dir();
        let ckpt = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"k", b"old").unwrap();
            db.flush().unwrap();
            db.put(b"k", b"new").unwrap();
            db.flush().unwrap();
            db.compact_with(CompactOptions::latest_only()).unwrap();
            let floor = db.earliest_readable_sequence();
            assert!(floor > 0);
            let s = db.stats();
            assert_eq!(s.earliest_readable_seq, floor);
            assert!(s.gc_line().contains(&format!("earliest_readable={floor}")));
            let meta = db.create_checkpoint(&ckpt).unwrap();
            assert_eq!(meta.earliest_readable_seq, floor);
            assert_eq!(
                read_checkpoint_meta(&StdEnv, &ckpt)
                    .unwrap()
                    .earliest_readable_seq,
                floor
            );
            db.close().unwrap();
        }
        let restored = Db::open(&ckpt).unwrap();
        assert!(restored.earliest_readable_sequence() > 0);
        assert_eq!(restored.get(b"k").as_deref(), Some(b"new".as_ref()));
        restored.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ckpt);
    }

    #[test]
    fn checkpoint_preserves_large_vlog_values() {
        let dir = temp_dir();
        let ckpt = temp_dir();
        let big = vec![0xABu8; 4096];
        {
            let mut db = Db::open_with(
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
            db.flush().unwrap();
            assert_eq!(db.get(b"huge").as_deref(), Some(big.as_slice()));
            db.create_checkpoint(&ckpt).unwrap();
            db.close().unwrap();
        }
        let restored = Db::open_with(
            &ckpt,
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
        let got = restored.get(b"huge");
        assert_eq!(
            got.as_deref(),
            Some(big.as_slice()),
            "checkpoint must include VALUES.vlog so VLG1 resolves (got {:?})",
            got.as_ref().map(|b| b.len())
        );
        restored.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ckpt);
    }

    /// F44: checkpoint after staged vlog GC (MANIFEST `vlog_use_new`, primary still old)
    /// must ship the live `.new` log (or promote) so remapped SST pointers resolve.
    ///
    /// Layout must **shift**: an orphaned large payload is GC'd so the live record's
    /// offset in `.new` differs from the primary — copying only `VALUES.vlog` then
    /// open+use_new falls back to primary and can silently return the wrong bytes.
    /// CHANGELOG is the durable change-feed cache; checkpoint must copy it so
    /// feed history survives restore after flush (WAL may be empty).
    #[test]
    fn checkpoint_copies_changelog_feed() {
        let dir = temp_dir();
        let ckpt = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"a", b"1").unwrap();
            db.put(b"b", b"2").unwrap();
            db.flush().unwrap();
            assert_eq!(db.changes_after(0).len(), 2);
            assert!(dir.join(crate::change_feed::CHANGELOG_FILE_NAME).exists());
            db.create_checkpoint(&ckpt).unwrap();
            db.close().unwrap();
        }
        assert!(
            ckpt.join(crate::change_feed::CHANGELOG_FILE_NAME).exists(),
            "checkpoint must include CHANGELOG"
        );
        let restored = Db::open(&ckpt).unwrap();
        let feed = restored.changes_after(0);
        assert_eq!(
            feed.len(),
            2,
            "restored feed must keep flushed history, got {feed:?}"
        );
        restored.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ckpt);
    }

    #[test]
    fn checkpoint_mid_vlog_gc_preserves_large_values() {
        let dir = temp_dir();
        let ckpt = temp_dir();
        let dead = vec![0x11u8; 2048];
        let live = vec![0x22u8; 3000];
        {
            let mut db = Db::open_with(
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
            db.put(b"dead", &dead).unwrap();
            db.put(b"live", &live).unwrap();
            db.delete(b"dead").unwrap();
            db.flush().unwrap();
            // Drop dead versions from SST so GC does not copy the orphan payload.
            db.compact_with(CompactOptions::latest_only()).unwrap();
            let stats = db.compact_vlog_stage_manifest().unwrap();
            assert!(db.vlog_use_new, "staged GC must set use_new");
            assert!(
                stats.bytes_after < stats.bytes_before,
                "GC must shrink so live offsets move (before={} after={})",
                stats.bytes_before,
                stats.bytes_after
            );
            assert_eq!(db.get(b"live").as_deref(), Some(live.as_slice()));
            assert!(db.get(b"dead").is_none());
            // Source of the bug: only primary vlog would be copied pre-fix.
            assert!(
                dir.join(crate::vlog::VLOG_NEW_NAME).exists(),
                "staged .new must exist"
            );
            db.create_checkpoint(&ckpt).unwrap();
            db.close().unwrap();
        }
        // Sanity: checkpoint must contain a usable vlog for remapped SSTs.
        assert!(
            ckpt.join(VLOG_FILE_NAME).exists() || ckpt.join(crate::vlog::VLOG_NEW_NAME).exists(),
            "checkpoint missing vlog files"
        );
        let restored = Db::open_with(
            &ckpt,
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
        let got = restored.get(b"live");
        assert_eq!(
            got.as_deref(),
            Some(live.as_slice()),
            "checkpoint mid-vlog-GC must resolve remapped VLG1 (got {:?})",
            got.as_ref().map(|b| (b.len(), b.first().copied()))
        );
        restored.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ckpt);
    }

    #[test]
    fn bloom_skips_absent_keys_across_ssts() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        for i in 0..50u32 {
            let k = format!("present-{i:04}");
            db.put(k.as_bytes(), b"v").unwrap();
        }
        db.flush().unwrap();
        assert_eq!(db.sst_count(), 1);
        // Present key still works through bloom + index.
        assert_eq!(db.get(b"present-0025").as_deref(), Some(b"v".as_ref()));
        // Absent keys must not invent values (bloom may F.P. but get still correct).
        for i in 0..50u32 {
            let k = format!("absent-{i:04}");
            assert_eq!(db.get(k.as_bytes()), None);
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_with_host_std_host_put_get() {
        use crate::host::StdHost;
        let dir = temp_dir();
        let host = StdHost::new();
        let mut db = Db::open_with_host(&dir, OpenOptions::default(), &host).unwrap();
        db.put(b"h", b"1").unwrap();
        assert_eq!(db.get(b"h").as_deref(), Some(b"1".as_ref()));
        db.close().unwrap();
        let db = Db::open_with_host(&dir, OpenOptions::default(), &host).unwrap();
        assert_eq!(db.get(b"h").as_deref(), Some(b"1".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Criterion 1: multi-level LSM after put/flush/compact.
    #[test]
    fn leveled_compaction_produces_multi_level_shape() {
        let dir = temp_dir();
        let mut db = Db::open_with(
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
        )
        .unwrap();
        // Two L0 flushes, then compact L0 → L1.
        db.put(b"a", b"1").unwrap();
        db.flush().unwrap();
        db.put(b"b", b"2").unwrap();
        db.flush().unwrap();
        assert_eq!(db.level_file_count(0), 2);
        db.compact().unwrap();
        assert!(
            db.max_level() >= 1,
            "compact must promote into L1+, max_level={}",
            db.max_level()
        );
        assert_eq!(
            db.level_file_count(0),
            0,
            "L0 should be empty after compact into L1"
        );
        assert!(db.level_file_count(1) >= 1);

        // New flush stays on L0 while L1 holds compacted data → ≥2 levels live.
        db.put(b"c", b"3").unwrap();
        db.flush().unwrap();
        assert!(
            db.level_file_count(0) >= 1 && db.max_level() >= 1,
            "expected L0 + L1+ coexistence: levels={:?}",
            db.sst_levels()
        );
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        assert_eq!(db.get(b"c").as_deref(), Some(b"3".as_ref()));
        let ranged: Vec<_> = db
            .range_limited(Bound::Unbounded, Bound::Unbounded, None)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(ranged.len(), 3);

        // Subset compact: only L0+L1 merge, not whole inventory into one if we add L2.
        db.compact().unwrap(); // L0 → L1 (merge with existing L1)
        db.put(b"d", b"4").unwrap();
        db.flush().unwrap();
        assert_eq!(db.get(b"d").as_deref(), Some(b"4".as_ref()));

        db.close().unwrap();
        // Levels survive reopen via MANIFEST v2.
        let db = Db::open(&dir).unwrap();
        assert!(db.max_level() >= 1 || db.sst_count() >= 1);
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"c").as_deref(), Some(b"3".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Criterion 2: streaming scan over thousands of keys matches model.
    #[test]
    fn streaming_scan_large_keyset_matches_model() {
        const N: u32 = 3000;
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
                sync: false,
                auto_flush_bytes: Some(64 * 1024),
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
                sst_payload_budget_bytes: None,
            },
        )
        .unwrap();
        let mut model = std::collections::BTreeMap::new();
        for i in 0..N {
            let k = format!("k{i:05}");
            let v = format!("v{i}");
            db.put(k.as_bytes(), v.as_bytes()).unwrap();
            model.insert(k, v);
        }
        db.flush().unwrap();

        // Ground truth via model; engine via streaming scan (public path).
        let streamed: Vec<_> = db
            .scan(Bound::Unbounded, Bound::Unbounded)
            .map(|kv| {
                (
                    String::from_utf8(kv.key.to_vec()).unwrap(),
                    String::from_utf8(kv.value.to_vec()).unwrap(),
                )
            })
            .collect();
        assert_eq!(streamed.len(), N as usize);
        for (i, (k, v)) in streamed.iter().enumerate() {
            let (mk, mv) = model.iter().nth(i).unwrap();
            assert_eq!(k, mk);
            assert_eq!(v, mv);
        }
        // Chunked: first page then exclusive continue.
        let page: Vec<_> = db
            .scan_at(
                db.last_sequence(),
                Bound::Unbounded,
                Bound::Unbounded,
                Some(100),
            )
            .collect();
        assert_eq!(page.len(), 100);
        let next_start = page.last().unwrap().key.clone();
        let rest: Vec<_> = db
            .scan(Bound::Excluded(next_start.as_ref()), Bound::Unbounded)
            .collect();
        assert_eq!(rest.len(), (N as usize) - 100);
        // range_at_limited uses the same streaming path.
        let via_range = db.range_limited(Bound::Unbounded, Bound::Unbounded, Some(50));
        assert_eq!(via_range.len(), 50);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Image #1 §3: multi-flush → multi-level shape + streaming scan vs model.
    #[test]
    fn multi_level_large_scan_matches_model() {
        const N: u32 = 2500;
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
                sync: false,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
                sst_payload_budget_bytes: None,
            },
        )
        .unwrap();
        db.bulk_route_enabled = false; // ladder mechanics; bulk would install at MAX
        let mut model = std::collections::BTreeMap::new();
        // Many small flushes to create multiple L0 SSTs, then compact subset to L1.
        for i in 0..N {
            let k = format!("m{i:05}");
            let v = format!("val{i}");
            db.put(k.as_bytes(), v.as_bytes()).unwrap();
            model.insert(k, v);
            if (i + 1) % 200 == 0 {
                db.flush().unwrap();
            }
        }
        db.flush().unwrap();
        assert!(db.sst_count() >= 2, "expected multiple SSTs before compact");
        db.compact().unwrap();
        // After compact L0→L1, new flushes recreate multi-level shape.
        for i in N..N + 100 {
            let k = format!("m{i:05}");
            let v = format!("val{i}");
            db.put(k.as_bytes(), v.as_bytes()).unwrap();
            model.insert(k, v);
        }
        db.flush().unwrap();
        assert!(
            db.max_level() >= 1 && db.level_file_count(0) >= 1,
            "need L0+L1 coexistence: levels={:?} max={}",
            db.sst_levels(),
            db.max_level()
        );
        // Compact is subset (levels remain; not necessarily single SST forever).
        assert!(
            db.sst_count() >= 2,
            "multi-level inventory should keep ≥2 files"
        );

        let streamed: Vec<_> = db
            .scan(Bound::Unbounded, Bound::Unbounded)
            .map(|kv| {
                (
                    String::from_utf8(kv.key.to_vec()).unwrap(),
                    String::from_utf8(kv.value.to_vec()).unwrap(),
                )
            })
            .collect();
        assert_eq!(streamed.len(), model.len());
        for ((k, v), (mk, mv)) in streamed.iter().zip(model.iter()) {
            assert_eq!(k, mk);
            assert_eq!(v, mv);
        }
        // Live keys only: delete + compact latest_only drops covered points.
        db.delete(b"m00000").unwrap();
        model.remove("m00000");
        db.flush().unwrap();
        db.compact_with(CompactOptions::latest_only()).unwrap();
        assert_eq!(db.get(b"m00000"), None);
        let after: Vec<_> = db.scan(Bound::Unbounded, Bound::Unbounded).collect();
        assert_eq!(after.len(), model.len());
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Criterion 4: table cache hits on second open; SST is lz4-compressed (v4).
    #[test]
    fn table_cache_and_compressed_sst_round_trip() {
        let dir = temp_dir();
        {
            let mut db = Db::open_with(
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
            )
            .unwrap();
            // Compressible payload.
            for i in 0..200u32 {
                let k = format!("ck{i:04}");
                db.put(k.as_bytes(), vec![b'Z'; 128]).unwrap();
            }
            db.flush().unwrap();
            let path = db.ssts[0].path().to_path_buf();
            let on_disk = fs::metadata(&path).unwrap().len();
            // Raw entries would be >> compressed for highly redundant values.
            assert!(on_disk > 0);
            db.table_cache.reset_stats();
            // First get_or_open of path after reset: may already be in cache from flush insert.
            let _ = db.table_cache.get_or_open(&db.env, &path).unwrap();
            let hits_before = db.table_cache.hits();
            let _ = db.table_cache.get_or_open(&db.env, &path).unwrap();
            assert!(
                db.table_cache.hits() > hits_before,
                "second open must be a table-cache hit"
            );
            assert_eq!(
                db.get(b"ck0001").as_deref(),
                Some(vec![b'Z'; 128].as_slice())
            );
            db.close().unwrap();
        }
        // Reopen: compressed SST v4 still readable.
        let db = Db::open(&dir).unwrap();
        assert_eq!(
            db.get(b"ck0001").as_deref(),
            Some(vec![b'Z'; 128].as_slice())
        );
        assert_eq!(
            db.get(b"ck0199").as_deref(),
            Some(vec![b'Z'; 128].as_slice())
        );
        // F217: verify must re-read the file from disk — the table cache is
        // primed by the very installs that create each table (flush, compact,
        // …), so serving verify from the cache would hide in-process bitrot
        // on a live file. Verify never consults the cache.
        db.table_cache.reset_stats();
        db.verify_checksums().unwrap();
        db.verify_checksums().unwrap();
        assert_eq!(
            db.table_cache.hits(),
            0,
            "verify must not serve SSTs from the table cache"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Criterion 5a: range delete hides interval; compact drops covered keys.
    #[test]
    fn delete_range_hides_interval_and_compact_gcs() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        for c in b'a'..=b'f' {
            db.put([c], b"v").unwrap();
        }
        db.flush().unwrap();
        db.delete_range(b"b", b"e").unwrap(); // [b,e) → hides b,c,d
        assert_eq!(db.get(b"a").as_deref(), Some(b"v".as_ref()));
        assert_eq!(db.get(b"b"), None);
        assert_eq!(db.get(b"c"), None);
        assert_eq!(db.get(b"d"), None);
        assert_eq!(db.get(b"e").as_deref(), Some(b"v".as_ref()));
        assert_eq!(db.get(b"f").as_deref(), Some(b"v".as_ref()));
        let live: Vec<_> = db
            .range_limited(Bound::Unbounded, Bound::Unbounded, None)
            .into_iter()
            .map(|(k, _)| k[0])
            .collect();
        assert_eq!(live, vec![b'a', b'e', b'f']);

        db.flush().unwrap();
        db.compact_with(CompactOptions::latest_only()).unwrap();
        assert_eq!(db.get(b"b"), None);
        assert_eq!(db.get(b"a").as_deref(), Some(b"v".as_ref()));
        // After latest_only GC, covered keys should not remain as live values.
        let after: Vec<_> = db.range_limited(Bound::Unbounded, Bound::Unbounded, None);
        assert_eq!(after.len(), 3);
        db.close().unwrap();
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"c"), None);
        assert_eq!(db.get(b"e").as_deref(), Some(b"v".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Criterion 5c: verify fails closed on bitflip of SST.
    #[test]
    fn verify_checksums_fails_closed_on_sst_bitflip() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"k", b"v").unwrap();
            db.flush().unwrap();
            db.close().unwrap();
        }
        // Corrupt the SST file on disk.
        let sst = fs::read_dir(&dir)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "sst"))
            .expect("sst file");
        let mut bytes = fs::read(&sst).unwrap();
        assert!(bytes.len() > 20);
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xff;
        fs::write(&sst, &bytes).unwrap();

        // Open may fail on CRC, or succeed if we don't re-read — open re-reads SST.
        match Db::open(&dir) {
            Err(_) => {
                // Fail-closed on open is fine.
            }
            Ok(db) => {
                let v = db.verify_checksums();
                assert!(v.is_err(), "verify must fail closed on corrupted SST");
                db.close().unwrap();
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0082 P0: production flush writes CURRENT+MANIFEST. Lie only on
    /// the CURRENT CRC line (`ffffffff`) with the MANIFEST bytes intact so
    /// decode still parses. `crc_match_ok` on `load` is Err; AS-IS would
    /// admit and `Db::open` would serve `k`.
    #[test]
    fn crc_mismatch_on_live_manifest_is_not_ok() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any CURRENT crc would match"
        );
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"k", b"v").unwrap();
            db.flush().unwrap();
            db.close().unwrap();
        }
        let current_path = dir.join(crate::manifest::CURRENT_FILE);
        let current = fs::read_to_string(&current_path).unwrap();
        let man_name = current.lines().next().unwrap_or("").trim();
        assert!(
            current.lines().count() >= 2,
            "production CURRENT must carry a CRC line, got {current:?}"
        );
        assert!(
            dir.join(man_name).is_file(),
            "MANIFEST named by CURRENT must stay intact"
        );
        fs::write(&current_path, format!("{man_name}\nffffffff\n")).unwrap();
        let err = match Db::open(&dir) {
            Ok(db) => {
                let served = db.get(b"k");
                let _ = db.close();
                let _ = fs::remove_dir_all(&dir);
                panic!("CURRENT crc lie must not open; AS-IS would serve k={served:?}");
            }
            Err(e) => e,
        };
        let msg = err.to_string();
        let _ = fs::remove_dir_all(&dir);
        assert!(
            msg.contains("crc mismatch"),
            "must fail on CURRENT crc_match_ok, not a parse error; got {msg}"
        );
    }

    /// RFC-0060 P2.15: live `verify_checksums` re-reads CURRENT (via
    /// `manifest::load`) and fails closed on a CRC mismatch.
    #[test]
    fn verify_checksums_fails_on_current_crc_mismatch() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"k", b"v").unwrap();
        db.flush().unwrap();
        db.verify_checksums().unwrap();
        let body = fs::read(dir.join(crate::manifest::CURRENT_FILE)).unwrap();
        let (name, _) =
            crate::manifest::parse_current_pointer(&String::from_utf8(body).unwrap()).unwrap();
        fs::write(
            dir.join(crate::manifest::CURRENT_FILE),
            format!("{name}\nffffffff\n"),
        )
        .unwrap();
        let err = db.verify_checksums().unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("crc mismatch") || msg.contains("CURRENT"),
            "got {msg}"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// F29 regression: multi-version flush into multi-block SST must return latest.
    #[test]
    fn multi_version_large_memtable_point_lookup() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        let mut latest = std::collections::HashMap::new();
        for i in 0..200u32 {
            for kid in 0..5u32 {
                let k = format!("k{kid:04}");
                let val = format!("v{i:05}-{kid}").repeat(20);
                db.put(k.as_bytes(), val.as_bytes()).unwrap();
                latest.insert(k, val);
            }
        }
        db.flush().unwrap();
        for (k, v) in &latest {
            let got = db.get(k.as_bytes());
            assert_eq!(
                got.as_deref(),
                Some(v.as_bytes()),
                "key={k} after multi-version flush"
            );
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// P3.5 metamorphic: same puts; auto-compact vs manual compact → same live map.
    #[test]
    fn metamorphic_auto_compact_vs_manual_same_logical_state() {
        fn run(dir: &std::path::Path, auto: bool) -> Vec<(Vec<u8>, Vec<u8>)> {
            let mut db = Db::open_with(
                dir,
                OpenOptions {
                    wal_full_fsync: true,
                    history: Default::default(),
                    wal_recovery: Default::default(),
                    sync: true,
                    auto_flush_bytes: Some(64),
                    auto_compact_sst_count: if auto { Some(2) } else { None },
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                    sst_payload_budget_bytes: None,
                },
            )
            .unwrap();
            // Deterministic workload: puts, overwrites, deletes.
            for i in 0..40u8 {
                let k = [b'k', i % 20];
                let v = [b'v', i];
                db.put(k, v).unwrap();
            }
            for i in 0..10u8 {
                db.delete([b'k', i]).unwrap();
            }
            for i in 10..20u8 {
                db.put([b'k', i], [b'z', i]).unwrap();
            }
            db.flush().unwrap();
            if auto {
                for i in 0..8u8 {
                    db.put([b'm', i], [b'n', i]).unwrap();
                }
                db.flush().unwrap();
                // Auto-compact may already have run; still ok if more SSTs remain.
            } else {
                // Force several SST files then one compact.
                for i in 0..8u8 {
                    db.put([b'm', i], [b'n', i]).unwrap();
                    db.flush().unwrap();
                }
                let _ = db.compact();
            }
            let mut out: Vec<(Vec<u8>, Vec<u8>)> = db
                .range_limited(Bound::Unbounded, Bound::Unbounded, None)
                .into_iter()
                .map(|(k, v)| (k.to_vec(), v.to_vec()))
                .collect();
            // Also compare scan path (streaming heap-merge).
            let via_scan: Vec<(Vec<u8>, Vec<u8>)> = db
                .scan(Bound::Unbounded, Bound::Unbounded)
                .map(|kv| (kv.key.to_vec(), kv.value.to_vec()))
                .collect();
            assert_eq!(out, via_scan, "range vs scan must agree");
            db.close().unwrap();
            out.sort();
            out
        }

        let d1 = temp_dir();
        let d2 = temp_dir();
        let a = run(&d1, true);
        let b = run(&d2, false);
        assert_eq!(
            a, b,
            "metamorphic: auto-compact vs manual compact must yield same live key/value map"
        );
        // Sanity: deleted keys gone, overwrites present.
        assert!(a.iter().all(|(k, _)| k[0] != b'k' || k[1] >= 10));
        assert!(a
            .iter()
            .any(|(k, v)| k == b"k\x0f".as_slice() && v[0] == b'z'));
        let _ = fs::remove_dir_all(&d1);
        let _ = fs::remove_dir_all(&d2);
    }

    // -------------------------------------------------------------------------
    // RFC-0019 — CAS, seq pin, change feed, multi_get, KeyOnly, apply soak
    // -------------------------------------------------------------------------

    #[test]
    fn rfc19_put_if_absent_eq_and_cas_mismatch() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();

        let s1 = db.put_if_absent(b"lease", b"holder-a").unwrap();
        assert!(s1 >= 1);
        assert_eq!(db.get(b"lease").as_deref(), Some(b"holder-a".as_ref()));

        // Second absent insert must fail closed.
        assert!(matches!(
            db.put_if_absent(b"lease", b"holder-b"),
            Err(CoreError::CasMismatch)
        ));
        assert_eq!(db.get(b"lease").as_deref(), Some(b"holder-a".as_ref()));

        // Wrong expected value.
        assert!(matches!(
            db.put_if_eq(b"lease", b"wrong", b"holder-c"),
            Err(CoreError::CasMismatch)
        ));

        // Correct CAS.
        let s2 = db
            .compare_and_swap(b"lease", b"holder-a", b"holder-c")
            .unwrap();
        assert!(s2 > s1);
        assert_eq!(db.get(b"lease").as_deref(), Some(b"holder-c".as_ref()));

        // After delete, absent succeeds again.
        db.delete(b"lease").unwrap();
        let s3 = db.put_if_absent(b"lease", b"holder-d").unwrap();
        assert!(s3 > s2);
        assert_eq!(db.get(b"lease").as_deref(), Some(b"holder-d".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rfc19_cas_winner_survives_crash_reopen() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put_if_absent(b"k", b"win").unwrap();
            assert!(matches!(
                db.put_if_absent(b"k", b"lose"),
                Err(CoreError::CasMismatch)
            ));
            db.close().unwrap();
        }
        {
            let db = Db::open(&dir).unwrap();
            assert_eq!(
                db.get(b"k").as_deref(),
                Some(b"win".as_ref()),
                "only CAS winner must recover"
            );
            db.close().unwrap();
        }
        {
            let mut db = Db::open(&dir).unwrap();
            assert!(matches!(
                db.put_if_absent(b"k", b"other"),
                Err(CoreError::CasMismatch)
            ));
            db.close().unwrap();
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rfc19_seq_pin_get_at_bounds() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();

        let seq = db.put_with_seq(b"k", b"v1").unwrap();
        assert_eq!(
            db.get_at(Snapshot::at(seq), b"k").unwrap().as_deref(),
            Some(b"v1".as_ref())
        );
        if seq > 0 {
            assert_eq!(
                db.get_at(Snapshot::at(seq - 1), b"k").unwrap(),
                None,
                "seq-1 must not see the put"
            );
        }

        let del_seq = db.delete_with_seq(b"k").unwrap();
        assert!(del_seq > seq);
        assert_eq!(db.get_at(Snapshot::at(del_seq), b"k").unwrap(), None);
        assert_eq!(
            db.get_at(Snapshot::at(seq), b"k").unwrap().as_deref(),
            Some(b"v1".as_ref()),
            "historical snapshot still sees pre-delete put"
        );

        let last = db
            .apply_batch([BatchOp::put(b"a", b"1"), BatchOp::put(b"b", b"2")])
            .unwrap();
        assert_eq!(
            db.get_at(Snapshot::at(last), b"a").unwrap().as_deref(),
            Some(b"1".as_ref())
        );
        assert_eq!(
            db.get_at(Snapshot::at(last), b"b").unwrap().as_deref(),
            Some(b"2".as_ref())
        );
        // First key of batch is last-1 when two ops.
        assert_eq!(
            db.get_at(Snapshot::at(last - 1), b"b").unwrap(),
            None,
            "second batch key not visible before its seq"
        );

        let mut tx = db.begin();
        tx.put(b"t1", b"x").unwrap();
        tx.put(b"t2", b"y").unwrap();
        let tx_seq = tx.commit().unwrap();
        assert_eq!(
            db.get_at(Snapshot::at(tx_seq), b"t2").unwrap().as_deref(),
            Some(b"y".as_ref())
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rfc19_change_feed_puts_deletes_reopen_pin() {
        let dir = temp_dir();
        let pin_after_first;
        {
            let mut db = Db::open(&dir).unwrap();
            let s1 = db.put_with_seq(b"a", b"1").unwrap();
            let s2 = db.put_with_seq(b"b", b"2").unwrap();
            let s3 = db.delete_with_seq(b"a").unwrap();
            pin_after_first = s3;

            let all = db.changes(0, s3).unwrap();
            assert_eq!(all.len(), 3);
            assert_eq!(all[0].sequence, s1);
            assert_eq!(all[0].kind, crate::ChangeKind::Put);
            assert_eq!(all[0].key.as_ref(), b"a");
            assert_eq!(all[1].sequence, s2);
            assert_eq!(all[2].sequence, s3);
            assert_eq!(all[2].kind, crate::ChangeKind::Delete);

            // No ghost seq beyond durable last.
            let tail = db.changes_after(s3);
            assert!(tail.is_empty());
            assert!(db.changes(s3, s3 + 100).unwrap().is_empty());

            // Mid multi-key: feed shows both or neither after commit.
            let mut tx = db.begin();
            tx.put(b"x", b"X").unwrap();
            tx.put(b"y", b"Y").unwrap();
            let end = tx.commit().unwrap();
            let batch = db.changes(s3, end).unwrap();
            assert_eq!(batch.len(), 2);
            assert!(batch.iter().all(|e| e.kind == crate::ChangeKind::Put));
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        let after = db.changes_after(pin_after_first);
        assert_eq!(after.len(), 2, "reopen continues feed after pin");
        assert_eq!(db.get(b"a"), None);
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        assert_eq!(db.get(b"x").as_deref(), Some(b"X".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rfc19_multi_get_parity_and_keyonly_scan() {
        use std::ops::Bound;
        let dir = temp_dir();
        let mut db = Db::open_with(&dir, vlog_opts()).unwrap();
        let big = vec![0xABu8; 2048];
        db.put(b"k0", b"v0").unwrap();
        db.put(b"k1", &big).unwrap();
        db.put(b"k2", b"v2").unwrap();
        db.delete(b"k0").unwrap();

        let keys: [&[u8]; 4] = [b"k0", b"k1", b"k2", b"missing"];
        let multi = db.multi_get(&keys);
        let sequential: Vec<_> = keys.iter().map(|k| db.get(k)).collect();
        assert_eq!(multi, sequential);
        assert_eq!(multi[0], None);
        assert_eq!(multi[1].as_deref(), Some(big.as_slice()));
        assert_eq!(multi[2].as_deref(), Some(b"v2".as_ref()));
        assert_eq!(multi[3], None);

        let snap = Snapshot::at(db.last_sequence());
        let multi_at = db.multi_get_at(snap, &keys).unwrap();
        assert_eq!(multi_at, multi);

        let full_keys: Vec<_> = db
            .scan(Bound::Unbounded, Bound::Unbounded)
            .map(|kv| kv.key)
            .collect();
        let only_keys: Vec<_> = db
            .scan_projected(Bound::Unbounded, Bound::Unbounded, ScanProjection::KeyOnly)
            .map(|kv| {
                assert!(kv.value.is_empty(), "KeyOnly must not load values");
                kv.key
            })
            .collect();
        assert_eq!(full_keys, only_keys);
        assert!(only_keys.iter().any(|k| k.as_ref() == b"k1"));
        assert!(!only_keys.iter().any(|k| k.as_ref() == b"k0"));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0019 P1.3: concurrent apply_batch under load; silent_wrong=0; record syncs.
    #[test]
    fn rfc19_apply_soak_group_commit_evidence() {
        use crate::rng::{Rng, SeedRng};
        use std::collections::HashMap;
        use std::sync::Arc;
        use std::thread;

        let dir = temp_dir();
        let db = Arc::new(
            crate::ConcurrentDb::open_with(
                &dir,
                OpenOptions {
                    wal_full_fsync: true,
                    history: Default::default(),
                    wal_recovery: Default::default(),
                    sync: true,
                    auto_flush_bytes: Some(8 * 1024),
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                    sst_payload_budget_bytes: None,
                },
            )
            .unwrap(),
        );

        let silent_wrong = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let mut handles = Vec::new();

        for t in 0..4u8 {
            let db = Arc::clone(&db);
            let silent_wrong = Arc::clone(&silent_wrong);
            handles.push(thread::spawn(move || {
                // Each thread owns a disjoint key prefix — no cross-thread model races.
                let rng = SeedRng::new(0x0019_9000 + u64::from(t));
                let mut model: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
                for step in 0..80u64 {
                    let k = format!("t{t}-k{:02}", rng.next_u64() % 16);
                    let key = k.as_bytes().to_vec();
                    let op = rng.next_u64() % 10;
                    if op < 6 {
                        let val = format!("v{t}-{step}").into_bytes();
                        let batch = vec![
                            BatchOp::put(key.clone(), val.clone()),
                            BatchOp::put(format!("idx-{t}-{}", step % 4), key.clone()),
                        ];
                        match db.apply_batch(batch) {
                            Ok(_) => {
                                model.insert(key, val);
                            }
                            Err(_) => {}
                        }
                    } else if op < 8 {
                        if db.delete(&key).is_ok() {
                            model.remove(&key);
                        }
                    } else {
                        let got = db.get(&key);
                        let expect = model.get(&key).map(Vec::as_slice);
                        if got.as_deref() != expect {
                            silent_wrong.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                }
                // Final per-thread check.
                for (k, v) in &model {
                    if db.get(k).as_deref() != Some(v.as_slice()) {
                        silent_wrong.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let syncs = db.wal_sync_count();
        let wrong = silent_wrong.load(std::sync::atomic::Ordering::Relaxed);
        assert_eq!(wrong, 0, "apply soak silent_wrong must be 0");
        assert!(
            syncs > 0,
            "group-commit soak must record wal_sync_count > 0 (got {syncs})"
        );
        eprintln!(
            "rfc19_apply_soak_group_commit_evidence wal_sync_count={syncs} silent_wrong={wrong}"
        );
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0031 P0.1: N-1 durable commits do not persist the CHANGELOG cache;
    /// reopen still sees every acked write (WAL is the durability source).
    #[test]
    fn changelog_debounce_n_minus_one_reopen_equivalent() {
        let dir = temp_dir();
        let chlog = dir.join(crate::change_feed::CHANGELOG_FILE_NAME);
        {
            let mut db = Db::open_with(
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
            )
            .unwrap();
            db.set_changelog_interval(64);
            for i in 0..63u8 {
                db.put([b'k', i], [b'v', i]).unwrap();
            }
            assert_eq!(db.changelog_store_count(), 0);
            assert!(
                !chlog.exists(),
                "CHANGELOG must stay absent before the interval fires"
            );
            // In-process feed is complete (G7: read-your-writes does not need disk).
            assert_eq!(db.changes_after(0).len(), 63);
            // Drop without close — no persist point. WAL has every Ok write (G1).
        }
        let db = Db::open_with(
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
        )
        .unwrap();
        for i in 0..63u8 {
            assert_eq!(
                db.get(&[b'k', i]).as_deref(),
                Some([b'v', i].as_slice()),
                "reopen must recover key {i} from WAL"
            );
        }
        assert_eq!(db.changes_after(0).len(), 63);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0031 P0.1: the N-th durable commit persists the cache.
    #[test]
    fn changelog_debounce_nth_commit_stores() {
        let dir = temp_dir();
        let chlog = dir.join(crate::change_feed::CHANGELOG_FILE_NAME);
        let mut db = Db::open_with(
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
        )
        .unwrap();
        db.set_changelog_interval(8);
        for i in 0..7u8 {
            db.put([b'k', i], [b'v', i]).unwrap();
        }
        assert_eq!(db.changelog_store_count(), 0);
        assert!(!chlog.exists());
        db.put(b"k7", b"v7").unwrap();
        assert_eq!(db.changelog_store_count(), 1);
        assert!(chlog.exists());
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0031 P0.1: interval 0 never stores on the commit path; flush and
    /// close are persist points (WAL-truncate / operator close).
    #[test]
    fn changelog_interval_zero_stores_on_flush_and_close() {
        let dir = temp_dir();
        let chlog = dir.join(crate::change_feed::CHANGELOG_FILE_NAME);
        {
            let mut db = Db::open_with(
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
            )
            .unwrap();
            db.set_changelog_interval(0);
            for i in 0..32u8 {
                db.put([b'k', i], [b'v', i]).unwrap();
            }
            assert_eq!(db.changelog_store_count(), 0);
            assert!(!chlog.exists());
            assert_eq!(
                db.changes_after(0).len(),
                32,
                "lazy feed still answers last-per-key from mem"
            );
            db.flush().unwrap();
            assert!(
                db.changelog_store_count() >= 1,
                "flush must persist CHANGELOG before WAL rotate"
            );
            assert!(chlog.exists());
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(&[b'k', 0]).as_deref(), Some([b'v', 0].as_slice()));
        assert_eq!(db.get(&[b'k', 31]).as_deref(), Some([b'v', 31].as_slice()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0039 P0.3 / RFC-0041 P1.1: above the lazy rebuild budget the
    /// explicit flush must not materialize the live set into the CHANGELOG
    /// cache (~3 live-set copies — the 25M guest settle OOM: killed at
    /// 3.3 GB RSS for a 0.61 GiB store). The cache stays stale; the feed is
    /// still served by the on-demand live rebuild.
    #[test]
    fn changelog_lazy_rebuild_skips_above_budget() {
        let dir = temp_dir();
        let chlog = dir.join(crate::change_feed::CHANGELOG_FILE_NAME);
        {
            let mut db = Db::open(&dir).unwrap();
            db.set_changelog_interval(0);
            db.set_changelog_rebuild_budget_entries(4);
            for i in 0..5u8 {
                db.put([b'k', i], [b'v', i]).unwrap();
            }
            db.flush().unwrap();
            let persisted = fs::metadata(&chlog).map(|m| m.len()).unwrap_or(0);
            assert!(
                persisted < 100,
                "above-budget flush must not materialize the feed, got {persisted} B"
            );
            assert_eq!(
                db.changes_after(0).len(),
                5,
                "lazy feed still answers last-per-key from the live set"
            );
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(&[b'k', 0]).as_deref(), Some([b'v', 0].as_slice()));
        assert_eq!(db.get(&[b'k', 4]).as_deref(), Some([b'v', 4].as_slice()));
        assert_eq!(
            db.changes_after(0).len(),
            5,
            "reopen rebuilds the feed on demand from the live set"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Within the budget the explicit flush keeps materializing and
    /// persisting the feed (fast reopen path unchanged).
    #[test]
    fn changelog_lazy_rebuild_within_budget_persists_feed() {
        let dir = temp_dir();
        let chlog = dir.join(crate::change_feed::CHANGELOG_FILE_NAME);
        {
            let mut db = Db::open(&dir).unwrap();
            db.set_changelog_interval(0);
            for i in 0..3u8 {
                db.put([b'k', i], [b'v', i]).unwrap();
            }
            db.flush().unwrap();
            assert!(chlog.is_file(), "within budget the flush persists the feed");
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.changes_after(0).len(), 3);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// F-found (World seed 502514): after a flush the lazy CHANGELOG cache
    /// is last-per-key. A later delete+restore of one key must still show
    /// Put as that key's latest even when other keys have a higher sequence.
    #[test]
    fn lazy_feed_restore_after_delete_survives_other_keys_higher_seq() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.set_changelog_interval(0);
        db.put(b"a", b"1").unwrap();
        db.put(b"z", b"1").unwrap();
        db.flush().unwrap();
        db.delete(b"a").unwrap();
        for i in 0..8u8 {
            db.put(b"z", [i]).unwrap();
        }
        db.put(b"a", b"2").unwrap();
        assert_eq!(db.get(b"a").as_deref(), Some(&b"2"[..]));
        let a: Vec<_> = db
            .changes_after(0)
            .into_iter()
            .filter(|e| e.key.as_ref() == b"a")
            .collect();
        assert!(
            !a.is_empty() && matches!(a.last().unwrap().kind, crate::ChangeKind::Put),
            "restore must be changelog-latest, got {a:?}"
        );
        assert_eq!(a.last().unwrap().value.as_ref(), b"2");
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0036: auto-flush with interval 0 must not rewrite CHANGELOG (apply tail).
    /// Keys stay visible; reopen rebuilds from SST if the cache is absent.
    #[test]
    fn auto_flush_interval_zero_skips_changelog_store() {
        let dir = temp_dir();
        let chlog = dir.join(crate::change_feed::CHANGELOG_FILE_NAME);
        {
            let mut db = Db::open_with(
                &dir,
                OpenOptions {
                    wal_full_fsync: true,
                    history: Default::default(),
                    wal_recovery: Default::default(),
                    sync: true,
                    auto_flush_bytes: Some(256),
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                    sst_payload_budget_bytes: None,
                },
            )
            .unwrap();
            db.set_changelog_interval(0);
            for i in 0..64u8 {
                db.put([b'k', i], vec![i; 64]).unwrap();
            }
            assert!(db.sst_count() >= 1, "auto-flush must have written SST");
            assert_eq!(
                db.changelog_store_count(),
                0,
                "auto-flush must not persist CHANGELOG when interval is 0"
            );
            assert!(!chlog.exists());
            assert_eq!(db.get(&[b'k', 0]).as_deref(), Some(vec![0; 64].as_slice()));
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(&[b'k', 0]).as_deref(), Some(vec![0; 64].as_slice()));
        assert_eq!(
            db.get(&[b'k', 63]).as_deref(),
            Some(vec![63; 64].as_slice())
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0031 P0.1: close persists even when the interval has not fired.
    #[test]
    fn changelog_close_persists_mid_debounce() {
        let dir = temp_dir();
        let chlog = dir.join(crate::change_feed::CHANGELOG_FILE_NAME);
        {
            let mut db = Db::open_with(
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
            )
            .unwrap();
            db.set_changelog_interval(64);
            db.put(b"a", b"1").unwrap();
            assert!(!chlog.exists());
            db.close().unwrap();
        }
        assert!(chlog.exists(), "close must persist the CHANGELOG cache");
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0033: last_under_prefix is the latest user key under the prefix,
    /// not a neighbour, and matches `lookup` visibility (tombstone / snapshot).
    #[test]
    fn last_under_prefix_versions_and_tombstone() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        for user in 0..8u32 {
            for ver in 1..=3u64 {
                let mut k = format!("u/{user:02}").into_bytes();
                k.extend_from_slice(&ver.to_be_bytes());
                db.put(&k, format!("v{ver}").as_bytes()).unwrap();
            }
        }
        let snap = db.last_sequence();
        let last = db
            .last_under_prefix(snap, b"u/03")
            .unwrap()
            .expect("user 03");
        assert!(last.starts_with(b"u/03"), "{last:?}");
        assert!(!last.starts_with(b"u/04"), "must not leak neighbour");
        assert_eq!(&last[last.len() - 8..], &3u64.to_be_bytes());

        // Delete the latest version of u/03; previous version remains.
        db.delete(&last).unwrap();
        let snap2 = db.last_sequence();
        let prev = db
            .last_under_prefix(snap2, b"u/03")
            .unwrap()
            .expect("older version");
        assert_eq!(&prev[prev.len() - 8..], &2u64.to_be_bytes());
        // Snapshot mid: still sees the deleted latest.
        let at_old = db
            .last_under_prefix(snap, b"u/03")
            .unwrap()
            .expect("pinned");
        assert_eq!(at_old, last);

        db.flush().unwrap();
        let after_flush = db
            .last_under_prefix(db.last_sequence(), b"u/03")
            .unwrap()
            .expect("sst");
        assert_eq!(after_flush, prev);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_under_prefix_range_tombstone_skips_tail() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"p/a", b"1").unwrap();
        db.put(b"p/b", b"2").unwrap();
        db.put(b"p/c", b"3").unwrap();
        db.put(b"p/d", b"4").unwrap();
        db.flush().unwrap();
        db.delete_range(b"p/c", b"p/z").unwrap();
        let last = db
            .last_under_prefix(db.last_sequence(), b"p/")
            .unwrap()
            .expect("live tail");
        assert_eq!(&last[..], b"p/b");
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn try_scan_at_limit_matches_prefix_of_unlimited() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        for i in 0..80u32 {
            db.put(format!("k{i:03}").as_bytes(), b"v").unwrap();
        }
        db.flush().unwrap();
        let snap = db.last_sequence();
        let all: Vec<_> = db
            .try_scan_at(snap, Bound::Unbounded, Bound::Unbounded, None)
            .unwrap()
            .map(|kv| kv.key)
            .collect();
        let limited: Vec<_> = db
            .try_scan_at(snap, Bound::Unbounded, Bound::Unbounded, Some(25))
            .unwrap()
            .map(|kv| kv.key)
            .collect();
        assert_eq!(limited.len(), 25);
        assert_eq!(limited, all[..25]);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_skips_older_versions_still_sees_later_users() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        for u in 0..8u8 {
            for ver in 1..=10u8 {
                db.put(format!("u/{u:02}").as_bytes(), [ver]).unwrap();
            }
        }
        db.flush().unwrap();
        db.delete(b"u/03").unwrap();
        let got: Vec<_> = db
            .try_scan_at(
                db.last_sequence(),
                Bound::Included(b"u/00"),
                Bound::Excluded(b"u/08"),
                None,
            )
            .unwrap()
            .map(|kv| (kv.key, kv.value))
            .collect();
        assert_eq!(got.len(), 7, "{got:?}");
        assert_eq!(&got[0].0[..], b"u/00");
        assert_eq!(&got[0].1[..], &[10]);
        assert!(got.iter().all(|(k, _)| k.as_ref() != b"u/03"));
        assert_eq!(&got.last().expect("last").0[..], b"u/07");
        let n = db
            .count_in_range(
                db.last_sequence(),
                Bound::Included(b"u/00"),
                Bound::Excluded(b"u/08"),
                None,
            )
            .unwrap();
        assert_eq!(n, got.len());
        let capped = db
            .count_in_range(
                db.last_sequence(),
                Bound::Included(b"u/00"),
                Bound::Excluded(b"u/08"),
                Some(3),
            )
            .unwrap();
        assert_eq!(capped, 3);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0044 `ycsb-longwindow`: count answers are window-scoped — writes
    /// outside the window must not cold the cache (the old wholesale
    /// `clear()` per publish made ycsb_e's 5% inserts cold every ~19 scans).
    #[test]
    fn count_cache_survives_writes_outside_window() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        let win = |db: &Db| {
            db.count_in_range(
                db.visible_sequence(),
                Bound::Included(&b"u/00"[..]),
                Bound::Excluded(&b"u/10"[..]),
                None,
            )
            .unwrap()
        };
        for i in 0..10u32 {
            db.put(format!("u/{i:02}").as_bytes(), b"v").unwrap();
        }
        assert_eq!(win(&db), 10);
        assert_eq!(win(&db), 10); // second read takes the cache path
                                  // ycsb_e shape: inserts land above every scanned window.
        for i in 0..2000u32 {
            db.put(format!("u/9{i:03}").as_bytes(), b"v").unwrap();
        }
        assert_eq!(win(&db), 10);
        // New key inside the window invalidates (count grows).
        db.put(b"u/05a", b"v").unwrap();
        assert_eq!(win(&db), 11);
        // Point delete inside the window invalidates (count shrinks).
        db.delete(b"u/05a").unwrap();
        assert_eq!(win(&db), 10);
        // Range deletion is untrackable: wholesale clear, still correct.
        db.delete_range(b"u/05", b"u/06").unwrap();
        assert_eq!(win(&db), 9);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0033 P0.3: `limit` must cut SST block decodes, not only emit.
    #[test]
    fn try_scan_at_limit_decodes_fewer_sst_blocks() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        let payload = vec![b'x'; 256];
        for i in 0..80u32 {
            db.put(format!("k{i:03}").as_bytes(), &payload).unwrap();
        }
        db.flush().unwrap();
        let snap = db.last_sequence();
        db.block_cache.clear();
        crate::sst::reset_sst_blocks_decoded();
        let all: Vec<_> = db
            .try_scan_at(snap, Bound::Unbounded, Bound::Unbounded, None)
            .unwrap()
            .collect();
        let decoded_all = crate::sst::sst_blocks_decoded();
        assert!(
            decoded_all >= 2,
            "need a multi-block SST, decoded {decoded_all}"
        );
        assert_eq!(all.len(), 80);

        db.block_cache.clear();
        crate::sst::reset_sst_blocks_decoded();
        let limited: Vec<_> = db
            .try_scan_at(snap, Bound::Unbounded, Bound::Unbounded, Some(5))
            .unwrap()
            .collect();
        let decoded_lim = crate::sst::sst_blocks_decoded();
        assert_eq!(limited.len(), 5);
        assert_eq!(
            limited.iter().map(|kv| kv.key.clone()).collect::<Vec<_>>(),
            all[..5].iter().map(|kv| kv.key.clone()).collect::<Vec<_>>()
        );
        assert!(
            decoded_lim < decoded_all,
            "limit must cut block I/O: limited={decoded_lim} full={decoded_all}"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// G2: a deleted prefix must not make a limited scan return empty.
    #[test]
    fn try_scan_at_limit_tombstone_does_not_hide_later_keys() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        for i in 0..40u32 {
            db.put(format!("k{i:03}").as_bytes(), b"v").unwrap();
        }
        db.flush().unwrap();
        db.delete_range(b"k000", b"k010").unwrap();
        let snap = db.last_sequence();
        let limited: Vec<_> = db
            .try_scan_at(snap, Bound::Unbounded, Bound::Unbounded, Some(5))
            .unwrap()
            .map(|kv| kv.key)
            .collect();
        assert_eq!(limited.len(), 5);
        assert_eq!(&limited[0][..], b"k010");
        assert_eq!(&limited[4][..], b"k014");
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0034, updated for the encoded-block seek: a point-get decodes only
    /// the candidate window (`blocks_for_point`), and a repeated get does the
    /// same bounded work for the same answer. The decoded-block cache stays
    /// on the scan paths.
    #[test]
    fn point_get_seek_decodes_bounded_blocks() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        let payload = vec![b'y'; 256];
        for i in 0..80u32 {
            db.put(format!("k{i:03}").as_bytes(), &payload).unwrap();
        }
        db.flush().unwrap();
        db.block_cache.clear();
        crate::sst::reset_sst_blocks_decoded();
        let k = b"k040";
        assert!(db.get(k).is_some());
        let first = crate::sst::sst_blocks_decoded();
        assert!(first >= 1, "first get must decode a block");
        assert!(
            first <= 2,
            "candidate window is at most previous block + run"
        );
        crate::sst::reset_sst_blocks_decoded();
        assert!(db.get(k).is_some());
        // The point cache may serve the repeat outright; when it falls
        // through, the seek does the same bounded work (≤ candidate window).
        assert!(
            crate::sst::sst_blocks_decoded() <= first,
            "second get of the same key must not decode more than the first \
             (first={first}, second={})",
            crate::sst::sst_blocks_decoded()
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// MVCC latest / deps_scan: second prefix seek is bounded (block-cache
    /// walk + one encoded-seek resolve); second range scan is cache-only.
    #[test]
    fn last_under_prefix_and_scan_second_seek_are_cached() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        let payload = vec![b'z'; 256];
        for user in 0..40u32 {
            for ver in 1..=3u64 {
                let mut k = format!("u/{user:02}").into_bytes();
                k.extend_from_slice(&ver.to_be_bytes());
                db.put(&k, &payload).unwrap();
            }
        }
        db.flush().unwrap();
        let snap = db.last_sequence();
        db.block_cache.clear();
        crate::sst::reset_sst_blocks_decoded();
        let last = db
            .last_under_prefix(snap, b"u/10")
            .unwrap()
            .expect("user 10");
        assert!(last.starts_with(b"u/10"));
        let first = crate::sst::sst_blocks_decoded();
        assert!(first >= 1, "first latest must decode");
        crate::sst::reset_sst_blocks_decoded();
        let last2 = db.last_under_prefix(snap, b"u/10").unwrap();
        assert_eq!(last2.as_deref(), Some(last.as_ref()));
        // The reverse block walk is block-cache served; the final point
        // resolve runs the encoded seek, which decompresses the one
        // candidate block (first decoded `first` blocks overall).
        assert!(
            crate::sst::sst_blocks_decoded() <= first,
            "second latest must do no more work than the first \
             (first={first}, second={})",
            crate::sst::sst_blocks_decoded()
        );

        db.block_cache.clear();
        crate::sst::reset_sst_blocks_decoded();
        let n = db
            .try_scan_at(
                snap,
                Bound::Included(b"u/10".as_ref()),
                Bound::Excluded(b"u/15".as_ref()),
                Some(25),
            )
            .unwrap()
            .count();
        assert!(n > 0);
        let scan_first = crate::sst::sst_blocks_decoded();
        crate::sst::reset_sst_blocks_decoded();
        let n2 = db
            .try_scan_at(
                snap,
                Bound::Included(b"u/10".as_ref()),
                Bound::Excluded(b"u/15".as_ref()),
                Some(25),
            )
            .unwrap()
            .count();
        assert_eq!(n2, n);
        assert_eq!(
            crate::sst::sst_blocks_decoded(),
            0,
            "second limited scan must not re-decode (first decoded {scan_first})"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_under_user_prefix_mem_hit_skips_sst_and_tombstone_still_falls_back() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        for ver in 1..=3u64 {
            let mut k = b"u/01".to_vec();
            k.extend_from_slice(&ver.to_be_bytes());
            db.put(&k, b"v").unwrap();
        }
        db.flush().unwrap();
        let mut k4 = b"u/01".to_vec();
        k4.extend_from_slice(&4u64.to_be_bytes());
        db.put(&k4, b"v4").unwrap();
        db.block_cache.clear();
        crate::sst::reset_sst_blocks_decoded();
        let got = db
            .last_under_user_prefix(db.last_sequence(), b"u/01")
            .unwrap()
            .expect("mem latest");
        assert_eq!(got, k4);
        assert_eq!(
            crate::sst::sst_blocks_decoded(),
            0,
            "newest mem live key must not probe SST"
        );

        db.delete(&k4).unwrap();
        let prev = db
            .last_under_user_prefix(db.last_sequence(), b"u/01")
            .unwrap()
            .expect("flushed v3");
        assert_eq!(&prev[prev.len() - 8..], &3u64.to_be_bytes());
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_probe_counts_mem_hit_fallback_and_scan() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"u/01\x00\x00\x00\x00\x00\x00\x00\x01", b"v")
            .unwrap();
        db.flush().unwrap();
        db.reset_read_probe();
        let _ = db
            .last_under_user_prefix(db.last_sequence(), b"u/01")
            .unwrap();
        let p = db.read_probe();
        assert_eq!(p.latest_ops, 1);
        assert_eq!(p.latest_mem_hit, 0);
        assert_eq!(p.latest_sst_fallback, 1);
        assert!(p.latest_sst_probed >= 1);
        assert!(p.sst_count >= 1);

        db.put(b"u/01\x00\x00\x00\x00\x00\x00\x00\x02", b"v2")
            .unwrap();
        db.reset_read_probe();
        let _ = db
            .last_under_user_prefix(db.last_sequence(), b"u/01")
            .unwrap();
        let p = db.read_probe();
        assert_eq!(p.latest_mem_hit, 1);
        assert_eq!(p.latest_sst_fallback, 0);

        db.reset_read_probe();
        let _ = db
            .try_scan_at(
                db.last_sequence(),
                Bound::Unbounded,
                Bound::Unbounded,
                Some(8),
            )
            .unwrap()
            .count();
        let p = db.read_probe();
        assert_eq!(p.scan_ops, 1);
        assert!(p.scan_sst_probed >= 1);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_mem_hit_skips_sst_and_still_sees_flushed() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"k", b"sst").unwrap();
        db.flush().unwrap();
        db.put(b"k", b"mem").unwrap();
        db.reset_read_probe();
        crate::sst::reset_sst_blocks_decoded();
        assert_eq!(db.get(b"k").as_deref(), Some(b"mem".as_ref()));
        let p = db.read_probe();
        assert_eq!(p.get_mem_hit, 1);
        assert_eq!(p.get_sst_fallback, 0);
        assert_eq!(p.get_inline, 1);
        assert_eq!(p.get_vlog, 0);
        assert_eq!(crate::sst::sst_blocks_decoded(), 0);

        db.delete(b"k").unwrap();
        assert_eq!(db.get(b"k"), None);
        db.flush().unwrap();
        // After flush the delete is in SST; get must still hide the old put.
        let db = {
            db.close().unwrap();
            Db::open(&dir).unwrap()
        };
        assert_eq!(db.get(b"k"), None);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn point_cache_invalidates_on_put() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"k", b"v1").unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(&b"v1"[..]));
        assert_eq!(db.get(b"k").as_deref(), Some(&b"v1"[..]));
        db.put(b"k", b"v2").unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(&b"v2"[..]));
        db.delete(b"k").unwrap();
        assert_eq!(db.get(b"k"), None);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_prefix_and_count_caches_invalidate_on_put() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        let mut k1 = b"u/01".to_vec();
        k1.extend_from_slice(&1u64.to_be_bytes());
        db.put(&k1, b"a").unwrap();
        let last = db
            .last_under_user_prefix(db.last_sequence(), b"u/01")
            .unwrap()
            .expect("v1");
        assert_eq!(last, k1);
        let mut k2 = b"u/01".to_vec();
        k2.extend_from_slice(&2u64.to_be_bytes());
        db.put(&k2, b"b").unwrap();
        let last2 = db
            .last_under_user_prefix(db.last_sequence(), b"u/01")
            .unwrap()
            .expect("v2");
        assert_eq!(last2, k2);
        let n = db
            .count_in_range(
                db.last_sequence(),
                Bound::Included(b"u/01"),
                Bound::Excluded(b"u/02"),
                Some(25),
            )
            .unwrap();
        assert_eq!(n, 2);
        let mut k3 = b"u/01".to_vec();
        k3.extend_from_slice(&3u64.to_be_bytes());
        db.put(&k3, b"c").unwrap();
        let n2 = db
            .count_in_range(
                db.last_sequence(),
                Bound::Included(b"u/01"),
                Bound::Excluded(b"u/02"),
                Some(25),
            )
            .unwrap();
        assert_eq!(n2, 3);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0037 P1.3: the borrowed count merge must agree with the streaming
    /// path on every window over a state with versions, deletes, range
    /// tombstones, and data split across memtables and SSTs.
    #[test]
    fn count_borrowed_matches_streaming_all_windows() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        // Deterministic schedule: puts (multi-version), point deletes, a
        // range delete, then flushes to split layers, then more puts.
        let key = |u: u8, ts: u64| {
            let mut k = vec![b'k', b'/', u];
            k.extend_from_slice(&ts.to_be_bytes());
            k
        };
        for u in 0u8..32 {
            for ts in 1..=4u64 {
                db.put(&key(u, ts), b"v").unwrap();
            }
        }
        for u in [3u8, 9, 17] {
            db.delete(&key(u, 5)).unwrap();
        }
        db.flush().unwrap();
        db.delete_range(&key(20, 0), &key(24, 0)).unwrap();
        db.flush().unwrap();
        for u in 32u8..48 {
            db.put(&key(u, 1), b"v").unwrap();
        }
        // Assert: every [a, b) window × limit must match the streaming count.
        for a in 0u8..50 {
            for b in a..=50u8 {
                for limit in [None, Some(1), Some(5), Some(1000)] {
                    let start = key(a.min(49), 0);
                    let end = key(b.min(49), 0);
                    let snap = db.last_sequence();
                    let fast = db.count_visible(
                        snap,
                        Bound::Included(start.as_slice()),
                        Bound::Excluded(end.as_slice()),
                        limit,
                    );
                    let slow = db
                        .scan_at_raw(
                            snap,
                            Bound::Included(start.as_slice()),
                            Bound::Excluded(end.as_slice()),
                            limit,
                            false,
                        )
                        .count();
                    assert_eq!(
                        fast, slow,
                        "window [{a},{b}) limit {limit:?}: fast={fast} slow={slow}"
                    );
                }
            }
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0039 P2.1: count and scan emit the same set (borrowed cursor).
    #[test]
    fn count_visible_matches_scan_set() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        for i in 0u8..16 {
            db.put([b'a', i], [b'v', i]).unwrap();
        }
        db.delete([b'a', 3]).unwrap();
        db.delete_range([b'a', 10], [b'a', 13]).unwrap();
        db.flush().unwrap();
        for i in 16u8..24 {
            db.put([b'a', i], [b'v', i]).unwrap();
        }
        let snap = db.last_sequence();
        let fast = db.count_visible(snap, Bound::Unbounded, Bound::Unbounded, None);
        let slow = db
            .scan_at_raw(snap, Bound::Unbounded, Bound::Unbounded, None, false)
            .count();
        assert_eq!(fast, slow);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0154: apply_batch (≥16 ops → insert_many) of default-raw keys
    /// must keep the live tail idx so count_visible matches scan without a
    /// full-tail replay.
    #[test]
    fn apply_batch_raw_keys_count_visible_matches_scan() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        let ops: Vec<BatchOp> = (0..64u32)
            .map(|i| {
                let k = format!("k{i:04}").into_bytes();
                BatchOp::put(k, b"x")
            })
            .collect();
        db.apply_batch(ops).unwrap();
        let snap = db.last_sequence();
        let start = b"k0010".as_slice();
        let end = b"k0035".as_slice();
        let fast = db.count_visible(snap, Bound::Included(start), Bound::Excluded(end), Some(25));
        let slow = db
            .scan_at_raw(
                snap,
                Bound::Included(start),
                Bound::Excluded(end),
                Some(25),
                false,
            )
            .count();
        assert_eq!(fast, slow);
        assert_eq!(fast, 25);
        for i in 0..64u32 {
            let k = format!("k{i:04}").into_bytes();
            assert_eq!(db.get(&k).as_deref(), Some(b"x".as_ref()), "k={i}");
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0154 P1.2: apply_batch of `write\0` keys, then reverse-seek
    /// (`last_under_user_prefix`) + count match `get` / scan (ordered shard).
    #[test]
    fn apply_batch_write_cf_last_matches_get() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        let ops: Vec<BatchOp> = (0..64u32)
            .map(|u| {
                let mut k = b"write\0u/".to_vec();
                k.extend_from_slice(format!("{u:02}").as_bytes());
                BatchOp::put(k, b"c")
            })
            .collect();
        db.apply_batch(ops).unwrap();
        let snap = db.last_sequence();
        let last = db
            .last_under_user_prefix(snap, b"write\0u/10")
            .unwrap()
            .expect("write prefix in mem");
        assert!(last.starts_with(b"write\0u/10"), "{last:?}");
        assert_eq!(db.get(&last).as_deref(), Some(b"c".as_ref()));
        let start = b"write\0u/10".as_slice();
        let end = b"write\0u/35".as_slice();
        let fast = db.count_visible(snap, Bound::Included(start), Bound::Excluded(end), Some(25));
        let slow = db
            .scan_at_raw(
                snap,
                Bound::Included(start),
                Bound::Excluded(end),
                Some(25),
                false,
            )
            .count();
        assert_eq!(fast, slow);
        assert_eq!(fast, 25);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0039 P2.2: flush+auto-compact must not leave L0 at the trigger
    /// for a following count/scan.
    #[test]
    fn apply_does_not_leave_l0_at_trigger_for_scan() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        for i in 0..L0_COMPACTION_TRIGGER {
            db.put([b'k', i as u8], b"v").unwrap();
            db.flush().unwrap();
        }
        assert!(
            db.level_file_count(0) < L0_COMPACTION_TRIGGER,
            "L0 after apply/flush must be below trigger, got {}",
            db.level_file_count(0)
        );
        let n = db
            .count_in_range(
                db.visible_sequence(),
                Bound::Unbounded,
                Bound::Unbounded,
                None,
            )
            .unwrap();
        let scan_n = db.scan(Bound::Unbounded, Bound::Unbounded).count();
        assert_eq!(n, scan_n);
        assert_eq!(n, L0_COMPACTION_TRIGGER);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_under_user_prefix_older_l0_still_visible() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        let mut k1 = b"u/01".to_vec();
        k1.extend_from_slice(&1u64.to_be_bytes());
        db.put(&k1, b"a").unwrap();
        db.flush().unwrap();
        let mut k2 = b"u/02".to_vec();
        k2.extend_from_slice(&1u64.to_be_bytes());
        db.put(&k2, b"b").unwrap();
        db.flush().unwrap();
        assert!(db.sst_count() >= 2);
        let got = db
            .last_under_user_prefix(db.last_sequence(), b"u/01")
            .unwrap()
            .expect("older L0");
        assert_eq!(got, k1);
        let got2 = db
            .last_under_user_prefix(db.last_sequence(), b"u/02")
            .unwrap()
            .expect("newer L0");
        assert_eq!(got2, k2);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    // ---- RFC-0046 P0.1–P0.3: history horizon + local archive ----

    /// StdEnv + injectable wall clock (+ optional fail-`create` fault for the
    /// crash-mid-archive test). Delegates every file op to `StdEnv`.
    #[derive(Clone)]
    struct ClockEnv {
        t: std::rc::Rc<std::cell::Cell<u64>>,
        fail_create: std::rc::Rc<std::cell::Cell<bool>>,
        /// P1.2: fail `create` only under this prefix (remote tier outage
        /// with a healthy local disk), shared across clones.
        fail_prefix: Option<std::rc::Rc<(PathBuf, std::rc::Rc<std::cell::Cell<bool>>)>>,
    }

    impl ClockEnv {
        fn new(t: std::rc::Rc<std::cell::Cell<u64>>) -> Self {
            Self {
                t,
                fail_create: std::rc::Rc::new(std::cell::Cell::new(false)),
                fail_prefix: None,
            }
        }
        fn with_fail_prefix(
            t: std::rc::Rc<std::cell::Cell<u64>>,
            prefix: PathBuf,
        ) -> (Self, std::rc::Rc<std::cell::Cell<bool>>) {
            let flag = std::rc::Rc::new(std::cell::Cell::new(false));
            (
                Self {
                    t,
                    fail_create: std::rc::Rc::new(std::cell::Cell::new(false)),
                    fail_prefix: Some(std::rc::Rc::new((prefix, std::rc::Rc::clone(&flag)))),
                },
                flag,
            )
        }
        fn unix_millis_impl(&self) -> u64 {
            self.t.get()
        }
    }

    impl Env for ClockEnv {
        type File = <StdEnv as Env>::File;
        fn unix_millis(&self) -> u64 {
            self.unix_millis_impl()
        }
        fn create_dir_all(&self, p: &Path) -> std::io::Result<()> {
            StdEnv.create_dir_all(p)
        }
        fn create(&self, p: &Path) -> std::io::Result<Self::File> {
            // Fault scoped to archive segments: flush/SST writes stay healthy
            // so the test isolates the mid-archive crash, not a dead disk.
            if self.fail_create.get() && p.extension().is_some_and(|e| e == "hist") {
                return Err(std::io::Error::other("injected archive failure"));
            }
            if let Some(rp) = &self.fail_prefix {
                let (prefix, flag) = (rp.0.clone(), std::rc::Rc::clone(&rp.1));
                if flag.get() && p.starts_with(&prefix) {
                    return Err(std::io::Error::other("injected remote outage"));
                }
            }
            StdEnv.create(p)
        }
        fn open_append(&self, p: &Path) -> std::io::Result<Self::File> {
            StdEnv.open_append(p)
        }
        fn open_read(&self, p: &Path) -> std::io::Result<Self::File> {
            StdEnv.open_read(p)
        }
        fn sync_dir(&self, p: &Path) -> std::io::Result<()> {
            StdEnv.sync_dir(p)
        }
        fn read_dir_names(&self, p: &Path) -> std::io::Result<Vec<String>> {
            StdEnv.read_dir_names(p)
        }
        fn remove_file(&self, p: &Path) -> std::io::Result<()> {
            StdEnv.remove_file(p)
        }
        fn rename(&self, a: &Path, b: &Path) -> std::io::Result<()> {
            StdEnv.rename(a, b)
        }
        fn exists(&self, p: &Path) -> bool {
            StdEnv.exists(p)
        }
        fn metadata_len(&self, p: &Path) -> std::io::Result<u64> {
            StdEnv.metadata_len(p)
        }
    }

    fn horizon_opts(window_ms: u64, cap_bytes: u64) -> OpenOptions {
        OpenOptions {
            wal_full_fsync: true,
            history: HistoryOptions {
                horizon: HistoryHorizon::Window(Duration::from_millis(window_ms)),
                cap_bytes,
            },
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: Some(1),
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
            wal_recovery: WalRecovery::FailClosed,
            sst_payload_budget_bytes: None,
        }
    }

    /// `horizon_opts` with auto-compaction OFF: the only archive passes
    /// are the ones `compact_horizon` runs explicitly (P2.7 tests need
    /// exactly one upload per wave so the shipped remote manifest is the
    /// pre-cap-drop generation — it still lists the locally-dropped
    /// segment, which is what the lazy remote read serves from).
    fn horizon_opts_manual(window_ms: u64, cap_bytes: u64) -> OpenOptions {
        OpenOptions {
            wal_full_fsync: true,
            auto_compact_sst_count: None,
            ..horizon_opts(window_ms, cap_bytes)
        }
    }

    #[test]
    fn snapshot_pinned_survives_horizon() {
        // RFC-0046 P0.3: the horizon GC is pin-aware — a pinned seq keeps
        // its view across aging + archive + GC; after release, the same seq
        // fails closed with SnapshotTooOld (typed, never silent).
        let dir = temp_dir();
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let env = ClockEnv::new(std::rc::Rc::clone(&clock));
        let mut db = Db::open_with_env(&dir, horizon_opts(1_000, 1 << 30), env).unwrap();
        for i in 0..40u32 {
            db.put(b"k", format!("v{i}").as_bytes()).unwrap();
        }
        let pin = db.pin_snapshot();
        assert_eq!(
            db.get_at(pin.snapshot(), b"k").unwrap().as_deref(),
            Some(&b"v39"[..])
        );
        clock.set(1_000_000 + 60_000); // way past the 1 s window
        db.flush().unwrap(); // flush → auto-compact → archive + horizon GC
        assert_eq!(
            db.get_at(pin.snapshot(), b"k").unwrap().as_deref(),
            Some(&b"v39"[..]),
            "pinned snapshot survives horizon GC"
        );
        assert_eq!(db.get(b"k").as_deref(), Some(&b"v39"[..]));
        let pinned_seq = pin.sequence();
        db.release_snapshot_pin(pin);
        clock.set(1_000_000 + 120_000);
        for i in 40..80u32 {
            db.put(b"k", format!("v{i}").as_bytes()).unwrap();
        }
        // Age the newest sample too, then GC again — the watermark must
        // pass the released pin's seq. P2.1: below the watermark the read
        // falls back to the retained tier and still answers; true loss
        // (cap drop without a mirror) stays SnapshotTooOld (cap test).
        clock.set(1_000_000 + 180_000);
        db.flush().unwrap();
        assert!(
            db.earliest_readable_sequence() > pinned_seq,
            "watermark must pass the released pin (earliest={})",
            db.earliest_readable_sequence()
        );
        assert_eq!(
            db.get_at(Snapshot::at(pinned_seq), b"k")
                .unwrap()
                .as_deref(),
            Some(&b"v39"[..]),
            "below-watermark read serves from the retained tier (P2.1)"
        );
        assert_eq!(
            db.get_at(Snapshot::at(pinned_seq), b"never-written")
                .unwrap(),
            None,
            "anchored coverage proves never-written (P2.1)"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn horizon_full_rewrite_bounds_disk() {
        // RFC-0046 P0.5: dead-weight-doubling trigger. On an overwrite
        // workload the horizon floor lags the L0 merges and old levels are
        // never rewritten, so without the trigger the window profile's LSM
        // stays byte-identical to `All` (findings/rfc0046-sizing). With it,
        // aged versions are rewritten away once SST bytes double, and
        // below-floor history stays readable through the archive.
        let dir = temp_dir();
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let env = ClockEnv::new(std::rc::Rc::clone(&clock));
        // Default auto-compact (no count/bytes override): L0 merges only,
        // so the bound can only come from the P0.5 trigger.
        let mut opts = horizon_opts(1_000, 1 << 30);
        opts.auto_compact_sst_count = None;
        let mut db = Db::open_with_env(&dir, opts, env).unwrap();
        let keys: Vec<Vec<u8>> = (0..64u32).map(|k| format!("k{k:04}").into()).collect();
        let val = |round: u32, k: u32| vec![(k * 7 + round) as u8; 1024];
        let mut round0 = Vec::new();
        for (k, key) in keys.iter().enumerate() {
            let v = val(0, k as u32);
            db.put(key, &v).unwrap();
            round0.push(v);
        }
        let pin = db.pin_snapshot();
        let seq0 = pin.sequence();
        db.release_snapshot_pin(pin);
        db.flush().unwrap();
        for round in 1..24u32 {
            for (k, key) in keys.iter().enumerate() {
                db.put(key, &val(round, k as u32)).unwrap();
            }
            db.flush().unwrap();
            clock.set(1_000_000 + (round as u64 + 1) * 1_500);
        }
        let live = 64 * 1024u64;
        let sst = db.stats().sst_bytes;
        assert!(
            sst < 12 * live,
            "SST bytes must stay bounded (live {live}, got {sst})"
        );
        for (k, key) in keys.iter().enumerate() {
            assert_eq!(
                db.get(key).as_deref(),
                Some(&val(23, k as u32)[..]),
                "latest value survives the full rewrites"
            );
        }
        assert!(
            db.earliest_readable_sequence() > seq0,
            "watermark advanced past the aged round-0 seq"
        );
        for (k, key) in keys.iter().enumerate() {
            assert_eq!(
                db.get_at(Snapshot::at(seq0), key).unwrap().as_deref(),
                Some(&round0[k][..]),
                "below-floor history reads from the archive after rewrites"
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_horizon_reclaims_aged_versions() {
        // RFC-0046 P0.5 public API: the operator-triggered horizon-aware
        // full compaction. Ages everything past a 1 s window, rewrites the
        // LSM down to the live set, and below-floor history stays readable
        // through the archive. `All` horizon is a no-op.
        let dir = temp_dir();
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let env = ClockEnv::new(std::rc::Rc::clone(&clock));
        let mut opts = horizon_opts(1_000, 1 << 30);
        opts.auto_compact_sst_count = None;
        let mut db = Db::open_with_env(&dir, opts, env).unwrap();
        let keys: Vec<Vec<u8>> = (0..32u32).map(|k| format!("k{k:04}").into()).collect();
        // Incompressible payloads: constant fill lets lz4 crush every round
        // to ~nothing (L0 flushes are compressed since v19) and the byte
        // ratios below would measure compression, not version reclamation.
        let val = |round: u32, k: u32| {
            let mut s = u64::from(k).wrapping_mul(0x9E37_79B9)
                ^ u64::from(round).wrapping_mul(0x85EB_CA6B)
                ^ 0x27D4_EB2F;
            let mut v = Vec::with_capacity(2048);
            for _ in 0..2048 {
                s = s
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                v.push((s >> 33) as u8);
            }
            v
        };
        let mut round0 = Vec::new();
        for (k, key) in keys.iter().enumerate() {
            let v = val(0, k as u32);
            db.put(key, &v).unwrap();
            round0.push(v);
        }
        let pin = db.pin_snapshot();
        let seq0 = pin.sequence();
        db.release_snapshot_pin(pin);
        for round in 1..8u32 {
            for (k, key) in keys.iter().enumerate() {
                db.put(key, &val(round, k as u32)).unwrap();
            }
            db.flush().unwrap();
        }
        let before = db.stats().sst_bytes;
        clock.set(1_000_000 + 60_000); // everything ages past the window
        db.compact_horizon().unwrap();
        let after = db.stats().sst_bytes;
        let live = 32 * 2048u64;
        assert!(
            after < before / 4,
            "explicit horizon compaction reclaims aged bytes (before {before}, after {after})"
        );
        assert!(
            after < 4 * live,
            "bounded near live set (live {live}, after {after})"
        );
        for (k, key) in keys.iter().enumerate() {
            assert_eq!(db.get(key).as_deref(), Some(&val(7, k as u32)[..]));
            assert_eq!(
                db.get_at(Snapshot::at(seq0), key).unwrap().as_deref(),
                Some(&round0[k][..]),
                "aged round-0 history answers from the archive"
            );
        }
        // `All` horizon: nothing to GC — Ok no-op.
        let dir_all = temp_dir();
        let mut o = horizon_opts(1_000, 1 << 30);
        o.history.horizon = HistoryHorizon::All;
        o.auto_compact_sst_count = None;
        let mut db_all =
            Db::open_with_env(&dir_all, o, ClockEnv::new(std::rc::Rc::clone(&clock))).unwrap();
        db_all.put(b"k", b"v").unwrap();
        db_all.flush().unwrap();
        assert!(db_all.compact_horizon().is_ok());
        assert_eq!(db_all.get(b"k").as_deref(), Some(&b"v"[..]));
        let _ = fs::remove_dir_all(&dir_all);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn below_watermark_lsm_fallback_serves_survivors() {
        // RFC-0046 P2.3: the watermark is global but survival is per key.
        // A single-version key below the GC floor survives the full
        // rewrite; after the archive cap drops its segment, the read must
        // still answer from the LSM. Shadowed (dropped) history and
        // never-written keys keep failing SnapshotTooOld — the fallback
        // never turns "gone" into None.
        let dir = temp_dir();
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let env = ClockEnv::new(std::rc::Rc::clone(&clock));
        // Tiny cap: the second archive pass must drop the first segments.
        let mut opts = horizon_opts(1_000, 64 * 1024);
        opts.auto_compact_sst_count = None;
        let mut db = Db::open_with_env(&dir, opts, env).unwrap();
        let solo = b"solo";
        let solo_val = vec![0xAB; 1024];
        db.put(solo, &solo_val).unwrap();
        let solo_seq = db.last_sequence();
        let hot = b"hot";
        let hot_val = |round: u32| vec![round as u8; 1024];
        let mut hot_shadow_seq = 0;
        let mut round = 0u32;
        // Two aging cycles: each archives + cap-drops the previous wave.
        for _cycle in 0..2 {
            for _ in 0..48 {
                db.put(hot, &hot_val(round)).unwrap();
                if round == 0 {
                    hot_shadow_seq = db.last_sequence();
                }
                round += 1;
                if db.last_sequence() % 16 == 0 {
                    db.flush().unwrap();
                }
            }
            db.flush().unwrap();
            clock.set(1_000_000 + 60_000 + u64::from(round) * 1_000);
            db.compact_horizon().unwrap();
        }
        assert!(
            db.earliest_readable_sequence() > solo_seq,
            "precondition: watermark above the solo seq (earliest={})",
            db.earliest_readable_sequence()
        );
        assert!(
            db.earliest_readable_sequence() > hot_shadow_seq,
            "precondition: watermark above the shadowed seq (earliest={})",
            db.earliest_readable_sequence()
        );
        assert_eq!(
            db.get_at(Snapshot::at(solo_seq), solo).unwrap().as_deref(),
            Some(&solo_val[..]),
            "survivor below the watermark serves from the LSM after the cap drop"
        );
        assert_eq!(
            db.multi_get_at(Snapshot::at(solo_seq), &[solo]).unwrap(),
            vec![Some(Bytes::from(solo_val.clone()))],
            "multi_get_at shares the get_at below-watermark legs (tier + LSM fallback)"
        );
        let mixed: &[&[u8]] = &[solo, b"never-written"];
        assert!(
            matches!(
                db.multi_get_at(Snapshot::at(solo_seq), mixed),
                Err(CoreError::SnapshotTooOld { .. })
            ),
            "a batch with an uncoverable key fails closed as a whole"
        );
        assert!(
            matches!(
                db.get_at(Snapshot::at(hot_shadow_seq), hot),
                Err(CoreError::SnapshotTooOld { .. })
            ),
            "shadowed history stays SnapshotTooOld (dropped from LSM, segment cap-dropped)"
        );
        assert!(
            matches!(
                db.get_at(Snapshot::at(solo_seq), b"never-written"),
                Err(CoreError::SnapshotTooOld { .. })
            ),
            "never-written below the watermark stays fail-closed (no silent None)"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn horizon_sample_ring_hard_capped_and_drained() {
        // RFC-0046 P0.1: the (seq, time) sample ring must be bounded in
        // COUNT, not just span — a long window under sustained writes
        // would otherwise grow memory without bound (one sample per 32
        // publishes; 24 h at 10k w/s ≈ 54 M samples). Consumed samples
        // (below the returned cutoff) are shed on read.
        let dir = temp_dir();
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let env = ClockEnv::new(std::rc::Rc::clone(&clock));
        // 1 h window: nothing ages during the write phase.
        let mut opts = horizon_opts(3_600_000, 1 << 30);
        opts.auto_compact_sst_count = None;
        let db = Db::open_with_env(&dir, opts, env).unwrap();
        let n = 400_000u64; // → 12 500 samples uncapped
        for i in 0..n {
            db.publish_sequence(i + 1);
            clock.set(1_000_000 + i); // +1 ms per publish: 400 s of sim time
        }
        let samples = db.history_stats().unwrap().seq_time_samples;
        assert!(
            samples <= HORIZON_SAMPLE_RING_CAP,
            "sample ring must be hard-capped (got {samples}, cap {HORIZON_SAMPLE_RING_CAP})"
        );
        // Age everything past the window: the cutoff still answers and the
        // consumed prefix drains.
        clock.set(1_000_000 + 7_200_000);
        let cut = db.horizon_cutoff_sequence().expect("cutoff after aging");
        assert!(cut > n - 1_000, "cutoff covers the aged writes (got {cut})");
        let drained = db.history_stats().unwrap().seq_time_samples;
        assert!(
            drained <= 1,
            "consumed samples drain on read (got {drained})"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn changes_feed_fails_closed_below_watermark() {
        // RFC-0046 P2.4: the change feed must not serve a silently partial
        // window. Below the GC watermark, versions — including lone
        // tombstones — are gone from every feed source; `changes` fails
        // SnapshotTooOld. From the watermark on, the tail stays exact.
        let dir = temp_dir();
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let env = ClockEnv::new(std::rc::Rc::clone(&clock));
        let mut opts = horizon_opts(1_000, 1 << 30);
        opts.auto_compact_sst_count = None;
        let mut db = Db::open_with_env(&dir, opts, env).unwrap();
        // Waves sized to cross the every-32-publishes sampling so the
        // horizon cutoff provably covers the old wave.
        for i in 0..64u32 {
            db.put(b"a", format!("v{i}").as_bytes()).unwrap();
            db.put(b"b", format!("v{i}").as_bytes()).unwrap();
        }
        db.delete(b"b").unwrap(); // lone tombstone for the GC to eat
        let old_to = db.last_sequence();
        db.flush().unwrap();
        clock.set(1_000_000 + 60_000);
        for i in 0..64u32 {
            db.put(b"a", format!("w{i}").as_bytes()).unwrap();
        }
        db.flush().unwrap();
        clock.set(1_000_000 + 120_000);
        db.compact_horizon().unwrap();
        let wm = db.earliest_readable_sequence();
        assert!(
            wm > old_to,
            "watermark advanced past the old wave ({wm} vs {old_to})"
        );
        assert!(
            matches!(db.changes(0, old_to), Err(CoreError::SnapshotTooOld { .. })),
            "window below the watermark fails closed, never a silent partial"
        );
        // From the watermark the tail is exact: the surviving wave answers.
        let tail = db.changes(wm - 1, db.last_sequence()).unwrap();
        assert!(!tail.is_empty(), "tail from the watermark answers");
        let last = tail.last().expect("non-empty");
        assert_eq!(last.key.as_ref(), b"a");
        assert_eq!(last.value.as_ref(), b"w63");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pitr_local_by_seq_within_window() {
        // RFC-0046 P0.3: PITR by seq within the window is served by the
        // local tier without any pin — nothing inside the horizon is GCed.
        let dir = temp_dir();
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let env = ClockEnv::new(std::rc::Rc::clone(&clock));
        let mut db = Db::open_with_env(&dir, horizon_opts(60_000, 1 << 30), env).unwrap();
        for i in 0..40u32 {
            db.put(b"k", format!("v{i}").as_bytes()).unwrap();
        }
        let mid = Snapshot::at(20);
        clock.set(1_000_000 + 30_000); // still inside the 60 s window
        db.flush().unwrap();
        assert_eq!(
            db.get_at(mid, b"k").unwrap().as_deref(),
            Some(&b"v19"[..]),
            "within-window seq reads its version"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_cap_overflow_advances_watermark_not_silent() {
        // RFC-0046 P0.3: cap overflow drops the oldest archive segments and
        // advances the readable watermark — old snaps fail typed once the
        // history is really gone (P2.1 serves it while any copy remains:
        // the re-archive duplicate keeps [1..cutoff] readable until the cap
        // evicts every copy — hence a cap small enough to empty the tier).
        // Latest reads unaffected; a pin holds its segment.
        let dir = temp_dir();
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let env = ClockEnv::new(std::rc::Rc::clone(&clock));
        // 100 publishes push the sampled cutoff to 96 (~3 KB archived per
        // pass); the 512 B cap evicts every segment covering seq 1.
        let mut db = Db::open_with_env(&dir, horizon_opts(1_000, 512), env).unwrap();
        for i in 0..100u32 {
            db.put(b"k", format!("v{i:08}").as_bytes()).unwrap();
        }
        clock.set(1_000_000 + 60_000);
        db.flush().unwrap();
        assert!(
            db.earliest_readable_sequence() > 1,
            "cap overflow must advance the watermark, got {}",
            db.earliest_readable_sequence()
        );
        assert!(
            db.history_tier
                .as_ref()
                .map(|t| t.segment_metas().is_empty())
                .unwrap_or(false),
            "every copy of the old segments must be evicted (P2.1 serves retained copies)"
        );
        assert!(matches!(
            db.get_at(Snapshot::at(1), b"k"),
            Err(CoreError::SnapshotTooOld { .. })
        ));
        assert_eq!(db.get(b"k").as_deref(), Some(&b"v00000099"[..]));
        assert!(
            db.history_tier
                .as_ref()
                .map(|t| t.bytes() <= 512)
                .unwrap_or(false),
            "cap must bound archived bytes, got {}",
            db.history_tier.as_ref().map(|t| t.bytes()).unwrap_or(0)
        );
        // The advanced watermark is durable: reopen re-raises it from the
        // archive manifest floor (the in-memory watermark may sit one above
        // it from the GC floor — the manifest floor is the durable part).
        let durable_floor = db.history_tier.as_ref().unwrap().archive_floor();
        assert!(durable_floor > 1);
        db.close().unwrap();
        let db = Db::open(&dir).unwrap();
        assert!(
            db.earliest_readable_sequence() >= durable_floor,
            "reopen must keep the cap-advanced watermark ({} < {durable_floor})",
            db.earliest_readable_sequence()
        );
        assert!(matches!(
            db.get_at(Snapshot::at(1), b"k"),
            Err(CoreError::SnapshotTooOld { .. })
        ));

        // With a pin, the cap cannot take the pinned segment away.
        let dir2 = temp_dir();
        let clock2 = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let env2 = ClockEnv::new(std::rc::Rc::clone(&clock2));
        let mut db2 = Db::open_with_env(&dir2, horizon_opts(1_000, 2_048), env2).unwrap();
        for i in 0..40u32 {
            db2.put(b"k", format!("v{i:08}").as_bytes()).unwrap();
        }
        let pin = db2.pin_snapshot();
        clock2.set(1_000_000 + 60_000);
        db2.flush().unwrap();
        assert_eq!(
            db2.get_at(pin.snapshot(), b"k").unwrap().as_deref(),
            Some(&b"v00000039"[..]),
            "pin holds its archive segment against the cap"
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dir2);
    }

    #[test]
    fn archive_crash_mid_upload_reopens_consistent() {
        // RFC-0046 P0.3: an I/O failure mid-archive skips the GC round
        // (fail-closed: never drop unarchived) and the DB reopens with all
        // history intact.
        let dir = temp_dir();
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let env = ClockEnv::new(std::rc::Rc::clone(&clock));
        let fail = std::rc::Rc::clone(&env.fail_create);
        let mut db = Db::open_with_env(&dir, horizon_opts(1_000, 1 << 30), env.clone()).unwrap();
        for i in 0..40u32 {
            db.put(b"k", format!("v{i}").as_bytes()).unwrap();
        }
        clock.set(1_000_000 + 60_000);
        fail.set(true); // every archive-segment create now fails
        db.flush().unwrap(); // flush ok; GC round skipped (archive failed)
        assert_eq!(
            db.get_at(Snapshot::at(2), b"k").unwrap().as_deref(),
            Some(&b"v1"[..]),
            "GC skipped: unarchived history stays local"
        );
        fail.set(false);
        drop(db);
        let env2 = ClockEnv {
            t: std::rc::Rc::clone(&clock),
            fail_create: fail,
            fail_prefix: None,
        };
        let db2 = Db::open_with_env(&dir, horizon_opts(1_000, 1 << 30), env2).unwrap();
        assert_eq!(db2.get(b"k").as_deref(), Some(&b"v39"[..]));
        assert_eq!(
            db2.get_at(Snapshot::at(2), b"k").unwrap().as_deref(),
            Some(&b"v1"[..])
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_horizon_all_keeps_all_versions() {
        // RFC-0046 P0.3: F20 is the explicit opt-out — `All` keeps every
        // version across aging + compaction (re-green of the old default).
        let dir = temp_dir();
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let env = ClockEnv::new(std::rc::Rc::clone(&clock));
        let mut opts = horizon_opts(1_000, 1 << 30);
        opts.history.horizon = HistoryHorizon::All;
        let mut db = Db::open_with_env(&dir, opts, env).unwrap();
        for i in 0..40u32 {
            db.put(b"k", format!("v{i}").as_bytes()).unwrap();
        }
        clock.set(1_000_000 + 60_000);
        db.flush().unwrap();
        assert_eq!(db.earliest_readable_sequence(), 0, "All never GCs");
        assert_eq!(
            db.get_at(Snapshot::at(2), b"k").unwrap().as_deref(),
            Some(&b"v1"[..])
        );
        let _ = fs::remove_dir_all(&dir);
    }

    // ---- RFC-0046 P1.2: remote upload pipeline + backpressure ----

    #[test]
    fn remote_backpressure_pauses_gc_until_uploaded() {
        // While the remote tier is down, the GC round pauses entirely
        // (history-preserving) — earliest stays put; once the destination
        // recovers, the same workload GCs and the mirror is complete.
        let dir = temp_dir();
        let remote_root = dir.join("remote");
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let (env, outage) =
            ClockEnv::with_fail_prefix(std::rc::Rc::clone(&clock), remote_root.clone());
        let mut db = Db::open_with_env(&dir, horizon_opts(1_000, 1 << 30), env.clone()).unwrap();
        db.set_remote_history(env.clone(), remote_root.clone());
        for i in 0..40u32 {
            db.put(b"k", format!("v{i:08}").as_bytes()).unwrap();
        }
        outage.set(true);
        clock.set(1_000_000 + 60_000);
        db.flush().unwrap();
        assert_eq!(
            db.earliest_readable_sequence(),
            0,
            "remote outage must pause GC (backpressure), not just the upload"
        );
        outage.set(false);
        db.put(b"k", b"tail").unwrap();
        db.flush().unwrap();
        assert!(
            db.earliest_readable_sequence() > 0,
            "recovered destination must let GC proceed"
        );
        db.close().unwrap();
        // The mirror is complete and self-describing.
        let remote = crate::history::RemoteTier::new(&remote_root);
        let names: Vec<String> = StdEnv
            .read_dir_names(&remote_root)
            .unwrap()
            .into_iter()
            .filter(|n| n.starts_with("seg-"))
            .collect();
        assert!(!names.is_empty(), "segments must be mirrored");
        assert!(remote.latest_manifest(&StdEnv).unwrap().is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remote_cap_holds_unuploaded_then_releases() {
        // Cap overflow with the destination down must NOT drop un-uploaded
        // segments (disk grows — the documented backpressure tradeoff);
        // after recovery the cap reclaims the uploaded ones.
        let dir = temp_dir();
        let remote_root = dir.join("remote");
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let (env, outage) =
            ClockEnv::with_fail_prefix(std::rc::Rc::clone(&clock), remote_root.clone());
        let mut db = Db::open_with_env(&dir, horizon_opts(1_000, 2_048), env.clone()).unwrap();
        db.set_remote_history(env.clone(), remote_root.clone());
        outage.set(true);
        for round in 0..3u32 {
            for i in 0..20u32 {
                db.put(b"k", format!("r{round}v{i:08}").as_bytes()).unwrap();
            }
            clock.set(1_000_000 + 60_000 + (round as u64) * 60_000);
            db.flush().unwrap();
        }
        let held = db.history_tier.as_ref().unwrap().bytes();
        assert!(
            held > 2_048,
            "cap must hold un-uploaded segments during the outage (held {held}B)"
        );
        assert_eq!(
            db.earliest_readable_sequence(),
            0,
            "nothing may be reclaimed while un-uploaded"
        );
        outage.set(false);
        db.put(b"k", b"tail").unwrap();
        db.flush().unwrap();
        assert!(
            db.history_tier.as_ref().unwrap().bytes() <= 2_048,
            "after recovery the cap reclaims uploaded segments"
        );
        assert!(db.earliest_readable_sequence() > 0);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remote_upload_resumes_across_reopen() {
        // Crash/resume: segments sealed before a failure re-upload as
        // AlreadyPresent after reopen (idempotent content addressing), the
        // manifest generation advances, and nothing is re-written.
        let dir = temp_dir();
        let remote_root = dir.join("remote");
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        {
            let (env, outage) =
                ClockEnv::with_fail_prefix(std::rc::Rc::clone(&clock), remote_root.clone());
            let mut db =
                Db::open_with_env(&dir, horizon_opts(1_000, 1 << 30), env.clone()).unwrap();
            db.set_remote_history(env.clone(), remote_root.clone());
            outage.set(true);
            for i in 0..20u32 {
                db.put(b"k", format!("v{i:08}").as_bytes()).unwrap();
            }
            clock.set(1_000_000 + 60_000);
            db.flush().unwrap(); // archives locally, upload fails, GC paused
            db.close().unwrap();
        }
        {
            let (env, _outage) =
                ClockEnv::with_fail_prefix(std::rc::Rc::clone(&clock), remote_root.clone());
            let mut db =
                Db::open_with_env(&dir, horizon_opts(1_000, 1 << 30), env.clone()).unwrap();
            db.set_remote_history(env.clone(), remote_root.clone());
            for i in 20..40u32 {
                db.put(b"k", format!("v{i:08}").as_bytes()).unwrap();
            }
            clock.set(1_000_000 + 120_000);
            db.flush().unwrap();
            let report = db.upload_history_now().unwrap();
            assert_eq!(
                report.segments_uploaded, 0,
                "everything already shipped inline"
            );
            assert!(
                report.segments_already_present >= 1,
                "resume must be idempotent, not a re-upload"
            );
            assert!(db.earliest_readable_sequence() > 0, "GC resumed");
            db.close().unwrap();
        }
        let remote = crate::history::RemoteTier::new(&remote_root);
        assert!(remote.latest_manifest(&StdEnv).unwrap().is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    // ---- RFC-0046 P2.1: lazy tier read below the watermark ----

    #[test]
    fn lazy_tier_read_below_watermark_local() {
        // Below the GC watermark, point reads fall back to the retained
        // local tier: exact versions, deletes, range deletes, and an
        // anchored never-written `None`. Nothing was dropped, so coverage
        // is provable.
        let dir = temp_dir();
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let env = ClockEnv::new(std::rc::Rc::clone(&clock));
        let mut db = Db::open_with_env(&dir, horizon_opts(1_000, 1 << 30), env).unwrap();
        for i in 0..40u32 {
            db.put(b"k", format!("v{i:02}").as_bytes()).unwrap();
        }
        db.put(b"d", b"gone").unwrap(); // seq 41
        db.delete(b"d").unwrap(); // seq 42
        db.put(b"r", b"ranged").unwrap(); // seq 43
        db.delete_range(b"r", b"s").unwrap(); // seq 44
                                              // Filler past the next 32-publish sample so the horizon cutoff
                                              // (sampled every 32 publishes) passes the deletes too.
        for i in 0..40u32 {
            db.put(b"f", format!("f{i:02}").as_bytes()).unwrap();
        }
        clock.set(1_000_000 + 60_000);
        db.flush().unwrap();
        assert!(
            db.earliest_readable_sequence() > 44,
            "everything must be below the watermark (earliest={})",
            db.earliest_readable_sequence()
        );
        assert_eq!(
            db.get_at(Snapshot::at(1), b"k").unwrap().as_deref(),
            Some(&b"v00"[..]),
            "oldest archived version reads from the tier"
        );
        assert_eq!(
            db.get_at(Snapshot::at(20), b"k").unwrap().as_deref(),
            Some(&b"v19"[..])
        );
        assert_eq!(
            db.get_at(Snapshot::at(41), b"d").unwrap().as_deref(),
            Some(&b"gone"[..]),
            "before its delete, d is visible"
        );
        assert_eq!(db.get_at(Snapshot::at(42), b"d").unwrap(), None);
        assert_eq!(
            db.get_at(Snapshot::at(43), b"r").unwrap().as_deref(),
            Some(&b"ranged"[..]),
            "before the range delete, r is visible"
        );
        assert_eq!(
            db.get_at(Snapshot::at(44), b"r").unwrap(),
            None,
            "covering range delete decides from the tier"
        );
        assert_eq!(
            db.get_at(Snapshot::at(44), b"never-written").unwrap(),
            None,
            "anchored coverage proves never-written"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lazy_tier_read_survives_local_cap_via_remote() {
        // After the local cap drops uploaded segments, below-watermark
        // reads are served from the remote mirror; corrupt remote bytes
        // fail closed with the typed error, never a wrong answer.
        let dir = temp_dir();
        let remote_root = dir.join("remote");
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let env = ClockEnv::new(std::rc::Rc::clone(&clock));
        let mut db = Db::open_with_env(&dir, horizon_opts(1_000, 2_048), env.clone()).unwrap();
        db.set_remote_history(env.clone(), remote_root.clone());
        // Fetch-path fail-closed semantics under test after reads; the
        // P2.8 read cache would serve the verified copy instead.
        db.set_remote_read_cache(0);
        // 100 publishes: the sampled cutoff reaches 96, so one archive
        // pass ships ~95 records (~3 KB) — past the 2 KB cap, forcing a
        // local drop of the uploaded segment (the remote keeps it).
        for i in 0..100u32 {
            db.put(b"k", format!("v{i:08}").as_bytes()).unwrap();
        }
        clock.set(1_000_000 + 60_000);
        db.flush().unwrap();
        assert!(
            db.history_tier.as_ref().unwrap().archive_floor() > 1,
            "the cap must have dropped local segments (floor={})",
            db.history_tier.as_ref().unwrap().archive_floor()
        );
        assert!(
            db.earliest_readable_sequence() > 1,
            "watermark past the oldest snap (earliest={})",
            db.earliest_readable_sequence()
        );
        assert_eq!(
            db.get_at(Snapshot::at(1), b"k").unwrap().as_deref(),
            Some(&b"v00000000"[..]),
            "dropped-locally segment serves from the remote mirror"
        );
        assert_eq!(
            db.get_at(Snapshot::at(20), b"k").unwrap().as_deref(),
            Some(&b"v00000019"[..])
        );
        // Corrupt every mirrored segment object: any read below the
        // watermark must fail closed with the typed history error.
        for entry in fs::read_dir(&remote_root).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("hist") {
                continue;
            }
            let mut bytes = fs::read(&path).unwrap();
            let mid = bytes.len() / 2;
            bytes[mid] ^= 0xff;
            fs::write(&path, bytes).unwrap();
        }
        assert!(
            matches!(
                db.get_at(Snapshot::at(1), b"k"),
                Err(CoreError::CorruptHistory(_))
            ),
            "corrupt remote history fails closed, typed"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remote_sidecar_prunes_segment_fetch() {
        // RFC-0046 P2.7: a remote-only segment whose valid CRC'd sidecar
        // proves the key is outside its key set is never fetched — a
        // corrupt object under it does not disturb the read. Delete the
        // sidecar and the same read walks into the corruption: typed
        // error. The never-written hole key sits inside every segment's
        // coverage bound (the P2.5 prune cannot help) but in no key set.
        //
        // Three same-keyspace waves (compact_horizon each, auto-compact
        // off): wave 2 supersedes wave 1 so the rewrite really sheds it;
        // wave 2's big segment [1..192] is then dropped by the cap when
        // wave 3 seals [193..288] — remote-only, still listed by the
        // shipped pre-drop manifest. Overlapping coverage on both.
        let dir = temp_dir();
        let remote_root = dir.join("remote");
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let env = ClockEnv::new(std::rc::Rc::clone(&clock));
        let mut db =
            Db::open_with_env(&dir, horizon_opts_manual(1_000, 9_000), env.clone()).unwrap();
        db.set_remote_history(env.clone(), remote_root.clone());
        // Fetch-path semantics under test (prune + fail-open); the P2.8
        // read cache is off so reads always refetch.
        db.set_remote_read_cache(0);
        for wave in [b'a', b'b', b'c'] {
            for i in 0..100u32 {
                if i == 42 {
                    continue; // the hole: k042 is never written
                }
                db.put(format!("k{i:03}").as_bytes(), &[wave, b'0', b'0', b'0'])
                    .unwrap();
            }
            clock.set(clock.get() + 60_000);
            db.compact_horizon().unwrap();
        }
        let tier = db.history_tier.as_ref().unwrap();
        let remote = crate::history::RemoteTier::new(&remote_root);
        let remote_names: Vec<String> = remote
            .latest_segments(&env)
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        let local_names: Vec<String> = tier.segment_metas().into_iter().map(|m| m.name).collect();
        assert!(
            remote_names.iter().any(|n| !local_names.contains(n)),
            "a listed segment is dropped locally (remote-only): {remote_names:?} vs {local_names:?}"
        );
        assert!(tier.archive_floor() >= 193);
        assert!(db.earliest_readable_sequence() > 288);
        let snap = Snapshot::at(288);
        assert_eq!(
            db.get_at(Snapshot::at(150), b"k000").unwrap().as_deref(),
            Some(&b"b000"[..]),
            "remote-only segment serves decisive reads"
        );
        // One sidecar object per segment object (incl. unlisted drops).
        let count = |ext: &str| {
            fs::read_dir(&remote_root)
                .unwrap()
                .filter(|e| {
                    e.as_ref()
                        .unwrap()
                        .path()
                        .extension()
                        .and_then(|x| x.to_str())
                        == Some(ext)
                })
                .count()
        };
        let hists = count("hist");
        assert!(hists >= 2, "at least the two listed segments shipped");
        assert_eq!(
            count("bloom"),
            hists,
            "sidecar shipped next to every object"
        );
        // Corrupt the remote segment bodies. The hole key reads as a
        // coverage-gap SnapshotTooOld — the pruned segment is never
        // fetched, so the corruption under it is inert (an unpruned walk
        // would surface CorruptHistory instead — that discrimination is
        // the point). Affected keys still walk and fail typed.
        for entry in fs::read_dir(&remote_root).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("hist") {
                continue;
            }
            let mut bytes = fs::read(&path).unwrap();
            let mid = bytes.len() / 2;
            bytes[mid] ^= 0xff;
            fs::write(&path, bytes).unwrap();
        }
        // The hole key's outcome class depends on which covering segment the
        // byte cap left remote-only — a size-threshold layout choice that
        // compressed L0 flushes legitimately shift. Either answer is honest
        // (the key was never written): a coverage-gap SnapshotTooOld when no
        // remote-only segment covers the snapshot, or a proven-absent None
        // when every covering segment's sidecar rules the key out. Never a
        // value — and never a fetch: the corruption above makes a fetched
        // segment fail CorruptHistory.
        let hole = db.get_at(snap, b"k042");
        assert!(
            matches!(hole, Err(CoreError::SnapshotTooOld { .. })) || matches!(hole, Ok(None)),
            "hole key must read too-old or absent, got {hole:?}"
        );
        assert!(matches!(
            db.get_at(Snapshot::at(150), b"k000"),
            Err(CoreError::CorruptHistory(_))
        ));
        // Fail-open proof: without the sidecar the same read walks into
        // the corrupt segment and fails typed.
        for entry in fs::read_dir(&remote_root).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) == Some("bloom") {
                fs::remove_file(&path).unwrap();
            }
        }
        assert!(matches!(
            db.get_at(snap, b"k042"),
            Err(CoreError::CorruptHistory(_))
        ));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Shared P2.8 harness: three same-keyspace waves archived and
    /// capped so every segment that can decide a read at seq 150 is
    /// dropped locally and lives only in the remote mirror.
    fn remote_only_mirror_150() -> (std::path::PathBuf, ClockEnv, Db<ClockEnv>) {
        let dir = temp_dir();
        let remote_root = dir.join("remote");
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let env = ClockEnv::new(std::rc::Rc::clone(&clock));
        let mut db =
            Db::open_with_env(&dir, horizon_opts_manual(1_000, 9_000), env.clone()).unwrap();
        db.set_remote_history(env.clone(), remote_root.clone());
        for wave in [b'a', b'b', b'c'] {
            for i in 0..100u32 {
                db.put(format!("k{i:03}").as_bytes(), &[wave, b'0', b'0', b'0'])
                    .unwrap();
            }
            clock.set(clock.get() + 60_000);
            db.compact_horizon().unwrap();
        }
        let floor = db.history_tier.as_ref().unwrap().archive_floor();
        assert!(
            floor > 150,
            "every segment deciding a read at 150 must be dropped locally (floor={floor})"
        );
        (remote_root, env, db)
    }

    #[test]
    fn remote_read_cache_trusts_verified_name() {
        // Verify-once contract: object bytes replaced under an existing
        // content-addressed name are not re-detected while the segment
        // is cached (the name is the identity) — the read answers from
        // the verified copy. With the cache disabled the same corruption
        // fails closed, typed (the pre-P2.8 behavior: every read
        // refetches and re-walks).
        let (remote_root, _env, mut db) = remote_only_mirror_150();
        assert_eq!(
            db.get_at(Snapshot::at(150), b"k000").unwrap().as_deref(),
            Some(&b"b000"[..])
        );
        assert!(
            db.history_stats().unwrap().remote_cache_entries > 0,
            "the read populated the cache"
        );
        for entry in fs::read_dir(&remote_root).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("hist") {
                continue;
            }
            let mut bytes = fs::read(&path).unwrap();
            let mid = bytes.len() / 2;
            bytes[mid] ^= 0xff;
            fs::write(&path, bytes).unwrap();
        }
        assert_eq!(
            db.get_at(Snapshot::at(150), b"k000").unwrap().as_deref(),
            Some(&b"b000"[..]),
            "cached entry serves without re-verifying the object"
        );
        db.set_remote_read_cache(0);
        assert!(
            matches!(
                db.get_at(Snapshot::at(150), b"k000"),
                Err(CoreError::CorruptHistory(_))
            ),
            "disabled cache refetches and fails closed on the corruption"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(remote_root.parent().unwrap());
    }

    #[test]
    fn remote_read_cache_oversize_never_caches() {
        // Budget smaller than any segment: nothing is ever cached, every
        // read refetches — the corruption under the object surfaces on
        // the very next read (a cached entry would answer instead).
        let (remote_root, _env, mut db) = remote_only_mirror_150();
        db.set_remote_read_cache(1);
        assert_eq!(
            db.get_at(Snapshot::at(150), b"k000").unwrap().as_deref(),
            Some(&b"b000"[..])
        );
        assert_eq!(
            db.history_stats().unwrap().remote_cache_entries,
            0,
            "oversize entries must not cache"
        );
        for entry in fs::read_dir(&remote_root).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("hist") {
                continue;
            }
            let mut bytes = fs::read(&path).unwrap();
            let mid = bytes.len() / 2;
            bytes[mid] ^= 0xff;
            fs::write(&path, bytes).unwrap();
        }
        assert!(
            matches!(
                db.get_at(Snapshot::at(150), b"k000"),
                Err(CoreError::CorruptHistory(_))
            ),
            "nothing was cached; the read refetches and fails closed"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(remote_root.parent().unwrap());
    }

    #[test]
    fn remote_read_cache_budget_cut_evicts() {
        // Cutting the budget below the held bytes evicts immediately
        // (LRU): an evicted entry is refetched on the next read — the
        // corruption under it surfaces again.
        let (remote_root, _env, mut db) = remote_only_mirror_150();
        assert_eq!(
            db.get_at(Snapshot::at(150), b"k000").unwrap().as_deref(),
            Some(&b"b000"[..])
        );
        assert!(db.history_stats().unwrap().remote_cache_entries > 0);
        db.set_remote_read_cache(1);
        assert_eq!(
            db.history_stats().unwrap().remote_cache_entries,
            0,
            "budget cut must evict held entries"
        );
        for entry in fs::read_dir(&remote_root).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("hist") {
                continue;
            }
            let mut bytes = fs::read(&path).unwrap();
            let mid = bytes.len() / 2;
            bytes[mid] ^= 0xff;
            fs::write(&path, bytes).unwrap();
        }
        assert!(
            matches!(
                db.get_at(Snapshot::at(150), b"k000"),
                Err(CoreError::CorruptHistory(_))
            ),
            "evicted entry is refetched and fails closed on the corruption"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(remote_root.parent().unwrap());
    }

    #[test]
    fn remote_manifest_bound_prunes_fetch() {
        // RFC-0046 P2.7: the remote listing carries the manifest v3
        // key-coverage bound. A corrupt remote segment whose bound
        // excludes the key is never fetched for that key's read — even
        // with its sidecar deleted (the bound alone prunes); its own
        // keys still hit the corruption (fail-closed).
        //
        // Waves a, a (supersede — sheds wave 1 from the LSM), then b:
        // wave 2's segment has a-only coverage and is dropped by the cap
        // when the b-wave seals its own — remote-only, still listed.
        let dir = temp_dir();
        let remote_root = dir.join("remote");
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let env = ClockEnv::new(std::rc::Rc::clone(&clock));
        let mut db =
            Db::open_with_env(&dir, horizon_opts_manual(1_000, 9_000), env.clone()).unwrap();
        db.set_remote_history(env.clone(), remote_root.clone());
        for (prefix, val) in [("a", &b"va"[..]), ("a", &b"wa"[..]), ("b", &b"vb"[..])] {
            for i in 0..100u32 {
                db.put(format!("{prefix}{i:03}").as_bytes(), val).unwrap();
            }
            clock.set(clock.get() + 60_000);
            db.compact_horizon().unwrap();
        }
        let tier = db.history_tier.as_ref().unwrap();
        let remote = crate::history::RemoteTier::new(&remote_root);
        let remote_names: Vec<String> = remote
            .latest_segments(&env)
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        let local_names: Vec<String> = tier.segment_metas().into_iter().map(|m| m.name).collect();
        assert!(
            remote_names.iter().any(|n| !local_names.contains(n)),
            "a listed segment is dropped locally (remote-only): {remote_names:?} vs {local_names:?}"
        );
        assert!(tier.archive_floor() >= 193);
        assert!(db.earliest_readable_sequence() > 288);
        // Corrupt only the a-only remote objects (coverage bound ends
        // before 'b'): those are the ones the b-key read must never
        // fetch. Mixed-range objects legitimately hold b-keys and stay
        // intact — fetching those is correct, not a prune failure.
        let remote = crate::history::RemoteTier::new(&remote_root);
        let mut corrupted = 0;
        for seg in remote.latest_segments(&env).unwrap() {
            let a_only = seg
                .key_hi
                .as_ref()
                .is_some_and(|hi| hi.as_slice() < &b"b000"[..]);
            if !a_only {
                continue;
            }
            let path = remote_root.join(&seg.name);
            let mut bytes = fs::read(&path).unwrap();
            let mid = bytes.len() / 2;
            bytes[mid] ^= 0xff;
            fs::write(&path, bytes).unwrap();
            corrupted += 1;
        }
        assert!(corrupted >= 1, "at least one a-only remote segment");
        let snap = Snapshot::at(288);
        // The b-key read never fetches the corrupt a-only remote segment.
        assert_eq!(
            db.get_at(snap, b"b000").unwrap().as_deref(),
            Some(&b"vb"[..]),
            "out-of-bound corrupt remote segment is inert for this key"
        );
        // The bound, not the sidecar, did it: delete every sidecar and
        // the b-key read still succeeds.
        for entry in fs::read_dir(&remote_root).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) == Some("bloom") {
                fs::remove_file(&path).unwrap();
            }
        }
        assert_eq!(
            db.get_at(snap, b"b000").unwrap().as_deref(),
            Some(&b"vb"[..]),
            "the coverage bound alone prunes the fetch"
        );
        // The a-keys live in the corrupt remote segment: fail-closed.
        assert!(matches!(
            db.get_at(snap, b"a000"),
            Err(CoreError::CorruptHistory(_))
        ));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remote_upload_ships_sidecars_idempotently() {
        // RFC-0046 P2.7: every upload pass ships one sidecar per segment
        // object; retries (AlreadyPresent segments) re-check the sidecar
        // without duplicating or mutating it. The remote is configured
        // after the flush so the two manual passes are the only ones.
        let dir = temp_dir();
        let remote_root = dir.join("remote");
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let env = ClockEnv::new(std::rc::Rc::clone(&clock));
        let mut db =
            Db::open_with_env(&dir, horizon_opts_manual(1_000, 8_192), env.clone()).unwrap();
        for i in 0..100u32 {
            db.put(format!("k{i:03}").as_bytes(), format!("v{i:03}").as_bytes())
                .unwrap();
        }
        clock.set(1_000_000 + 60_000);
        // Seal via the one explicit pass; the remote is configured only
        // after, so the two manual upload passes are the only ones.
        db.compact_horizon().unwrap();
        db.set_remote_history(env, remote_root.clone());
        let count = |ext: &str| {
            fs::read_dir(&remote_root)
                .unwrap()
                .filter(|e| {
                    e.as_ref()
                        .unwrap()
                        .path()
                        .extension()
                        .and_then(|x| x.to_str())
                        == Some(ext)
                })
                .count()
        };
        db.upload_history_now().unwrap();
        let hists = count("hist");
        let blooms = count("bloom");
        assert!(hists >= 1);
        assert_eq!(blooms, hists, "one sidecar per segment object");
        let report = db.upload_history_now().unwrap();
        assert_eq!(
            report.segments_uploaded, 0,
            "second pass is a no-op on segments"
        );
        assert_eq!(count("hist"), hists, "no segment duplication");
        assert_eq!(count("bloom"), blooms, "no sidecar duplication");
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    // ---- RFC-0046 P2.2: bandwidth limiter + metrics ----

    #[test]
    fn upload_bandwidth_limits_rounds_then_completes() {
        // A byte budget ships at most one segment per upload step and —
        // critically — ships no manifest while segments are missing at the
        // destination. The backlog drains step by step; the cap holds the
        // un-uploaded segments until then (backpressure), and stats see it.
        let dir = temp_dir();
        let remote_root = dir.join("remote");
        let clock = std::rc::Rc::new(std::cell::Cell::new(1_000_000u64));
        let (env, outage) =
            ClockEnv::with_fail_prefix(std::rc::Rc::clone(&clock), remote_root.clone());
        let mut db = Db::open_with_env(&dir, horizon_opts(1_000, 2_048), env.clone()).unwrap();
        db.set_remote_history(env.clone(), remote_root.clone());
        outage.set(true);
        for round in 0..3u32 {
            for i in 0..20u32 {
                db.put(b"k", format!("r{round}v{i:08}").as_bytes()).unwrap();
            }
            clock.set(1_000_000 + 60_000 + (round as u64) * 60_000);
            db.flush().unwrap();
        }
        let stats = db.history_stats().unwrap();
        assert!(
            stats.local_segments >= 2,
            "backlog built up (segs={})",
            stats.local_segments
        );
        assert_eq!(stats.pending_uploads, stats.local_segments);
        assert!(stats.last_archive_age_millis.is_some());
        outage.set(false);
        db.set_upload_bandwidth(Some(1)); // one segment per step, effectively
        while db.history_stats().unwrap().pending_uploads > 0 {
            let report = db.upload_history_now().unwrap();
            let stats = db.history_stats().unwrap();
            if stats.pending_uploads > 0 {
                assert_eq!(
                    report.manifest, None,
                    "no manifest may ship while segments are still missing"
                );
            } else {
                assert!(
                    report.manifest.is_some(),
                    "the completing step ships the manifest"
                );
            }
        }
        let stats = db.history_stats().unwrap();
        assert_eq!(stats.pending_uploads, 0);
        let remote = stats.remote.expect("remote summary");
        assert!(
            remote.segments >= 2,
            "mirror complete ({})",
            remote.segments
        );
        assert!(remote.bytes >= stats.local_bytes);
        // With the backlog drained, a flush lets the cap reclaim again.
        db.put(b"k", b"tail").unwrap();
        clock.set(1_000_000 + 300_000);
        db.flush().unwrap();
        assert!(db.history_tier.as_ref().unwrap().bytes() <= 2_048);
        assert!(db.earliest_readable_sequence() > 0, "GC resumed");
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0054 P1.3 micro: cold-window cost of `count_in_range` in the
    /// deps-scan shape (write-CF windows over an MVCC keyspace seeded by
    /// fat `apply_batch` commits, ts suffix in the key — one kernel user
    /// key per version, cap stops after `limit` keys). Run:
    /// `cargo test -p pedradb-core --lib --release count_scan_micro -- --ignored --nocapture`
    /// `MEM_MICRO_N` sets windows per phase (default 999).
    #[test]
    #[ignore]
    fn count_scan_micro() {
        let users = 1024usize;
        let versions = 64u64;
        let n: usize = std::env::var("MEM_MICRO_N")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(999)
            .min(users - 25);
        let rounds: u32 = std::env::var("MEM_MICRO_ROUNDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3);
        let dir = temp_dir();
        let mut opts = OpenOptions::default();
        opts.sync = false;
        // Bench keeps the whole keyspace in mem (264k entries, no SST
        // cursors on the count path); auto-flush would change the shape.
        opts.auto_flush_bytes = None;
        let mut db = Db::open_with(&dir, opts).unwrap();
        let val = Bytes::from_static(b"d");
        let key = |cf: &str, u: usize, ts: Option<u64>| -> Bytes {
            let mut k = Vec::with_capacity(cf.len() + 1 + 8 + 8);
            k.extend_from_slice(cf.as_bytes());
            k.push(0);
            k.extend_from_slice(format!("u/{u:06}").as_bytes());
            if let Some(ts) = ts {
                k.extend_from_slice(&ts.to_be_bytes());
            }
            Bytes::from(k)
        };
        // Seed: fat prewrite+commit rounds (bench deps_apply_batch shape).
        let mut ts = 0u64;
        for _ in 0..versions {
            let mut pre = Vec::with_capacity(users * 2);
            let mut com = Vec::with_capacity(users * 2);
            for u in 0..users {
                ts += 1;
                pre.push(BatchOp::Put {
                    key: key("lock", u, None),
                    value: Bytes::from_static(b"l"),
                });
                pre.push(BatchOp::Put {
                    key: key("default", u, Some(ts)),
                    value: val.clone(),
                });
                com.push(BatchOp::Put {
                    key: key("write", u, Some(ts)),
                    value: Bytes::from_static(b"c"),
                });
                com.push(BatchOp::Delete {
                    key: key("lock", u, None),
                });
            }
            db.apply_batch(pre).unwrap();
            db.apply_batch(com).unwrap();
        }
        // Phase A — cold windows: a dirty key inside the window forces the
        // kernel count recompute (CountCache::record_dirty range check).
        // The dirtying put is OUTSIDE the timed section.
        let mut cold_ns: u128 = 0;
        for _ in 0..rounds.max(1) {
            for u in 0..n {
                ts += 1;
                db.apply_batch([BatchOp::Put {
                    key: key("write", u, Some(ts)),
                    value: Bytes::from_static(b"c"),
                }])
                .unwrap();
                let t0 = std::time::Instant::now();
                let c = db
                    .count_in_range(
                        db.visible_sequence(),
                        Bound::Included(&key("write", u, None)),
                        Bound::Excluded(&key("write", u + 25, None)),
                        Some(25),
                    )
                    .unwrap();
                cold_ns += t0.elapsed().as_nanos();
                std::hint::black_box(c);
            }
        }
        let cold = cold_ns as f64 / (n as f64 * rounds.max(1) as f64) / 1000.0;
        // Phase B — repeated windows, no writes (kernel count-cache hits
        // need `latest`, so read visible_sequence per call).
        let t0 = std::time::Instant::now();
        for u in 0..n {
            let c = db
                .count_in_range(
                    db.visible_sequence(),
                    Bound::Included(&key("write", u, None)),
                    Bound::Excluded(&key("write", u + 25, None)),
                    Some(25),
                )
                .unwrap();
            std::hint::black_box(c);
        }
        let warm = t0.elapsed().as_secs_f64() / n as f64 * 1e6;
        println!(
            "count scan micro: users={users} vers={versions} rounds={rounds} cold={cold:.3}µs/window warm={warm:.3}µs/window (n={n}, mem_entries={}, ssts={})",
            db.stats().mem_entries,
            db.stats().sst_count
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0159 P0.2: a latched family's pure-append span installs directly
    /// at the bottom level; the repeated-key meta family stays on the
    /// ladder; settle does not rewrite the bulk chunks; reopen restores
    /// the levels.
    #[test]
    fn bulk_ingest_installs_latched_family_at_bottom_level() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                sync: false,
                ..OpenOptions::default()
            },
        )
        .unwrap();
        db.set_physical_cfs(vec!["data".into(), "meta".into()]);
        let v = vec![b'v'; 200];
        let mut keys = Vec::new();
        for b in 0..40u32 {
            let mut batch = Vec::new();
            for j in 0..16u32 {
                let k = format!("data\0{b:04}-{j:04}").into_bytes();
                batch.push(BatchOp::put(k.clone(), v.clone()));
                keys.push(k);
            }
            // The slipstream shape: one repeated cursor key in another
            // family every batch.
            batch.push(BatchOp::put(b"meta\0cursor".to_vec(), b"c".to_vec()));
            db.apply_batch(batch).unwrap();
        }
        db.flush().unwrap();

        let mut data_max = 0usize;
        let mut data_elsewhere = 0usize;
        let mut meta_l0 = 0usize;
        let mut bulk_paths = Vec::new();
        for (t, &lvl) in db.ssts.iter().zip(db.sst_levels.iter()) {
            if t.cf() == "data" {
                if lvl == MAX_LSM_LEVEL {
                    data_max += 1;
                    bulk_paths.push(t.path().to_path_buf());
                } else {
                    data_elsewhere += 1;
                }
            } else if t.cf() == "meta" && lvl == 0 {
                meta_l0 += 1;
            }
        }
        assert_eq!(
            data_elsewhere, 0,
            "every data chunk must land at the bottom level"
        );
        assert_eq!(data_max, 1, "one flush = one bulk chunk");
        assert_eq!(meta_l0, 1, "repeated cursor key never latches");

        for (i, k) in keys.iter().enumerate() {
            assert_eq!(
                db.get(k).as_deref(),
                Some(&v[..]),
                "bulk key {i} must read back"
            );
        }
        assert_eq!(db.get(b"meta\0cursor").as_deref(), Some(&b"c"[..]));

        // Settle is a no-op for the bulk family: chunk files unchanged.
        db.compact().unwrap();
        let after: Vec<std::path::PathBuf> = db
            .ssts
            .iter()
            .zip(db.sst_levels.iter())
            .filter(|(_, &l)| l == MAX_LSM_LEVEL)
            .map(|(t, _)| t.path().to_path_buf())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|f| bulk_paths.iter().any(|b| b.file_name() == Some(f)))
            })
            .collect();
        assert_eq!(after.len(), bulk_paths.len(), "compact rewrote bulk chunks");
        for k in &keys {
            assert_eq!(db.get(k).as_deref(), Some(&v[..]));
        }

        drop(db);
        let db2 = Db::open(&dir).unwrap();
        let max_files = db2
            .ssts
            .iter()
            .zip(db2.sst_levels.iter())
            .filter(|(_, &l)| l == MAX_LSM_LEVEL)
            .count();
        assert_eq!(max_files, 1, "reopen must restore the bottom-level chunk");
        for (i, k) in keys.iter().enumerate() {
            assert_eq!(db2.get(k).as_deref(), Some(&v[..]), "post-reopen {i}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reference for the run-bisect oracle: the pre-run flat walk, copied
    /// verbatim from the old `lookup` SST loop (newest-first, lazy tombstone
    /// collect up to the winner, first-found-point wins).
    fn lookup_linear_reference(db: &Db, key: &[u8], snapshot: SequenceNumber) -> Lookup {
        let mut best_point_seq: Option<SequenceNumber> = None;
        let mut best_point = Lookup::NotFound;
        let mut range_tombs = Vec::new();
        for table in db.mem_layers() {
            Db::<StdEnv>::scan_mem_for_lookup(
                table,
                key,
                snapshot,
                &mut best_point_seq,
                &mut best_point,
                &mut range_tombs,
            );
            if let Some(seq) = best_point_seq {
                return match best_point {
                    Lookup::Found(_)
                        if !crate::merge::visible_at(
                            crate::key::ValueType::Value,
                            range_deleted(key, seq, &range_tombs),
                        ) =>
                    {
                        Lookup::Deleted
                    }
                    other => other,
                };
            }
        }
        let mut seek_scratch = crate::sst::PointSeekScratch::default();
        for &sst_i in db.sst_indices_newest_first() {
            let table = &db.ssts[sst_i];
            table.collect_range_tombstones(snapshot, &mut range_tombs);
            if let (Some(lo), Some(hi)) = (table.smallest_user_key(), table.largest_user_key()) {
                if key < lo || key > hi {
                    continue;
                }
            }
            match table.point_at_seeking(key, snapshot, &mut seek_scratch) {
                Ok(Some((seq, look))) => {
                    best_point_seq = Some(seq);
                    best_point = look;
                    break;
                }
                Ok(None) => {}
                Err(e) => fail_stop_corrupt_block(table.path(), &e),
            }
        }
        match best_point {
            Lookup::Found(v) => {
                let seq = best_point_seq.unwrap_or(0);
                if crate::merge::visible_at(
                    crate::key::ValueType::Value,
                    range_deleted(key, seq, &range_tombs),
                ) {
                    Lookup::Found(v)
                } else {
                    Lookup::Deleted
                }
            }
            Lookup::Deleted => Lookup::Deleted,
            Lookup::NotFound => {
                if range_deleted(key, 0, &range_tombs) {
                    Lookup::Deleted
                } else {
                    Lookup::NotFound
                }
            }
        }
    }

    /// Point-lookup run bisect over disjoint bottom-level chunks: `lookup`
    /// must agree with the flat-walk reference and with a ground-truth model
    /// for live keys, deleted keys, range-tombstone spans, chunk boundaries,
    /// and in-gap keys — live and after reopen (lazy tables).
    #[test]
    fn lookup_bisect_disjoint_run_matches_linear_walk() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                sync: false,
                ..OpenOptions::default()
            },
        )
        .unwrap();
        db.set_physical_cfs(vec!["data".into()]);

        // Three ascending flushes → three pairwise-disjoint bottom chunks.
        let mut model: std::collections::BTreeMap<Vec<u8>, Option<Vec<u8>>> =
            std::collections::BTreeMap::new();
        for tag in ["a", "b", "c"] {
            for j in 0..50u32 {
                let k = format!("data\0{tag}{j:03}").into_bytes();
                let v = format!("v{tag}{j:03}").into_bytes();
                db.put(&k, &v).unwrap();
                model.insert(k, Some(v));
            }
            db.flush().unwrap();
        }
        // Descending batch kills the family latch so everything after lands
        // on L0 above the chunks: overwrite, point delete, range tombstone
        // spanning the a/b chunk boundary, second overwrite outside it.
        let kill = vec![
            BatchOp::put(b"data\0z999".to_vec(), b"z".to_vec()),
            BatchOp::put(b"data\0a998".to_vec(), b"z".to_vec()),
        ];
        db.apply_batch(kill).unwrap();
        model.insert(b"data\0z999".to_vec(), Some(b"z".to_vec()));
        model.insert(b"data\0a998".to_vec(), Some(b"z".to_vec()));

        db.put(b"data\0a050", b"over").unwrap();
        model.insert(b"data\0a050".to_vec(), Some(b"over".to_vec()));
        db.delete(b"data\0a000").unwrap();
        model.insert(b"data\0a000".to_vec(), None);
        db.delete_range(b"data\0b020", b"data\0b040").unwrap();
        for j in 20..40u32 {
            model.insert(format!("data\0b{j:03}").into_bytes(), None);
        }
        db.put(b"data\0b060", b"over2").unwrap();
        model.insert(b"data\0b060".to_vec(), Some(b"over2".to_vec()));
        db.flush().unwrap();

        // Shape: a bottom-level run with ≥3 disjoint tables (bisect armed)
        // and an L0 run above it (linear walk).
        let bottom = db
            .sst_runs
            .iter()
            .find(|r| r.level == MAX_LSM_LEVEL)
            .expect("bottom-level run");
        assert!(
            bottom.tables_newest_first.len() >= 3,
            "want ≥3 bottom chunks, got {}",
            bottom.tables_newest_first.len()
        );
        let by_lo = bottom
            .disjoint_by_lo
            .as_ref()
            .expect("bottom chunks must bisect");
        assert_eq!(by_lo.len(), bottom.tables_newest_first.len());
        assert!(db.sst_runs.iter().any(|r| r.level == 0));
        assert!(db.sst_runs[0].level == 0, "L0 run comes first");
        assert!(
            db.sst_runs[0].disjoint_by_lo.is_none(),
            "L0 keeps the linear walk"
        );

        // Oracle sweep: model keys + chunk boundaries + gaps + tombstone
        // endpoints + outside-all keys.
        let mut probes: Vec<Vec<u8>> = model.keys().cloned().collect();
        for k in [
            &b"data\0"[..],
            &b"data\0a"[..],
            &b"data\0a049x"[..],
            &b"data\0b"[..],
            &b"data\0b019x"[..],
            &b"data\0b020"[..],
            &b"data\0b039"[..],
            &b"data\0b039x"[..],
            &b"data\0b040"[..],
            &b"data\0c"[..],
            &b"data\0c049x"[..],
            &b"data\0zzz"[..],
        ] {
            probes.push(k.to_vec());
        }
        let snapshot = db.last_sequence();
        for k in &probes {
            let got = db.lookup(k, snapshot);
            let want = lookup_linear_reference(&db, k, snapshot);
            assert_eq!(got, want, "bisect vs flat walk at {k:?}");
            let expected = model.get(k).map_or(Lookup::NotFound, |m| match m {
                Some(v) => Lookup::Found(v.clone().into()),
                None => Lookup::NotFound,
            });
            // The model cannot tell Deleted from NotFound; both read absent.
            let agree = match (&got, &expected) {
                (Lookup::Found(_), Lookup::Found(_)) => got == expected,
                (Lookup::Found(_), _) | (_, Lookup::Found(_)) => false,
                _ => true,
            };
            assert!(
                agree,
                "model mismatch at {k:?}: got {got:?}, model {expected:?}"
            );
        }

        db.close().unwrap();
        let db = Db::open(&dir).unwrap();
        let snapshot = db.last_sequence();
        for k in &probes {
            let got = db.lookup(k, snapshot);
            let want = lookup_linear_reference(&db, k, snapshot);
            assert_eq!(got, want, "post-reopen bisect vs flat walk at {k:?}");
        }
        let bottom = db
            .sst_runs
            .iter()
            .find(|r| r.level == MAX_LSM_LEVEL)
            .expect("bottom-level run after reopen");
        assert!(bottom.disjoint_by_lo.is_some(), "reopen rebuilds runs");
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0159 P0.2: a descending batch kills the family; later spans
    /// install at L0 again and every version stays readable.
    #[test]
    fn bulk_ingest_descent_falls_back_to_ladder() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                sync: false,
                ..OpenOptions::default()
            },
        )
        .unwrap();
        db.set_physical_cfs(vec!["data".into()]);
        let v = vec![b'v'; 120];
        for b in 0..20u32 {
            let mut batch = Vec::new();
            for j in 0..4u32 {
                let k = format!("data\0a{b:04}-{j:04}").into_bytes();
                batch.push(BatchOp::put(k, v.clone()));
            }
            db.apply_batch(batch).unwrap();
        }
        db.flush().unwrap();
        assert!(
            db.ssts
                .iter()
                .zip(db.sst_levels.iter())
                .any(|(t, &l)| t.cf() == "data" && l == MAX_LSM_LEVEL),
            "ascending stream must bulk"
        );

        // Descent: a key below the flushed range kills the family.
        db.apply_batch(vec![BatchOp::put(b"data\0a0000-0000".to_vec(), v.clone())])
            .unwrap();
        for b in 20..24u32 {
            let mut batch = Vec::new();
            for j in 0..4u32 {
                let k = format!("data\0a{b:04}-{j:04}").into_bytes();
                batch.push(BatchOp::put(k, v.clone()));
            }
            db.apply_batch(batch).unwrap();
        }
        db.flush().unwrap();
        assert!(
            db.ssts
                .iter()
                .zip(db.sst_levels.iter())
                .any(|(t, &l)| t.cf() == "data" && l == 0),
            "post-kill span must stay on the L0 ladder"
        );
        // Overwrite of an existing key still reads the newest version.
        assert_eq!(db.get(b"data\0a0000-0000").as_deref(), Some(&v[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0159 P0.2: a delete in the span keeps the family latched (the
    /// P0.1 rule) but routes that span's flush to L0 — a bottom-level
    /// chunk may not carry an unmerged tombstone.
    #[test]
    fn bulk_ingest_tombstone_span_routes_ladder() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                sync: false,
                ..OpenOptions::default()
            },
        )
        .unwrap();
        db.set_physical_cfs(vec!["data".into()]);
        let v = vec![b'v'; 120];
        for b in 0..12u32 {
            let mut batch = Vec::new();
            for j in 0..4u32 {
                let k = format!("data\0b{b:04}-{j:04}").into_bytes();
                batch.push(BatchOp::put(k, v.clone()));
            }
            db.apply_batch(batch).unwrap();
        }
        db.flush().unwrap();
        assert!(db
            .ssts
            .iter()
            .zip(db.sst_levels.iter())
            .any(|(t, &l)| t.cf() == "data" && l == MAX_LSM_LEVEL));

        // Delete of an already-bulked key rides with the next span: that
        // flush carries a tombstone so it routes the ladder, and the L0
        // tombstone must shadow the bottom-level bulk chunk on reads.
        db.apply_batch(vec![BatchOp::delete(b"data\0b0000-0000")])
            .unwrap();
        for b in 0..4u32 {
            let mut batch = Vec::new();
            for j in 0..4u32 {
                let k = format!("data\0c{b:04}-{j:04}").into_bytes();
                batch.push(BatchOp::put(k, v.clone()));
            }
            db.apply_batch(batch).unwrap();
        }
        db.flush().unwrap();
        assert!(
            db.ssts
                .iter()
                .zip(db.sst_levels.iter())
                .any(|(t, &l)| t.cf() == "data" && l == 0),
            "tombstone-carrying span must install at L0"
        );
        assert_eq!(
            db.get(b"data\0b0000-0000").as_deref(),
            None,
            "L0 tombstone must shadow the bulk chunk at the bottom level"
        );
        assert_eq!(
            db.get(b"data\0c0001-0001").as_deref(),
            Some(&v[..]),
            "put after the delete must survive"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0159 P0.2: no physical CFs — the single "default" family latches
    /// and installs at the bottom level too.
    #[test]
    fn bulk_ingest_default_family_installs_at_bottom() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                sync: false,
                ..OpenOptions::default()
            },
        )
        .unwrap();
        for b in 0..12u32 {
            let mut batch = Vec::new();
            for j in 0..4u32 {
                batch.push(BatchOp::put(
                    format!("k{b:04}-{j:04}").into_bytes(),
                    b"v".to_vec(),
                ));
            }
            db.apply_batch(batch).unwrap();
        }
        db.flush().unwrap();
        assert!(
            db.ssts
                .iter()
                .zip(db.sst_levels.iter())
                .any(|(_, &l)| l == MAX_LSM_LEVEL),
            "default-family append stream must bulk"
        );
        assert_eq!(db.get(b"k0011-0003").as_deref(), Some(&b"v"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0159 P1.11: bulk SST installs with empty payload; first get
    /// promotes into the pool when there is room (no ghost-register).
    #[test]
    fn bulk_empty_payload_promotes_on_first_get() {
        let dir = temp_dir();
        let mut opts = OpenOptions {
            sync: false,
            ..OpenOptions::default()
        };
        opts.sst_payload_budget_bytes = Some(8 << 20);
        let mut db = Db::<StdEnv>::open_with_env_bounded(&dir, opts, StdEnv).unwrap();
        db.set_physical_cfs(vec!["data".into()]);
        let v = vec![b'v'; 40];
        let mut keys = Vec::new();
        for b in 0..12u32 {
            let mut batch = Vec::new();
            for j in 0..8u32 {
                let k = format!("data\0{b:04}-{j:04}").into_bytes();
                batch.push(BatchOp::put(k.clone(), v.clone()));
                keys.push(k);
            }
            db.apply_batch(batch).unwrap();
        }
        db.flush().unwrap();
        assert!(
            db.ssts.iter().all(|t| !t.payload_resident()),
            "bulk install must leave payloads empty (no whole-file pin)"
        );
        let before = db.sst_payload_pool().resident_bytes();
        assert_eq!(db.get(&keys[0]).as_deref(), Some(&v[..]));
        assert!(
            db.sst_payload_pool().resident_bytes() > before
                || db.ssts.iter().any(|t| t.payload_resident()),
            "first get must promote a file that fits the budget"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0161: a byte-budgeted BlockCache must not tax cold point gets.
    /// lookup_100's keys never repeat; `point_at_cached` decode-insert was
    /// guest v73b's 5.25 ms get_loop (v71 seeking 3.868 ms). Answers stay
    /// correct; occupancy stays 0.
    #[test]
    fn cold_point_get_does_not_insert_decoded_block_cache() {
        let dir = temp_dir();
        let mut db = Db::<StdEnv>::open_with_env_bounded(
            &dir,
            OpenOptions {
                sync: false,
                auto_flush_bytes: Some(4 * 1024),
                sst_payload_budget_bytes: Some(1),
                ..OpenOptions::default()
            },
            StdEnv,
        )
        .unwrap();
        db.install_block_cache(BlockCache::with_budget_bytes(4 << 20));
        db.put(b"k1", vec![b'v'; 64]).unwrap();
        db.put(b"k2", vec![b'w'; 64]).unwrap();
        db.flush().unwrap();
        db.block_cache().reset_stats();
        let before = db.block_cache().used_bytes();
        crate::sst::force_get_stages(true);
        crate::sst::reset_get_stages();
        assert_eq!(db.get(b"k1").as_deref(), Some(&[b'v'; 64][..]));
        assert_eq!(db.get(b"k2").as_deref(), Some(&[b'w'; 64][..]));
        let (pread, crc, walk) = crate::sst::take_get_stages();
        crate::sst::force_get_stages(false);
        assert_eq!(
            db.block_cache().used_bytes(),
            before,
            "cold point get must not decode-insert into BlockCache"
        );
        assert_eq!(
            db.block_cache().misses(),
            0,
            "point get must not miss-fill BlockCache, misses={}",
            db.block_cache().misses()
        );
        assert!(
            pread > 0 && crc > 0 && walk > 0,
            "budgeted cache must still use seeking miss path, stages=({pread},{crc},{walk})"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0160 P0.5: `auto_flush_bytes: None` still caps BulkRun (the
    /// 100M SIGKILL was an unbounded open tail).
    #[test]
    fn bulk_chunk_cap_defaults_when_no_flush_threshold() {
        let dir = temp_dir();
        let db = Db::open_with(
            &dir,
            OpenOptions {
                sync: false,
                auto_flush_bytes: None,
                ..OpenOptions::default()
            },
        )
        .unwrap();
        assert_eq!(db.bulk_chunk_cap(), DEFAULT_BULK_CHUNK_BYTES);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0161 P0.5: a 256 MiB write buffer (Slipstream) must not raise
    /// the BulkRun chunk above 64 MiB on the 3.9 GiB guest.
    #[test]
    fn bulk_chunk_cap_clamps_large_write_buffer() {
        let dir = temp_dir();
        let db = Db::open_with(
            &dir,
            OpenOptions {
                sync: false,
                auto_flush_bytes: Some(256 * 1024 * 1024),
                ..OpenOptions::default()
            },
        )
        .unwrap();
        assert_eq!(db.bulk_chunk_cap(), DEFAULT_BULK_CHUNK_BYTES);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0162 P0.2: BSTAGE line format and off-by-default.
    #[test]
    fn bulk_stage_timing_default_off_and_line_format() {
        assert!(!bulk_stage_timing_on());
        let line = bulk_stage_line(7, 12_345, 678, 262_144, 67_108_864, "worker", true);
        assert!(
            line.starts_with("BSTAGE idx=7 epoch_ms=12345 t_ms=678 "),
            "{line}"
        );
        assert!(line.contains("entries=262144 bytes=67108864 caller=worker sync=true"));
        let inline = bulk_stage_line(0, 0, 0, 1, 1, "inline", false);
        assert!(inline.contains("caller=inline sync=false"));
    }
}

#[cfg(all(test, feature = "buggify"))]
mod buggify_engine_tests {
    use super::*;

    /// RFC-0050 P1.3: feature + seeded table ⇒ engine sites inject delay or
    /// fail-stop `io::Error` (never silent wrong); reopen stays clean.
    /// Without the feature every site is a no-op (default suite proves it).
    #[test]
    fn engine_matrix_fires_and_survives_fixed_seed() {
        let dir = std::env::temp_dir().join(format!("pedra-buggify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        crate::buggify_hooks::clear_table();
        crate::buggify_hooks::install_from_seed(0xB175_F1ED);

        let mut injected_errs = 0u32;
        {
            let mut db = Db::open(&dir).unwrap();
            for i in 0..64u64 {
                let k = format!("k{i}");
                if db.put(k.as_bytes(), b"v").is_err() {
                    injected_errs += 1;
                }
            }
            let _ = db.flush();
            let _ = db.compact();
        }
        let counts = crate::buggify_hooks::installed_fire_counts();
        let total: u64 = counts.iter().map(|(_, c)| c).sum();
        assert!(total > 0, "matrix must fire >=1 site under fixed seed");
        assert!(
            injected_errs > 0,
            "fixed seed must produce >=1 fail-stop injection, counts={counts:?}"
        );
        crate::buggify_hooks::clear_table();

        // Fail-stop only: reopen clean, no silent corruption.
        let db = Db::open(&dir).unwrap();
        let _ = db.get(b"k0");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Chunked, lazy memtable stream (F221): refills bounded chunks of distinct
/// user keys on demand instead of materializing the whole `[start, end)`
/// range per window refill — a window's `limit` bounds only merge emission,
/// so collection used to sweep from the window start to the range end
/// (quadratic over long scans; sampled as Vec realloc + memmove in the scan
/// hot path). Chunks keep the vlog prefetch batching (RFC-0029): each
/// resolved chunk still goes through [`Db::prefetch_resolve_stream`].
struct MemChunkStream<'a, E: Env = StdEnv> {
    table: &'a MemTable,
    start: Bound<Bytes>,
    end: Bound<Bytes>,
    last: Option<Bytes>,
    snapshot: SequenceNumber,
    resolve: bool,
    db: &'a Db<E>,
    buf: std::vec::IntoIter<(InternalKey, Bytes)>,
}

/// Distinct user keys collected per chunk: bounds upfront work for
/// window-limited consumers (compat `ITER_WINDOW` = 64) while amortizing
/// chunk setup for full-range walkers.
const MEM_STREAM_CHUNK: usize = 256;

impl<'a, E: Env> MemChunkStream<'a, E> {
    fn refill(&mut self) -> bool {
        // The internal iterator borrows its bounds for its whole lifetime;
        // building it per chunk from owned copies keeps those borrows local
        // to this call. Resume position: every version of `last` was already
        // emitted or skipped, so `Excluded(last)` is exact.
        let seek_from = match self.last.clone() {
            Some(last) => Bound::Excluded(last),
            None => self.start.clone(),
        };
        let end = self.end.clone();
        let table = self.table;
        let mut iter = table.iter_internal_iter_at(
            crate::merge::bound_as_ref(&seek_from),
            crate::merge::bound_as_ref(&end),
            self.snapshot,
        );
        let mut chunk: Vec<(InternalKey, Bytes)> = Vec::with_capacity(MEM_STREAM_CHUNK);
        while chunk.len() < MEM_STREAM_CHUNK {
            let Some((k, v)) = iter.next() else { break };
            if k.kind == ValueType::RangeDeletion || k.sequence > self.snapshot {
                continue;
            }
            if self.last.as_ref().is_some_and(|u| u == &k.user_key) {
                continue;
            }
            self.last = Some(k.user_key.clone());
            let value = if self.resolve {
                v.clone()
            } else {
                Bytes::new()
            };
            chunk.push((k.clone(), value));
        }
        drop(iter);
        if chunk.is_empty() {
            return false;
        }
        if self.resolve {
            self.db.prefetch_resolve_stream(&mut chunk);
        }
        self.buf = chunk.into_iter();
        true
    }
}

impl<'a, E: Env> Iterator for MemChunkStream<'a, E> {
    type Item = (InternalKey, Bytes);
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(x) = self.buf.next() {
            return Some(x);
        }
        if self.refill() {
            self.buf.next()
        } else {
            None
        }
    }
}
