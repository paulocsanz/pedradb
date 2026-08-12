//! Table and block caches for the read path (RocksDB-class feature shape).
//!
//! - [`TableCache`]: reuses decoded [`SstTable`] handles by path so a second
//!   open of the same SST does not re-read the full file from the [`Env`].
//! - [`BlockCache`]: caches decompressed SST data blocks by `(path, block_idx)`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use parking_lot::Mutex;

use crate::env::Env;
use crate::error::Result;
use crate::key::InternalKey;
use crate::sst::SstTable;

/// Shared decoded block payload.
pub type CachedBlock = Arc<Vec<(InternalKey, Bytes)>>;

/// LRU-ish table cache (capacity-capped; eviction drops arbitrary entries when full).
#[derive(Debug, Default)]
pub struct TableCache {
    inner: Mutex<TableCacheInner>,
}

#[derive(Debug, Default)]
struct TableCacheInner {
    map: HashMap<PathBuf, Arc<SstTable>>,
    capacity: usize,
    hits: u64,
    misses: u64,
}

impl TableCache {
    /// Create a cache that keeps at most `capacity` tables (0 = unlimited).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(TableCacheInner {
                map: HashMap::new(),
                capacity,
                hits: 0,
                misses: 0,
            }),
        }
    }

    /// Cache hit count (second+ open of the same path).
    #[must_use]
    pub fn hits(&self) -> u64 {
        self.inner.lock().hits
    }

    /// Cache miss count (Env open performed).
    #[must_use]
    pub fn misses(&self) -> u64 {
        self.inner.lock().misses
    }

    /// Reset counters (tests).
    pub fn reset_stats(&self) {
        let mut g = self.inner.lock();
        g.hits = 0;
        g.misses = 0;
    }

    /// Number of tables currently cached.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().map.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Insert or replace a table (e.g. after flush).
    pub fn insert(&self, table: Arc<SstTable>) {
        let path = table.path().to_path_buf();
        let mut g = self.inner.lock();
        if g.capacity > 0 && g.map.len() >= g.capacity && !g.map.contains_key(&path) {
            // Drop one arbitrary entry to stay under capacity.
            if let Some(k) = g.map.keys().next().cloned() {
                g.map.remove(&k);
            }
        }
        g.map.insert(path, table);
    }

    /// Lookup without opening.
    #[must_use]
    pub fn get(&self, path: &Path) -> Option<Arc<SstTable>> {
        let mut g = self.inner.lock();
        if let Some(t) = g.map.get(path).cloned() {
            g.hits = g.hits.saturating_add(1);
            Some(t)
        } else {
            None
        }
    }

    /// Get cached table or open via `env` and insert.
    ///
    /// # Errors
    /// I/O or corrupt SST from [`SstTable::open_on`].
    pub fn get_or_open<E: Env>(&self, env: &E, path: impl AsRef<Path>) -> Result<Arc<SstTable>> {
        let path = path.as_ref();
        if let Some(t) = self.get(path) {
            return Ok(t);
        }
        let table = Arc::new(SstTable::open_on(env, path)?);
        {
            let mut g = self.inner.lock();
            g.misses = g.misses.saturating_add(1);
            if g.capacity > 0 && g.map.len() >= g.capacity {
                if let Some(k) = g.map.keys().next().cloned() {
                    g.map.remove(&k);
                }
            }
            g.map.insert(path.to_path_buf(), Arc::clone(&table));
        }
        Ok(table)
    }

    /// Drop all cached tables.
    pub fn clear(&self) {
        self.inner.lock().map.clear();
    }
}

/// Block cache for decompressed SST blocks (keyed by absolute path + block index).
#[derive(Debug, Default)]
pub struct BlockCache {
    inner: Mutex<BlockCacheInner>,
}

#[derive(Debug, Default)]
struct BlockCacheInner {
    map: HashMap<(PathBuf, usize), CachedBlock>,
    capacity: usize,
    hits: u64,
    misses: u64,
}

impl BlockCache {
    /// Create with max cached blocks (`0` = unlimited).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(BlockCacheInner {
                map: HashMap::new(),
                capacity,
                hits: 0,
                misses: 0,
            }),
        }
    }

    /// Hit count.
    #[must_use]
    pub fn hits(&self) -> u64 {
        self.inner.lock().hits
    }

    /// Miss count.
    #[must_use]
    pub fn misses(&self) -> u64 {
        self.inner.lock().misses
    }

    /// Reset hit/miss counters.
    pub fn reset_stats(&self) {
        let mut g = self.inner.lock();
        g.hits = 0;
        g.misses = 0;
    }

    /// Lookup or insert via `load` on miss.
    pub fn get_or_insert_with<F>(&self, path: &Path, block_idx: usize, load: F) -> CachedBlock
    where
        F: FnOnce() -> Vec<(InternalKey, Bytes)>,
    {
        let key = (path.to_path_buf(), block_idx);
        {
            let mut g = self.inner.lock();
            if let Some(b) = g.map.get(&key).cloned() {
                g.hits = g.hits.saturating_add(1);
                return b;
            }
        }
        let block = Arc::new(load());
        let mut g = self.inner.lock();
        g.misses = g.misses.saturating_add(1);
        if g.capacity > 0 && g.map.len() >= g.capacity && !g.map.contains_key(&key) {
            if let Some(k) = g.map.keys().next().cloned() {
                g.map.remove(&k);
            }
        }
        g.map.insert(key, Arc::clone(&block));
        block
    }

    /// Clear all blocks.
    pub fn clear(&self) {
        self.inner.lock().map.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::StdEnv;
    use crate::key::ValueType;
    use crate::memtable::MemTable;
    use crate::sst::write_sst;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path() -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("pedradb-cache-{n}.sst"))
    }

    #[test]
    fn table_cache_second_open_is_hit() {
        let mut mem = MemTable::new();
        mem.put(b"k".as_slice(), 1, b"v".as_slice());
        let path = temp_path();
        let _ = write_sst(&path, &mem).unwrap();

        let cache = TableCache::new(8);
        let env = StdEnv;
        let a = cache.get_or_open(&env, &path).unwrap();
        assert_eq!(cache.misses(), 1);
        assert_eq!(cache.hits(), 0);
        let b = cache.get_or_open(&env, &path).unwrap();
        assert_eq!(cache.misses(), 1);
        assert_eq!(cache.hits(), 1);
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(a.get(b"k", 10), crate::memtable::Lookup::Found(Bytes::from_static(b"v")));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn block_cache_hit_on_second_lookup() {
        let cache = BlockCache::new(4);
        let path = Path::new("/tmp/fake.sst");
        let b1 = cache.get_or_insert_with(path, 0, || {
            vec![(
                InternalKey::new(Bytes::from_static(b"a"), 1, ValueType::Value),
                Bytes::from_static(b"1"),
            )]
        });
        assert_eq!(cache.misses(), 1);
        let b2 = cache.get_or_insert_with(path, 0, || panic!("should not load"));
        assert_eq!(cache.hits(), 1);
        assert!(Arc::ptr_eq(&b1, &b2));
    }
}
