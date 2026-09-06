//! On-disk [`SnapshotStore`] backed by [fjall](https://docs.rs/fjall).
//!
//! Ported from beyondoss/slipstream `src/snapshot_fjall.rs` (MIT); the
//! artifact export/import surface is not part of this harness.
//!
//! ## How it honors the [`SnapshotStore`] invariants
//!
//! - **Atomic data + cursor.** Each [`apply`](SnapshotStore::apply) is a single
//!   fjall write batch: every put/delete *and* the resume cursor land under one
//!   sequence number and commit together. There is no window where the cursor
//!   names a revision whose data is missing.
//! - **Self-sufficient under NO_SYNC.** The durability mode is configurable.
//!   With sync off (the default), a commit is not fsync'd; a power-loss crash
//!   can lose the un-synced *tail*. That is safe precisely because data and
//!   cursor are one atomic batch: whatever survived has its matching cursor,
//!   so on reopen the fold resumes from the recovered cursor. Set
//!   `sync = true` to fsync every commit.
//! - **Queryable.** [`get`](SnapshotStore::get) and
//!   [`range`](SnapshotStore::range) read straight from fjall's block-cached,
//!   `Slice`-backed storage — no full-DB deserialization.
//!
//! ## Tuning
//!
//! [`open`](FjallSnapshot::open) applies the same route-scale workload tuning
//! as the RocksDB backend (see `snapshot_rocksdb.rs`'s Tuning docs for the
//! full model: ~1e9 entries, bulk hydration, point-gets that are ~always
//! hits, per-service prefix scans). fjall's defaults are already closer to
//! that workload than RocksDB's — bloom filters on by default (0.01% FP at
//! L0, 10 bits/key deeper), index blocks pinned at L0/L1, index and filter
//! partitioning from L3 down, lz4 from L2 down, journal capped at 512 MiB —
//! so the constants below adjust only the three levers that aren't:
//! worker-thread count (fjall caps at 4 by default), memtable size, and data
//! block size — plus pinning L0+L1 filter blocks. Skipping last-level filter
//! construction was tried and rejected: on a tree carrying compaction debt it
//! multiplied cold point-get cost (~10 ms at 500M routes), and it makes every
//! absent-key lookup a guaranteed disk probe. Everything else is deliberately
//! left at fjall's defaults.

use std::path::Path;

use fjall::config::{BlockSizePolicy, PinningPolicy};
use fjall::{Database, Keyspace, KeyspaceCreateOptions, PersistMode};

use crate::kv::{KvEntry, KvUpdate, VersionToken, WatchCursor};
use crate::snapshot::{SnapshotError, SnapshotStore};
use crate::snapshot_record::{decode_entry, encode_value_into};

/// Partition holding the folded KV state: `key` → encoded `(version, value)`.
const DATA_PARTITION: &str = "data";
/// Partition holding fold metadata (just the resume cursor today).
const META_PARTITION: &str = "meta";
/// Key under [`META_PARTITION`] storing the resume cursor's raw version bytes.
const CURSOR_KEY: &[u8] = b"cursor";

// --- Tuned constants (see the module-level `## Tuning` docs). ---

/// Flush/compaction worker threads. fjall's default is `min(cores, 4)`,
/// which starves a multi-GB hydration on a many-core box; this matches the
/// RocksDB backend's parallelism (also capped at 16 — diminishing returns,
/// and beyond it compaction competes with the serving path for CPU).
const MAX_WORKER_THREADS: usize = 16;

/// Data-partition memtable. fjall's 64 MiB default means 4× the flush (and
/// L0 compaction) count of a 256 MiB buffer during a route-scale hydration;
/// matches the RocksDB backend's write buffer. Memtables fill lazily, so
/// small stores don't pay this up front.
const DATA_MEMTABLE_BYTES: u64 = 256 << 20;

/// Meta partition holds exactly one key (the cursor), rewritten every
/// `apply`; 8 MiB is generous (parity with the RocksDB meta CF).
const META_MEMTABLE_BYTES: u64 = 8 << 20;

/// Data block size. Same math as the RocksDB backend: 4 KiB blocks at a
/// 1e9-key fold produce multi-GB block indexes; 16 KiB quarters that and
/// gives compression more context, at the cost of decompressing 16 KiB
/// instead of 4 KiB on a cache-miss point read.
const DATA_BLOCK_SIZE: u32 = 16 * 1024;

/// Durability and read-cache configuration for [`FjallSnapshot`].
///
/// Defaults to NO_SYNC (`sync: false`).
#[derive(Debug, Clone, Copy)]
pub struct FjallConfig {
    /// `fsync` every [`apply`](SnapshotStore::apply) commit when `true`. When
    /// `false` (the default), commits are not fsync'd (NO_SYNC): faster, and a
    /// tail lost to power loss is rebuilt by resuming the fold from the
    /// recovered cursor — the snapshot is a cache.
    pub sync: bool,

    /// Block-cache capacity in bytes for the LSM. fjall's own default is 32 MiB,
    /// which starves reads against a multi-hundred-MB fold: a working-set
    /// hydration (a prefix range over one service's keys) then misses the cache
    /// and hits disk, and the miss rate climbs as the fold grows (measured:
    /// 32 MiB → p50 174 us / p99 1.45 ms at 4M routes; a 2 GiB cache → 7 us /
    /// 13 us). This default sizes the cache to the hot set so hydrations stay
    /// cache-resident. `0` falls back to fjall's 32 MiB default. Set this to
    /// roughly the resident working-set size.
    pub cache_size_bytes: u64,
}

impl Default for FjallConfig {
    fn default() -> Self {
        Self {
            sync: false,
            // 1 GiB: holds index/data blocks for a ~1e6-service working set
            // resident, matching the routing registries' default resident cap.
            cache_size_bytes: 1024 * 1024 * 1024,
        }
    }
}

/// On-disk durable fold backed by fjall. See the [module docs](self).
pub struct FjallSnapshot {
    // fjall 3 renamed its types: the database root is `Database` (was `Keyspace`)
    // and each named partition is a `Keyspace` (was `PartitionHandle`).
    db: Database,
    data: Keyspace,
    meta: Keyspace,
    config: FjallConfig,
    cursor: WatchCursor,
}

impl FjallSnapshot {
    /// Open or resume the store at `path` with explicit durability config.
    ///
    /// `path` is a directory (fjall keyspace), created if absent. Returns the
    /// persisted resume cursor — [`WatchCursor::none`] when fresh — and the store.
    pub fn open(path: &Path, config: FjallConfig) -> Result<(WatchCursor, Self), SnapshotError> {
        std::fs::create_dir_all(path)?;
        let workers = std::thread::available_parallelism()
            .map(std::num::NonZero::get)
            .unwrap_or(4)
            .min(MAX_WORKER_THREADS);
        let mut builder = Database::builder(path).worker_threads(workers);
        // Size the LSM block cache to the working set (default 1 GiB). fjall's own
        // default is 32 MiB, far too small for the fold — see
        // `FjallConfig::cache_size_bytes`. `0` keeps fjall's default.
        if config.cache_size_bytes > 0 {
            builder = builder.cache_size(config.cache_size_bytes);
        }
        let db: Database = builder.open().map_err(map_fjall)?;
        let data = db
            .keyspace(DATA_PARTITION, || {
                KeyspaceCreateOptions::default()
                    .max_memtable_size(DATA_MEMTABLE_BYTES)
                    .data_block_size_policy(BlockSizePolicy::all(DATA_BLOCK_SIZE))
                    // Last-level filters are kept: they are the only in-memory
                    // rejection for absent-key lookups, and on a tree carrying
                    // compaction debt they reject the overlapping runs a
                    // point-get must otherwise probe on disk (measured:
                    // skipping them cost ~10 ms cold gets on an unsettled
                    // 500M fold).
                    //
                    // Pin L0+L1 filters so the hot lookup path never faults its
                    // filter roots (fjall's default pins L0 only).
                    .filter_block_pinning_policy(PinningPolicy::new([true, true, false]))
            })
            .map_err(map_fjall)?;
        let meta = db
            .keyspace(META_PARTITION, || {
                KeyspaceCreateOptions::default().max_memtable_size(META_MEMTABLE_BYTES)
            })
            .map_err(map_fjall)?;

        let cursor = match meta.get(CURSOR_KEY).map_err(map_fjall)? {
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
                db,
                data,
                meta,
                config,
                cursor,
            },
        ))
    }

    /// A cheap, concurrent-read-safe handle to the fold's data partition.
    ///
    /// fjall serves readers concurrently with the writer, so a consumer can
    /// clone this out and then `get`/`range` the fold from a separate serving
    /// task. That is the working-set-serving pattern
    /// for a fold too large to hold resident: seed the hot set, serve it from
    /// RAM, and `range` the cold tail from the fold on a cache miss — without
    /// the serving path ever touching the writer.
    pub fn reader(&self) -> FjallReader {
        FjallReader {
            data: self.data.clone(),
        }
    }

    /// Force a major compaction of the data partition, blocking until done.
    ///
    /// fjall's background compaction is write-driven: after a bulk hydration
    /// stops, residual overlapping runs can persist indefinitely and inflate
    /// cold-read latency (every unrejected run costs an extra disk probe —
    /// measured ~10 ms cold gets unsettled vs 542 µs p50 settled at 500M).
    /// Call this after hydrating and before latency-sensitive serving begins;
    /// steady-state folding does not need it.
    ///
    /// This is a full tree rewrite — budget for it: ~19 minutes and a
    /// transient ~2× disk footprint at 500M routes (105 GiB store), measured
    /// while old and new generations coexist. The RocksDB backend's
    /// `RocksDbSnapshot::settle` merely drains already-queued compactions
    /// (~40 s at the same scale).
    pub fn settle(&self) -> Result<(), SnapshotError> {
        self.data.major_compact().map_err(map_fjall)
    }
}

/// A concurrent read handle over a [`FjallSnapshot`]'s data partition, cloned via
/// [`FjallSnapshot::reader`]. Reads share the same on-disk fold as the writer and
/// are safe to run concurrently with it.
#[derive(Clone)]
pub struct FjallReader {
    data: Keyspace,
}

impl FjallReader {
    /// Live entry for `key`, or `None` if absent/deleted.
    pub fn get(&self, key: &str) -> Result<Option<KvEntry>, SnapshotError> {
        match self.data.get(key.as_bytes()).map_err(map_fjall)? {
            Some(raw) => Ok(Some(decode_entry(key, &raw)?)),
            None => Ok(None),
        }
    }

    /// Stream every live entry whose key starts with `prefix`, ascending, without
    /// buffering the whole match set — the memory-bounded scan for an on-disk fold.
    pub fn for_each_in_range(
        &self,
        prefix: &str,
        mut f: impl FnMut(KvEntry) -> Result<(), SnapshotError>,
    ) -> Result<(), SnapshotError> {
        for guard in self.data.prefix(prefix.as_bytes()) {
            let (raw_key, raw_val) = guard.into_inner().map_err(map_fjall)?;
            let key = std::str::from_utf8(&raw_key).map_err(|e| {
                SnapshotError::InvalidFormat(format!("non-UTF-8 key in fjall store: {e}"))
            })?;
            f(decode_entry(key, &raw_val)?)?;
        }
        Ok(())
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

impl SnapshotStore for FjallSnapshot {
    fn load(path: &Path) -> Result<(WatchCursor, Self), SnapshotError> {
        Self::open(path, FjallConfig::default())
    }

    fn apply(&mut self, batch: &[KvUpdate], cursor: &WatchCursor) -> Result<(), SnapshotError> {
        // One atomic batch: every data mutation AND the cursor commit under a
        // single sequence number. Either the whole fold step is durable or none of
        // it is — the cursor never outraces its data.
        let mut wb = self.db.batch().durability(self.durability());
        // One scratch buffer reused across the whole batch. `insert` converts its
        // value into fjall's owned `Slice` eagerly — it copies the bytes before
        // returning — so the buffer is free to be refilled for the next entry. That
        // turns N per-`Put` assembly allocations into one amortized allocation.
        let mut scratch = Vec::new();
        for update in batch {
            match update {
                KvUpdate::Put(entry) => {
                    encode_value_into(&mut scratch, &entry.value, &entry.version)?;
                    wb.insert(&self.data, entry.key.as_bytes(), scratch.as_slice());
                }
                KvUpdate::Delete { key, .. } | KvUpdate::Purge { key, .. } => {
                    wb.remove(&self.data, key.as_bytes());
                }
            }
        }
        // Cursor in the SAME batch as the data it names.
        wb.insert(&self.meta, CURSOR_KEY, cursor.version().as_bytes());
        wb.commit().map_err(map_fjall)?;

        self.cursor = cursor.clone();
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<KvEntry>, SnapshotError> {
        match self.data.get(key.as_bytes()).map_err(map_fjall)? {
            Some(raw) => Ok(Some(decode_entry(key, &raw)?)),
            None => Ok(None),
        }
    }

    fn range(&self, prefix: &str) -> Result<Vec<KvEntry>, SnapshotError> {
        // Collect the streaming scan — same decode path as `for_each_in_range`,
        // just buffered. fjall yields keys in ascending byte order, so the result
        // is already sorted.
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
        mut f: impl FnMut(KvEntry) -> Result<(), SnapshotError>,
    ) -> Result<(), SnapshotError> {
        // fjall's prefix iterator is lazy — entries are decoded and handed to `f`
        // one at a time, so a 1B-route consumer building a serving index never
        // holds more than a single `KvEntry` in memory at once.
        for guard in self.data.prefix(prefix.as_bytes()) {
            // fjall 3 yields a lazy `Guard` per entry; `into_inner` resolves it to
            // the `(key, value)` pair (loading the value, which keeps the scan lazy
            // for key-only iterations elsewhere).
            let (raw_key, raw_val) = guard.into_inner().map_err(map_fjall)?;
            let key = std::str::from_utf8(&raw_key).map_err(|e| {
                SnapshotError::InvalidFormat(format!("non-UTF-8 key in fjall store: {e}"))
            })?;
            f(decode_entry(key, &raw_val)?)?;
        }
        Ok(())
    }

    fn cursor(&self) -> WatchCursor {
        self.cursor.clone()
    }
}

impl FjallSnapshot {
    /// Per-commit durability: `fsync` when configured, otherwise NO_SYNC.
    fn durability(&self) -> Option<PersistMode> {
        if self.config.sync {
            Some(PersistMode::SyncAll)
        } else {
            // Explicit NO_SYNC: flush to OS buffers only — survives a process crash,
            // not a power loss, which is exactly the cache semantics the module docs
            // promise. Stating `Buffer` rather than `None` keeps that guarantee
            // independent of whatever default durability the keyspace was opened
            // with, so a future change to fjall's default can't silently make
            // `sync: false` durable (or weaker).
            Some(PersistMode::Buffer)
        }
    }
}

/// Map a [`fjall::Error`] into the backend-agnostic [`SnapshotError`].
fn map_fjall(e: fjall::Error) -> SnapshotError {
    match e {
        // Surface I/O failures (disk full, permission denied, …) as a real
        // `io::Error` so the OS errno and the `#[source]` chain survive, instead
        // of being flattened into an opaque backend string.
        fjall::Error::Io(io) => SnapshotError::Io(io),
        // Everything else keeps fjall's own variant name so it stays legible
        // without leaking the `fjall` type into this error enum.
        other => SnapshotError::Backend(other.to_string()),
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
                FjallSnapshot::open(&path, FjallConfig::default()).expect("initial open");
            // Write an 11-byte blob straight into the meta partition under the
            // cursor key, bypassing the apply path's bounded encoding.
            store
                .meta
                .insert(CURSOR_KEY, [0u8; 11])
                .expect("insert oversized cursor");
            store.db.persist(PersistMode::SyncAll).expect("persist");
        }

        // `FjallSnapshot` isn't `Debug`, so match the result rather than `unwrap_err`.
        match FjallSnapshot::open(&path, FjallConfig::default()) {
            Err(SnapshotError::InvalidFormat(_)) => {}
            Err(other) => panic!("expected InvalidFormat, got {other:?}"),
            Ok(_) => panic!("expected open to reject the oversized cursor"),
        }
    }
}
