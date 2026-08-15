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
//!   return only after the WAL record is appended and **`fsync`/`sync_all` completes**.
//! - After that `Ok`, a **process crash** (kill -9) must not lose the write if the OS
//!   and disk honor fsync. Reopen replays complete WAL records into the MemTable and
//!   loads SST files.
//! - A crash **during** append may leave a truncated trailing record; recovery **skips**
//!   it (no partial TX visible). Multi-key commit is one WAL record → all-or-nothing.
//! - Uncommitted transactions leave no WAL record (drop/abort = no durability side effect).
//! - [`OpenOptions::sync`] = `false` is for bulk load/benches only: process crash may
//!   still retain OS-buffered data; **power loss can lose recent acks** (JetStream/Jepsen lesson).
//! - **`Err` after a required WAL sync does not mean “record absent on disk”** (uncertain):
//!   append may have succeeded while `sync_all` failed. The open handle is then
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
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;

use crate::batch::{WriteOp, WriteRecord};
use crate::cache::{BlockCache, TableCache};
use crate::change_feed::{ChangeEntry, ChangeKind, ChangeLog};
use crate::changelog_kernel::changelog_needs_sst_rebuild;
use crate::env::{Env, EnvFile, StdEnv};
use crate::error::{CoreError, Result};
use crate::host::Host;
use crate::key::{InternalKey, SequenceNumber, ValueType, MAX_SEQUENCE_NUMBER};
use crate::lock::DirLock;
use crate::manifest::{self, VersionSet};
use crate::memtable::{Lookup, MemTable};
use crate::merge::{range_deleted, StreamingVisibleIter, VisibleKv};
use crate::sst::{write_sst_entries_on, write_sst_on, SstTable};
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
    /// When true (default), each successful `put`/`delete`/`commit` syncs the WAL
    /// before returning (RFC-0001 O1). Overridable per write via [`WriteOptions`].
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
    /// WAL `sync_all` calls (group commit amortizes this under concurrent writers).
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
}

/// Metadata written next to a checkpoint (ops / restore tooling).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointMeta {
    /// Sequence frozen at checkpoint time.
    pub last_sequence: SequenceNumber,
    /// Number of SST files copied.
    pub sst_count: usize,
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

/// What a range scan yields (RFC-0019 P1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScanProjection {
    /// Full key + resolved value (default).
    #[default]
    Full,
    /// User keys only; `VisibleKv::value` is empty (no vlog resolve).
    KeyOnly,
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
    wal: Wal<E::File>,
    /// Active memtable (new writes).
    mem: MemTable,
    /// Immutable memtable being flushed (Rocks dual-memtable / pipeline).
    /// Reads consult `mem` then `imm` then SSTs. Writers only mutate `mem`.
    imm: Option<MemTable>,
    /// Clone of the table taken by [`Self::prepare_flush_imm`] so readers still
    /// see acked keys while SST I/O runs off the write lock.
    flush_read_pin: Option<MemTable>,
    /// Immutable tables, oldest → newest within inventory order.
    ssts: Vec<SstTable>,
    /// LSM level for each entry in [`Self::ssts`] (parallel array; 0 = L0).
    sst_levels: Vec<u32>,
    /// Next SST file number (`000001.sst`, …).
    next_file_num: u64,
    /// Last written MANIFEST file number (0 = none yet).
    manifest_file_num: u64,
    /// MANIFEST flag: open `VALUES.vlog.new` (SST pointers already remapped).
    vlog_use_new: bool,
    /// Next sequence to assign (1-based; 0 means “no writes yet”).
    next_seq: SequenceNumber,
    sync: bool,
    auto_flush_bytes: Option<usize>,
    auto_compact_sst_count: Option<usize>,
    auto_compact_sst_bytes: Option<u64>,
    /// Reuses decoded SST handles (verify / reopen path).
    table_cache: TableCache,
    /// Decompressed block cache (hit stats for read path).
    block_cache: BlockCache,
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
    /// Count of successful WAL `sync_all` (observability / group-commit tests).
    wal_sync_count: u64,
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
        let block_cache = BlockCache::new(256);
        let (ssts, sst_levels, next_file_num, manifest_file_num, vlog_use_new, mut max_seq) =
            recover_ssts(&env, &dir, opts.sync, &table_cache)?;

        let wal_path = dir.join(WAL_FILE_NAME);
        let mut mem = MemTable::new();
        let mut change_log = ChangeLog::load_on(&env, &dir)?;

        if env.exists(&wal_path) {
            // Tiny WAL + Truncated(0): failed first append after rotate (or crash
            // before any complete record). Tolerate empty so SSTs still load (F6).
            // Large WAL + Truncated(0): bitrot of the first record — fail-stop (F4).
            let records = match Wal::recover_on(&env, &wal_path) {
                Ok(r) => r,
                Err(CoreError::Truncated(0)) => {
                    let len = env.metadata_len(&wal_path).unwrap_or(0);
                    if len < 64 {
                        Vec::new()
                    } else {
                        return Err(CoreError::Truncated(0));
                    }
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
        }

        let wal = if env.exists(&wal_path) {
            Wal::append_on(&env, &wal_path)?
        } else {
            Wal::create_on(&env, &wal_path)?
        };

        let next_seq = max_seq.saturating_add(1).max(1);
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
            mem,
            imm: None,
            flush_read_pin: None,
            ssts,
            sst_levels,
            next_file_num,
            manifest_file_num,
            vlog_use_new,
            next_seq,
            sync: opts.sync,
            auto_flush_bytes: opts.auto_flush_bytes.filter(|n| *n > 0),
            auto_compact_sst_count: opts.auto_compact_sst_count.filter(|n| *n > 0),
            auto_compact_sst_bytes: opts.auto_compact_sst_bytes.filter(|n| *n > 0),
            table_cache,
            block_cache,
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
            wal_sync_count: 0,
            bytes_ingested: 0,
            bytes_written_wal: 0,
            bytes_written_sst: 0,
            compact_count: 0,
            vlog_gc_count: 0,
            change_log,
        };
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
        self.wal_sync_count
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

    /// Number of SST files currently loaded.
    #[must_use]
    pub fn sst_count(&self) -> usize {
        self.ssts.len()
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
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            seq: self.last_sequence(),
        }
    }

    /// Point lookup at the latest committed sequence (MemTable ∪ SSTs).
    #[must_use]
    pub fn get(&self, key: &[u8]) -> Option<Bytes> {
        self.get_at(self.snapshot(), key)
    }

    /// Point lookup at an explicit [`Snapshot`].
    #[must_use]
    pub fn get_at(&self, snap: Snapshot, key: &[u8]) -> Option<Bytes> {
        if snap.seq == 0 {
            return None;
        }
        match self.lookup(key, snap.seq) {
            Lookup::Found(v) => self.resolve_stored_value(v).ok(),
            Lookup::Deleted | Lookup::NotFound => None,
        }
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
    #[must_use]
    pub fn range(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> Vec<(Bytes, Bytes)> {
        self.range_at(self.last_sequence(), start, end)
    }

    /// Range scan at an explicit snapshot sequence.
    #[must_use]
    pub fn range_at(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
    ) -> Vec<(Bytes, Bytes)> {
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
        self.range_at_limited(self.last_sequence(), start, end, limit)
    }

    /// Range at `snapshot` with optional live-key `limit`.
    ///
    /// Uses the streaming merge path ([`Self::scan_at`]) so the full keyspace is
    /// not required as a single materialised `Vec` of all live pairs.
    #[must_use]
    pub fn range_at_limited(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
    ) -> Vec<(Bytes, Bytes)> {
        self.scan_at(snapshot, start, end, limit)
            .filter_map(|VisibleKv { key, value }| {
                let value = self.resolve_stored_value(value).ok()?;
                Some((key, value))
            })
            .collect()
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
        self.scan_at_projected(self.last_sequence(), start, end, None, projection)
    }

    /// Streaming range at `snapshot` with optional live-key `limit`.
    pub fn scan_at(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
    ) -> impl Iterator<Item = VisibleKv> + '_ {
        self.scan_at_projected(snapshot, start, end, limit, ScanProjection::Full)
    }

    /// [`scan_at`](Self::scan_at) with projection.
    pub fn scan_at_projected(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
        projection: ScanProjection,
    ) -> impl Iterator<Item = VisibleKv> + '_ {
        let resolve = matches!(projection, ScanProjection::Full);
        let raw = self.scan_at_raw(snapshot, start, end, limit, resolve);
        raw.map(move |VisibleKv { key, value }| match projection {
            ScanProjection::Full => VisibleKv { key, value },
            ScanProjection::KeyOnly => VisibleKv {
                key,
                value: Bytes::new(),
            },
        })
    }

    fn scan_at_raw(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
        resolve_values: bool,
    ) -> StreamingVisibleIter {
        if snapshot == 0 {
            return StreamingVisibleIter::new(Vec::new(), 0, start, end, limit);
        }
        let mut streams: Vec<Vec<(InternalKey, Bytes)>> = Vec::with_capacity(3 + self.ssts.len());
        // F110: include flush_read_pin so range/scan see acked keys during off-lock flush.
        for table in self.mem_layers() {
            streams.push(self.memtable_stream(table, start, end, resolve_values));
        }
        for table in &self.ssts {
            let mut s = table.entries_in_user_range(start, end);
            if resolve_values {
                self.prefetch_resolve_stream(&mut s);
            }
            streams.push(s);
        }
        StreamingVisibleIter::new(streams, snapshot, start, end, limit)
    }

    fn memtable_stream(
        &self,
        table: &MemTable,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        resolve_values: bool,
    ) -> Vec<(InternalKey, Bytes)> {
        let mut stream = Vec::new();
        for (k, v) in table.iter_internal() {
            if k.kind == ValueType::RangeDeletion
                || crate::merge::user_key_in_range(k.user_key.as_ref(), start, end)
            {
                stream.push((k.clone(), v.clone()));
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
    /// Order of `stream` is unchanged. Missing/corrupt values become empty bytes
    /// (same as [`Self::resolve_stream_value`]).
    fn prefetch_resolve_stream(&self, stream: &mut [(InternalKey, Bytes)]) {
        let n = self.scan_prefetch.max(1);
        let mut i = 0;
        while i < stream.len() {
            let end = (i + n).min(stream.len());
            let mut issued = 0u64;
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
            wal_sync_count: self.wal_sync_count,
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
        let chlog = self.dir.join(crate::change_feed::CHANGELOG_FILE_NAME);
        if self.env.exists(&chlog) {
            self.env
                .copy_file(&chlog, &dest.join(crate::change_feed::CHANGELOG_FILE_NAME))?;
        }

        let meta = CheckpointMeta {
            last_sequence: self.last_sequence(),
            sst_count: self.ssts.len(),
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
        self.imm = Some(std::mem::replace(&mut self.mem, MemTable::new()));
        self.flush_imm_to_l0()?;
        self.try_rotate_wal()?;
        self.run_auto_compact_best_effort();
        Ok(())
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
        if let Some(ref table) = taken {
            self.flush_read_pin = Some(table.clone());
        }
        Ok(taken)
    }

    /// Drop the off-lock flush read pin (after a test wants the pre-fix hole).
    pub fn clear_flush_read_pin(&mut self) {
        self.flush_read_pin = None;
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

    /// Write `imm` to L0 using a **pre-allocated** file number (no Db write lock).
    ///
    /// Prefer [`Self::alloc_file_num`] under the write lock, then this for I/O.
    ///
    /// # Errors
    /// SST I/O.
    pub fn write_memtable_to_l0_file_num(
        &self,
        imm: &MemTable,
        num: u64,
    ) -> Result<(SstTable, u64, PathBuf)> {
        let final_path = self.dir.join(format!("{num:06}.sst"));
        let tmp_path = self.dir.join(format!("{num:06}.sst.tmp"));
        match write_sst_on(&self.env, &tmp_path, imm) {
            Ok(table) => {
                drop(table);
                self.env.rename(&tmp_path, &final_path)?;
                if self.sync {
                    self.env.sync_dir(&self.dir)?;
                }
                let table = SstTable::open_on(&self.env, &final_path)?;
                Ok((table, num, final_path))
            }
            Err(e) => {
                let _ = self.env.remove_file(&tmp_path);
                let _ = self.env.remove_file(&final_path);
                Err(e)
            }
        }
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
        self.note_sst_bytes_written(table.path());
        self.table_cache.insert(Arc::new(table.clone()));
        let prev_next = self.next_file_num;
        // Pre-allocated: next already > file_num. Exclusive peek path: advance.
        if self.next_file_num <= file_num {
            self.next_file_num = file_num.saturating_add(1);
        }
        self.ssts.push(table);
        self.sst_levels.push(0);
        if let Err(e) = self.persist_manifest() {
            let _ = self.ssts.pop();
            let _ = self.sst_levels.pop();
            self.next_file_num = prev_next;
            return Err(e);
        }
        // SST now holds this pipeline's table; drop the read pin (not `imm` — F45).
        self.flush_read_pin = None;
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
        self.mem.is_empty() && self.imm.is_none() && self.flush_read_pin.is_none()
    }

    /// Whether an immutable memtable is present.
    #[must_use]
    pub fn has_imm(&self) -> bool {
        self.imm.is_some()
    }

    /// Mem / imm / off-lock flush pin — every table `get`/`scan` must consult.
    fn mem_layers(&self) -> impl Iterator<Item = &MemTable> {
        std::iter::once(&self.mem)
            .chain(self.imm.as_ref())
            .chain(self.flush_read_pin.as_ref())
    }

    /// After L0 install: rotate WAL if safe + opportunistic compact.
    ///
    /// # Errors
    /// WAL rotate I/O.
    pub fn finish_flush_pipeline(&mut self) -> Result<()> {
        self.try_rotate_wal()?;
        self.run_auto_compact_best_effort();
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
        if !self.mem_is_empty_for_rotate() {
            return Ok(());
        }
        self.rotate_wal_now()
    }

    fn rotate_wal_now(&mut self) -> Result<()> {
        let wal_path = self.dir.join(WAL_FILE_NAME);
        let old = std::mem::replace(&mut self.wal, Wal::create_on(&self.env, &wal_path)?);
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
        self.persist_manifest()?;

        for path in old_paths {
            if path != final_path {
                let _ = self.env.remove_file(&path);
            }
        }
        self.compact_count = self.compact_count.saturating_add(1);
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
            if options.gc.keep_only_latest || options.gc.min_sequence > 0 {
                return self.compact_levels(MAX_LSM_LEVEL, MAX_LSM_LEVEL, options);
            }
            return Ok(());
        };
        let to = (from + 1).min(MAX_LSM_LEVEL);
        // Skip no-op when single file already at `to` and no GC requested.
        if from == to
            && self.ssts.len() == 1
            && !options.gc.keep_only_latest
            && options.gc.min_sequence == 0
        {
            return Ok(());
        }
        self.compact_levels(from, to, options)
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
            && !options.gc.keep_only_latest
            && options.gc.min_sequence == 0
        {
            return Ok(());
        }

        let mut merged: Vec<(InternalKey, Bytes)> = Vec::new();
        for &i in &input_idxs {
            merged.extend(self.ssts[i].entries_cloned());
        }
        let merged = crate::merge::gc_compact_entries(merged, options.gc);

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
        keep_tables.push(new_table);
        keep_levels.push(to_level);
        self.ssts = keep_tables;
        self.sst_levels = keep_levels;

        if let Ok(len) = self.env.metadata_len(&final_path) {
            self.bytes_written_sst = self.bytes_written_sst.saturating_add(len);
        }
        self.persist_manifest()?;

        for path in old_paths {
            if path != final_path {
                let _ = self.env.remove_file(&path);
            }
        }
        self.compact_count = self.compact_count.saturating_add(1);
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

        if let Err(e) = self.persist_manifest() {
            self.ssts = prev_ssts;
            self.sst_levels = prev_levels;
            self.next_file_num = prev_next;
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

        if let Err(e) = self.persist_manifest() {
            self.ssts = prev_ssts;
            self.sst_levels = prev_levels;
            self.next_file_num = prev_next;
            self.vlog_use_new = false;
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
    pub fn multi_get_at(&self, snap: Snapshot, keys: &[impl AsRef<[u8]>]) -> Vec<Option<Bytes>> {
        keys.iter().map(|k| self.get_at(snap, k.as_ref())).collect()
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
        Ok(self.change_log.changes_in(from_seq, to))
    }

    /// All durable changes with `sequence > from_seq` (tail / watch catch-up).
    #[must_use]
    pub fn changes_after(&self, from_seq: SequenceNumber) -> Vec<ChangeEntry> {
        self.change_log
            .changes_after(from_seq.min(self.last_sequence()))
    }

    /// When CHANGELOG is missing after flush (WAL already truncated), rebuild a
    /// last-per-key feed from MemTable ∪ SSTs so fold/journal are not empty.
    fn maybe_rebuild_feed_from_live(&mut self) {
        let feed_empty = self.change_log.max_sequence().unwrap_or(0) == 0;
        if !changelog_needs_sst_rebuild(feed_empty, self.last_sequence()) {
            return;
        }
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
        if latest.is_empty() {
            return;
        }
        let entries: Vec<ChangeEntry> = latest
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
            .collect();
        self.change_log.replace_sorted(entries);
        let _ = self.change_log.store_on(&self.env, &self.dir);
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
        self.wal.sync_all()
    }

    /// Close the WAL and release the directory lock via [`Env`] when held.
    ///
    /// Prefer this over bare `drop` so unlock is fault-injectable (RFC-0015 H3).
    ///
    /// # Errors
    /// I/O from WAL flush or lock release.
    pub fn close(mut self) -> Result<()> {
        self.release_lock()?;
        // Flush in place — `Db` implements `Drop` (Env unlock), so we cannot move `wal`.
        self.wal.flush()
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
        }
        for table in &self.ssts {
            // Open-time range tombstones (no full-table materialize).
            table.collect_range_tombstones(snapshot, &mut range_tombs);
            // Lazy single-block point probe (sequence-aware across layers).
            if let Some((seq, look)) = table.point_at(key, snapshot) {
                if best_point_seq.is_none_or(|s| seq > s) {
                    best_point_seq = Some(seq);
                    best_point = look;
                }
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
        for (ikey, value) in table.iter_internal() {
            if ikey.sequence > snapshot {
                continue;
            }
            if ikey.kind == ValueType::RangeDeletion {
                range_tombs.push(crate::merge::RangeTombstone {
                    start: ikey.user_key.clone(),
                    end: value.clone(),
                    sequence: ikey.sequence,
                });
                continue;
            }
            if ikey.user_key.as_ref() != key {
                continue;
            }
            if best_point_seq.is_none_or(|s| ikey.sequence > s) {
                *best_point_seq = Some(ikey.sequence);
                *best_point = match ikey.kind {
                    ValueType::Value => Lookup::Found(value.clone()),
                    ValueType::Deletion => Lookup::Deleted,
                    ValueType::RangeDeletion => Lookup::NotFound,
                };
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
        let feed_entries: Vec<ChangeEntry> =
            records.iter().map(ChangeEntry::from_write_op).collect();
        let rec = WriteRecord { ops: records };
        // Append then sync: if either fails, caller rolls back sequence; mem not applied.
        // RFC-0015 H1: if append OK and required sync fails, fence so later fsyncs
        // cannot silently publish an unacked prefix while in-process mem diverges.
        let encoded = rec.encode();
        self.bytes_written_wal = self.bytes_written_wal.saturating_add(encoded.len() as u64);
        self.wal.append_record(&encoded)?;
        let do_sync = durability.sync.unwrap_or(self.sync);
        if do_sync {
            if let Err(e) = self.wal.sync_all() {
                self.durability_fenced = true;
                return Err(e);
            }
            self.wal_sync_count = self.wal_sync_count.saturating_add(1);
        }
        // In-memory change feed after durable WAL. CHANGELOG on disk is a cache:
        // never gate commit success on a second fsync/rename (RFC-0019) — reopen
        // rebuilds missing entries from WAL. Always apply mem once WAL is durable
        // so get and feed stay aligned and sequences are not rolled back.
        self.change_log.extend(feed_entries);
        if do_sync {
            if let Err(e) = self.change_log.store_on(&self.env, &self.dir) {
                tracing::warn!(
                    error = %e,
                    "CHANGELOG store failed after durable WAL; feed rebuilt on open"
                );
            }
        }
        apply_record(&mut self.mem, &rec);
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
            return Ok((records, self.last_sequence()));
        }
        let last = records.last().map_or(self.last_sequence(), |o| o.sequence);
        Ok((records, last))
    }

    /// Append one logical WAL record without fsync (group-commit leader path).
    pub(crate) fn wal_append_ops(&mut self, ops: Vec<WriteOp>) -> Result<()> {
        self.ensure_not_fenced()?;
        let rec = WriteRecord { ops };
        let encoded = rec.encode();
        self.bytes_written_wal = self.bytes_written_wal.saturating_add(encoded.len() as u64);
        self.wal.append_record(&encoded)
    }

    /// One WAL fsync for a group of already-appended records.
    pub(crate) fn wal_sync_group(&mut self) -> Result<()> {
        self.ensure_not_fenced()?;
        if let Err(e) = self.wal.sync_all() {
            self.durability_fenced = true;
            return Err(e);
        }
        self.wal_sync_count = self.wal_sync_count.saturating_add(1);
        Ok(())
    }

    /// Apply prepared ops to the memtable after durable WAL.
    pub(crate) fn apply_ops_to_mem(&mut self, ops: Vec<WriteOp>) {
        apply_record(&mut self.mem, &WriteRecord { ops });
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
        if batches.is_empty() {
            return Vec::new();
        }
        let n = batches.len();
        let mut results: Vec<Option<Result<SequenceNumber>>> = (0..n).map(|_| None).collect();
        let mut prepared: Vec<(usize, Vec<WriteOp>, SequenceNumber)> = Vec::new();
        let mut any_sync = false;

        for (i, (ops, do_sync)) in batches.into_iter().enumerate() {
            if ops.is_empty() {
                results[i] = Some(Ok(self.last_sequence()));
                continue;
            }
            match self.prepare_write_ops(ops) {
                Ok((write_ops, last_seq)) => {
                    if do_sync {
                        any_sync = true;
                    }
                    prepared.push((i, write_ops, last_seq));
                }
                Err(e) => results[i] = Some(Err(e)),
            }
        }

        let mut appended: Vec<(usize, Vec<WriteOp>, SequenceNumber)> = Vec::new();
        for (i, write_ops, last_seq) in prepared {
            match self.wal_append_ops(write_ops.clone()) {
                Ok(()) => appended.push((i, write_ops, last_seq)),
                Err(e) => {
                    results[i] = Some(Err(e));
                    break;
                }
            }
        }

        if appended.is_empty() {
            return finish_group_results(results);
        }

        if any_sync {
            if let Err(e) = self.wal_sync_group() {
                let msg = e.to_string();
                for (i, _, _) in &appended {
                    results[*i] = Some(Err(CoreError::Internal(format!(
                        "group wal sync failed: {msg}"
                    ))));
                }
                // Fence already set inside wal_sync_group; do not apply mem.
                return finish_group_results(results);
            }
        }

        // RFC-0019: same feed seam as commit_ops_with. After WAL is durable,
        // CHANGELOG store is best-effort (not a commit gate); always apply mem.
        let mut feed_batch: Vec<ChangeEntry> = Vec::new();
        for (_, write_ops, _) in &appended {
            for op in write_ops {
                feed_batch.push(ChangeEntry::from_write_op(op));
            }
        }
        self.change_log.extend(feed_batch);
        if any_sync {
            if let Err(e) = self.change_log.store_on(&self.env, &self.dir) {
                tracing::warn!(
                    error = %e,
                    "group CHANGELOG store failed after durable WAL; feed rebuilt on open"
                );
            }
        }

        for (i, write_ops, last_seq) in appended {
            self.apply_ops_to_mem(write_ops);
            results[i] = Some(Ok(last_seq));
        }
        self.maybe_auto_flush_best_effort();
        finish_group_results(results)
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

    pub(crate) fn maybe_auto_flush(&mut self) -> Result<()> {
        let Some(limit) = self.auto_flush_bytes else {
            return Ok(());
        };
        if self.mem.approx_memory_usage() >= limit {
            self.flush()?;
        }
        Ok(())
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
        if count_hit || bytes_hit || l0_hit {
            // F20: do **not** use latest_only here — that dropped historical versions
            // and broke `get_at` / Snapshot for sequences still "open" in the app.
            // Space-bound GC is explicit: `compact_with(CompactOptions::latest_only())`.
            self.compact_with(CompactOptions::default())?;
            // Successful auto-compact clears the last-error slot (counter stays cumulative).
            self.last_auto_compact_error = None;
        }
        Ok(())
    }

    /// Run auto-compact after flush; record failures without failing the flush.
    fn run_auto_compact_best_effort(&mut self) {
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

    /// Write MANIFEST + CURRENT for the live SST set (with levels).
    fn persist_manifest(&mut self) -> Result<()> {
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
        };
        vs.normalize_levels();
        manifest::install_next(&self.env, &self.dir, &mut vs, self.sync)?;
        self.manifest_file_num = vs.manifest_file_num;
        Ok(())
    }
}

fn write_checkpoint_meta(env: &impl Env, dest: &Path, meta: &CheckpointMeta) -> Result<()> {
    let path = dest.join(CHECKPOINT_META_FILE);
    let mut body = Vec::new();
    body.extend_from_slice(b"PDBCKP01");
    body.extend_from_slice(&meta.last_sequence.to_le_bytes());
    body.extend_from_slice(&(meta.sst_count as u64).to_le_bytes());
    let crc = crc32c::crc32c(&body);
    body.extend_from_slice(&crc.to_le_bytes());
    let mut f = env.create(&path)?;
    f.write_all(&body)?;
    f.sync_all()?;
    Ok(())
}

/// Read [`CHECKPOINT_META_FILE`] written by [`Db::create_checkpoint`].
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
    if &payload[0..8] != b"PDBCKP01" {
        return Err(CoreError::Internal("bad checkpoint meta magic".into()));
    }
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
    Ok(CheckpointMeta {
        last_sequence,
        sst_count,
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
/// Recovered SST inventory: tables, levels, next file num, manifest num, `vlog_use_new`, max seq.
type RecoveredSsts = (Vec<SstTable>, Vec<u32>, u64, u64, bool, SequenceNumber);

/// Returns `(tables, levels, next_file_num, manifest_file_num, vlog_use_new, max_sequence)`.
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
            assert_eq!(tx.get(b"row").as_deref(), Some(b"R".as_ref()));
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

        let mid = db.range(Bound::Included(b"b"), Bound::Excluded(b"d"));
        assert_eq!(mid.len(), 1);
        assert_eq!(mid[0].0.as_ref(), b"b");
        assert_eq!(mid[0].1.as_ref(), b"2b");

        let all: Vec<_> = db
            .range(Bound::Unbounded, Bound::Unbounded)
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
            let before: Vec<_> = db.range(Bound::Unbounded, Bound::Unbounded);
            db.compact().unwrap();
            assert_eq!(db.sst_count(), 1);
            let after: Vec<_> = db.range(Bound::Unbounded, Bound::Unbounded);
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
            .range(Bound::Unbounded, Bound::Unbounded)
            .into_iter()
            .map(|(k, _)| k.to_vec())
            .collect();
        assert_eq!(keys, vec![b"a".to_vec(), b"b".to_vec()]);
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
        db.put(b"k", b"old").unwrap();
        let snap = db.snapshot();
        db.flush().unwrap();
        db.put(b"k", b"new").unwrap();
        db.flush().unwrap(); // triggers auto-compact at count >= 2
        assert_eq!(db.sst_count(), 1);
        assert_eq!(db.get(b"k").as_deref(), Some(b"new".as_ref()));
        // Historical read at pre-overwrite snapshot must still see "old".
        assert_eq!(
            db.get_at(snap, b"k").as_deref(),
            Some(b"old".as_ref()),
            "F20: auto-compact must not GC versions still visible at open snapshots"
        );
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
            assert_eq!(db.get_at(snap, b"row").as_deref(), Some(b"R".as_ref()));
            assert_eq!(db.get_at(snap, b"idx").as_deref(), Some(b"I".as_ref()));
            // Old snapshot still empty world
            assert_eq!(db.get_at(before, b"row"), None);
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
            assert!(
                ckpt.join(CHECKPOINT_META_FILE).exists(),
                "CHECKPOINT meta file must be written"
            );
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
            .range(Bound::Unbounded, Bound::Unbounded)
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
            .range(Bound::Unbounded, Bound::Unbounded)
            .into_iter()
            .map(|(k, _)| k[0])
            .collect();
        assert_eq!(live, vec![b'a', b'e', b'f']);

        db.flush().unwrap();
        db.compact_with(CompactOptions::latest_only()).unwrap();
        assert_eq!(db.get(b"b"), None);
        assert_eq!(db.get(b"a").as_deref(), Some(b"v".as_ref()));
        // After latest_only GC, covered keys should not remain as live values.
        let after: Vec<_> = db.range(Bound::Unbounded, Bound::Unbounded);
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
                .range(Bound::Unbounded, Bound::Unbounded)
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
            db.get_at(Snapshot::at(seq), b"k").as_deref(),
            Some(b"v1".as_ref())
        );
        if seq > 0 {
            assert_eq!(
                db.get_at(Snapshot::at(seq - 1), b"k"),
                None,
                "seq-1 must not see the put"
            );
        }

        let del_seq = db.delete_with_seq(b"k").unwrap();
        assert!(del_seq > seq);
        assert_eq!(db.get_at(Snapshot::at(del_seq), b"k"), None);
        assert_eq!(
            db.get_at(Snapshot::at(seq), b"k").as_deref(),
            Some(b"v1".as_ref()),
            "historical snapshot still sees pre-delete put"
        );

        let last = db
            .apply_batch([BatchOp::put(b"a", b"1"), BatchOp::put(b"b", b"2")])
            .unwrap();
        assert_eq!(
            db.get_at(Snapshot::at(last), b"a").as_deref(),
            Some(b"1".as_ref())
        );
        assert_eq!(
            db.get_at(Snapshot::at(last), b"b").as_deref(),
            Some(b"2".as_ref())
        );
        // First key of batch is last-1 when two ops.
        assert_eq!(
            db.get_at(Snapshot::at(last - 1), b"b"),
            None,
            "second batch key not visible before its seq"
        );

        let mut tx = db.begin();
        tx.put(b"t1", b"x").unwrap();
        tx.put(b"t2", b"y").unwrap();
        let tx_seq = tx.commit().unwrap();
        assert_eq!(
            db.get_at(Snapshot::at(tx_seq), b"t2").as_deref(),
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
        let multi_at = db.multi_get_at(snap, &keys);
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
}
