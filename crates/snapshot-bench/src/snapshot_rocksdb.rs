//! On-disk [`SnapshotStore`] backed by [RocksDB](https://docs.rs/rust-rocksdb)
//! — the battle-tested C++ LSM, as the comparison peer.
//!
//! Ported from beyondoss/slipstream `src/snapshot_rocksdb.rs` (MIT); the
//! artifact export/import surface is not part of this harness.
//!
//! RocksDB is a C++ library: building this backend needs a C++ toolchain and
//! libclang (for bindgen), and the first `librocksdb-sys` compile takes
//! several minutes. That cost is why it is opt-in behind
//! `feature = "rocksdb"`. The binding is the maintained `rust-rocksdb`
//! fork, renamed to `rocksdb` in `Cargo.toml` so a future binding switch is
//! a one-line manifest change.
//!
//! ## How it honors the [`SnapshotStore`] invariants
//!
//! - **Atomic data + cursor.** Each [`apply`](SnapshotStore::apply) is a single
//!   RocksDB [`WriteBatch`]: every put/delete *and* the resume cursor land in
//!   one WAL entry — atomic even across column families. There is no window
//!   where the cursor names a revision whose data is missing.
//! - **Self-sufficient under NO_SYNC.** The WAL is always on; `sync` only
//!   controls whether each commit fsyncs it. With sync off (the default), a
//!   commit reaches the OS but is not fsync'd: it survives a process crash
//!   (WAL replay on reopen), while a power-loss crash can lose the un-synced
//!   *tail*. That is safe precisely because data and cursor are one atomic
//!   batch: whatever survived has its matching cursor, so on reopen the fold
//!   resumes from the recovered cursor. Set `sync = true` to fsync every
//!   commit.
//! - **Queryable.** [`get`](SnapshotStore::get) and
//!   [`range`](SnapshotStore::range) read straight from RocksDB's
//!   block-cached storage — no full-DB deserialization.
//!
//! ## Tuning
//!
//! [`open`](RocksDbSnapshot::open) configures RocksDB for the workload this
//! backend exists for — a route-scale fold (model: 1e9 entries, ~60 B keys,
//! ~200 B values ≈ 270 GB raw ≈ 125 GB on disk after compression, NVMe), bulk
//! hydration through `apply`, then steady-state churn with concurrent serving
//! reads that are overwhelmingly point-gets for keys that *exist*, plus
//! per-service prefix scans. RocksDB's own defaults (no filters at all,
//! index/filter blocks outside the cache, 4 KiB blocks, 64 MiB memtables, two
//! background jobs) are wrong at that scale; the constants below encode the
//! corrected configuration, each with its own rationale. The user-facing knobs
//! stay [`RocksDbConfig`]'s `sync` and `cache_size_bytes` — everything else is
//! an opinionated constant.
//!
//! Deliberately rejected, so nobody re-litigates them silently:
//!
//! - **Prefix extractor + prefix bloom.** The fold's prefixes are
//!   variable-length strings; a fixed/capped extractor mis-set is a famous
//!   correctness footgun, and the scans already pass iterate bounds
//!   ([`PrefixRange`]) which bound the scan without one.
//! - **Universal compaction.** Leveled + dynamic level bytes wins read and
//!   space amplification for a read-heavy fold; write amp during hydration is
//!   absorbed by parallel compaction.
//! - **Direct I/O.** The NO_SYNC WAL story leans on OS page-cache semantics;
//!   mixing direct-I/O SST reads in changes the caching contract for marginal
//!   gain at this cache size.
//! - **`atomic_flush`.** Only needed when the WAL is off; ours is always on,
//!   and WAL recovery already keeps the cross-CF batch atomic.
//! - **Statistics.** `enable_statistics` costs ~5–10% on the hot path; turn it
//!   on locally when investigating, not in the library default.
//! - **zstd dictionary training.** Dictionaries pay off when compression units
//!   are too small to self-contextualize; a 16 KiB block already holds ~60
//!   similar route records, so plain bottommost zstd captures the redundancy.

use std::path::Path;
use std::sync::Arc;

use rocksdb::{
    BlockBasedIndexType, BlockBasedOptions, Cache, ColumnFamily, ColumnFamilyDescriptor, DB,
    DBCompressionType, ErrorKind, IteratorMode, Options, PrefixRange, ReadOptions,
    WaitForCompactOptions, WriteBatch, WriteOptions,
};

use crate::kv::{KvEntry, KvUpdate, VersionToken, WatchCursor};
use crate::snapshot::{SnapshotError, SnapshotStore};
use crate::snapshot_record::{decode_entry, encode_value_into};

/// Column family holding the folded KV state: `key` → encoded `(version, value)`.
const DATA_CF: &str = "data";
/// Column family holding fold metadata (just the resume cursor today).
const META_CF: &str = "meta";
/// Key under [`META_CF`] storing the resume cursor's raw version bytes.
const CURSOR_KEY: &[u8] = b"cursor";

// --- Tuned constants (see the module-level `## Tuning` docs for the workload
// model behind the math in each comment). ---

/// Data-block size for the data CF. RocksDB's 4 KiB default generates one index
/// entry (~64 B) per block over *uncompressed* data: at 270 GB raw that is
/// ~4.2 GB of index. 16 KiB cuts the index to ~1.05 GB, improves compression
/// (more context per block), and speeds scans; the cost — a point-get
/// decompresses 16 KiB instead of 4 KiB — is a few µs, paid only on a cache miss.
const DATA_BLOCK_SIZE: usize = 16 * 1024;

/// Partition granule for the two-level index and partitioned filters. Leaf
/// index/filter partitions of this size fault through the block cache on
/// demand, so index/filter memory is cache-bounded instead of resident-per-SST.
const METADATA_BLOCK_SIZE: usize = 4096;

/// Filter bits per key (~1% false positives), all levels (~1.25 GB
/// cache-charged at 1e9 keys — see the rationale where the data CF options
/// keep bottommost filters).
const FILTER_BITS_PER_KEY: f64 = 10.0;

/// Data CF memtable size. Hydrating 270 GB through RocksDB's 64 MiB default
/// means ~4,300 flushes and 64 MB L0 files; 256 MiB quarters the flush count
/// and gives compaction bigger, fewer units of work. Memtable arena blocks
/// allocate lazily, so tiny stores don't pay this up front.
const DATA_WRITE_BUFFER_BYTES: usize = 256 << 20;

/// Up to 4 memtables (1 active + 3 immutable draining): ingest keeps moving
/// while flushes ride out compaction I/O bursts. Peak memtable RAM 1 GiB —
/// acceptable for a deliberately on-disk fold. The default of 2 stalls writes
/// whenever a single flush falls behind.
const DATA_MAX_WRITE_BUFFERS: i32 = 4;

/// Meta CF memtable. It holds exactly one key (the cursor), rewritten every
/// `apply`; 8 MiB is generous. It flushes only under WAL pressure (below).
const META_WRITE_BUFFER_BYTES: usize = 8 << 20;

/// Hard WAL cap. Every `apply` writes the meta CF, so *every* WAL file holds
/// un-flushed meta data and can only be reclaimed by flushing meta — which only
/// happens under this limit's pressure. The auto default (4× total write
/// buffers ≈ 4.1 GiB here) would mean minutes of WAL replay on a crash reopen;
/// 1 GiB bounds replay to seconds and makes the forced meta flush (a ~KB SST)
/// routine.
const MAX_TOTAL_WAL_BYTES: u64 = 1 << 30;

/// `sync_file_range` writeback smoothing for SST and WAL writes. Without it,
/// hydration accumulates multi-GB of dirty pages that the OS flushes in latency
/// spiking storms. This is a smoothing hint, NOT durability — the NO_SYNC
/// promise in the module docs is unchanged.
const SYNC_SMOOTHING_BYTES: u64 = 1 << 20;

/// Cap on flush/compaction parallelism. Background-job throughput shows
/// diminishing returns past this; beyond it, compaction competes with the
/// serving path for CPU.
const MAX_COMPACTION_PARALLELISM: usize = 16;

/// Durability and read-cache configuration for [`RocksDbSnapshot`].
///
/// Defaults to NO_SYNC (`sync: false`).
#[derive(Debug, Clone, Copy)]
pub struct RocksDbConfig {
    /// `fsync` the WAL on every [`apply`](SnapshotStore::apply) commit when
    /// `true`. When `false` (the default), commits are written to the WAL but not
    /// fsync'd (NO_SYNC): faster, survives a process crash via WAL replay, and
    /// a tail lost to power loss is rebuilt by resuming the fold from the
    /// recovered cursor — the snapshot is a cache.
    pub sync: bool,

    /// Block-cache capacity in bytes. RocksDB's own default is 32 MiB — the
    /// same starvation problem as fjall's 32 MiB default
    /// (`FjallConfig::cache_size_bytes`): a working-set hydration (a prefix
    /// range over one service's keys) misses the cache and hits disk, and the
    /// miss rate climbs as the fold grows.
    ///
    /// Index and filter blocks live *inside* this cache (see the module-level
    /// Tuning docs), so the value is an honest bound on the store's read
    /// memory. Budget at the 1 GiB default against a 1e9-key fold:
    /// ~150–175 MB of metadata (pinned top-level index partitions + upper-level
    /// filters + hot leaf index partitions), leaving ~850 MB ≈ 53k × 16 KiB
    /// data blocks ≈ ~3M resident entries — keys cluster by service prefix, so
    /// hot blocks pack hot services densely. Size it at roughly
    /// `resident working set + ~200 MB metadata`; `0` falls back to RocksDB's
    /// 32 MiB default.
    ///
    /// Measured with `benches/snapshot_backends.rs` (NVMe ext4, default
    /// 1 GiB cache, 500M-route fold: ~105 GiB settled on disk vs 27 GiB RAM,
    /// so uniform reads mostly hit disk): hot-set point-gets ~2 µs; cold
    /// uniform point-gets p50 292 µs / p99 686 µs / p999 898 µs; absent-key
    /// lookups ~320 ns (in-RAM filter rejection); per-service prefix scans
    /// ~129 ns/entry; hydration 0.20 M entries/s. The cache buys hot-set
    /// residency — the µs-vs-hundreds-of-µs gap — so size it to the working
    /// set.
    pub cache_size_bytes: u64,
}

impl Default for RocksDbConfig {
    fn default() -> Self {
        Self {
            sync: false,
            // 1 GiB: holds index/data blocks for a ~1e6-service working set
            // resident, matching the routing registries' default resident cap.
            cache_size_bytes: 1024 * 1024 * 1024,
        }
    }
}

/// On-disk durable fold backed by RocksDB. See the [module docs](self).
pub struct RocksDbSnapshot {
    // Arc so `reader()` handles share the instance: RocksDB serves reads from
    // `&DB` concurrently with writes, and `DB` is `Send + Sync`.
    db: Arc<DB>,
    config: RocksDbConfig,
    cursor: WatchCursor,
}

impl RocksDbSnapshot {
    /// Open or resume the store at `path` with explicit durability config.
    ///
    /// `path` is a directory (RocksDB database), created if absent. Returns the
    /// persisted resume cursor — [`WatchCursor::none`] when fresh — and the store.
    pub fn open(path: &Path, config: RocksDbConfig) -> Result<(WatchCursor, Self), SnapshotError> {
        std::fs::create_dir_all(path)?;

        // --- DB-wide: parallelism, WAL bound, writeback smoothing. ---
        // Left at their (good) defaults on purpose: `max_open_files = -1` (a few
        // thousand table handles are cheap; table-cache misses are not),
        // `format_version` 7, `target_file_size_base` 64 MB, `compaction_pri`
        // (min-overlapping-ratio), iterator `auto_readahead_size`, and
        // `level_compaction_dynamic_level_bytes = true` — the last is
        // load-bearing: `optimize_filters_for_hits` below assumes the bottommost
        // level holds ~90% of the data, which dynamic leveling guarantees.
        let mut db_opts = Options::default();
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);
        let cores = std::thread::available_parallelism()
            .map(std::num::NonZero::get)
            .unwrap_or(4)
            .min(MAX_COMPACTION_PARALLELISM);
        // `increase_parallelism` sets `max_background_jobs` internally — do not
        // also call `set_max_background_jobs`.
        db_opts.increase_parallelism(cores as i32);
        // Let big L0→L1 compactions (the hydration bottleneck) split into
        // parallel subcompactions instead of running single-threaded.
        db_opts.set_max_subcompactions((cores / 2).max(1) as u32);
        db_opts.set_max_total_wal_size(MAX_TOTAL_WAL_BYTES);
        db_opts.set_bytes_per_sync(SYNC_SMOOTHING_BYTES);
        db_opts.set_wal_bytes_per_sync(SYNC_SMOOTHING_BYTES);

        // --- Block cache, shared by both CFs so memory accounting is unified.
        // HyperClockCache over LRU: reads are lock-free, where LRU's sharded
        // mutexes are the known contention point for many concurrent reader
        // handles. Entry charge 0 = auto. (Falling back to LRU is a one-line
        // change: `Cache::new_lru_cache(capacity)`.) `cache_size_bytes == 0`
        // keeps RocksDB's 32 MiB default LRU but still applies every other
        // table option.
        let cache = if config.cache_size_bytes > 0 {
            let capacity = usize::try_from(config.cache_size_bytes).map_err(|_| {
                SnapshotError::InvalidFormat(format!(
                    "cache_size_bytes {} exceeds usize on this platform",
                    config.cache_size_bytes
                ))
            })?;
            Some(Cache::new_hyper_clock_cache(capacity, 0))
        } else {
            None
        };

        // --- Data CF table format: blocks, filters, partitioned index. ---
        let mut data_tbl = BlockBasedOptions::default();
        if let Some(cache) = &cache {
            data_tbl.set_block_cache(cache);
        }
        data_tbl.set_block_size(DATA_BLOCK_SIZE);
        // Hybrid ribbon: bloom at L0 (files live minutes; ribbon's ~4× build CPU
        // on every memtable flush isn't worth it there), ribbon below (same FP
        // rate at ~70% of bloom's memory). RocksDB's default is NO filters at
        // all — every miss would probe data blocks in every level.
        data_tbl.set_hybrid_ribbon_filter(FILTER_BITS_PER_KEY, 1);
        data_tbl.set_optimize_filters_for_memory(true);
        // Two-level index + partitioned filters: leaf partitions fault through
        // the block cache instead of living whole-and-resident per SST.
        // `set_partition_filters` is a silent no-op without
        // `TwoLevelIndexSearch` — keep these adjacent.
        data_tbl.set_index_type(BlockBasedIndexType::TwoLevelIndexSearch);
        data_tbl.set_partition_filters(true);
        data_tbl.set_metadata_block_size(METADATA_BLOCK_SIZE);
        // Index/filter blocks count against (and live in) the cache, so
        // `cache_size_bytes` is an honest bound on the store's read memory; pin
        // L0 and the top-level index partitions (~25 MB at 1e9 keys) so the hot
        // lookup path never faults its roots.
        data_tbl.set_cache_index_and_filter_blocks(true);
        data_tbl.set_pin_l0_filter_and_index_blocks_in_cache(true);
        // Requires `cache_index_and_filter_blocks(true)` (set above).
        data_tbl.set_pin_top_level_index_and_filter(true);

        // --- Data CF: compression, hit-optimized filters, memtables. ---
        // Only lz4 and zstd are compiled into the binding (build-time trim of
        // the default five compression libs); RocksDB's own default is Snappy,
        // which would fail at the first flush/compaction — not at open — if
        // left unset. lz4 on the write-hot upper levels, zstd where ~90% of the
        // bytes settle.
        let mut data_opts = Options::default();
        data_opts.set_compression_type(DBCompressionType::Lz4);
        data_opts.set_bottommost_compression_type(DBCompressionType::Zstd);
        // Bottommost-level filters are kept (no `optimize_filters_for_hits`):
        // they are the only in-memory rejection for absent-key lookups, and on
        // a tree carrying compaction debt (mid-hydration, post-bulk-load) they
        // also reject the overlapping runs a point-get must otherwise probe on
        // disk. ~1.25 GB of cache-charged filter mass at 1e9 keys is the
        // cheapest read-latency insurance available at that scale.
        data_opts.set_write_buffer_size(DATA_WRITE_BUFFER_BYTES);
        data_opts.set_max_write_buffer_number(DATA_MAX_WRITE_BUFFERS);
        // Must come after every `data_tbl` mutation — the factory snapshots the
        // table options.
        data_opts.set_block_based_table_factory(&data_tbl);

        // --- Meta CF: one key (the cursor); tiny memtable, shared cache. ---
        let mut meta_opts = Options::default();
        meta_opts.set_compression_type(DBCompressionType::Lz4);
        meta_opts.set_write_buffer_size(META_WRITE_BUFFER_BYTES);
        if let Some(cache) = &cache {
            let mut meta_tbl = BlockBasedOptions::default();
            meta_tbl.set_block_cache(cache);
            meta_opts.set_block_based_table_factory(&meta_tbl);
        }

        let db = DB::open_cf_descriptors(
            &db_opts,
            path,
            [
                ColumnFamilyDescriptor::new(DATA_CF, data_opts),
                ColumnFamilyDescriptor::new(META_CF, meta_opts),
            ],
        )
        .map_err(map_rocksdb)?;

        let cursor = match db
            .get_cf(cf(&db, META_CF)?, CURSOR_KEY)
            .map_err(map_rocksdb)?
        {
            Some(raw) => VersionToken::from_raw(&raw)
                .map(WatchCursor::from_version)
                .ok_or_else(|| {
                    SnapshotError::InvalidFormat(format!(
                        "stored cursor is {} bytes, exceeds version token capacity",
                        raw.len()
                    ))
                })?,
            None => WatchCursor::none(),
        };

        Ok((
            cursor.clone(),
            Self {
                db: Arc::new(db),
                config,
                cursor,
            },
        ))
    }

    /// A cheap, concurrent-read-safe handle to the fold's data column family.
    ///
    /// RocksDB serves readers concurrently with the writer, so a consumer can
    /// clone this out and then `get`/`range` from a separate serving task.
    /// That is the working-set-serving pattern for a fold too large to hold
    /// resident: seed the hot set, serve it from RAM, and `range` the cold
    /// tail from the fold on a cache miss — without the serving path ever
    /// touching the writer.
    pub fn reader(&self) -> RocksDbReader {
        RocksDbReader {
            db: Arc::clone(&self.db),
        }
    }

    /// Flush memtables and block until background compaction debt is fully
    /// drained.
    ///
    /// A bulk hydration leaves the tree with pending compactions that inflate
    /// cold-read latency until they drain (measured: ~8× on cold point-gets
    /// at 500M routes). Call this after hydrating and before
    /// latency-sensitive serving begins; steady-state folding does not need
    /// it. Cheap relative to the fjall backend's full-rewrite
    /// `FjallSnapshot::settle`: ~40 s at 500M routes, because the background
    /// threads drained most debt during hydration.
    pub fn settle(&self) -> Result<(), SnapshotError> {
        let mut opts = WaitForCompactOptions::default();
        opts.set_flush(true);
        self.db.wait_for_compact(&opts).map_err(map_rocksdb)
    }
}

/// A concurrent read handle over a [`RocksDbSnapshot`]'s data column family,
/// cloned via [`RocksDbSnapshot::reader`]. Reads share the same on-disk fold as
/// the writer and are safe to run concurrently with it.
#[derive(Clone)]
pub struct RocksDbReader {
    db: Arc<DB>,
}

impl RocksDbReader {
    /// Live entry for `key`, or `None` if absent/deleted.
    pub fn get(&self, key: &str) -> Result<Option<KvEntry>, SnapshotError> {
        get_entry(&self.db, key)
    }

    /// Batched point lookups: one RocksDB `MultiGet` instead of N independent
    /// `get`s. RocksDB sorts the keys internally and coalesces filter probes
    /// and index lookups per SST.
    ///
    /// Measured (`benches/snapshot_backends.rs`, 100-key uniform random
    /// batches against a 500M-route fold, most reads hitting NVMe): on a
    /// settled tree, 19.5 ms per batch vs 23.7 ms for a loop of
    /// [`get`](Self::get)s (~1.2×); on a tree mid-compaction-debt the gap was
    /// 5.5× (18.5 ms vs 103 ms) — `MultiGet` overlaps the cold block reads
    /// the loop pays sequentially, so its edge grows with reads-per-get.
    /// Against a cache-resident working set the loop is marginally *faster*
    /// (per-batch marshaling, nothing left to coalesce). Use this when
    /// batches are likely to miss; use the loop when they're hot.
    ///
    /// Results are positionally aligned with the input; `None` means
    /// absent/deleted, exactly as [`get`](Self::get).
    pub fn multi_get<'k>(
        &self,
        keys: impl IntoIterator<Item = &'k str>,
    ) -> Result<Vec<Option<KvEntry>>, SnapshotError> {
        let keys: Vec<&str> = keys.into_iter().collect();
        let data = cf(&self.db, DATA_CF)?;
        // `sorted_input = false`: callers pass arbitrary route keys; RocksDB
        // sorts a copy internally.
        let results = self
            .db
            .batched_multi_get_cf(data, keys.iter().map(|k| k.as_bytes()), false);
        keys.iter()
            .zip(results)
            .map(|(key, res)| match res.map_err(map_rocksdb)? {
                // The pinnable slice borrows the block cache; `decode_entry`
                // copies out of it, so nothing is held past this closure.
                Some(raw) => Ok(Some(decode_entry(key, &raw)?)),
                None => Ok(None),
            })
            .collect()
    }

    /// Stream every live entry whose key starts with `prefix`, ascending, without
    /// buffering the whole match set — the memory-bounded scan for an on-disk fold.
    pub fn for_each_in_range(
        &self,
        prefix: &str,
        f: impl FnMut(KvEntry) -> Result<(), SnapshotError>,
    ) -> Result<(), SnapshotError> {
        scan_prefix(&self.db, prefix, f)
    }

    /// Buffered counterpart to [`for_each_in_range`](Self::for_each_in_range) for
    /// bounded prefixes (e.g. one service's routes).
    pub fn range(&self, prefix: &str) -> Result<Vec<KvEntry>, SnapshotError> {
        let mut out = Vec::new();
        self.for_each_in_range(prefix, |e| {
            out.push(e);
            Ok(())
        })?;
        Ok(out)
    }
}

impl SnapshotStore for RocksDbSnapshot {
    fn load(path: &Path) -> Result<(WatchCursor, Self), SnapshotError> {
        Self::open(path, RocksDbConfig::default())
    }

    fn apply(&mut self, batch: &[KvUpdate], cursor: &WatchCursor) -> Result<(), SnapshotError> {
        let data = cf(&self.db, DATA_CF)?;
        let meta = cf(&self.db, META_CF)?;
        // One atomic batch: a WriteBatch is a single WAL entry even across column
        // families, so every data mutation AND the cursor commit together. Either
        // the whole fold step is durable or none of it is — the cursor never
        // outraces its data.
        let mut wb = WriteBatch::default();
        // One scratch buffer reused across the whole batch. `put_cf` copies the
        // bytes into the batch's internal representation before returning, and
        // `encode_value_into` clears `buf` before refilling it (its documented
        // contract — stale bytes from the previous entry can never leak into the
        // next value). That turns N per-`Put` assembly allocations into one
        // amortized allocation.
        let mut scratch = Vec::new();
        for update in batch {
            match update {
                KvUpdate::Put(entry) => {
                    encode_value_into(&mut scratch, &entry.value, &entry.version)?;
                    wb.put_cf(data, entry.key.as_bytes(), scratch.as_slice());
                }
                KvUpdate::Delete { key, .. } | KvUpdate::Purge { key, .. } => {
                    wb.delete_cf(data, key.as_bytes());
                }
            }
        }
        // Cursor in the SAME batch as the data it names.
        wb.put_cf(meta, CURSOR_KEY, cursor.version().as_bytes());

        // The WAL is always on; `set_sync` only toggles the per-commit fsync.
        // NO_SYNC (sync: false) reaches the OS — survives a process crash via WAL
        // replay, not a power loss — exactly the cache semantics the module docs
        // promise.
        let mut wo = WriteOptions::default();
        wo.set_sync(self.config.sync);
        self.db.write_opt(&wb, &wo).map_err(map_rocksdb)?;

        self.cursor = cursor.clone();
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<KvEntry>, SnapshotError> {
        get_entry(&self.db, key)
    }

    fn range(&self, prefix: &str) -> Result<Vec<KvEntry>, SnapshotError> {
        // Collect the streaming scan — same decode path as `for_each_in_range`,
        // just buffered. RocksDB yields keys in ascending byte order, so the
        // result is already sorted.
        let mut out = Vec::new();
        self.for_each_in_range(prefix, |entry| {
            out.push(entry);
            Ok(())
        })?;
        Ok(out)
    }

    fn for_each_in_range(
        &self,
        prefix: &str,
        f: impl FnMut(KvEntry) -> Result<(), SnapshotError>,
    ) -> Result<(), SnapshotError> {
        scan_prefix(&self.db, prefix, f)
    }

    fn cursor(&self) -> WatchCursor {
        self.cursor.clone()
    }
}

/// Resolve a column family handle, mapping the impossible-after-`open` absence
/// to a backend error rather than a panic.
fn cf<'a>(db: &'a DB, name: &str) -> Result<&'a ColumnFamily, SnapshotError> {
    db.cf_handle(name)
        .ok_or_else(|| SnapshotError::Backend(format!("missing column family: {name}")))
}

/// Point lookup shared by [`RocksDbSnapshot::get`] and [`RocksDbReader::get`].
fn get_entry(db: &DB, key: &str) -> Result<Option<KvEntry>, SnapshotError> {
    match db
        .get_cf(cf(db, DATA_CF)?, key.as_bytes())
        .map_err(map_rocksdb)?
    {
        Some(raw) => Ok(Some(decode_entry(key, &raw)?)),
        None => Ok(None),
    }
}

/// Streaming prefix scan shared by the snapshot and its reader handles.
///
/// The iterator is lazy — entries are decoded and handed to `f` one at a time,
/// so a 1B-route consumer building a serving index never holds more than a
/// single `KvEntry` in memory at once. `PrefixRange` sets both iterate bounds
/// from the prefix, so RocksDB terminates the scan internally at the bound
/// (never scanning tombstones past the prefix) and handles the all-`0xFF`
/// successor edge case; an empty prefix is an unbounded full scan.
fn scan_prefix(
    db: &DB,
    prefix: &str,
    mut f: impl FnMut(KvEntry) -> Result<(), SnapshotError>,
) -> Result<(), SnapshotError> {
    let mut read_opts = ReadOptions::default();
    read_opts.set_iterate_range(PrefixRange(prefix.as_bytes()));
    // An empty prefix is the build-a-serving-index full scan: streaming ~the
    // whole fold through the block cache would evict the entire hot set, so
    // don't populate it. Bounded prefix scans keep `fill_cache = true`
    // deliberately — a per-service hydration *defines* the working set.
    if prefix.is_empty() {
        read_opts.fill_cache(false);
    }
    for item in db.iterator_cf_opt(cf(db, DATA_CF)?, read_opts, IteratorMode::Start) {
        let (raw_key, raw_val) = item.map_err(map_rocksdb)?;
        let key = std::str::from_utf8(&raw_key).map_err(|e| {
            SnapshotError::InvalidFormat(format!("non-UTF-8 key in rocksdb store: {e}"))
        })?;
        f(decode_entry(key, &raw_val)?)?;
    }
    Ok(())
}

/// Map a [`rocksdb::Error`] into the backend-agnostic [`SnapshotError`].
fn map_rocksdb(e: rocksdb::Error) -> SnapshotError {
    match e.kind() {
        // Keep I/O failures (disk full, permission denied, …) in the variant
        // operators already match across backends. RocksDB statuses are strings —
        // there is no real errno to preserve — so wrap the message rather than
        // flatten it into the opaque backend bucket.
        ErrorKind::IOError => SnapshotError::Io(std::io::Error::other(e.into_string())),
        // Everything else keeps RocksDB's own status text. Deliberately NOT
        // mapping `Corruption` to `SnapshotError::Corrupted`: that variant's
        // `Display` is the append log's hardcoded "CRC mismatch" text, which would
        // mask RocksDB's far more detailed corruption status.
        _ => SnapshotError::Backend(e.into_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A persisted cursor blob larger than the version-token capacity must surface
    /// as a recoverable `InvalidFormat` at `open`, not a panic or a silently
    /// truncated cursor that would resume the fold from the wrong position.
    #[test]
    fn open_rejects_corrupted_cursor() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("store");

        {
            let (_c, store) =
                RocksDbSnapshot::open(&path, RocksDbConfig::default()).expect("initial open");
            // Write an 11-byte blob straight into the meta column family under
            // the cursor key, bypassing the apply path's bounded encoding. The
            // default WriteOptions keep the WAL on, so the write survives the
            // drop below via WAL replay.
            store
                .db
                .put_cf(
                    cf(&store.db, META_CF).expect("meta cf"),
                    CURSOR_KEY,
                    [0u8; 11],
                )
                .expect("insert oversized cursor");
        }

        match RocksDbSnapshot::open(&path, RocksDbConfig::default()) {
            Err(SnapshotError::InvalidFormat(_)) => {}
            Err(other) => panic!("expected InvalidFormat, got {other:?}"),
            Ok(_) => panic!("expected open to reject the oversized cursor"),
        }
    }
}
