//! Table and block caches for the read path (RocksDB-class feature shape).
//!
//! - [`TableCache`]: reuses decoded [`SstTable`] handles by path so a second
//!   open of the same SST does not re-read the full file from the [`Env`].
//! - [`BlockCache`]: caches decompressed SST data blocks by `(path, block_idx)`.
//! - [`PointCache`]: latest-snapshot point-get answers (invalidated on write).

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

fn path_id(path: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    h.finish()
}

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

#[derive(Debug, Clone)]
struct CachedSlot {
    block: CachedBlock,
    /// Recency tick; higher is hotter. Evict min-tick on overflow.
    tick: u64,
}

#[derive(Debug, Default)]
struct BlockCacheInner {
    map: HashMap<(u64, usize), CachedSlot>,
    /// Monotonic recency. Hit is O(1) — a `VecDeque` walk on every hit was
    /// O(capacity) and ate `deps_scan` after the cache grew to 8192 (RFC-0035).
    tick: u64,
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
                tick: 0,
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
        let key = (path_id(path), block_idx);
        {
            let mut g = self.inner.lock();
            if let Some(block) = g.map.get(&key).map(|s| Arc::clone(&s.block)) {
                g.hits = g.hits.saturating_add(1);
                let tick = g.tick.saturating_add(1);
                g.tick = tick;
                if let Some(slot) = g.map.get_mut(&key) {
                    slot.tick = tick;
                }
                return block;
            }
        }
        let block = Arc::new(load());
        let mut g = self.inner.lock();
        if let Some(hit) = g.map.get(&key).map(|s| Arc::clone(&s.block)) {
            g.hits = g.hits.saturating_add(1);
            let tick = g.tick.saturating_add(1);
            g.tick = tick;
            if let Some(slot) = g.map.get_mut(&key) {
                slot.tick = tick;
            }
            return hit;
        }
        g.misses = g.misses.saturating_add(1);
        if g.capacity > 0 {
            while g.map.len() >= g.capacity {
                let victim = g.map.iter().min_by_key(|(_, s)| s.tick).map(|(k, _)| *k);
                match victim {
                    Some(old) => {
                        g.map.remove(&old);
                    }
                    None => break,
                }
            }
        }
        let tick = g.tick.saturating_add(1);
        g.tick = tick;
        g.map.insert(
            key,
            CachedSlot {
                block: Arc::clone(&block),
                tick,
            },
        );
        block
    }

    /// Clear all blocks.
    pub fn clear(&self) {
        let mut g = self.inner.lock();
        g.map.clear();
        g.tick = 0;
    }
}

/// Latest-snapshot answers (point / last-prefix / count). Cleared on write.
///
/// Hit is O(1). Capacity 0 = disabled.
#[derive(Debug, Default)]
pub struct AnswerCache<V> {
    inner: Mutex<AnswerCacheInner<V>>,
}

/// Latest-snapshot point get (`None` = cached absence).
pub type PointCache = AnswerCache<Option<Bytes>>;

/// Fixed-key fast hash (fxhash-class). Cache keys are compared exactly on
/// every hit, so a weak (non-DoS-resistant) hasher only trades speed for
/// collisions inside the map — never correctness.
#[derive(Debug, Default)]
pub(crate) struct FxHasher {
    hash: u64,
}

const FX_SEED: u64 = 0x517c_c1b7_2722_0a95;

impl FxHasher {
    fn combine(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(FX_SEED);
    }
}

impl std::hash::Hasher for FxHasher {
    fn write(&mut self, mut bytes: &[u8]) {
        while bytes.len() >= 8 {
            let (chunk, rest) = bytes.split_at(8);
            self.combine(u64::from_le_bytes(chunk.try_into().unwrap()));
            bytes = rest;
        }
        if bytes.len() >= 4 {
            let (chunk, rest) = bytes.split_at(4);
            self.combine(u32::from_le_bytes(chunk.try_into().unwrap()) as u64);
            bytes = rest;
        }
        for &b in bytes {
            self.combine(u64::from(b));
        }
    }
    fn write_u8(&mut self, i: u8) {
        self.combine(u64::from(i));
    }
    fn write_u32(&mut self, i: u32) {
        self.combine(u64::from(i));
    }
    fn write_u64(&mut self, i: u64) {
        self.combine(i);
    }
    fn write_usize(&mut self, i: usize) {
        self.combine(i as u64);
    }
    fn finish(&self) -> u64 {
        self.hash
    }
}

type FxBuild = std::hash::BuildHasherDefault<FxHasher>;

#[derive(Debug, Default)]
struct AnswerCacheInner<V> {
    map: std::collections::HashMap<Bytes, V, FxBuild>,
    /// Insertion order for O(1) FIFO eviction (no full-map LRU scan per
    /// insert — miss-heavy workloads insert on every op).
    order: std::collections::VecDeque<Bytes>,
    capacity: usize,
}

impl<V: Clone> AnswerCache<V> {
    /// Create with max cached keys (`0` = disabled).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(AnswerCacheInner {
                map: std::collections::HashMap::default(),
                order: std::collections::VecDeque::new(),
                capacity,
            }),
        }
    }

    /// `None` = miss.
    #[must_use]
    pub fn get(&self, key: &[u8]) -> Option<V> {
        let g = self.inner.lock();
        if g.capacity == 0 {
            return None;
        }
        g.map.get(key).cloned()
    }

    /// Store a latest-snapshot answer.
    pub fn insert(&self, key: &[u8], value: V) {
        let mut g = self.inner.lock();
        if g.capacity == 0 {
            return;
        }
        if let Some(v) = g.map.get_mut(key) {
            *v = value;
            return;
        }
        if g.map.len() >= g.capacity {
            // FIFO: drop the oldest inserted key (cloned below while `g` is
            // still borrowed, then remove from the map).
            if let Some(old) = g.order.pop_front() {
                g.map.remove(&old);
            }
        }
        let owned = Bytes::copy_from_slice(key);
        g.order.push_back(owned.clone());
        g.map.insert(owned, value);
    }

    /// Drop every entry (call after a write that can change latest visibility).
    pub fn clear(&self) {
        let mut g = self.inner.lock();
        g.map.clear();
        g.order.clear();
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
        assert_eq!(
            a.get(b"k", 10),
            crate::memtable::Lookup::Found(Bytes::from_static(b"v"))
        );
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

    #[test]
    fn block_cache_lru_evicts_coldest_not_arbitrary() {
        let cache = BlockCache::new(2);
        let path = Path::new("/tmp/lru.sst");
        cache.get_or_insert_with(path, 0, || Vec::new());
        cache.get_or_insert_with(path, 1, || Vec::new());
        // Touch 0 so 1 is coldest.
        cache.get_or_insert_with(path, 0, || panic!("0 must stay"));
        cache.get_or_insert_with(path, 2, || Vec::new());
        cache.get_or_insert_with(path, 0, || panic!("0 was hot and must remain"));
        let mut loaded = false;
        cache.get_or_insert_with(path, 1, || {
            loaded = true;
            Vec::new()
        });
        assert!(loaded, "1 was LRU and must be evicted");
    }

    #[test]
    fn point_cache_hit_and_clear() {
        let c = PointCache::new(4);
        assert!(c.get(b"k").is_none());
        c.insert(b"k", Some(Bytes::from_static(b"v")));
        assert_eq!(c.get(b"k").unwrap().as_deref(), Some(&b"v"[..]));
        c.insert(b"missing", None);
        assert_eq!(c.get(b"missing"), Some(None));
        c.clear();
        assert!(c.get(b"k").is_none());
    }

    #[test]
    fn block_cache_hit_is_not_a_linear_walk() {
        // Capacity large enough that a VecDeque touch-on-hit would be O(n).
        let cache = BlockCache::new(64);
        let path = Path::new("/tmp/lru-hot.sst");
        for i in 0..64 {
            cache.get_or_insert_with(path, i, || Vec::new());
        }
        for _ in 0..8 {
            cache.get_or_insert_with(path, 0, || panic!("0 is hot"));
        }
        cache.get_or_insert_with(path, 64, || Vec::new());
        cache.get_or_insert_with(path, 0, || panic!("0 must survive insert of 64"));
    }
}
