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

use std::collections::BTreeMap;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use bytes::Bytes;

use crate::batch::{WriteOp, WriteRecord};
use crate::cache::{AnswerCache, BlockCache, PointCache, TableCache};
use crate::change_feed::{ChangeEntry, ChangeKind, ChangeLog};
use crate::changelog_kernel::{changelog_needs_sst_rebuild, changelog_should_store};
use crate::env::{Env, EnvFile, StdEnv};
use crate::error::{CoreError, Result};
use crate::host::Host;
use crate::key::{InternalKey, SequenceNumber, ValueType, MAX_SEQUENCE_NUMBER};
use crate::lock::DirLock;
use crate::manifest::{self, VersionSet};
use crate::memtable::{Lookup, MemTable};
use crate::merge::{range_deleted, StreamingVisibleIter, VisibleKv};
use crate::sst::{write_sst_entries_on, write_sst_on_with, write_sst_try_sorted_on, SstTable};
use crate::tx::Transaction;
use crate::vlog::{self, ValueLog, VlogRewriteStats, VLOG_FILE_NAME};
use crate::wal::Wal;
use parking_lot::Mutex;
use std::io::{Read, Write};
use std::sync::Arc;

/// Max LSM level we promote into (L0 = flush target, L1+ = compacted).
pub const MAX_LSM_LEVEL: u32 = 3;
/// When L0 file count reaches this, auto-compact merges L0 → L1 (subset compact).
pub const L0_COMPACTION_TRIGGER: usize = 4;

/// Default WAL file name inside the DB directory.
pub const WAL_FILE_NAME: &str = "CURRENT.log";

/// Options for [`Db::open`].
#[derive(Debug, Clone, Copy)]
pub struct OpenOptions {
    /// When true (default), each successful `put`/`delete`/`commit` `fdatasync`s
    /// the WAL before returning (RFC-0001 O1 / RFC-0036). Overridable per write
    /// via [`WriteOptions`].
    pub sync: bool,
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
            value: Bytes::copy_from_slice(value.as_ref()),
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
            // 4 MiB default encourages SST creation under load without manual flush.
            auto_flush_bytes: Some(4 * 1024 * 1024),
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
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

/// What a range scan yields (RFC-0019 P1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScanProjection {
    /// Full key + resolved value (default).
    #[default]
    Full,
    /// User keys only; `VisibleKv::value` is empty (no vlog resolve).
    KeyOnly,
}

/// L0 files snapshotted for an off-lock rewrite (RFC-0037 P1.2).
///
/// Inputs stay in the live inventory until [`Db::install_prepared_l0_compact`].
/// [`Self::write`] does not need the `Db` write lock.
pub struct PreparedL0Compact<E: Env> {
    inputs: Vec<SstTable>,
    file_num: u64,
    gc: crate::merge::CompactGcOptions,
    dir: PathBuf,
    env: E,
    sync: bool,
}

impl<E: Env> PreparedL0Compact<E> {
    /// Paths of L0 files this job will replace.
    #[must_use]
    pub fn input_paths(&self) -> Vec<PathBuf> {
        self.inputs.iter().map(|t| t.path().to_path_buf()).collect()
    }

    /// Merge inputs into a new SST (streaming when `gc` is default).
    ///
    /// # Errors
    /// SST encode / I/O. On error the live L0 inventory is unchanged.
    pub fn write(&self) -> Result<SstTable> {
        write_merged_tables(
            &self.env,
            &self.dir,
            self.file_num,
            &self.inputs,
            self.gc,
            self.sync,
        )
    }
}

/// Options for [`Db::compact_with`] (RFC-0009 compaction / version GC).
#[derive(Debug, Clone, Copy, Default)]
pub struct CompactOptions {
    /// Version GC policy applied while rewriting SSTs.
    pub gc: crate::merge::CompactGcOptions,
}

impl CompactOptions {
    /// Compact keeping only the newest version of each user key.
    #[must_use]
    pub fn latest_only() -> Self {
        Self {
            gc: crate::merge::CompactGcOptions::latest_only(),
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
pub struct Db<E: Env = StdEnv> {
    dir: PathBuf,
    env: E,
    /// Shared so ConcurrentDb can `fdatasync` without the Db write lock
    /// (RFC-0041 P1.1). Rotate waits for [`Self::commit_inflight`] == 0.
    wal: Arc<Mutex<Wal<E::File>>>,
    /// Group-commit appends in flight (not yet applied). Blocks WAL rotate.
    commit_inflight: AtomicUsize,
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
    /// Flushed pins waiting to be folded (cheap push on the write path).
    retired_pending: Vec<MemTable>,
    /// Single BTree of flushed versions (built off-lock when writers idle).
    retired_fold: MemTable,
    /// How many L0 files the retired cache covers (pending + fold).
    retired_l0s: usize,
    /// Cached [`Self::sst_indices_newest_first`] (L0 newest → L1+).
    sst_order_newest: Vec<usize>,
    /// Immutable tables, oldest → newest within inventory order.
    ssts: Vec<SstTable>,
    /// LSM level for each entry in [`Self::ssts`] (parallel array; 0 = L0).
    sst_levels: Vec<u32>,
    /// L0 files written without `fdatasync`. Must be synced before WAL rotate
    /// or any MANIFEST publish (RFC-0041). Crash before that is recovered
    /// from WAL; `gc_orphan_ssts` drops the unsynced files.
    unsynced_ssts: Vec<PathBuf>,
    /// Next SST file number (`000001.sst`, …).
    next_file_num: u64,
    /// Last written MANIFEST file number (0 = none yet).
    manifest_file_num: u64,
    /// MANIFEST flag: open `VALUES.vlog.new` (SST pointers already remapped).
    vlog_use_new: bool,
    /// Next sequence to assign (1-based; 0 means “no writes yet”).
    next_seq: SequenceNumber,
    /// Highest sequence default reads may observe. Assigned (`next_seq-1`)
    /// may be ahead while mem is applied but WAL `fdatasync` has not finished
    /// (G1: Ok and `get` wait for publish after fd).
    published_seq: AtomicU64,
    sync: bool,
    auto_flush_bytes: Option<usize>,
    auto_compact_sst_count: Option<usize>,
    auto_compact_sst_bytes: Option<u64>,
    /// Reuses decoded SST handles (verify / reopen path).
    table_cache: TableCache,
    /// Decompressed block cache (hit stats for read path).
    block_cache: BlockCache,
    /// Latest-snapshot point answers; cleared on write (RFC-0035).
    point_cache: PointCache,
    /// Latest `last_under_user_prefix` answers; cleared on write.
    last_prefix_cache: AnswerCache<Option<Bytes>>,
    /// Latest `count_in_range` answers; cleared on write.
    count_cache: AnswerCache<usize>,
    /// Exclusive directory lock (released via Env on close/drop when possible).
    dir_lock: Option<DirLock>,
    /// Set when append succeeded but required WAL `sync_all` failed (RFC-0015 H1).
    durability_fenced: bool,
    /// Auto-compact failures after successful flush (RFC-0015 M4 / P2.2).
    auto_compact_failures: u64,
    /// Last auto-compact error message (cleared only on successful auto-compact).
    last_auto_compact_error: Option<String>,
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
    /// Durable commits since the last CHANGELOG store.
    commits_since_changelog: u64,
    /// Successful CHANGELOG stores since open.
    changelog_store_count: u64,
}

impl Db<StdEnv> {
    /// Open or create a database at `path` (directory) on the real filesystem.
    ///
    /// Loads `*.sst` files, then recovers the WAL into the MemTable.
    ///
    /// # Errors
    /// I/O failures, corrupt logical records, or CRC errors on non-truncated data.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, OpenOptions::default())
    }

    /// Open with explicit options on the real filesystem.
    ///
    /// # Errors
    /// Same as [`Self::open`].
    pub fn open_with(path: impl AsRef<Path>, opts: OpenOptions) -> Result<Self> {
        Self::open_with_env(path, opts, StdEnv)
    }
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
    /// # Errors
    /// I/O failures, corrupt logical records, CRC errors, [`CoreError::AlreadyOpen`],
    /// or corrupt MANIFEST.
    #[allow(clippy::too_many_lines)] // recover WAL + CHANGELOG + vlog in one open path
    pub fn open_with_env(path: impl AsRef<Path>, opts: OpenOptions, env: E) -> Result<Self> {
        let _ = crate::buggify_hooks::maybe_arm(crate::buggify_hooks::sites::AFTER_OPEN_LOCK);
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
        let point_cache = PointCache::new(2048);
        let last_prefix_cache = AnswerCache::new(2048);
        let count_cache = AnswerCache::new(2048);
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

        if env.exists(&wal_path) {
            // Tiny WAL + Truncated(0): failed first append after rotate (or crash
            // before any complete record). Tolerate empty so SSTs still load (F6).
            // Large WAL + Truncated(0): bitrot of the first record — fail-stop (F4),
            // journaled for escalation (RFC-0038 D: repeated events refuse open).
            let (records, last_good) = match Wal::recover_span_on(&env, &wal_path) {
                Ok(r) => r,
                Err(CoreError::Truncated(0)) => {
                    let len = env.metadata_len(&wal_path).unwrap_or(0);
                    if len < 64 {
                        (Vec::new(), 0)
                    } else {
                        return Err(crate::corrupt::escalate_or_fail(
                            &env,
                            &dir,
                            "truncated_head",
                            0,
                            CoreError::Truncated(0),
                        ));
                    }
                }
                Err(e @ CoreError::Crc { offset, .. }) => {
                    // Mid-WAL bitflip: fail-stop (silent skip is G8-forbidden),
                    // journaled; the Nth event escalates (RFC-0038 D).
                    return Err(crate::corrupt::escalate_or_fail(
                        &env, &dir, "crc", offset, e,
                    ));
                }
                Err(e) => return Err(e),
            };
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

        let wal = Arc::new(Mutex::new(if env.exists(&wal_path) {
            Wal::append_on(&env, &wal_path)?
        } else {
            Wal::create_on(&env, &wal_path)?
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
        let vlog = if blob_active > 0 {
            Some(Mutex::new(ValueLog::open_blob(&env, &dir, blob_active)?))
        } else if large_value_threshold.is_some()
            || env.exists(&vlog_path)
            || (vlog_use_new && env.exists(&vlog_new))
        {
            Some(Mutex::new(ValueLog::open_with_flag(
                &env,
                &dir,
                vlog_use_new,
            )?))
        } else {
            None
        };

        let mut db = Self {
            dir,
            env,
            wal,
            commit_inflight: AtomicUsize::new(0),
            mem,
            imm: None,
            flush_read_pin: None,
            parked_unflushed: Vec::new(),
            retired_pending: Vec::new(),
            retired_fold: MemTable::new(),
            retired_l0s: 0,
            sst_order_newest: Vec::new(),
            ssts,
            sst_levels,
            next_file_num,
            manifest_file_num,
            vlog_use_new,
            next_seq,
            published_seq: AtomicU64::new(next_seq.saturating_sub(1)),
            sync: opts.sync,
            auto_flush_bytes: opts.auto_flush_bytes.filter(|n| *n > 0),
            auto_compact_sst_count: opts.auto_compact_sst_count.filter(|n| *n > 0),
            auto_compact_sst_bytes: opts.auto_compact_sst_bytes.filter(|n| *n > 0),
            table_cache,
            block_cache,
            point_cache,
            last_prefix_cache,
            count_cache,
            dir_lock: lock,
            durability_fenced: false,
            auto_compact_failures: 0,
            last_auto_compact_error: None,
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
            wal_sync_count: AtomicU64::new(0),
            bytes_ingested: 0,
            bytes_written_wal: 0,
            bytes_written_sst: 0,
            compact_count: 0,
            vlog_gc_count: 0,
            change_log,
            changelog_interval: changelog_interval_from_env(),
            commits_since_changelog: 0,
            changelog_store_count: 0,
            unsynced_ssts: Vec::new(),
        };
        db.rebuild_sst_order();
        db.maybe_rebuild_feed_from_live();
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

    /// Whether this handle refused further writes after a failed required WAL sync.
    ///
    /// Cleared only by constructing a new `Db` (reopen).
    #[must_use]
    pub fn is_durability_fenced(&self) -> bool {
        self.durability_fenced
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

    /// Latest sequence default reads may observe (durable or no-sync apply).
    #[must_use]
    pub fn visible_sequence(&self) -> SequenceNumber {
        self.published_seq.load(Ordering::Acquire)
    }

    /// Publish `seq` as visible and drop read caches (after WAL is durable).
    pub(crate) fn publish_sequence(&self, seq: SequenceNumber) {
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
        self.invalidate_read_answers();
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
        self.get_inline.store(0, Ordering::Relaxed);
        self.get_vlog.store(0, Ordering::Relaxed);
        self.mvcc_split_ops.store(0, Ordering::Relaxed);
        self.mvcc_ns_encode.store(0, Ordering::Relaxed);
        self.mvcc_ns_last.store(0, Ordering::Relaxed);
        self.mvcc_ns_get.store(0, Ordering::Relaxed);
        self.mvcc_ns_copy.store(0, Ordering::Relaxed);
        self.block_cache.reset_stats();
        crate::sst::reset_sst_blocks_decoded();
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
        if self.feed_is_lazy() && self.change_log.max_sequence().unwrap_or(0) < self.last_sequence()
        {
            let entries = self.collect_feed_from_live();
            if !entries.is_empty() {
                self.change_log.replace_sorted(entries);
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

    /// Raise the GC watermark (monotonic). Used after history-dropping compact.
    fn raise_earliest_readable(&mut self, floor: SequenceNumber) {
        if floor > self.earliest_readable_seq {
            self.earliest_readable_seq = floor;
        }
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
    #[must_use]
    pub fn get(&self, key: &[u8]) -> Option<Bytes> {
        // Latest snapshot is always ≥ watermark when watermark is raised from
        // last_sequence / pin floor after GC; fall back to None only on fence.
        if let Some(cached) = self.point_cache.get(key) {
            return cached;
        }
        let got = self.get_at(self.snapshot(), key).ok().flatten();
        self.point_cache.insert(key, got.clone());
        got
    }

    /// Point lookup at an explicit [`Snapshot`].
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`] if `snap` is below the version-GC watermark
    /// (history may have been dropped by reclaim / `latest_only`).
    pub fn get_at(&self, snap: Snapshot, key: &[u8]) -> Result<Option<Bytes>> {
        if snap.seq == 0 {
            return Ok(None);
        }
        self.ensure_snapshot_readable(snap)?;
        Ok(match self.lookup(key, snap.seq) {
            Lookup::Found(v) => {
                if vlog::decode_vlog_ptr(v.as_ref()).is_some() {
                    self.get_vlog.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.get_inline.fetch_add(1, Ordering::Relaxed);
                }
                self.resolve_stored_value(v).ok()
            }
            Lookup::Deleted | Lookup::NotFound => None,
        })
    }

    /// Whether any version of `key` has `sequence > snapshot` (OCC conflict probe).
    ///
    /// Includes point puts/deletes **and** range tombstones that cover `key`
    /// (F30: a concurrent `delete_range` that covers a read/write key must conflict).
    #[must_use]
    pub fn key_has_write_after(&self, key: &[u8], snapshot: SequenceNumber) -> bool {
        use crate::key::ValueType;
        for table in self.mem_layers() {
            for (ikey, value) in table.iter_internal() {
                if ikey.sequence <= snapshot {
                    continue;
                }
                if ikey.user_key.as_ref() == key {
                    return true;
                }
                // Range tombstone `[start, end)` covers key even when start != key.
                if ikey.kind == ValueType::RangeDeletion {
                    let start = ikey.user_key.as_ref();
                    let end = value.as_ref();
                    if crate::merge::range_tombstone_covers(start, end, key) {
                        return true;
                    }
                }
            }
        }
        for table in &self.ssts {
            // point_at ignores range tombstones; any newer point/del counts.
            if let Some((seq, _)) = table.point_at(key, MAX_SEQUENCE_NUMBER) {
                if seq > snapshot {
                    return true;
                }
            }
            // Any newer range tombstone that **covers** this key (not only start==key).
            let mut tombs = Vec::new();
            table.collect_range_tombstones(MAX_SEQUENCE_NUMBER, &mut tombs);
            for t in tombs {
                if t.sequence > snapshot && t.covers(key) {
                    return true;
                }
            }
        }
        false
    }

    /// Resolve vlog pointer to payload (or return inline value).
    fn resolve_stored_value(&self, stored: Bytes) -> Result<Bytes> {
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

    /// Maybe rewrite a large put value into the vlog; returns stored value bytes.
    fn maybe_spill_large_value(&mut self, value: Bytes) -> Result<Bytes> {
        let Some(threshold) = self.large_value_threshold else {
            return Ok(value);
        };
        if value.len() < threshold {
            return Ok(value);
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
        let (off, len, crc) = guard.append(value.as_ref())?;
        drop(guard);
        Ok(vlog::encode_vlog_ptr(vlog::VlogPtr {
            file_num: self.blob_active,
            offset: off,
            len,
            crc,
        }))
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
        let mut best: Option<Bytes> = None;
        for table in self.mem_layers() {
            if let Some((k, _)) = table.last_visible_under_prefix(prefix, snapshot, None) {
                best = Some(k);
                break;
            }
        }
        let out = if let Some(k) = best {
            self.latest_mem_hit.fetch_add(1, Ordering::Relaxed);
            Some(k)
        } else {
            self.latest_sst_fallback.fetch_add(1, Ordering::Relaxed);
            self.last_under_user_prefix_sst(snapshot, prefix)?
        };
        if latest {
            self.last_prefix_cache.insert(prefix, out.clone());
        }
        Ok(out)
    }

    /// L0 newest → older → L1+ (same single-writer invariant as the mem hit).
    fn sst_indices_newest_first(&self) -> &[usize] {
        &self.sst_order_newest
    }

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
            let mut cand_from = 0usize;
            for (i, &sst_i) in order.iter().enumerate() {
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
                    cand_from = i;
                    break;
                }
            }
            let Some(k) = cand else {
                return Ok(None);
            };
            if self.user_prefix_hidden_by_newer(snapshot, k.as_ref(), &order[..cand_from]) {
                before = Some(k.to_vec());
                continue;
            }
            return Ok(Some(k));
        }
    }

    /// Newer mem / L0 has a point tombstone (or newer point) for `key`.
    fn user_prefix_hidden_by_newer(
        &self,
        snapshot: SequenceNumber,
        key: &[u8],
        newer_sst: &[usize],
    ) -> bool {
        for table in self.mem_layers() {
            match table.get_entry(key, snapshot) {
                Some((_, Lookup::Deleted)) => return true,
                Some((_, Lookup::Found(_))) => return false,
                Some((_, Lookup::NotFound)) | None => {}
            }
        }
        for &sst_i in newer_sst {
            let table = &self.ssts[sst_i];
            if let Some((_, look)) = table.point_at_with(key, snapshot, |bi| {
                Some(self.block_cache.get_or_insert_with(table.path(), bi, || {
                    table.decode_block(bi).unwrap_or_default()
                }))
            }) {
                return matches!(look, Lookup::Deleted);
            }
        }
        false
    }

    /// Range at `snapshot` with optional live-key `limit`.
    ///
    /// Uses the streaming merge path ([`Self::try_scan_at`]) so the full keyspace is
    /// not required as a single materialised `Vec` of all live pairs.
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`] if `snapshot` is below the version-GC watermark.
    pub fn range_at_limited(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
    ) -> Result<Vec<(Bytes, Bytes)>> {
        Ok(self
            .try_scan_at(snapshot, start, end, limit)?
            .filter_map(|VisibleKv { key, value }| {
                let value = self.resolve_stored_value(value).ok()?;
                Some((key, value))
            })
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
        let ck = count_cache_key(start, end, limit);
        if latest {
            if let Some(n) = self.count_cache.get(ck.as_slice()) {
                self.scan_ops.fetch_add(1, Ordering::Relaxed);
                return Ok(n);
            }
        }
        let n = self.count_visible(snapshot, start, end, limit);
        if latest {
            self.count_cache.insert(ck.as_slice(), n);
        }
        Ok(n)
    }

    fn invalidate_read_answers(&self) {
        self.point_cache.clear();
        self.last_prefix_cache.clear();
        self.count_cache.clear();
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
        self.scan_ops.fetch_add(1, Ordering::Relaxed);
        // Range tombstones first (G2), exactly like `scan_at_raw`.
        let mut range_dels = Vec::new();
        let mut cursors: Vec<CountCursor<'_>> = Vec::with_capacity(3 + self.ssts.len());
        // Live + parked-without-SST only. Retired BTrees are a point/MVCC
        // cache; their L0 files are in `ssts` (retire2 scan died merging them).
        for table in self.scan_mem_layers() {
            table.collect_range_tombstones(snapshot, &mut range_dels);
            if table.is_empty() {
                continue;
            }
            cursors.push(CountCursor::Mem(MemCountCursor::new(
                table, start, end, snapshot,
            )));
        }
        for table in self.ssts.iter() {
            table.collect_range_tombstones(snapshot, &mut range_dels);
            if !table.overlaps_user_range(start, end) {
                continue;
            }
            self.scan_sst_probed.fetch_add(1, Ordering::Relaxed);
            cursors.push(CountCursor::Sst(SstCountCursor::new(
                table,
                start,
                end,
                snapshot,
                &self.block_cache,
            )));
        }
        let cap = limit.unwrap_or(usize::MAX);
        let mut count = 0usize;
        while count < cap {
            // Min head across layers by InternalKey order (user asc, seq
            // desc, kind desc) — the global newest version of that user.
            let mut best: Option<usize> = None;
            for (i, c) in cursors.iter().enumerate() {
                let Some(h) = c.head() else { continue };
                match best {
                    None => best = Some(i),
                    Some(b) => {
                        if internal_less(h, cursors[b].head().expect("best head")) {
                            best = Some(i);
                        }
                    }
                }
            }
            let Some(bi) = best else { break };
            let head = cursors[bi].head().expect("best head");
            let kind = head.kind;
            let seq = head.sequence;
            // Stack copy of the user key so we can step cursors without
            // cloning `Bytes` (RFC-0040 P0.3). Bench keys fit in 192 B.
            const STACK: usize = 192;
            let ulen = head.user_key.len();
            let visible = if ulen <= STACK {
                let mut buf = [0u8; STACK];
                buf[..ulen].copy_from_slice(head.user_key.as_ref());
                let user = &buf[..ulen];
                let vis = kind == ValueType::Value
                    && !crate::merge::range_deleted(user, seq, &range_dels);
                for c in cursors.iter_mut() {
                    if c.head().is_some_and(|h| h.user_key.as_ref() == user) {
                        c.step_user(user);
                    }
                }
                vis
            } else {
                let user = head.user_key.clone();
                let vis = kind == ValueType::Value
                    && !crate::merge::range_deleted(user.as_ref(), seq, &range_dels);
                for c in cursors.iter_mut() {
                    if c.head().is_some_and(|h| h.user_key == user) {
                        c.step_user(user.as_ref());
                    }
                }
                vis
            };
            if visible {
                count += 1;
            }
        }
        count
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
        // Range tombstones first (G2): a covering delete whose start sits
        // before `start` must still hide keys in the window. Point streams
        // are lazy — later SST blocks are not decoded after `limit` emits.
        let mut range_dels = Vec::new();
        let mut streams: Vec<crate::merge::LayerStream<'_>> =
            Vec::with_capacity(3 + self.ssts.len());
        for table in self.scan_mem_layers() {
            table.collect_range_tombstones(snapshot, &mut range_dels);
            let pts = self.memtable_stream(table, start, end, snapshot, resolve_values);
            if !pts.is_empty() {
                streams.push(Box::new(pts.into_iter()));
            }
        }
        for table in self.ssts.iter() {
            table.collect_range_tombstones(snapshot, &mut range_dels);
            if !table.overlaps_user_range(start, end) {
                continue;
            }
            self.scan_sst_probed.fetch_add(1, Ordering::Relaxed);
            let cache = &self.block_cache;
            let path = table.path();
            let db = self;
            let load: Box<
                dyn FnMut(usize) -> Option<std::sync::Arc<Vec<(InternalKey, Bytes)>>> + '_,
            > = Box::new(move |bi| {
                let cached = cache
                    .get_or_insert_with(path, bi, || table.decode_block(bi).unwrap_or_default());
                if resolve_values {
                    let mut entries = cached.as_ref().clone();
                    db.prefetch_resolve_stream(&mut entries);
                    Some(std::sync::Arc::new(entries))
                } else {
                    Some(cached)
                }
            });
            streams.push(Box::new(table.iter_user_range(
                start,
                end,
                snapshot,
                resolve_values,
                load,
            )));
        }
        StreamingVisibleIter::from_point_streams(streams, range_dels, snapshot, start, end, limit)
    }

    fn memtable_stream(
        &self,
        table: &MemTable,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        snapshot: SequenceNumber,
        resolve_values: bool,
    ) -> Vec<(InternalKey, Bytes)> {
        let mut stream = Vec::new();
        let mut last: Option<Bytes> = None;
        let push = |stream: &mut Vec<(InternalKey, Bytes)>,
                    last: &mut Option<Bytes>,
                    k: &InternalKey,
                    v: &Bytes| {
            if k.kind == ValueType::RangeDeletion || k.sequence > snapshot {
                return;
            }
            if last.as_ref().is_some_and(|u| u == &k.user_key) {
                return;
            }
            *last = Some(k.user_key.clone());
            let value = if resolve_values {
                v.clone()
            } else {
                Bytes::new()
            };
            stream.push((k.clone(), value));
        };
        if table.has_range_tombstones() {
            for (k, v) in table.iter_internal() {
                if crate::merge::user_key_in_range(k.user_key.as_ref(), start, end) {
                    push(&mut stream, &mut last, k, v);
                }
            }
        } else {
            for (k, v) in table.iter_internal_range(start, end) {
                push(&mut stream, &mut last, k, v);
            }
        }
        if resolve_values {
            self.prefetch_resolve_stream(&mut stream);
        }
        stream
    }

    /// Resolve vlog pointers in windows of [`Self::scan_prefetch`] (RFC-0029 P0.3).
    ///
    /// Single-threaded: each window issues up to N `Env` reads then continues.
    /// Before resolving a window, best-effort [`Env::advise`] `WillNeed` on each
    /// vlog pointer range (RFC-0029 P1.2). Order of `stream` is unchanged.
    /// Missing/corrupt values become empty bytes (same as [`Self::resolve_stream_value`]).
    fn prefetch_resolve_stream(&self, stream: &mut [(InternalKey, Bytes)]) {
        let n = self.scan_prefetch.max(1);
        let mut i = 0;
        while i < stream.len() {
            let end = (i + n).min(stream.len());
            let mut issued = 0u64;
            // Kernel readahead hints (no-op on sim / non-Linux).
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
    fn resolve_stream_value(&self, kind: ValueType, stored: Bytes) -> Bytes {
        if kind == ValueType::RangeDeletion {
            return stored;
        }
        self.resolve_stored_value(stored)
            .unwrap_or_else(|_| Bytes::new())
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
            for (_, v) in t.entries_cloned() {
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
            // Re-open via table cache (second verify hits cache; first may miss).
            let re = self.table_cache.get_or_open(&self.env, table.path())?;
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
        let _ = crate::buggify_hooks::maybe_arm(crate::buggify_hooks::sites::BEFORE_SST_RENAME);
        let _ =
            crate::buggify_hooks::maybe_arm(crate::buggify_hooks::sites::BEFORE_MANIFEST_RENAME);
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
        self.mem.spill_tail();
        self.imm = Some(std::mem::replace(&mut self.mem, MemTable::new()));
        self.flush_imm_to_l0()?;
        self.finish_flush_pipeline()?;
        // Explicit flush: persist the cache even when interval is 0 (WAL gone).
        if self.changelog_interval == 0 {
            self.persist_changelog_best_effort();
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
        self.mem.spill_tail();
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
            self.mem.spill_tail();
            Some(std::mem::replace(&mut self.mem, MemTable::new()))
        };
        // Keep a read pin so get/scan still see acked keys during off-lock SST I/O.
        if let Some(mut table) = taken {
            table.spill_tail();
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
            }
        }
    }

    /// Take pending pins so the host can fold them without the write lock.
    pub fn take_retired_pending(&mut self) -> Vec<MemTable> {
        std::mem::take(&mut self.retired_pending)
    }

    /// Install a fold built off-lock (union of pending pins).
    pub fn install_retired_fold(&mut self, built: MemTable) {
        if self.retired_fold.is_empty() {
            self.retired_fold = built;
        } else {
            self.retired_fold.absorb(built);
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

    /// Snapshot of what off-lock L0 write needs (`Env` is [`Clone`]).
    #[must_use]
    pub fn l0_write_ctx(&self) -> (E, PathBuf, bool) {
        (self.env.clone(), self.dir.clone(), self.sync)
    }

    /// Write `imm` to `{num:06}.sst` without borrowing `Db` (caller drops the lock).
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
        match write_sst_on_with(env, &tmp_path, imm, false) {
            Ok(table) => {
                drop(table);
                env.rename(&tmp_path, &final_path)?;
                if sync {
                    env.sync_dir(dir)?;
                }
                let table = SstTable::open_on(env, &final_path)?;
                Ok((table, num, final_path))
            }
            Err(e) => {
                let _ = env.remove_file(&tmp_path);
                let _ = env.remove_file(&final_path);
                Err(e)
            }
        }
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
        // In-memory only. MANIFEST + SST `fdatasync` wait for WAL rotate so a
        // write burst is not charged one extra fd per 64 MiB flush (RFC-0041).
        let _undo = self.apply_l0_install(table, file_num);
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

    /// Active mem empty and no imm (safe to rotate WAL).
    #[must_use]
    pub fn mem_is_empty_for_rotate(&self) -> bool {
        self.mem.is_empty()
            && self.imm.is_none()
            && self.flush_read_pin.is_none()
            && self.parked_unflushed.is_empty()
    }

    /// Whether an immutable memtable is present.
    #[must_use]
    pub fn has_imm(&self) -> bool {
        self.imm.is_some()
    }

    /// Mem / imm / pin / parked (no SST yet) / folded retired / pending pins.
    fn mem_layers(&self) -> impl Iterator<Item = &MemTable> {
        self.scan_mem_layers()
            .chain((!self.retired_fold.is_empty()).then_some(&self.retired_fold))
            .chain(self.retired_pending.iter().rev())
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
        let mut t = self.imm.take()?;
        t.spill_tail();
        Some(t)
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

    /// Pop the oldest parked table after its L0 exists.
    pub fn take_oldest_parked(&mut self) -> Option<MemTable> {
        if self.parked_unflushed.is_empty() {
            None
        } else {
            let arc = self.parked_unflushed.remove(0);
            Some(Arc::try_unwrap(arc).unwrap_or_else(|a| (*a).clone()))
        }
    }

    /// How many flushed mems still lack an L0 file.
    #[must_use]
    pub fn parked_unflushed_count(&self) -> usize {
        self.parked_unflushed.len()
    }

    /// Cheap `Arc` snapshot of the two oldest parked tables. Fold deep-clones
    /// off the Db lock, then [`Self::replace_oldest_parked_pair`].
    #[must_use]
    pub fn parked_oldest_pair_arcs(&self) -> Option<(Arc<MemTable>, Arc<MemTable>)> {
        if self.parked_unflushed.len() < 2 {
            return None;
        }
        Some((
            Arc::clone(&self.parked_unflushed[0]),
            Arc::clone(&self.parked_unflushed[1]),
        ))
    }

    /// Replace the two oldest parked tables with one folded union.
    pub fn replace_oldest_parked_pair(&mut self, built: MemTable) {
        if self.parked_unflushed.len() < 2 {
            if !built.is_empty() {
                self.parked_unflushed.push(Arc::new(built));
            }
            return;
        }
        self.parked_unflushed.remove(0);
        self.parked_unflushed.remove(0);
        if !built.is_empty() {
            self.parked_unflushed.insert(0, Arc::new(built));
        }
    }

    /// Keep `mem` as a point/MVCC cache covering one newly installed L0.
    pub fn retire_mem_as_l0_cache(&mut self, mem: MemTable) {
        if !mem.is_empty() {
            self.retired_pending.push(mem);
            self.retired_l0s = self.retired_l0s.saturating_add(1);
        }
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
        let (table, file_num, _) = match self.write_memtable_to_l0_file(&imm) {
            Ok(t) => t,
            Err(e) => {
                // Put imm back so data is not lost in memory.
                self.imm = Some(imm);
                return Err(e);
            }
        };
        if let Err(e) = self.install_l0_sst(table, file_num) {
            self.imm = Some(imm);
            return Err(e);
        }
        Ok(())
    }

    /// Rotate WAL only when mem, imm, **and** the off-lock flush pin are empty.
    ///
    /// After [`Self::prepare_flush_imm`] the only copy of acked keys may be the
    /// pin (and an in-flight SST). Truncating WAL here leaves a checkpoint or
    /// crash with nothing to replay.
    fn try_rotate_wal(&mut self) -> Result<()> {
        if self.commit_inflight.load(Ordering::Acquire) > 0 {
            return Ok(());
        }
        if !self.mem_is_empty_for_rotate() {
            return Ok(());
        }
        self.rotate_wal_now()
    }

    fn rotate_wal_now(&mut self) -> Result<()> {
        // SST + MANIFEST must be durable before the WAL that covers those
        // keys is discarded (G1). L0 flush skips file fsync; this is the pay
        // point.
        self.persist_manifest_durable()?;
        // WAL truncate drops the rebuild source for the CHANGELOG cache.
        // Persist first when debounce is on. interval 0: skip on auto-flush
        // (RFC-0036) — F53 SST rebuild covers crash+reopen; explicit flush
        // / close still store.
        if self.changelog_interval > 0 {
            self.persist_changelog_best_effort();
        }
        let wal_path = self.dir.join(WAL_FILE_NAME);
        let new = Wal::create_on(&self.env, &wal_path)?;
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

    /// Compact with version GC options (RFC-0009 P1.3).
    ///
    /// # Errors
    /// I/O while writing the compacted SST or deleting old files.
    pub fn compact_with(&mut self, options: CompactOptions) -> Result<()> {
        self.flush()?;
        self.compact_with_ssts_only(options)
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
        let oldest = self
            .oldest_pinned_sequence()
            .unwrap_or_else(|| self.last_sequence());
        self.compact_with(CompactOptions {
            gc: crate::merge::CompactGcOptions::for_oldest_snapshot(oldest),
        })
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
        let mut merged: Vec<(InternalKey, Bytes)> = Vec::new();
        for t in &self.ssts {
            merged.extend(t.entries_cloned());
        }
        let merged = crate::merge::gc_compact_entries(merged, CompactOptions::latest_only().gc);

        let num = self.next_file_num;
        let final_path = self.dir.join(format!("{num:06}.sst"));
        let tmp_path = self.dir.join(format!("{num:06}.sst.tmp"));
        let new_table = write_sst_entries_on(&self.env, &tmp_path, &merged)?;
        drop(new_table);
        self.env.rename(&tmp_path, &final_path)?;
        self.sync_dir_if_required(&self.dir)?;
        let new_table = SstTable::open_on(&self.env, &final_path)?;
        self.table_cache.insert(Arc::new(new_table.clone()));
        self.next_file_num = num + 1;

        let old_paths: Vec<PathBuf> = self.ssts.iter().map(|t| t.path().to_path_buf()).collect();
        self.ssts = vec![new_table];
        self.sst_levels = vec![MAX_LSM_LEVEL];

        if let Ok(len) = self.env.metadata_len(&final_path) {
            self.bytes_written_sst = self.bytes_written_sst.saturating_add(len);
        }
        // Watermark must be raised before MANIFEST so reopen recovers it.
        self.note_version_gc_watermark(CompactOptions::latest_only().gc);
        self.persist_manifest()?;

        for path in old_paths {
            if path != final_path {
                let _ = self.env.remove_file(&path);
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
        // Pick lowest level that has files and can promote (N → N+1).
        let mut from_level = None;
        for lvl in 0..=MAX_LSM_LEVEL {
            if self.level_file_count(lvl) > 0 {
                // Prefer compacting when we have multiple files at L0, or any
                // L0 while L1+ exists, or multiple files at higher levels.
                if lvl < MAX_LSM_LEVEL {
                    from_level = Some(lvl);
                    break;
                }
            }
        }
        let Some(from) = from_level else {
            // Only files at MAX level: optional GC rewrite of all of them.
            if options.gc.requests_gc() {
                return self.compact_levels(MAX_LSM_LEVEL, MAX_LSM_LEVEL, options);
            }
            return Ok(());
        };
        let to = (from + 1).min(MAX_LSM_LEVEL);
        // Skip no-op when single file already at `to` and no GC requested.
        if from == to && self.ssts.len() == 1 && !options.gc.requests_gc() {
            return Ok(());
        }
        self.compact_levels(from, to, options)
    }

    /// Promote L0 files into one new L1 file. Existing L1+ SSTs are left
    /// untouched so a write burst does not rewrite the whole level (RFC-0036).
    /// Visibility is unchanged: every version stays in some file.
    fn compact_l0_into_l1(&mut self, options: CompactOptions) -> Result<()> {
        let input_idxs: Vec<usize> = self
            .sst_levels
            .iter()
            .enumerate()
            .filter(|(_, &lvl)| lvl == 0)
            .map(|(i, _)| i)
            .collect();
        if input_idxs.is_empty() {
            return Ok(());
        }
        self.rewrite_ssts(input_idxs, 1, options)
    }

    /// Snapshot current L0 tables and reserve an output file number.
    ///
    /// Inputs stay readable. Call [`PreparedL0Compact::write`] without this
    /// lock, then [`Self::install_prepared_l0_compact`].
    ///
    /// # Errors
    /// None today (reservation cannot fail); `Result` for fence / I/O later.
    pub fn prepare_l0_compact(
        &mut self,
        options: CompactOptions,
    ) -> Result<Option<PreparedL0Compact<E>>> {
        self.ensure_not_fenced()?;
        let inputs: Vec<SstTable> = self
            .ssts
            .iter()
            .zip(self.sst_levels.iter())
            .filter(|(_, &lvl)| lvl == 0)
            .map(|(t, _)| t.clone())
            .collect();
        if inputs.is_empty() {
            return Ok(None);
        }
        let file_num = self.alloc_file_num();
        Ok(Some(PreparedL0Compact {
            inputs,
            file_num,
            gc: options.gc,
            dir: self.dir.clone(),
            env: self.env.clone(),
            sync: self.sync,
        }))
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
        new_table: SstTable,
    ) -> Result<()> {
        let Some(undo) = self.apply_prepared_l0_compact(job, new_table) else {
            return Ok(());
        };
        let old_paths = undo.old_paths().to_vec();
        if let Err(e) = self.persist_manifest() {
            self.undo_prepared_l0_compact(undo);
            return Err(e);
        }
        for path in old_paths {
            let _ = self.env.remove_file(&path);
        }
        self.compact_count = self.compact_count.saturating_add(1);
        Ok(())
    }

    /// Merge all SSTs at `from_level` and `to_level` into one SST at `to_level`.
    fn compact_levels(
        &mut self,
        from_level: u32,
        to_level: u32,
        options: CompactOptions,
    ) -> Result<()> {
        let mut input_idxs: Vec<usize> = Vec::new();
        for (i, &lvl) in self.sst_levels.iter().enumerate() {
            if lvl == from_level || lvl == to_level {
                input_idxs.push(i);
            }
        }
        if input_idxs.is_empty() {
            return Ok(());
        }
        // Single file at target, no GC → nothing to do.
        if input_idxs.len() == 1
            && self.sst_levels[input_idxs[0]] == to_level
            && !options.gc.requests_gc()
        {
            return Ok(());
        }
        self.rewrite_ssts(input_idxs, to_level, options)
    }

    /// Rewrite `input_idxs` into one SST at `to_level`; keep every other file.
    fn rewrite_ssts(
        &mut self,
        input_idxs: Vec<usize>,
        to_level: u32,
        options: CompactOptions,
    ) -> Result<()> {
        let num = self.next_file_num;
        let tables: Vec<SstTable> = input_idxs.iter().map(|&i| self.ssts[i].clone()).collect();
        let new_table =
            write_merged_tables(&self.env, &self.dir, num, &tables, options.gc, self.sync)?;
        self.table_cache.insert(Arc::new(new_table.clone()));
        self.next_file_num = num + 1;

        let old_paths: Vec<PathBuf> = input_idxs
            .iter()
            .map(|&i| self.ssts[i].path().to_path_buf())
            .collect();

        // Keep SSTs not in the input set; append the new file at `to_level`.
        let mut keep_tables = Vec::new();
        let mut keep_levels = Vec::new();
        for (i, (t, &lvl)) in self.ssts.iter().zip(self.sst_levels.iter()).enumerate() {
            if !input_idxs.contains(&i) {
                keep_tables.push(t.clone());
                keep_levels.push(lvl);
            }
        }
        let new_path = new_table.path().to_path_buf();
        keep_tables.push(new_table);
        keep_levels.push(to_level);
        self.ssts = keep_tables;
        self.sst_levels = keep_levels;
        self.note_sst_inventory_changed();

        if let Ok(len) = self.env.metadata_len(&new_path) {
            self.bytes_written_sst = self.bytes_written_sst.saturating_add(len);
        }
        // Raise GC watermark before MANIFEST install (durable across reopen).
        if options.gc.requests_gc() {
            self.note_version_gc_watermark(options.gc);
        }
        self.persist_manifest()?;

        for path in old_paths {
            if path != new_path {
                let _ = self.env.remove_file(&path);
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
        // Durable empty WAL of unflushed pointers that would break after remap.
        self.flush()?;

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
                            self.durability_fenced = true;
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
                self.durability_fenced = true;
                return Err(e);
            }
        }
        self.vlog_use_new = false;
        if let Err(e) = self.persist_manifest() {
            // Primary is live; flag false in memory. Fence so callers reopen.
            self.durability_fenced = true;
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
        let pick = self
            .blob_gc_candidates()?
            .into_iter()
            .find(|c| !c.is_active && c.bytes > 0 && c.dead_ratio + f64::EPSILON >= min);
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
                let _ = self.env.remove_file(&vlog::blob_path(&self.dir, dest_num));
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
            let _ = self.env.remove_file(&vlog::blob_path(&self.dir, dest_num));
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
            let _ = self.env.remove_file(&path);
        }
        let _ = self.env.remove_file(&src);
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
            for (_, v) in t.entries_cloned() {
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
            let mentions = table.entries_cloned().iter().any(|(_, v)| {
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
                .entries_cloned()
                .into_iter()
                .map(|(k, v)| (k, remap_one(&v)))
                .collect();
            match write_sst_entries_on(&self.env, &tmp, &entries) {
                Ok(_) => {}
                Err(e) => {
                    for p in &staged_paths {
                        let _ = self.env.remove_file(p);
                    }
                    return Err(e);
                }
            }
            if let Err(e) = self.env.rename(&tmp, &dest) {
                for p in &staged_paths {
                    let _ = self.env.remove_file(p);
                }
                return Err(CoreError::Io(e));
            }
            let written = self.env.metadata_len(&dest).unwrap_or(0);
            bytes_written = bytes_written.saturating_add(written);
            match SstTable::open_on(&self.env, dest) {
                Ok(t) => {
                    old_paths.push(table.path().to_path_buf());
                    new_tables.push(t);
                    new_levels.push(level);
                }
                Err(e) => {
                    for p in &staged_paths {
                        let _ = self.env.remove_file(p);
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
                self.durability_fenced = true;
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
                let _ = self
                    .env
                    .remove_file(&self.dir.join(crate::vlog::VLOG_NEW_NAME));
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
            let _ = self
                .env
                .remove_file(&self.dir.join(crate::vlog::VLOG_NEW_NAME));
            return Err(e);
        }

        // Commit point: MANIFEST durable with remapped SSTs + use_new.
        // Open .new handle BEFORE remapping mem so a failed open does not leave
        // remapped mem + old handle. Never clear vlog first.
        if let Err(e) = self.replace_vlog_handle(true) {
            // Inventory is committed; must not serve mismatched handle.
            self.durability_fenced = true;
            return Err(e);
        }

        let remap_fn = |stored: &Bytes| vlog::remap_stored_value(stored, &remap);
        self.mem.map_values(remap_fn);
        if let Some(ref mut imm) = self.imm {
            imm.map_values(remap_fn);
        }
        self.bytes_written_sst = self.bytes_written_sst.saturating_add(staged_bytes);
        for t in &self.ssts {
            self.table_cache.insert(Arc::new(t.clone()));
        }
        for path in old_paths {
            let _ = self.env.remove_file(&path);
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
            for (_, v) in t.entries_cloned() {
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
            let entries = table.entries_cloned();
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
                        let _ = self.env.remove_file(&tmp_path);
                        cleanup_staged(&self.env, &staged_paths);
                        return Err(e.into());
                    }
                    if let Err(e) = self.sync_dir_if_required(&self.dir) {
                        let _ = self.env.remove_file(&final_path);
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
                            new_tables.push(new_table);
                            new_levels.push(level);
                        }
                        Err(e) => {
                            let _ = self.env.remove_file(&final_path);
                            cleanup_staged(&self.env, &staged_paths);
                            return Err(e);
                        }
                    }
                }
                Err(e) => {
                    let _ = self.env.remove_file(&tmp_path);
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
        let _ = crate::buggify_hooks::maybe_arm(crate::buggify_hooks::sites::AFTER_MEM_INSERT);
        let _ = crate::buggify_hooks::maybe_arm(crate::buggify_hooks::sites::AFTER_WAL_APPEND);
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
    /// Order matches `keys`; each entry is the same as [`Self::get`] for that key.
    #[must_use]
    pub fn multi_get(&self, keys: &[impl AsRef<[u8]>]) -> Vec<Option<Bytes>> {
        keys.iter().map(|k| self.get(k.as_ref())).collect()
    }

    /// [`multi_get`](Self::multi_get) at an explicit [`Snapshot`].
    #[must_use]
    /// Multi-get at an explicit snapshot.
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`] if `snap` is below the version-GC watermark.
    pub fn multi_get_at(
        &self,
        snap: Snapshot,
        keys: &[impl AsRef<[u8]>],
    ) -> Result<Vec<Option<Bytes>>> {
        self.ensure_snapshot_readable(snap)?;
        Ok(keys
            .iter()
            .map(|k| {
                if snap.seq == 0 {
                    return None;
                }
                match self.lookup(k.as_ref(), snap.seq) {
                    Lookup::Found(v) => self.resolve_stored_value(v).ok(),
                    Lookup::Deleted | Lookup::NotFound => None,
                }
            })
            .collect())
    }

    /// Changes with `from_seq < sequence <= to_seq` (RFC-0019 change feed).
    ///
    /// # Errors
    /// Never fails today (in-memory + loaded log); reserved for I/O.
    pub fn changes(
        &self,
        from_seq: SequenceNumber,
        to_seq: SequenceNumber,
    ) -> Result<Vec<ChangeEntry>> {
        let to = to_seq.min(self.last_sequence());
        if self.feed_is_lazy() {
            return Ok(self
                .lazy_feed_entries()
                .into_iter()
                .filter(|e| e.sequence > from_seq && e.sequence <= to)
                .collect());
        }
        Ok(self.change_log.changes_in(from_seq, to))
    }

    /// All durable changes with `sequence > from_seq` (tail / watch catch-up).
    #[must_use]
    pub fn changes_after(&self, from_seq: SequenceNumber) -> Vec<ChangeEntry> {
        let from = from_seq.min(self.last_sequence());
        if self.feed_is_lazy() {
            return self
                .lazy_feed_entries()
                .into_iter()
                .filter(|e| e.sequence > from)
                .collect();
        }
        self.change_log.changes_after(from)
    }

    /// `changelog_interval == 0`: do not grow an in-memory ChangeEntry vec on
    /// every write (RFC-0039 P0.3 / RFC-0041 P1.1). Watchers rebuild last-per-key
    /// from mem+SST; flush/close still persist.
    fn feed_is_lazy(&self) -> bool {
        self.changelog_interval == 0
    }

    /// Full WAL history when the log is still live; last-per-key after rotate.
    fn lazy_feed_entries(&self) -> Vec<ChangeEntry> {
        let from_wal = self.collect_feed_from_wal();
        if !from_wal.is_empty() {
            return from_wal;
        }
        if !self.change_log.is_empty() {
            return self.change_log.changes_after(0);
        }
        self.collect_feed_from_live()
    }

    fn collect_feed_from_wal(&self) -> Vec<ChangeEntry> {
        let path = self.dir.join(WAL_FILE_NAME);
        if !self.env.exists(&path) {
            return Vec::new();
        }
        let Ok((records, _)) = crate::wal::Wal::<E::File>::recover_span_on(&self.env, &path) else {
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

    fn collect_feed_from_live(&self) -> Vec<ChangeEntry> {
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
            for (ik, v) in sst.iter_internal() {
                consider(&mut latest, ik, v);
            }
        }
        latest
            .into_values()
            .map(|(ik, v)| {
                let value = self.resolve_stored_value(v.clone()).unwrap_or(v);
                ChangeEntry {
                    sequence: ik.sequence,
                    key: ik.user_key,
                    kind: match ik.kind {
                        ValueType::Value => ChangeKind::Put,
                        ValueType::Deletion => ChangeKind::Delete,
                        ValueType::RangeDeletion => ChangeKind::DeleteRange,
                    },
                    value,
                }
            })
            .collect()
    }

    /// When CHANGELOG is missing after flush (WAL already truncated), rebuild a
    /// last-per-key feed from MemTable ∪ SSTs so fold/journal are not empty.
    fn maybe_rebuild_feed_from_live(&mut self) {
        let feed_empty = self.change_log.max_sequence().unwrap_or(0) == 0;
        if !changelog_needs_sst_rebuild(feed_empty, self.last_sequence()) {
            return;
        }
        let entries = self.collect_feed_from_live();
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
        self.ensure_write_admitted()?;
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

    /// Close the WAL and release the directory lock via [`Env`] when held.
    ///
    /// Prefer this over bare `drop` so unlock is fault-injectable (RFC-0015 H3).
    ///
    /// # Errors
    /// I/O from WAL flush or lock release.
    pub fn close(mut self) -> Result<()> {
        // RFC-0031: close is a persist point for the CHANGELOG cache.
        self.persist_changelog_best_effort();
        self.release_lock()?;
        // Flush in place — `Db` implements `Drop` (Env unlock), so we cannot move `wal`.
        self.wal.lock().flush()
    }

    /// Lookup visible version at `snapshot` across mem + imm + SSTs.
    ///
    /// Merges point versions and range tombstones across all layers so a range
    /// delete in a newer layer correctly hides older puts.
    pub(crate) fn lookup(&self, key: &[u8], snapshot: SequenceNumber) -> Lookup {
        let mut best_point_seq: Option<SequenceNumber> = None;
        let mut best_point: Lookup = Lookup::NotFound;
        let mut range_tombs = Vec::new();

        for table in self.mem_layers() {
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
                    Lookup::Found(_) if range_deleted(key, seq, &range_tombs) => Lookup::Deleted,
                    other => other,
                };
            }
        }
        self.get_sst_fallback.fetch_add(1, Ordering::Relaxed);
        // Newest file with a point wins (L0 before L1). Older files cannot
        // hide a newer point; a newer tombstone is seen first.
        for &sst_i in self.sst_indices_newest_first() {
            let table = &self.ssts[sst_i];
            table.collect_range_tombstones(snapshot, &mut range_tombs);
            if let Some((seq, look)) = table.point_at_with(key, snapshot, |bi| {
                Some(self.block_cache.get_or_insert_with(table.path(), bi, || {
                    table.decode_block(bi).unwrap_or_default()
                }))
            }) {
                best_point_seq = Some(seq);
                best_point = look;
                break;
            }
        }

        match best_point {
            Lookup::Found(v) => {
                let seq = best_point_seq.unwrap_or(0);
                if range_deleted(key, seq, &range_tombs) {
                    Lookup::Deleted
                } else {
                    Lookup::Found(v)
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
        table.collect_range_tombstones(snapshot, range_tombs);
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
        let n = self.wal.lock().append_write_ops(&records)?;
        self.bytes_written_wal = self.bytes_written_wal.saturating_add(n);
        let do_sync = durability.sync.unwrap_or(self.sync);
        if do_sync {
            if let Err(e) = self.wal.lock().sync_data() {
                self.durability_fenced = true;
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
            return Ok((records, self.last_sequence()));
        }
        let last = records.last().map_or(self.last_sequence(), |o| o.sequence);
        Ok((records, last))
    }

    /// One WAL `fdatasync` for a group of already-appended records.
    pub(crate) fn wal_sync_group(&mut self) -> Result<()> {
        self.ensure_not_fenced()?;
        if let Err(e) = self.wal.lock().sync_data() {
            self.durability_fenced = true;
            return Err(e);
        }
        self.note_wal_sync();
        Ok(())
    }

    /// Apply prepared ops to the memtable after durable WAL.
    pub(crate) fn apply_ops_to_mem(&mut self, ops: Vec<WriteOp>) {
        apply_ops_owned(&mut self.mem, ops);
        self.publish_sequence(self.last_sequence());
    }

    /// Shared WAL handle for off-lock `fdatasync` (ConcurrentDb group leader).
    pub(crate) fn wal_arc(&self) -> Arc<Mutex<Wal<E::File>>> {
        Arc::clone(&self.wal)
    }

    pub(crate) fn begin_commit(&self) {
        self.commit_inflight.fetch_add(1, Ordering::Release);
    }

    pub(crate) fn end_commit(&self) {
        self.commit_inflight.fetch_sub(1, Ordering::Release);
    }

    /// WAL appends whose `fdatasync`/mem-apply has not finished.
    #[must_use]
    pub fn commit_inflight(&self) -> usize {
        self.commit_inflight.load(Ordering::Acquire)
    }

    pub(crate) fn fence_durability(&mut self) {
        self.durability_fenced = true;
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
        if let Err(e) = self.wal.lock().write_pending_frame() {
            self.durability_fenced = true;
            return g.fail_sync(e);
        }
        if g.needs_sync() {
            if let Err(e) = self.wal_sync_group() {
                return g.fail_sync(e);
            }
        }
        let pub_seq = g.max_appended_seq();
        let results = self.group_apply(g);
        self.publish_sequence(pub_seq);
        results
    }

    /// Mem apply + feed after WAL is durable. No fsync (RFC-0041: leader may
    /// have `fdatasync`'d off the write lock).
    pub(crate) fn group_apply(&mut self, g: GroupInFlight) -> Vec<Result<SequenceNumber>> {
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
    /// Seq-assigned ops not yet WAL-appended (ConcurrentDb encodes these
    /// without the Db write lock).
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
        self.appended
            .iter()
            .map(|(_, _, seq)| *seq)
            .max()
            .unwrap_or(0)
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
        // Mem bound first: flush is the natural drain for mem pressure.
        if let Some(limit) = self.write_stall_mem_bytes {
            let mut mem_bytes = self.mem.approx_memory_usage();
            if mem_bytes >= limit {
                if self.write_stall_drain {
                    let _ = self.flush();
                    mem_bytes = self.mem.approx_memory_usage();
                }
                if mem_bytes >= limit {
                    self.write_stall_count = self.write_stall_count.saturating_add(1);
                    return Err(CoreError::WriteStallMem { mem_bytes, limit });
                }
            }
        }

        // Soft pressure (b): drain once when L0 is elevated, then continue to hard check.
        if let Some(soft) = self.write_pressure_l0 {
            if self.level_file_count(0) >= soft {
                self.drain_l0_once();
                self.write_pressure_count = self.write_pressure_count.saturating_add(1);
            }
        }

        let Some(limit) = self.write_stall_l0 else {
            return Ok(());
        };
        let mut l0 = self.level_file_count(0);
        if l0 < limit {
            return Ok(());
        }
        if self.write_stall_drain {
            // One honest self-help pass — no sleep, no unbounded loop.
            self.drain_l0_once();
            l0 = self.level_file_count(0);
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
        self.mem.spill_tail();
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
            let opts = if self.auto_reclaim {
                let oldest = self
                    .oldest_pinned_sequence()
                    .unwrap_or_else(|| self.last_sequence());
                CompactOptions {
                    gc: crate::merge::CompactGcOptions::for_oldest_snapshot(oldest),
                }
            } else {
                CompactOptions::default()
            };
            self.compact_l0_into_l1(opts)?;
            self.last_auto_compact_error = None;
        } else if count_hit || bytes_hit {
            if self.auto_reclaim {
                let oldest = self
                    .oldest_pinned_sequence()
                    .unwrap_or_else(|| self.last_sequence());
                self.compact_with_ssts_only(CompactOptions {
                    gc: crate::merge::CompactGcOptions::for_oldest_snapshot(oldest),
                })?;
            } else {
                self.compact_with(CompactOptions::default())?;
            }
            self.last_auto_compact_error = None;
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
        self.fsync_unsynced_ssts()?;
        self.take_manifest_persist()?.write()
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
        Ok(ManifestPersist {
            env: self.env.clone(),
            dir: self.dir.clone(),
            vs,
            sync: self.sync,
        })
    }

    /// Push a flushed L0 SST into the in-memory inventory (no MANIFEST I/O).
    pub fn apply_l0_install(&mut self, table: SstTable, file_num: u64) -> L0InstallUndo {
        let undo = L0InstallUndo {
            prev_next: self.next_file_num,
            prev_manifest: self.manifest_file_num,
        };
        self.note_sst_bytes_written(table.path());
        self.table_cache.insert(Arc::new(table.clone()));
        if self.next_file_num <= file_num {
            self.next_file_num = file_num.saturating_add(1);
        }
        self.unsynced_ssts.push(table.path().to_path_buf());
        self.ssts.push(table);
        self.sst_levels.push(0);
        self.note_sst_inventory_changed();
        undo
    }

    /// Undo [`Self::apply_l0_install`] after a failed off-lock MANIFEST persist.
    pub fn undo_l0_install(&mut self, undo: L0InstallUndo) {
        if let Some(t) = self.ssts.last() {
            let p = t.path().to_path_buf();
            self.unsynced_ssts.retain(|x| x != &p);
        }
        let _ = self.ssts.pop();
        let _ = self.sst_levels.pop();
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
        new_table: SstTable,
    ) -> Option<L0CompactUndo> {
        let input_paths: Vec<PathBuf> = job.input_paths();
        let still_live = self
            .ssts
            .iter()
            .any(|t| input_paths.iter().any(|p| t.path() == p.as_path()));
        if !still_live {
            let _ = self.env.remove_file(new_table.path());
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
        self.note_sst_bytes_written(new_table.path());
        self.table_cache.insert(Arc::new(new_table.clone()));
        keep_tables.push(new_table);
        keep_levels.push(1);
        let prev_tables = std::mem::replace(&mut self.ssts, keep_tables);
        let prev_levels = std::mem::replace(&mut self.sst_levels, keep_levels);
        let prev_manifest = self.manifest_file_num;
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
            old_paths,
        })
    }

    /// Undo [`Self::apply_prepared_l0_compact`] after a failed MANIFEST persist.
    pub fn undo_prepared_l0_compact(&mut self, undo: L0CompactUndo) {
        self.ssts = undo.prev_tables;
        self.sst_levels = undo.prev_levels;
        self.manifest_file_num = undo.prev_manifest;
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
}

impl<E: Env> ManifestPersist<E> {
    /// `fsync` MANIFEST + CURRENT. Does not touch `Db`.
    ///
    /// # Errors
    /// Env I/O.
    pub fn write(self) -> Result<()> {
        manifest::store(&self.env, &self.dir, &self.vs, self.sync)
    }
}

/// Rollback token for [`Db::apply_l0_install`].
pub struct L0InstallUndo {
    prev_next: u64,
    prev_manifest: u64,
}

/// Rollback token for [`Db::apply_prepared_l0_compact`].
pub struct L0CompactUndo {
    prev_tables: Vec<SstTable>,
    prev_levels: Vec<u32>,
    prev_manifest: u64,
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
    if stored != computed {
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
        // Skip nested dirs for base layout (checkpoints are flat).
        if env.metadata_len(&from).is_ok() {
            env.copy_file(&from, &to)?;
        }
    }
    Ok(())
}

/// Count-cache key buffer: inline for the small windows real queries use,
/// heap fallback for pathological bounds. Avoids a malloc per scan op.
enum CountKeyBuf {
    Inline { buf: [u8; 64], len: usize },
    Heap(Vec<u8>),
}

impl CountKeyBuf {
    fn as_slice(&self) -> &[u8] {
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

    fn step_user(&mut self, user: &[u8]) {
        match self {
            Self::Mem(c) => c.step_user(user),
            Self::Sst(c) => c.step_user(user),
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
    /// Rare: range tombstones whose start sits outside the window.
    Filter(Box<dyn Iterator<Item = (&'a InternalKey, &'a Bytes)> + 'a>),
}

impl<'a> Iterator for MemCountIter<'a> {
    type Item = (&'a InternalKey, &'a Bytes);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Range(it) => it.next(),
            Self::Filter(it) => it.next(),
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
            MemCountIter::Filter(Box::new(
                table
                    .iter_internal()
                    .filter(move |(k, _)| {
                        crate::merge::user_key_in_range(k.user_key.as_ref(), start, end)
                    })
                    .map(|(k, v)| (k, v)),
            ))
        } else {
            MemCountIter::Range(table.iter_internal_iter(start, end))
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
        while self.head.is_some_and(|h| h.user_key.as_ref() == user) {
            self.head = None;
            self.settle();
        }
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
    /// `(bytes, inclusive)` bound pairs resolved once.
    start: Option<(Bytes, bool)>,
    end: Option<(Bytes, bool)>,
    snapshot: SequenceNumber,
    exhausted: bool,
}

impl<'a> SstCountCursor<'a> {
    fn new(
        table: &'a crate::sst::SstTable,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        snapshot: SequenceNumber,
        cache: &'a crate::cache::BlockCache,
    ) -> Self {
        let start = match start {
            Bound::Unbounded => None,
            Bound::Included(s) => Some((Bytes::copy_from_slice(s), true)),
            Bound::Excluded(s) => Some((Bytes::copy_from_slice(s), false)),
        };
        let end = match end {
            Bound::Unbounded => None,
            Bound::Included(e) => Some((Bytes::copy_from_slice(e), true)),
            Bound::Excluded(e) => Some((Bytes::copy_from_slice(e), false)),
        };
        let mut c = Self {
            current: None,
            idx: 0,
            blocks: if table.is_lazy() {
                table
                    .blocks_overlapping_range(start_bound_ref(&start), end_bound_ref(&end))
                    .into_iter()
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
                .into_iter()
                .filter(|(k, _)| {
                    k.kind != ValueType::RangeDeletion
                        && crate::merge::user_key_in_range(
                            k.user_key.as_ref(),
                            start_bound_ref(&c.start),
                            end_bound_ref(&c.end),
                        )
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
                    let past_end = match &self.end {
                        Some((e, true)) => uk > e.as_ref(),
                        Some((e, false)) => uk >= e.as_ref(),
                        None => false,
                    };
                    if past_end {
                        self.exhausted = true;
                        self.current = None;
                        self.blocks = Vec::new().into_iter();
                        return;
                    }
                    let before_start = match &self.start {
                        Some((s, true)) => uk < s.as_ref(),
                        Some((s, false)) => uk <= s.as_ref(),
                        None => false,
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
            self.idx = match (&self.current, &self.start) {
                (Some(block), Some((s, true))) => {
                    block.partition_point(|(k, _)| k.user_key.as_ref() < s.as_ref())
                }
                (Some(block), Some((s, false))) => {
                    block.partition_point(|(k, _)| k.user_key.as_ref() <= s.as_ref())
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
}

/// Rebuild `Bound<&[u8]>` views of the resolved start/end pairs.
fn start_bound_ref(b: &Option<(Bytes, bool)>) -> Bound<&[u8]> {
    match b {
        None => Bound::Unbounded,
        Some((s, true)) => Bound::Included(s.as_ref()),
        Some((s, false)) => Bound::Excluded(s.as_ref()),
    }
}

fn end_bound_ref(b: &Option<(Bytes, bool)>) -> Bound<&[u8]> {
    match b {
        None => Bound::Unbounded,
        Some((e, true)) => Bound::Included(e.as_ref()),
        Some((e, false)) => Bound::Excluded(e.as_ref()),
    }
}

fn count_cache_key(start: Bound<&[u8]>, end: Bound<&[u8]>, limit: Option<usize>) -> CountKeyBuf {
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
fn apply_ops_owned(mem: &mut MemTable, ops: Vec<WriteOp>) {
    for op in ops {
        mem.insert(
            crate::key::InternalKey::new(op.key, op.sequence, op.kind),
            op.value,
        );
    }
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
fn recover_ssts<E: Env>(
    env: &E,
    dir: &Path,
    sync: bool,
    table_cache: &TableCache,
) -> Result<RecoveredSsts> {
    if let Some(mut vs) = manifest::load(env, dir)? {
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
            tables.push((*t).clone());
            levels.push(vs.sst_levels.get(i).copied().unwrap_or(0));
        }
        return Ok((
            tables,
            levels,
            vs.next_file_num,
            vs.manifest_file_num,
            vs.vlog_use_new,
            max_seq,
            vs.earliest_readable_seq,
        ));
    }

    // Legacy / first open: scan directory, then write initial MANIFEST.
    let (tables, next_file_num, max_seq) = load_ssts_scan(env, dir)?;
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
    };
    // Always install so subsequent opens use inventory (even if empty).
    manifest::install_next(env, dir, &mut vs, sync)?;
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

/// Load `NNNNNN.sst` files ascending; return tables, next file num, max sequence.
fn load_ssts_scan<E: Env>(env: &E, dir: &Path) -> Result<(Vec<SstTable>, u64, SequenceNumber)> {
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
        max_seq = max_seq.max(t.max_sequence());
        tables.push(t);
    }
    Ok((tables, next_file_num, max_seq))
}

/// Merge `tables` into `{file_num:06}.sst` (RFC-0037 streaming when `!gc.requests_gc()`).
fn write_merged_tables(
    env: &impl Env,
    dir: &Path,
    file_num: u64,
    tables: &[SstTable],
    gc: crate::merge::CompactGcOptions,
    do_sync_dir: bool,
) -> Result<SstTable> {
    let final_path = dir.join(format!("{file_num:06}.sst"));
    let tmp_path = dir.join(format!("{file_num:06}.sst.tmp"));
    let written = if gc.requests_gc() {
        let mut merged: Vec<(InternalKey, Bytes)> = Vec::new();
        for t in tables {
            merged.extend(t.entries_cloned());
        }
        let merged = crate::merge::gc_compact_entries(merged, gc);
        write_sst_entries_on(env, &tmp_path, &merged)
    } else {
        let bloom_hint: usize = tables.iter().map(SstTable::len).sum();
        let streams: Vec<_> = tables
            .iter()
            .map(SstTable::iter_internal_streaming)
            .collect();
        let mut merge = crate::merge::KwayInternalMerge::from_streams(streams)?;
        write_sst_try_sorted_on(
            env,
            &tmp_path,
            std::iter::from_fn(|| merge.next_entry().transpose()),
            bloom_hint,
        )
    };
    match written {
        Ok(table) => {
            drop(table);
            env.rename(&tmp_path, &final_path)?;
            if do_sync_dir {
                let _ = env.sync_dir(dir);
            }
            SstTable::open_on(env, &final_path)
        }
        Err(e) => {
            let _ = env.remove_file(&tmp_path);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

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

    fn vlog_opts() -> OpenOptions {
        OpenOptions {
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: Some(512),
        }
    }

    fn sync_opts() -> OpenOptions {
        OpenOptions {
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
        }
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
                    sync: true,
                    auto_flush_bytes: Some(8 * 1024),
                    auto_compact_sst_count: Some(4),
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: Some(256),
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
                    sync: true,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: Some(256),
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
                    sync: false,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
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
                    sync: true,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
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
                concat.extend(t.entries_cloned());
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
        assert_eq!(l1.entries_cloned(), expected);
        assert_eq!(db.get(b"a"), None);
        assert_eq!(db.get(b"b").as_deref(), Some(b"b2".as_slice()));
        assert_eq!(db.get(b"c").as_deref(), Some(b"c1".as_slice()));
        assert_eq!(db.get(b"d").as_deref(), Some(b"d1".as_slice()));
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
        let table = job.write().unwrap();
        assert_eq!(db.level_file_count(0), extra_l0);
        db.install_prepared_l0_compact(job, table).unwrap();
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

    #[test]
    fn stage_flush_imm_leaves_has_imm_for_worker() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                sync: false,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
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
                sync: true,
                auto_flush_bytes: Some(200),
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
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
                    sync: true,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
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

    #[test]
    fn auto_compact_when_sst_count_reaches_threshold() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: Some(3),
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
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
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: Some(2),
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
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
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: Some(2),
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
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
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
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
                sync: true,
                auto_flush_bytes: None, // no auto flush — mem grows
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
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
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
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
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
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
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
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
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: Some(2),
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
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
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: false,
            large_value_threshold: None,
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
        assert!(
            current.trim().starts_with(crate::manifest::MANIFEST_PREFIX),
            "CURRENT={current:?}"
        );
        let man_path = dir.join(current.trim());
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
            db.flush().unwrap();
            assert_eq!(db.get(b"huge").as_deref(), Some(big.as_slice()));
            db.create_checkpoint(&ckpt).unwrap();
            db.close().unwrap();
        }
        let restored = Db::open_with(
            &ckpt,
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
                    sync: true,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: Some(512),
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
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: Some(512),
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
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
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
                sync: false,
                auto_flush_bytes: Some(64 * 1024),
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
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
                sync: false,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            },
        )
        .unwrap();
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
                    sync: true,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
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
        // verify uses table cache path.
        db.table_cache.reset_stats();
        db.verify_checksums().unwrap();
        db.verify_checksums().unwrap();
        assert!(
            db.table_cache.hits() >= 1,
            "second verify should hit table cache for SSTs"
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
                    sync: true,
                    auto_flush_bytes: Some(64),
                    auto_compact_sst_count: if auto { Some(2) } else { None },
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
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
                    sync: true,
                    auto_flush_bytes: Some(8 * 1024),
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
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
                    sync: true,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
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
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
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
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
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
                    sync: true,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
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
                    sync: true,
                    auto_flush_bytes: Some(256),
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
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
                    sync: true,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
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

    /// RFC-0034: zipfian point-get must hit the block cache on the second seek.
    #[test]
    fn point_get_second_seek_hits_block_cache() {
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
        crate::sst::reset_sst_blocks_decoded();
        assert!(db.get(k).is_some());
        assert_eq!(
            crate::sst::sst_blocks_decoded(),
            0,
            "second get of the same key must not lz4-decode again"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// MVCC latest / deps_scan: second seek of the same prefix/range is cache-only.
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
        assert_eq!(
            crate::sst::sst_blocks_decoded(),
            0,
            "second latest must hit the block cache"
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
        let mut x = 0x1357_9BDF_2468_ACE0_u64;
        let mut step = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
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
}
