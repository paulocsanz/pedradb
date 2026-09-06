//! On-disk [`SnapshotStore`] backed by PedraDB via its `rocksdb-compat`
//! surface — a pure-Rust LSM with a rust-rocksdb-shaped API, for comparing
//! Pedra against the fjall and RocksDB backends on the same route-fold
//! workload.
//!
//! Ported from beyondoss/slipstream `src/snapshot_pedradb.rs` (MIT); the
//! artifact export/import surface is not part of this harness.
//!
//! ## How it honors the [`SnapshotStore`] invariants
//!
//! - **Atomic data + cursor.** Each [`apply`](SnapshotStore::apply) is a single
//!   Pedra `WriteBatch`: every put/delete *and* the resume cursor land in one
//!   atomic apply — Pedra's `apply_batch` is all-or-nothing across column
//!   families (CF keys share one WAL).
//! - **Self-sufficient under NO_SYNC.** Pedra's drop-in default matches
//!   Rocks-shaped `WriteOptions.sync = false`: commits reach the WAL without
//!   an `fdatasync` barrier. Set `sync = true` to fsync every commit (Pedra's
//!   G1 durability — `fdatasync` before `Ok`). Either way, data and cursor
//!   share one batch, so a crash-lost tail never desynchronizes them; on
//!   reopen the fold resumes from the recovered cursor.
//! - **Queryable.** [`get`](SnapshotStore::get) and
//!   [`range`](SnapshotStore::range) read from Pedra's block-cached storage.
//!
//! ## Tuning
//!
//! Pedra's compat layer accepts many Rocks knobs as no-ops (filters, block
//! size, parallelism, WAL byte caps, …) — Pedra policy is not Rocks. The
//! knobs that *do* land are the ones that matter for this fold:
//!
//! - per-CF `write_buffer_size` (memtable flush threshold)
//! - `Options::set_block_cache` / `block_cache_bytes` (SST block-cache budget)
//! - per-write `WriteOptions::sync` (durability barrier)
//!
//! [`settle`](PedraDbSnapshot::settle) flushes the memtable and runs Pedra's
//! full compact: the rust-rocksdb `wait_for_compact` shim is a no-op here, so
//! settle must drive compaction itself.

use std::path::Path;
use std::sync::Arc;

use rocksdb_compat::{
    ColumnFamily, ColumnFamilyDescriptor, Direction, ErrorKind, IteratorMode, Options, ReadOptions,
    WriteBatch, WriteOptions, DB,
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

/// Data CF memtable size — matches the RocksDB / fjall backends' 256 MiB.
const DATA_WRITE_BUFFER_BYTES: usize = 256 << 20;

/// Meta CF memtable. It holds exactly one key (the cursor); 8 MiB is generous.
const META_WRITE_BUFFER_BYTES: usize = 8 << 20;

/// Durability and read-cache configuration for [`PedraDbSnapshot`].
///
/// Defaults to NO_SYNC (`sync: false`) — same cache philosophy as the RocksDB
/// / fjall backends.
#[derive(Debug, Clone, Copy)]
pub struct PedraDbConfig {
    /// `fdatasync` the WAL on every [`apply`](SnapshotStore::apply) commit when
    /// `true`. When `false` (the default), commits are written to the WAL but
    /// not fsync'd (NO_SYNC): faster, survives a process crash via WAL replay,
    /// and a tail lost to power loss is rebuilt by resuming the fold from the
    /// recovered cursor — the snapshot is a cache.
    pub sync: bool,

    /// SST block-cache budget in bytes. Pedra's compat default is an
    /// entry-count cache; setting this installs a byte-budgeted block cache
    /// via `Options::set_block_cache`. `0` leaves Pedra's default.
    pub cache_size_bytes: u64,
}

impl Default for PedraDbConfig {
    fn default() -> Self {
        Self {
            sync: false,
            // 1 GiB: same default as the RocksDB / fjall backends, so the
            // comparative bench compares engines under equal memory budget.
            cache_size_bytes: 1024 * 1024 * 1024,
        }
    }
}

/// On-disk durable fold backed by PedraDB. See the [module docs](self).
pub struct PedraDbSnapshot {
    // Arc so `reader()` handles share the instance: Pedra serves reads from
    // `&DB` concurrently with writes, and `DB` is `Send + Sync`.
    db: Arc<DB>,
    config: PedraDbConfig,
    cursor: WatchCursor,
}

impl PedraDbSnapshot {
    /// Open or resume the store at `path` with explicit durability config.
    ///
    /// `path` is a directory (Pedra database), created if absent. Returns the
    /// persisted resume cursor — [`WatchCursor::none`] when fresh — and the store.
    pub fn open(path: &Path, config: PedraDbConfig) -> Result<(WatchCursor, Self), SnapshotError> {
        std::fs::create_dir_all(path)?;

        let mut db_opts = Options::default();
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);
        // Pedra's Options.sync is the DB-wide default write barrier; keep it
        // false so per-apply WriteOptions.sync is the sole control (matches
        // the RocksDB backend's always-on-WAL + set_sync pattern).
        db_opts.set_sync(false);
        if config.cache_size_bytes > 0 {
            let capacity = usize::try_from(config.cache_size_bytes).map_err(|_| {
                SnapshotError::InvalidFormat(format!(
                    "cache_size_bytes {} exceeds usize on this platform",
                    config.cache_size_bytes
                ))
            })?;
            db_opts.set_block_cache(&rocksdb_compat::Cache::new_lru_cache(capacity));
        }

        let mut data_opts = Options::default();
        data_opts.set_write_buffer_size(DATA_WRITE_BUFFER_BYTES);

        let mut meta_opts = Options::default();
        meta_opts.set_write_buffer_size(META_WRITE_BUFFER_BYTES);

        let db = DB::open_cf_descriptors(
            &db_opts,
            path,
            [
                ColumnFamilyDescriptor::new(DATA_CF, data_opts),
                ColumnFamilyDescriptor::new(META_CF, meta_opts),
            ],
        )
        .map_err(map_pedradb)?;

        let cursor = match db
            .get_cf(&cf(&db, META_CF)?, CURSOR_KEY)
            .map_err(map_pedradb)?
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
    /// Pedra serves readers concurrently with the writer, so a consumer can
    /// clone this out and then `get`/`range` from a separate serving task.
    pub fn reader(&self) -> PedraDbReader {
        PedraDbReader {
            db: Arc::clone(&self.db),
        }
    }

    /// Flush the memtable and run a full Pedra compact.
    ///
    /// Pedra's rust-rocksdb `wait_for_compact` shim is a no-op, so settle
    /// drives flush + compact itself. Call after bulk hydration and before
    /// latency-sensitive serving; steady-state folding does not need it.
    pub fn settle(&self) -> Result<(), SnapshotError> {
        self.db.flush().map_err(map_pedradb)?;
        self.db.compact().map_err(map_pedradb)
    }
}

/// A concurrent read handle over a [`PedraDbSnapshot`]'s data column family,
/// cloned via [`PedraDbSnapshot::reader`].
#[derive(Clone)]
pub struct PedraDbReader {
    db: Arc<DB>,
}

impl PedraDbReader {
    /// Live entry for `key`, or `None` if absent/deleted.
    pub fn get(&self, key: &str) -> Result<Option<KvEntry>, SnapshotError> {
        get_entry(&self.db, key)
    }

    /// Batched point lookups. Pedra's `multi_get_cf` is currently a sequential
    /// loop of `get_cf` (no SST coalescing yet); kept for API parity with
    /// `RocksDbReader::multi_get` so the comparative bench can measure both.
    pub fn multi_get<'k>(
        &self,
        keys: impl IntoIterator<Item = &'k str>,
    ) -> Result<Vec<Option<KvEntry>>, SnapshotError> {
        let data = cf(&self.db, DATA_CF)?;
        let keys: Vec<&str> = keys.into_iter().collect();
        let results = self
            .db
            .multi_get_cf(keys.iter().map(|k| (&data, k.as_bytes())));
        keys.iter()
            .zip(results)
            .map(|(key, res)| match res.map_err(map_pedradb)? {
                Some(raw) => Ok(Some(decode_entry(key, &raw)?)),
                None => Ok(None),
            })
            .collect()
    }

    /// Stream every live entry whose key starts with `prefix`, ascending.
    pub fn for_each_in_range(
        &self,
        prefix: &str,
        f: impl FnMut(KvEntry) -> Result<(), SnapshotError>,
    ) -> Result<(), SnapshotError> {
        scan_prefix(&self.db, prefix, f)
    }

    /// Buffered counterpart to [`for_each_in_range`](Self::for_each_in_range).
    pub fn range(&self, prefix: &str) -> Result<Vec<KvEntry>, SnapshotError> {
        let mut out = Vec::new();
        self.for_each_in_range(prefix, |e| {
            out.push(e);
            Ok(())
        })?;
        Ok(out)
    }
}

impl SnapshotStore for PedraDbSnapshot {
    fn load(path: &Path) -> Result<(WatchCursor, Self), SnapshotError> {
        Self::open(path, PedraDbConfig::default())
    }

    fn apply(&mut self, batch: &[KvUpdate], cursor: &WatchCursor) -> Result<(), SnapshotError> {
        let data = cf(&self.db, DATA_CF)?;
        let meta = cf(&self.db, META_CF)?;
        let mut wb = WriteBatch::default();
        let mut scratch = Vec::new();
        for update in batch {
            match update {
                KvUpdate::Put(entry) => {
                    encode_value_into(&mut scratch, &entry.value, &entry.version)?;
                    wb.put_cf(&data, entry.key.as_bytes(), scratch.as_slice());
                }
                KvUpdate::Delete { key, .. } | KvUpdate::Purge { key, .. } => {
                    wb.delete_cf(&data, key.as_bytes());
                }
            }
        }
        wb.put_cf(&meta, CURSOR_KEY, cursor.version().as_bytes());

        let mut wo = WriteOptions::default();
        wo.set_sync(self.config.sync);
        self.db.write_opt(&wb, &wo).map_err(map_pedradb)?;

        self.cursor = cursor.clone();
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<KvEntry>, SnapshotError> {
        get_entry(&self.db, key)
    }

    fn range(&self, prefix: &str) -> Result<Vec<KvEntry>, SnapshotError> {
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

impl Drop for PedraDbSnapshot {
    fn drop(&mut self) {
        // Pedra's NO_SYNC path (`WriteOptions.sync = false`) does not make the
        // WAL recoverable on reopen until `flush_wal(true)` — unlike RocksDB,
        // where a clean drop leaves the WAL on the OS enough for process-crash
        // replay. Flush here so a clean process exit matches the Rocks / fjall
        // "survives restart" contract; power-loss still loses the un-synced
        // tail by design.
        let _ = self.db.flush_wal(true);
    }
}

/// Resolve a column family handle. Pedra returns an owned name-keyed handle.
fn cf(db: &DB, name: &str) -> Result<ColumnFamily, SnapshotError> {
    db.cf_handle(name)
        .ok_or_else(|| SnapshotError::Backend(format!("missing column family: {name}")))
}

fn get_entry(db: &DB, key: &str) -> Result<Option<KvEntry>, SnapshotError> {
    match db
        .get_cf(&cf(db, DATA_CF)?, key.as_bytes())
        .map_err(map_pedradb)?
    {
        Some(raw) => Ok(Some(decode_entry(key, &raw)?)),
        None => Ok(None),
    }
}

/// Streaming prefix scan. Pedra's `ReadOptions` iterate bounds are accepted
/// but not yet applied by `iterator_cf_opt`, so we start at the prefix and
/// stop when keys leave it — the classic portable Rocks prefix walk.
fn scan_prefix(
    db: &DB,
    prefix: &str,
    mut f: impl FnMut(KvEntry) -> Result<(), SnapshotError>,
) -> Result<(), SnapshotError> {
    let data = cf(db, DATA_CF)?;
    let mode = if prefix.is_empty() {
        IteratorMode::Start
    } else {
        IteratorMode::From(prefix.as_bytes(), Direction::Forward)
    };
    // Bounds are recorded for observability / future Pedra support; the
    // walk below still stops on the prefix check.
    let mut read_opts = ReadOptions::default();
    if !prefix.is_empty() {
        read_opts.set_iterate_lower_bound(prefix.as_bytes().to_vec());
        if let Some(upper) = prefix_upper_bound(prefix.as_bytes()) {
            read_opts.set_iterate_upper_bound(upper);
        }
    }
    let iter = db
        .iterator_cf_opt(&data, mode, read_opts)
        .map_err(map_pedradb)?;
    for item in iter {
        let (raw_key, raw_val) = item.map_err(map_pedradb)?;
        if !prefix.is_empty() && !raw_key.starts_with(prefix.as_bytes()) {
            break;
        }
        let key = std::str::from_utf8(&raw_key).map_err(|e| {
            SnapshotError::InvalidFormat(format!("non-UTF-8 key in pedradb store: {e}"))
        })?;
        f(decode_entry(key, &raw_val)?)?;
    }
    Ok(())
}

/// Exclusive upper bound for a byte-prefix range: the shortest successor of
/// `prefix`, or `None` when `prefix` is all `0xFF` (unbounded above).
fn prefix_upper_bound(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut end = prefix.to_vec();
    while let Some(b) = end.last_mut() {
        if *b != 0xff {
            *b += 1;
            return Some(end);
        }
        end.pop();
    }
    None
}

fn map_pedradb(e: rocksdb_compat::Error) -> SnapshotError {
    match e.kind() {
        ErrorKind::Io => SnapshotError::Io(std::io::Error::other(e.to_string())),
        // Deliberately NOT mapping Corruption → SnapshotError::Corrupted:
        // that variant's Display is the append log's "CRC mismatch" text.
        _ => SnapshotError::Backend(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn open_rejects_corrupted_cursor() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("store");

        {
            let (_c, store) =
                PedraDbSnapshot::open(&path, PedraDbConfig::default()).expect("initial open");
            store
                .db
                .put_cf(
                    &cf(&store.db, META_CF).expect("meta cf"),
                    CURSOR_KEY,
                    [0u8; 11],
                )
                .expect("insert oversized cursor");
        }

        match PedraDbSnapshot::open(&path, PedraDbConfig::default()) {
            Err(SnapshotError::InvalidFormat(_)) => {}
            Err(other) => panic!("expected InvalidFormat, got {other:?}"),
            Ok(_) => panic!("expected open to reject the oversized cursor"),
        }
    }

    #[test]
    fn prefix_upper_bound_increments_last_non_ff() {
        assert_eq!(prefix_upper_bound(b"abc"), Some(b"abd".to_vec()));
        assert_eq!(prefix_upper_bound(b"ab\xff"), Some(b"ac".to_vec()));
        assert_eq!(prefix_upper_bound(b"\xff\xff"), None);
    }
}
