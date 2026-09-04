//! Table and block caches for the read path (RocksDB-class feature shape).
//!
//! - [`TableCache`]: reuses decoded [`SstTable`] handles by path so a second
//!   open of the same SST does not re-read the full file from the [`Env`].
//! - [`BlockCache`]: caches decompressed SST data blocks by `(path, block_idx)`.
//! - [`PointCache`]: latest-snapshot point-get answers (invalidated on write).

use std::collections::HashMap;
use std::collections::VecDeque;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use parking_lot::Mutex;

use crate::env::Env;
use crate::error::Result;
use crate::key::InternalKey;
use crate::sst::SstTable;

/// Shared decoded block payload.
pub type CachedBlock = Arc<Vec<(InternalKey, Bytes)>>;

/// Stable 64-bit cache id for an SST path.
///
/// Hashed once per stream and reused for every block fetch — re-hashing the
/// path string per fetch was ~4% of a prefix scan at 6M entries.
#[must_use]
pub(crate) fn path_id(path: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    h.finish()
}

/// LRU-ish table cache (capacity-capped; eviction drops arbitrary entries when full).
#[derive(Debug, Default)]
pub struct TableCache {
    inner: Mutex<TableCacheInner>,
    /// Payload pool kit (RFC-0042 v18): attached to every table this cache
    /// opens so recovery-time registration bounds residency during reopen.
    kit: Mutex<Option<PayloadKit>>,
}

/// Attached file source + pool handed to tables (RFC-0042 v18).
#[derive(Clone)]
pub(crate) struct PayloadKit {
    pub source: Arc<dyn crate::env::SstFileSource>,
    pub pool: Arc<SstPayloadPool>,
}

impl std::fmt::Debug for PayloadKit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PayloadKit")
            .field("source", &self.source)
            .field("pool", &self.pool)
            .finish()
    }
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
            kit: Mutex::new(None),
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
        if let Some(kit) = self.payload_kit() {
            table.attach_payload_kit(&kit.source, &kit.pool);
        }
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

    /// Install the payload pool kit: every table this cache opens is attached
    /// and registered (RFC-0042 v18). Called by a bounded `Db` open before
    /// recovery.
    pub(crate) fn set_payload_kit(&self, kit: PayloadKit) {
        *self.kit.lock() = Some(kit);
    }

    /// Clone of the installed kit, if any.
    pub(crate) fn payload_kit(&self) -> Option<PayloadKit> {
        self.kit.lock().clone()
    }

    /// Drop all cached tables.
    pub fn clear(&self) {
        self.inner.lock().map.clear();
    }
}

/// Shared, evictable SST payload slot (RFC-0042 v18).
///
/// Non-empty = file body resident; empty = evicted, blocks served from file.
/// `verified` carries per-block CRC marks for exactly the resident image:
/// the point seek re-verifies a block's CRC32C only on its first probe of a
/// residency (RocksDB's checksum-on-read-into-cache contract); any payload
/// write installs fresh, empty marks — fail-closed under replacement.
pub type PayloadSlot = parking_lot::RwLock<ResidentBody>;

/// Resident payload image plus its CRC-verified block marks.
#[derive(Debug, Clone, Default)]
pub struct ResidentBody {
    /// Resident image; empty = evicted.
    pub img: Arc<[u8]>,
    /// One bit per data block (index order), trusted only for `img`.
    verified: Option<Box<[u64]>>,
}

impl ResidentBody {
    /// Resident image, nothing yet verified.
    #[must_use]
    pub fn from_image(img: Arc<[u8]>) -> Self {
        Self {
            img,
            verified: None,
        }
    }

    /// Empty (evicted) body.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Whether block `bi`'s CRC is already verified for this image.
    #[must_use]
    pub fn is_verified(&self, bi: usize) -> bool {
        self.verified
            .as_ref()
            .is_some_and(|bits| bi / 64 < bits.len() && bits[bi / 64] & (1 << (bi % 64)) != 0)
    }

    /// Mark block `bi` verified; `block_count` sizes the bit set on first mark.
    pub fn mark_verified(&mut self, bi: usize, block_count: usize) {
        let words = block_count.div_ceil(64);
        let bits = self
            .verified
            .get_or_insert_with(|| vec![0u64; words].into_boxed_slice());
        if bi / 64 < bits.len() {
            bits[bi / 64] |= 1 << (bi % 64);
        }
    }
}

/// Bounds the total resident bytes of SST file bodies (RFC-0042 v18).
///
/// Every v2+ SST retains its CRC-stripped file body for lazy block decode, so
/// an unbounded LSM keeps its whole dataset in RAM (25M slipstream: 5.9 GB of
/// payloads on a 3.9 GB guest — the v16 OOM; RocksDB reads blocks on demand).
/// The pool keeps the most recently registered payloads resident up to
/// `budget` and clears the rest. Evicted blocks are re-read through the
/// table's [`SstFileSource`](crate::env::SstFileSource) with CRC verification
/// (per-block on v5+, whole-file on ≤v4), so eviction never weakens the
/// fail-closed integrity gate.
///
/// Eviction only runs once `arm`ed — i.e. when every registered table carries
/// a file source — so a legacy open (no source) keeps payloads resident.
/// Decode never touches the pool: registration order is the eviction order
/// and there is no hot-path locking.
#[derive(Debug)]
pub struct SstPayloadPool {
    inner: Mutex<PoolInner>,
    /// Copy of the budget so `can_admit` can reject a full pool without
    /// taking the mutex (25M lookup_100: every evicted get retried
    /// `can_admit` under the lock after the 256 MiB cap filled).
    budget: Option<u64>,
    /// Last published `inner.total`. Relaxed: a stale-high value skips a
    /// promote (safe); a stale-low value falls through to the locked check.
    total: AtomicU64,
}

#[derive(Debug, Default)]
struct PoolInner {
    budget: Option<u64>,
    /// Evict only when registered tables can reload (source attached).
    armed: bool,
    tick: u64,
    total: u64,
    map: HashMap<PathBuf, PoolEntry>,
}

#[derive(Debug)]
struct PoolEntry {
    slot: std::sync::Weak<PayloadSlot>,
    bytes: u64,
    last: u64,
}

impl SstPayloadPool {
    /// Create a pool; `None` = unbounded (never evicts, no bookkeeping).
    #[must_use]
    pub fn with_budget(budget: Option<u64>) -> Self {
        Self {
            inner: Mutex::new(PoolInner {
                budget,
                ..PoolInner::default()
            }),
            budget,
            total: AtomicU64::new(0),
        }
    }

    /// Allow eviction. Caller guarantees every registered (and future)
    /// table has a file source attached.
    pub(crate) fn arm(&self) {
        let mut g = self.inner.lock();
        g.armed = true;
        Self::evict_to_budget(&mut g);
        self.total.store(g.total, Ordering::Relaxed);
    }

    /// Record (or refresh) a resident payload and enforce the budget.
    /// No-op when unbounded. `bytes == 0` unregisters (empty / released
    /// slot) so a streaming bulk SST does not ghost-consume the budget.
    pub(crate) fn register(&self, path: &Path, slot: std::sync::Weak<PayloadSlot>, bytes: u64) {
        let mut g = self.inner.lock();
        if g.budget.is_none() {
            return;
        }
        if let Some(old) = g.map.remove(path) {
            g.total = g.total.saturating_sub(old.bytes);
        }
        if bytes == 0 {
            self.total.store(g.total, Ordering::Relaxed);
            return;
        }
        g.tick = g.tick.wrapping_add(1);
        let last = g.tick;
        g.total = g.total.saturating_add(bytes);
        g.map
            .insert(path.to_path_buf(), PoolEntry { slot, bytes, last });
        Self::evict_to_budget(&mut g);
        self.total.store(g.total, Ordering::Relaxed);
    }

    /// Whether a currently-empty file of `bytes` can become resident without
    /// evicting another table. Used by bulk get_hit: hydrate leaves payloads
    /// empty (100M OOM otherwise); 1M/10M point gets promote into the leftover
    /// budget so they are not a `pread`+CRC per probe.
    #[must_use]
    pub(crate) fn can_admit(&self, path: &Path, bytes: u64) -> bool {
        let Some(budget) = self.budget else {
            return false;
        };
        if bytes == 0 || bytes > budget {
            return false;
        }
        if self.total.load(Ordering::Relaxed).saturating_add(bytes) > budget {
            return false;
        }
        let g = self.inner.lock();
        if let Some(e) = g.map.get(path) {
            let without = g.total.saturating_sub(e.bytes);
            return without.saturating_add(bytes) <= budget;
        }
        g.total.saturating_add(bytes) <= budget
    }

    /// Resident bytes currently accounted (≤ budget once armed).
    #[must_use]
    pub fn resident_bytes(&self) -> u64 {
        self.inner.lock().total
    }

    /// Tracked table count (observability).
    #[must_use]
    pub fn tracked_tables(&self) -> usize {
        self.inner.lock().map.len()
    }

    /// Drop the oldest registrations until within budget. Clearing a slot is
    /// safe under concurrent readers: they hold `Arc` clones of the payload,
    /// which frees when the last reader finishes.
    fn evict_to_budget(g: &mut PoolInner) {
        while g.armed && g.budget.is_some_and(|budget| g.total > budget) {
            let Some(victim) = g
                .map
                .iter()
                .min_by_key(|(_, e)| e.last)
                .map(|(p, _)| p.clone())
            else {
                break;
            };
            let Some(entry) = g.map.remove(&victim) else {
                continue;
            };
            // A dropped table leaves a dead Weak: entry removed, bytes returned.
            if let Some(slot) = entry.slot.upgrade() {
                *slot.write() = ResidentBody::empty();
            }
            g.total = g.total.saturating_sub(entry.bytes);
        }
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
    /// Epoch of the live `order` entry for this slot. Bumped on a hit's
    /// re-push; older queue entries for the same key become ghosts.
    ins_epoch: u64,
    /// Key + value + 16 B trailer estimate (RFC-0153).
    bytes: u64,
}

#[derive(Debug, Default)]
struct BlockCacheInner {
    map: HashMap<(u64, usize), CachedSlot>,
    /// Recency queue, least-recent-first: `(key, epoch)` in push order. A
    /// hit re-pushes with a fresh epoch (lazy LRU); the entry's earlier
    /// queue slots become ghosts that eviction skips and drains. This
    /// replaces the old `min_by_key(tick)` full-map scan, which made every
    /// insert O(capacity) once the cache filled — on a byte-budgeted
    /// 256 MiB cache (~65k blocks) each post-fill point get paid the whole
    /// scan (~500 µs; slipstream fold, guest v9f). Hit and evict are now
    /// both O(1) amortized.
    order: VecDeque<((u64, usize), u64)>,
    /// Monotonic push epoch (ghost matching).
    epoch: u64,
    /// Max entries; `0` = no entry cap.
    capacity: usize,
    /// Max payload bytes; `0` = no byte cap (RFC-0153).
    budget_bytes: u64,
    used_bytes: u64,
    hits: u64,
    misses: u64,
}

fn block_payload_bytes(block: &[(InternalKey, Bytes)]) -> u64 {
    block.iter().fold(0u64, |acc, (k, v)| {
        acc.saturating_add((k.user_key.len() + v.len() + 16) as u64)
    })
}

impl BlockCache {
    /// Create with max cached blocks (`0` = unlimited entry count).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(BlockCacheInner {
                map: HashMap::new(),
                order: VecDeque::new(),
                epoch: 0,
                capacity,
                budget_bytes: 0,
                used_bytes: 0,
                hits: 0,
                misses: 0,
            }),
        }
    }

    /// Rocks-shaped LRU: cap by payload bytes, no entry cap (RFC-0153).
    ///
    /// `0` is treated as 1 byte so a host that asked for an empty cache
    /// does not get the unlimited convention of [`Self::new(0)`].
    #[must_use]
    pub fn with_budget_bytes(bytes: u64) -> Self {
        Self {
            inner: Mutex::new(BlockCacheInner {
                map: HashMap::new(),
                order: VecDeque::new(),
                epoch: 0,
                capacity: 0,
                budget_bytes: bytes.max(1),
                used_bytes: 0,
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

    /// Occupancy in bytes (Rocks `block-cache-usage`).
    #[must_use]
    pub fn used_bytes(&self) -> u64 {
        self.inner.lock().used_bytes
    }

    /// Configured byte budget (`0` = no byte cap).
    #[must_use]
    pub fn budget_bytes(&self) -> u64 {
        self.inner.lock().budget_bytes
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
        self.get_or_insert_with_id(path_id(path), block_idx, load)
    }

    /// Hit-only probe (RFC-0160 P2.3). Miss does not invoke a loader, so
    /// the caller can `decode_block` (fail-closed) and insert only on Ok.
    pub fn get(&self, path: &Path, block_idx: usize) -> Option<CachedBlock> {
        self.get_id(path_id(path), block_idx)
    }

    /// [`Self::get`] keyed by a precomputed path id.
    pub fn get_id(&self, id: u64, block_idx: usize) -> Option<CachedBlock> {
        let key = (id, block_idx);
        let mut guard = self.inner.lock();
        let g = &mut *guard;
        let live = g.map.len();
        let slot = g.map.get_mut(&key)?;
        g.hits = g.hits.saturating_add(1);
        Self::touch(&mut g.order, &mut g.epoch, live, &key, slot);
        Some(Arc::clone(&slot.block))
    }

    /// [`Self::get_or_insert_with`] keyed by a precomputed path id, so a
    /// stream hashes its path once instead of once per block fetch. Ids from
    /// different tag domains (e.g. value-resolved slots) share the same map;
    /// a 64-bit hash collision has the same effect as colliding paths.
    pub fn get_or_insert_with_id<F>(&self, id: u64, block_idx: usize, load: F) -> CachedBlock
    where
        F: FnOnce() -> Vec<(InternalKey, Bytes)>,
    {
        let key = (id, block_idx);
        {
            let mut guard = self.inner.lock();
            let g = &mut *guard;
            let live = g.map.len();
            if let Some(slot) = g.map.get_mut(&key) {
                g.hits = g.hits.saturating_add(1);
                Self::touch(&mut g.order, &mut g.epoch, live, &key, slot);
                return Arc::clone(&slot.block);
            }
        }
        let block = Arc::new(load());
        let mut guard = self.inner.lock();
        let g = &mut *guard;
        let live = g.map.len();
        if let Some(slot) = g.map.get_mut(&key) {
            g.hits = g.hits.saturating_add(1);
            Self::touch(&mut g.order, &mut g.epoch, live, &key, slot);
            return Arc::clone(&slot.block);
        }
        g.misses = g.misses.saturating_add(1);
        let extra = block_payload_bytes(block.as_ref());
        while Self::needs_room(g, extra) {
            if !Self::evict_one(g) {
                break;
            }
        }
        g.epoch = g.epoch.wrapping_add(1);
        let ins_epoch = g.epoch;
        g.used_bytes = g.used_bytes.saturating_add(extra);
        g.map.insert(
            key,
            CachedSlot {
                block: Arc::clone(&block),
                ins_epoch,
                bytes: extra,
            },
        );
        g.order.push_back((key, ins_epoch));
        block
    }

    /// Lazy-LRU touch: re-push at the back with a fresh epoch. Bounded —
    /// beyond `4 * live + 64` pending entries the hit keeps its old queue
    /// position (CLOCK-like degradation) so a hit-heavy cache with no
    /// eviction pressure cannot grow the queue without bound.
    fn touch(
        order: &mut VecDeque<((u64, usize), u64)>,
        epoch: &mut u64,
        live: usize,
        key: &(u64, usize),
        slot: &mut CachedSlot,
    ) {
        if order.len() >= live.saturating_mul(4).max(64) {
            return;
        }
        *epoch = epoch.wrapping_add(1);
        slot.ins_epoch = *epoch;
        order.push_back((*key, *epoch));
    }

    fn needs_room(g: &BlockCacheInner, extra: u64) -> bool {
        let count_full = g.capacity > 0 && g.map.len() >= g.capacity;
        let bytes_full = g.budget_bytes > 0 && g.used_bytes.saturating_add(extra) > g.budget_bytes;
        (count_full || bytes_full) && !g.map.is_empty()
    }

    /// O(1)-amortized LRU evict: pop queue entries until one's epoch still
    /// matches its live slot (ghosts from hit re-pushes and re-inserts are
    /// skipped and drained here — the `AnswerCache` F178 pattern).
    fn evict_one(g: &mut BlockCacheInner) -> bool {
        while let Some((old, epoch)) = g.order.pop_front() {
            if g.map.get(&old).is_some_and(|s| s.ins_epoch == epoch) {
                if let Some(slot) = g.map.remove(&old) {
                    g.used_bytes = g.used_bytes.saturating_sub(slot.bytes);
                }
                return true;
            }
        }
        false
    }

    /// Clear all blocks.
    pub fn clear(&self) {
        let mut g = self.inner.lock();
        g.map.clear();
        g.order.clear();
        g.epoch = 0;
        g.used_bytes = 0;
    }
}

/// Latest-snapshot answers (point / last-prefix). Cleared on write.
///
/// Hit is O(1). Capacity 0 = disabled. (Count answers moved to
/// [`CountCache`] — range-aware invalidation.)
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

/// Buckets for compat TLS last-get invalidation (RFC-0154 P1.5).
///
/// A 1-key put bumps one slot instead of the process-wide get epoch, so zipf
/// gets of other keys stay cached. Fat apply still bumps the point TLS
/// epoch. Collisions are false misses, never stale hits (the slot also
/// compares the user key).
pub(crate) const KEY_GEN_N: usize = 4096;

/// Per-encoded-key generation for TLS point answers.
pub(crate) struct KeyGenMap {
    buckets: Box<[AtomicU64]>,
}

impl KeyGenMap {
    /// Empty map: every bucket starts at generation 1.
    pub(crate) fn new() -> Self {
        Self {
            buckets: (0..KEY_GEN_N).map(|_| AtomicU64::new(1)).collect(),
        }
    }

    fn bucket_of(key: &[u8]) -> usize {
        let mut fx = FxHasher::default();
        fx.write(key);
        fx.finish() as usize & (KEY_GEN_N - 1)
    }

    /// Hash of `pfx || 0 || key`, same bytes as a CF-prefixed user key.
    fn bucket_prefixed(pfx: &[u8], key: &[u8]) -> usize {
        if pfx.is_empty() {
            return Self::bucket_of(key);
        }
        const STACK: usize = 192;
        let n = pfx.len() + 1 + key.len();
        if n <= STACK {
            let mut buf = [0u8; STACK];
            buf[..pfx.len()].copy_from_slice(pfx);
            buf[pfx.len()] = 0;
            buf[pfx.len() + 1..n].copy_from_slice(key);
            Self::bucket_of(&buf[..n])
        } else {
            let mut v = Vec::with_capacity(n);
            v.extend_from_slice(pfx);
            v.push(0);
            v.extend_from_slice(key);
            Self::bucket_of(&v)
        }
    }

    /// Current generation for an encoded user key.
    pub(crate) fn gen(&self, key: &[u8]) -> u64 {
        self.buckets[Self::bucket_of(key)].load(Ordering::Acquire)
    }

    /// Current generation for `pfx || 0 || key` (named CF, no alloc on short keys).
    pub(crate) fn gen_prefixed(&self, pfx: &[u8], key: &[u8]) -> u64 {
        self.buckets[Self::bucket_prefixed(pfx, key)].load(Ordering::Acquire)
    }

    /// Bump the bucket for `key` (1-key publish).
    pub(crate) fn touch(&self, key: &[u8]) {
        self.buckets[Self::bucket_of(key)].fetch_add(1, Ordering::Release);
    }
}

type FxBuild = std::hash::BuildHasherDefault<FxHasher>;

#[derive(Debug, Default)]
struct AnswerCacheInner<V> {
    map: std::collections::HashMap<Bytes, (u64, u64, V), FxBuild>,
    /// Insertion order of the frozen working set. Once `map.len() ==
    /// capacity`, further unique inserts are dropped (uniform get_hit /
    /// lookup_100 must not FIFO-churn). `clear` on write starts a new fill.
    order: std::collections::VecDeque<(Bytes, u64)>,
    capacity: usize,
    /// Bumped on [`AnswerCache::clear`] so stale entries miss without a walk.
    gen: u64,
    /// Monotonic insertion epoch (FIFO pop correctness, F178).
    epoch: u64,
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
                gen: 0,
                epoch: 0,
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
        match g.map.get(key) {
            Some((gen, _, v)) if *gen == g.gen => Some(v.clone()),
            _ => None,
        }
    }

    /// Store a latest-snapshot answer.
    pub fn insert(&self, key: &[u8], value: V) {
        let mut g = self.inner.lock();
        if g.capacity == 0 {
            return;
        }
        let now = g.gen;
        if let Some((gen, _, v)) = g.map.get_mut(key) {
            *gen = now;
            *v = value;
            return;
        }
        if g.map.len() >= g.capacity {
            // Freeze once full. lookup_100 / get_hit are uniform-random
            // over 25M keys: FIFO evict + `Bytes` copy on every miss was
            // the fill tax (8192-cap never hits). Zipf's hot set fits in
            // 8192 so the first fill stays; writes `clear()`.
            return;
        }
        let epoch = g.epoch;
        g.epoch = g.epoch.wrapping_add(1);
        let owned = Bytes::copy_from_slice(key);
        g.order.push_back((owned.clone(), epoch));
        g.map.insert(owned, (now, epoch, value));
    }

    /// Invalidate every entry without walking the map (write path).
    pub fn clear(&self) {
        let mut g = self.inner.lock();
        g.gen = g.gen.wrapping_add(1);
        if g.gen == 0 {
            g.map.clear();
            g.order.clear();
            g.gen = 1;
        }
    }

    /// Drop one key so other latest-snapshot hits stay (YCSB B/D 95/5).
    pub fn invalidate(&self, key: &[u8]) {
        self.inner.lock().map.remove(key);
    }

    /// Drop several keys under **one** lock (publish path: one acquire per
    /// written batch instead of one per key).
    pub fn invalidate_many(&self, keys: &[Bytes]) {
        let mut g = self.inner.lock();
        for k in keys {
            g.map.remove(k);
        }
    }

    /// No cached answers (RFC-0062 P0.4: skip per-key dirty clones).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.lock().map.is_empty()
    }
}

/// One side of a cached count window: `None` = unbounded,
/// `Some((key, included))` = `Bound::Included`/`Excluded(key)`.
type CountSide = Option<(Bytes, bool)>;

fn count_side(b: std::ops::Bound<&[u8]>) -> CountSide {
    match b {
        std::ops::Bound::Unbounded => None,
        std::ops::Bound::Included(s) => Some((Bytes::copy_from_slice(s), true)),
        std::ops::Bound::Excluded(s) => Some((Bytes::copy_from_slice(s), false)),
    }
}

/// Latest-snapshot `count_in_range` answers with range-aware invalidation.
///
/// A distinct-key count over a window only changes when a key INSIDE the
/// window is written; writes elsewhere leave it valid. Entries carry the
/// published sequence observed BEFORE the answer was computed, and
/// [`CountCache::record_dirty`] tracks written keys in a bounded log
/// (newest sequence per key). A hit is served only when no tracked write
/// newer than the entry touches its window — RFC-0044 `ycsb-longwindow`:
/// the previous wholesale `clear()` on every publish made the 5% inserts
/// of `ycsb_e` cold every ~19 scans while scans walked retained versions.
///
/// Anything untrackable (range deletions, fat applies, dirty-log overflow)
/// retires entries conservatively: overflow evicts the oldest half of the
/// log and drops every entry overlapping the evicted keys' bounding box.
///
/// One mutex guards entries AND the dirty log: retirement must be atomic
/// with eviction (a get that validated against the truncated log could
/// otherwise serve a pre-write answer after the write published).
#[derive(Debug, Default)]
pub struct CountCache {
    state: Mutex<CountCacheState>,
}

#[derive(Debug, Default)]
struct CountCacheState {
    map: std::collections::HashMap<Bytes, CountEntry, FxBuild>,
    /// Insertion order for O(1) FIFO eviction.
    order: std::collections::VecDeque<Bytes>,
    capacity: usize,
    dirty: CountDirty,
    /// Highest publish sequence whose dirty keys were NOT recorded because
    /// the entry map was empty (write-heavy shapes record 100% of publishes
    /// for a cache no reader ever fills). An entry observed before such a
    /// publish must not validate — a lock-free `count_cache_handle` reader
    /// can still insert it after the skip (F204's interleaving, one flight).
    skipped_below: u64,
    /// RFC-0054 P0.2: conservative key-space envelope of the cached windows
    /// (min start / max end). Sticky through eviction — retirement only
    /// loses precision, never correctness. Keys published outside it cannot
    /// invalidate any present entry and skip the per-key dirty log (the
    /// raftdb-CF shape paid two allocations per key per publish once any
    /// scan had filled the cache: publish 0.57 µs → 4.2 µs).
    env_lo: EnvSide,
    env_hi: EnvSide,
    /// Highest publish sequence with envelope-dropped keys. An answer
    /// computed before such a publish cannot know whether a dropped key
    /// lay inside its window (F204's in-flight reader interleaving), so
    /// [`CountCache::insert`] refuses to cache anything observed below it.
    dropped_below_max: u64,
}

/// One sticky side of the [`CountCacheState`] envelope.
#[derive(Debug, Default, Clone)]
enum EnvSide {
    /// No window inserted yet (the empty-map path in `record_dirty`
    /// returns before the filter ever runs).
    #[default]
    Empty,
    /// An unbounded window was inserted: this side extends to infinity
    /// until `clear` — eviction never shrinks the envelope.
    Unbounded,
    /// Extreme bound seen so far (min start / max end).
    At(Bytes),
}

impl EnvSide {
    /// Keep `k` (it may lie inside a cached window)? `lo` picks the side:
    /// start bounds only drop keys below the minimum, end bounds only
    /// keys above the maximum (equal bytes stay — an `Included` end at
    /// the envelope edge still contains its own key).
    fn keeps(&self, k: &[u8], lo: bool) -> bool {
        match self {
            Self::Empty | Self::Unbounded => true,
            Self::At(b) => {
                if lo {
                    k >= b.as_ref()
                } else {
                    k <= b.as_ref()
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct CountEntry {
    start: CountSide,
    end: CountSide,
    /// Published sequence observed before the answer was computed: any
    /// write published after it carries a strictly greater sequence.
    seq: u64,
    n: usize,
}

impl CountEntry {
    /// Conservative overlap with the bounding box `[lo, hi]` of evicted
    /// dirty keys (loose on Included/Excluded edges — extra retirement is
    /// safe, missed retirement is not).
    fn overlaps_box(&self, lo: &[u8], hi: &[u8]) -> bool {
        let starts_before_hi = self.start.as_ref().is_none_or(|(s, _)| s.as_ref() <= hi);
        let ends_after_lo = self.end.as_ref().is_none_or(|(e, _)| e.as_ref() >= lo);
        starts_before_hi && ends_after_lo
    }
}

/// Bounded dirty-key log: publish order for eviction, key order for the
/// get-time overlap check.
#[derive(Debug, Default)]
struct CountDirty {
    order: std::collections::VecDeque<(u64, Box<[u8]>)>,
    by_key: std::collections::BTreeMap<Box<[u8]>, u64>,
    capacity: usize,
}

/// Dirty-log capacity; overflow evicts down to half (RFC-0044 amplifier).
const COUNT_DIRTY_CAP: usize = 1024;

impl CountDirty {
    /// Any tracked write with `seq > entry_seq` whose key lies in
    /// `[start, end)`? O(log n + overlapping hits).
    fn overlaps_newer_than(
        &self,
        start: std::ops::Bound<&[u8]>,
        end: std::ops::Bound<&[u8]>,
        entry_seq: u64,
    ) -> bool {
        self.by_key
            .range::<[u8], _>((start, end))
            .any(|(_, &s)| s > entry_seq)
    }
}

impl CountCacheState {
    /// Widen the envelope for a newly inserted window. Unbounded sides are
    /// sticky-infinite until `clear`.
    fn widen(&mut self, entry: &CountEntry) {
        match &entry.start {
            None => self.env_lo = EnvSide::Unbounded,
            Some((s, _)) => {
                if matches!(self.env_lo, EnvSide::Empty)
                    || matches!(&self.env_lo, EnvSide::At(cur) if s < cur)
                {
                    self.env_lo = EnvSide::At(s.clone());
                }
            }
        }
        match &entry.end {
            None => self.env_hi = EnvSide::Unbounded,
            Some((e, _)) => {
                if matches!(self.env_hi, EnvSide::Empty)
                    || matches!(&self.env_hi, EnvSide::At(cur) if e > cur)
                {
                    self.env_hi = EnvSide::At(e.clone());
                }
            }
        }
    }

    /// Could `k` lie inside some cached window? Conservative: only keys
    /// strictly outside the envelope are droppable.
    fn envelope_drops(&self, k: &[u8]) -> bool {
        !(self.env_lo.keeps(k, true) && self.env_hi.keeps(k, false))
    }
}

impl CountCache {
    /// Create with max cached windows (`0` = disabled).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(CountCacheState {
                map: std::collections::HashMap::default(),
                order: std::collections::VecDeque::new(),
                capacity,
                dirty: CountDirty {
                    order: std::collections::VecDeque::new(),
                    by_key: std::collections::BTreeMap::new(),
                    capacity: COUNT_DIRTY_CAP,
                },
                skipped_below: 0,
                env_lo: EnvSide::Empty,
                env_hi: EnvSide::Empty,
                dropped_below_max: 0,
            }),
        }
    }

    /// `None` = miss (absent, retired, or invalidated by a newer write
    /// inside the window). Validity is checked against the dirty log, so
    /// a hit never serves a pre-write answer.
    #[must_use]
    pub fn get(
        &self,
        start: std::ops::Bound<&[u8]>,
        end: std::ops::Bound<&[u8]>,
        limit: Option<usize>,
    ) -> Option<usize> {
        let ck = crate::db::count_cache_key(start, end, limit);
        let g = self.state.lock();
        if g.capacity == 0 {
            return None;
        }
        let e = g.map.get(ck.as_slice())?;
        if e.seq < g.skipped_below || g.dirty.overlaps_newer_than(start, end, e.seq) {
            return None;
        }
        Some(e.n)
    }

    /// Store a latest-snapshot answer computed while `seq` was the visible
    /// sequence (read BEFORE computing, so later writes compare strictly
    /// greater).
    pub fn insert(
        &self,
        start: std::ops::Bound<&[u8]>,
        end: std::ops::Bound<&[u8]>,
        limit: Option<usize>,
        n: usize,
        seq: u64,
    ) {
        let ck = crate::db::count_cache_key(start, end, limit);
        let mut g = self.state.lock();
        if g.capacity == 0 {
            return;
        }
        // RFC-0054: an answer observed before an envelope-dropped publish
        // cannot know whether a dropped key lay inside its window — cache
        // nothing below that watermark (F204's in-flight reader).
        if seq < g.dropped_below_max {
            return;
        }
        let entry = CountEntry {
            start: count_side(start),
            end: count_side(end),
            seq,
            n,
        };
        g.widen(&entry);
        if let Some(e) = g.map.get_mut(ck.as_slice()) {
            *e = entry;
            return;
        }
        if g.map.len() >= g.capacity {
            if let Some(old) = g.order.pop_front() {
                g.map.remove(&old);
            }
        }
        let owned = Bytes::copy_from_slice(ck.as_slice());
        g.order.push_back(owned.clone());
        g.map.insert(owned, entry);
    }

    /// Record the keys of one publish. Overflow evicts the oldest half of
    /// the log and retires entries overlapping the evicted keys' box —
    /// atomically, so no get can validate against the truncated log first.
    pub fn record_dirty(&self, seq: u64, keys: &[Bytes]) {
        let mut g = self.state.lock();
        // Empty entry map: no cached answer can exist from before this
        // publish under the Db read/write lock, and a lock-free reader that
        // still inserts one observed earlier will carry `seq <
        // skipped_below` and miss in `get`. Skipping keeps write-only
        // shapes from allocating two Boxes per written key per publish.
        if g.map.is_empty() {
            g.skipped_below = g.skipped_below.max(seq);
            return;
        }
        // F204: track even when no entries are cached yet. A publish that
        // lands while a reader is between its seq observation and its
        // insert (both run under the Db read lock; the publish too) is NOT
        // a "past" write for the entry that reader is about to insert —
        // without this record the pre-write answer validates forever,
        // because `get` only checks the dirty log.
        if g.dirty.capacity == 0 {
            return;
        }
        for k in keys {
            // RFC-0054 P0.2 envelope filter: a key outside every cached
            // window cannot invalidate a present entry — no dirty-log
            // allocation. `insert` refuses answers observed below the
            // bumped watermark, closing the in-flight-reader hole.
            if g.envelope_drops(k.as_ref()) {
                if seq > g.dropped_below_max {
                    g.dropped_below_max = seq;
                }
                continue;
            }
            g.dirty.order.push_back((seq, Box::from(k.as_ref())));
            g.dirty.by_key.insert(Box::from(k.as_ref()), seq);
        }
        let d = &mut g.dirty;
        if d.order.len() <= d.capacity {
            return;
        }
        let keep = d.capacity / 2;
        let mut lo: Option<Box<[u8]>> = None;
        let mut hi: Option<Box<[u8]>> = None;
        while d.order.len() > keep {
            let Some((s, k)) = d.order.pop_front() else {
                break;
            };
            if d.by_key.get(&k) == Some(&s) {
                d.by_key.remove(&k);
            }
            lo = Some(match lo {
                Some(cur) if cur.as_ref() <= k.as_ref() => cur,
                _ => k.clone(),
            });
            hi = Some(match hi {
                Some(cur) if cur.as_ref() >= k.as_ref() => cur,
                _ => k,
            });
        }
        if let Some((lo, hi)) = lo.zip(hi) {
            let inner = &mut *g;
            inner
                .map
                .retain(|_, e| !e.overlaps_box(lo.as_ref(), hi.as_ref()));
            let map = &inner.map;
            inner.order.retain(|k| map.contains_key(k));
        }
    }

    /// Wholesale drop (range deletion / fat apply / unknown dirt).
    pub fn clear(&self) {
        let mut g = self.state.lock();
        g.map.clear();
        g.order.clear();
        g.dirty.order.clear();
        g.dirty.by_key.clear();
        g.env_lo = EnvSide::Empty;
        g.env_hi = EnvSide::Empty;
        g.dropped_below_max = 0;
    }

    /// No cached count windows (RFC-0062 P0.4: skip per-key dirty clones).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.state.lock().map.is_empty()
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
    fn block_cache_get_is_hit_only() {
        let cache = BlockCache::with_budget_bytes(64 * 1024);
        let path = Path::new("/tmp/hit-only.sst");
        assert!(cache.get(path, 0).is_none());
        cache.get_or_insert_with(path, 0, || {
            vec![(
                InternalKey::new(Bytes::from_static(b"a"), 1, ValueType::Value),
                Bytes::from_static(b"1"),
            )]
        });
        assert!(cache.get(path, 0).is_some());
        assert_eq!(cache.hits(), 1);
        assert!(cache.get(path, 1).is_none(), "unknown block is not a load");
        assert_eq!(cache.misses(), 1, "get-miss must not count as a load miss");
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
    fn point_cache_invalidate_one_keeps_other() {
        let c = PointCache::new(8);
        c.insert(b"a", Some(Bytes::from_static(b"1")));
        c.insert(b"b", Some(Bytes::from_static(b"2")));
        c.invalidate(b"a");
        assert!(c.get(b"a").is_none());
        assert_eq!(c.get(b"b").unwrap().as_deref(), Some(&b"2"[..]));
    }

    #[test]
    fn point_cache_freezes_at_capacity() {
        let c = PointCache::new(2);
        c.insert(b"a", Some(Bytes::from_static(b"1")));
        c.insert(b"b", Some(Bytes::from_static(b"2")));
        c.insert(b"c", Some(Bytes::from_static(b"3")));
        assert_eq!(c.get(b"a").unwrap().as_deref(), Some(&b"1"[..]));
        assert_eq!(c.get(b"b").unwrap().as_deref(), Some(&b"2"[..]));
        assert!(
            c.get(b"c").is_none(),
            "full cache must not FIFO-evict on miss"
        );
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

    #[test]
    fn block_cache_byte_budget_evicts_cold() {
        let cache = BlockCache::with_budget_bytes(80);
        let path = Path::new("/tmp/byte-budget.sst");
        let fat = || {
            vec![(
                InternalKey::new(Bytes::from_static(b"k"), 1, ValueType::Value),
                Bytes::from(vec![b'v'; 40]),
            )]
        };
        cache.get_or_insert_with(path, 0, fat);
        let first = cache.used_bytes();
        assert!(first > 0 && first <= 80, "first={first}");
        cache.get_or_insert_with(path, 1, fat);
        assert!(
            cache.used_bytes() <= 80,
            "over budget {}",
            cache.used_bytes()
        );
        let mut reloaded = false;
        cache.get_or_insert_with(path, 0, || {
            reloaded = true;
            fat()
        });
        assert!(reloaded, "block 0 must have been evicted by byte budget");
    }

    #[test]
    fn block_cache_churn_keeps_budget_under_ghost_pressure() {
        // Regression shape for the lazy-LRU queue: sustained miss-insert
        // churn past a byte budget interleaved with hit re-pushes (ghost
        // producers). The budget must hold on every insert — ghost skips
        // cannot strand accounting — and hits must keep landing.
        let budget = 2_000u64;
        let cache = BlockCache::with_budget_bytes(budget);
        let path = Path::new("/tmp/churn.sst");
        let block = |i: usize| {
            vec![(
                InternalKey::new(
                    Bytes::from(format!("k{i:04}").into_bytes()),
                    1,
                    ValueType::Value,
                ),
                Bytes::from(vec![b'v'; 96]),
            )]
        };
        // ~130 B payload each: ~15 blocks fit in budget.
        let mut i = 0usize;
        for round in 0..40 {
            for _ in 0..8 {
                cache.get_or_insert_with(path, i, || block(i));
                i += 1;
            }
            // Re-push pressure: cycle hits over everything still live.
            for hot in i.saturating_sub(15)..i {
                cache.get_or_insert_with(path, hot, || panic!("{hot} just inserted"));
            }
            assert!(
                cache.used_bytes() <= budget,
                "round {round}: used {} > budget {}",
                cache.used_bytes(),
                budget
            );
        }
        assert!(cache.hits() > 0, "churn produced no hits");
        let hits_before = cache.hits();
        cache.get_or_insert_with(path, i - 1, || panic!("latest block must be cached"));
        assert_eq!(cache.hits(), hits_before + 1);
    }

    #[test]
    fn count_cache_skip_while_empty_retires_racy_insert() {
        use std::ops::Bound;
        let c = CountCache::new(8);
        let (s, e) = (Bound::Included(&b"k"[..]), Bound::Excluded(&b"m"[..]));
        // Publish while the entry map is empty: recorded only as a
        // watermark. A lock-free reader that observed seq BEFORE this
        // publish may still insert its answer afterwards — it must miss.
        c.record_dirty(6, &[Bytes::from_static(b"key00")]);
        c.insert(s, e, Some(25), 3, 5);
        assert!(c.get(s, e, Some(25)).is_none(), "pre-skip answer served");
        // An answer observed at/after the skipped publish includes it and
        // stays valid.
        c.insert(s, e, Some(25), 4, 6);
        assert_eq!(c.get(s, e, Some(25)), Some(4));
        // The watermark also retires entries re-inserted after clear().
        c.clear();
        c.insert(s, e, Some(25), 4, 5);
        assert!(c.get(s, e, Some(25)).is_none(), "stale seq below watermark");
    }

    #[test]
    fn count_cache_invalidates_only_overlapping_writes() {
        use std::ops::Bound;
        let c = CountCache::new(8);
        let (s, e) = (Bound::Included(&b"k"[..]), Bound::Excluded(&b"m"[..]));
        assert!(c.get(s, e, Some(25)).is_none());
        c.insert(s, e, Some(25), 3, 1);
        assert_eq!(c.get(s, e, Some(25)), Some(3));
        // Writes outside the window keep the cached answer.
        c.record_dirty(2, &[Bytes::from_static(b"a")]);
        c.record_dirty(3, &[Bytes::from_static(b"zz")]);
        assert_eq!(c.get(s, e, Some(25)), Some(3));
        // A strictly newer write inside the window invalidates.
        c.record_dirty(4, &[Bytes::from_static(b"key05")]);
        assert!(c.get(s, e, Some(25)).is_none());
        // An answer computed AFTER that write (seq == write seq) is valid.
        c.insert(s, e, Some(25), 4, 4);
        assert_eq!(c.get(s, e, Some(25)), Some(4));
        c.clear();
        assert!(c.get(s, e, Some(25)).is_none());
    }

    #[test]
    fn count_cache_overflow_retires_only_overlapping_boxes() {
        use std::ops::Bound;
        let c = CountCache::new(8);
        let (s, e) = (Bound::Included(&b"k"[..]), Bound::Excluded(&b"m"[..]));
        c.insert(s, e, Some(25), 3, 1);
        // More writes than the dirty-log cap, all far above the window:
        // evicted-key boxes stay disjoint, the entry survives.
        for i in 0..(COUNT_DIRTY_CAP + 2) {
            c.record_dirty(2 + i as u64, &[Bytes::from(format!("zz{i:06}"))]);
        }
        assert_eq!(c.get(s, e, Some(25)), Some(3));
        // Same flood with keys inside the window: boxes overlap, entry dies.
        for i in 0..(COUNT_DIRTY_CAP + 2) {
            c.record_dirty(4000 + i as u64, &[Bytes::from(format!("key{i:06}"))]);
        }
        assert!(c.get(s, e, Some(25)).is_none());
    }

    #[test]
    fn count_cache_envelope_skips_outside_publishes() {
        use std::ops::Bound;
        let c = CountCache::new(8);
        let (s, e) = (Bound::Included(&b"k"[..]), Bound::Excluded(&b"m"[..]));
        c.insert(s, e, Some(25), 3, 1);
        assert_eq!(c.get(s, e, Some(25)), Some(3));
        // "zz" is outside the envelope [k, m]: dropped from the dirty log…
        c.record_dirty(5, &[Bytes::from_static(b"zz")]);
        // …and the cached window stays valid — no overlap either way.
        assert_eq!(c.get(s, e, Some(25)), Some(3));
        // A racy reader that observed seq 4 (< 5) tries to insert a window
        // containing "zz": refused below the dropped watermark.
        let (ys, ye) = (Bound::Included(&b"y"[..]), Bound::Excluded(&b"zzz"[..]));
        c.insert(ys, ye, Some(25), 7, 4);
        assert_eq!(c.get(ys, ye, Some(25)), None);
        // An answer observed at/after the dropped publish caches normally.
        c.insert(ys, ye, Some(25), 8, 5);
        assert_eq!(c.get(ys, ye, Some(25)), Some(8));
        // Inside-envelope publishes still record and invalidate precisely.
        c.record_dirty(6, &[Bytes::from_static(b"kk")]);
        assert_eq!(c.get(s, e, Some(25)), None);
    }

    #[test]
    fn count_cache_dropped_watermark_blocks_stale_insert_only() {
        use std::ops::Bound;
        let c = CountCache::new(8);
        let (ds, de) = (
            Bound::Included(&b"data\0a"[..]),
            Bound::Excluded(&b"data\0z"[..]),
        );
        c.insert(ds, de, Some(25), 5, 1);
        // A raftlog-CF publish outside the data envelope: dropped, only
        // the global dropped watermark moves.
        c.record_dirty(9, &[Bytes::from_static(b"raftlog\0x")]);
        assert_eq!(c.get(ds, de, Some(25)), Some(5));
        // A racy raftlog window (observed seq 8 < 9) is refused entry.
        let (rs, re) = (
            Bound::Included(&b"raftlog\0a"[..]),
            Bound::Excluded(&b"raftlog\0z"[..]),
        );
        c.insert(rs, re, Some(25), 2, 8);
        assert_eq!(c.get(rs, re, Some(25)), None);
        // The data window is untouched by the raftlog drop.
        assert_eq!(c.get(ds, de, Some(25)), Some(5));
    }

    #[test]
    fn count_cache_envelope_unbounded_start_drops_above_end() {
        use std::ops::Bound;
        let c = CountCache::new(8);
        let (s, e) = (Bound::<&[u8]>::Unbounded, Bound::Excluded(&b"m"[..]));
        c.insert(s, e, Some(25), 3, 1);
        assert_eq!(c.get(s, e, Some(25)), Some(3));
        // "zz" is above the envelope's end: no cached window contains it.
        c.record_dirty(5, &[Bytes::from_static(b"zz")]);
        assert_eq!(c.get(s, e, Some(25)), Some(3));
        // …but an insert observed below that drop (seq 4) is refused.
        let (ys, ye) = (Bound::Included(&b"y"[..]), Bound::Excluded(&b"zzz"[..]));
        c.insert(ys, ye, Some(25), 9, 4);
        assert_eq!(c.get(ys, ye, Some(25)), None);
        // Publishes below "m" may hit the unbounded window: recorded.
        c.record_dirty(6, &[Bytes::from_static(b"j")]);
        assert_eq!(c.get(s, e, Some(25)), None);
    }

    #[test]
    fn key_gen_prefixed_matches_encoded() {
        let m = KeyGenMap::new();
        let encoded = {
            let mut v = b"lock".to_vec();
            v.push(0);
            v.extend_from_slice(b"user-key");
            v
        };
        assert_eq!(m.gen(&encoded), m.gen_prefixed(b"lock", b"user-key"));
        m.touch(&encoded);
        assert_eq!(m.gen(&encoded), m.gen_prefixed(b"lock", b"user-key"));
        let other = m.gen(b"untouched");
        m.touch(&encoded);
        assert_eq!(other, m.gen(b"untouched"));
        assert_ne!(m.gen(&encoded), other);
    }

    #[test]
    fn payload_pool_register_zero_does_not_ghost_charge() {
        use std::sync::Arc as StdArc;
        let pool = SstPayloadPool::with_budget(Some(1000));
        let slot = StdArc::new(parking_lot::RwLock::new(ResidentBody::empty()));
        pool.register(Path::new("bulk.sst"), StdArc::downgrade(&slot), 0);
        assert_eq!(pool.resident_bytes(), 0);
        assert_eq!(pool.tracked_tables(), 0);
        assert!(!pool.can_admit(Path::new("bulk.sst"), 0));
        assert!(pool.can_admit(Path::new("bulk.sst"), 100));
        assert!(!pool.can_admit(Path::new("fat.sst"), 2000));
    }

    /// RFC-0042 v18: the pool evicts oldest-first down to budget; a dropped
    /// table's dead Weak self-cleans on the next pass.
    #[test]
    fn payload_pool_evicts_fifo_to_budget() {
        use std::sync::Arc as StdArc;

        let pool = SstPayloadPool::with_budget(Some(150));
        assert_eq!(pool.resident_bytes(), 0);

        let mk = |bytes: &[u8]| {
            StdArc::new(parking_lot::RwLock::new(ResidentBody::from_image(
                Arc::from(bytes.to_vec()),
            )))
        };
        let s1 = mk(&[1u8; 100]);
        let s2 = mk(&[2u8; 100]);
        let s3 = mk(&[3u8; 100]);

        // Unarmed: registration records but never evicts.
        pool.register(Path::new("a.sst"), StdArc::downgrade(&s1), 100);
        pool.register(Path::new("b.sst"), StdArc::downgrade(&s2), 100);
        assert_eq!(pool.resident_bytes(), 200);
        assert!(!s1.read().img.is_empty());

        pool.arm();
        // Eviction clears whole entries: 200 - 100 (oldest) = 100 ≤ 150.
        assert_eq!(pool.resident_bytes(), 100, "arming enforces the budget");
        assert!(
            s1.read().img.is_empty(),
            "oldest registration evicted first"
        );
        assert!(!s2.read().img.is_empty());

        // New table: evicts s2, keeps the newcomer.
        pool.register(Path::new("c.sst"), StdArc::downgrade(&s3), 100);
        assert_eq!(pool.resident_bytes(), 100);
        assert!(s2.read().img.is_empty());
        assert!(!s3.read().img.is_empty());
        assert_eq!(pool.tracked_tables(), 1, "evicted entries leave the map");

        // A reader holding a payload clone keeps its bytes: eviction clears
        // the slot only; the clone frees when the reader drops it.
        let held: Arc<[u8]> = {
            let g = s3.read();
            Arc::clone(&g.img)
        };
        let s4 = mk(&[4u8; 100]);
        pool.register(Path::new("d.sst"), StdArc::downgrade(&s4), 100);
        assert_eq!(pool.resident_bytes(), 100);
        assert!(s3.read().img.is_empty());
        assert_eq!(
            &held[..3],
            &[3u8, 3, 3],
            "held reader bytes survive eviction"
        );
    }
}
