//! In-memory versioned key-value buffer (MemTable).
//!
//! Holds puts and deletions as [`InternalKey`] entries until flushed to SST
//! (later) or rebuilt from the WAL on recovery. Point lookups and ranges honor
//! a **snapshot sequence**: only entries with `sequence <= snapshot` are visible;
//! the newest such entry wins (delete tombstone hides the key).

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::hash::{BuildHasherDefault, Hasher};
use std::ops::Bound;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;

use crate::key::{InternalKey, SequenceNumber, ValueType};

/// Copy key for ≤32-byte tail index: `pack32` + len (injective; zero-pad
/// ties broken by len).
type PackKey = (u128, u128, u16);

/// Result of a point lookup that found a visible version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// Key exists with this value at the snapshot.
    Found(Bytes),
    /// Newest visible version is a deletion tombstone.
    Deleted,
    /// No version with `sequence <= snapshot`.
    NotFound,
}

/// One version of a user key (seq-desc, then kind-desc — same as [`InternalKey`]).
#[derive(Debug, Clone)]
struct Version {
    key: InternalKey,
    value: Bytes,
}

/// One or more versions of a user key. The first put is inline so apply /
/// YCSB / raftlog (almost all distinct keys) do not heap-allocate a `Vec`
/// per key (RFC-0041 P1.1 write CPU).
///
/// `Many` holds versions newest-first and is a `VecDeque`: the common insert
/// (a newer version) lands at index 0, which is O(1) front space on a
/// deque — the `Vec` shape paid a full memmove per hot-key overwrite
/// (one parked-fold core burned in `insert_map`, ycsb_a profile 2026-08-22).
#[derive(Debug, Clone)]
enum Versions {
    One(Version),
    Many(std::collections::VecDeque<Version>),
}

/// Concrete per-version iterator (no `Box<dyn>` on the scan/count refill).
enum VersIter<'a> {
    One(Option<&'a Version>),
    Many(std::collections::vec_deque::Iter<'a, Version>),
}

impl<'a> Iterator for VersIter<'a> {
    type Item = &'a Version;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::One(v) => v.take(),
            Self::Many(it) => it.next(),
        }
    }
}

impl Versions {
    fn iter(&self) -> VersIter<'_> {
        match self {
            Self::One(v) => VersIter::One(Some(v)),
            Self::Many(vs) => VersIter::Many(vs.iter()),
        }
    }

    fn iter_mut(&mut self) -> VersIterMut<'_> {
        match self {
            Self::One(v) => VersIterMut::One(Some(v)),
            Self::Many(vs) => VersIterMut::Many(vs.iter_mut()),
        }
    }
}

/// Mutable counterpart of [`VersIter`] (value-log remap walks every version).
enum VersIterMut<'a> {
    One(Option<&'a mut Version>),
    Many(std::collections::vec_deque::IterMut<'a, Version>),
}

impl<'a> Iterator for VersIterMut<'a> {
    type Item = &'a mut Version;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::One(v) => v.take(),
            Self::Many(it) => it.next(),
        }
    }
}

/// Counters for versions dropped by fold-GC (keeps `entries` /
/// `approx_bytes` exact — range tombstones are never dropped, see F200 in
/// `gc_below_floor`).
#[derive(Default)]
struct Dropped {
    versions: usize,
    bytes: usize,
}

impl<'a> IntoIterator for &'a Versions {
    type Item = &'a Version;
    type IntoIter = VersIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Borrowed walk of `BTreeMap` user-key range, newest-first versions per key.
/// Concrete so count/scan do not `Box<dyn Iterator>` on every refill.
pub(crate) struct MemInternalRange<'a> {
    users: std::iter::Peekable<std::collections::btree_map::Range<'a, Bytes, Versions>>,
    cur: VersIter<'a>,
}

/// Merge of the sorted BTree with a **sorted tail** (O(tail log tail), not
/// O((map+tail) log) — YCSB E/scan must not sort the whole memtable).
pub(crate) struct MemInternalMerge<'a> {
    map: MemInternalRange<'a>,
    tail: std::iter::Peekable<std::vec::IntoIter<(&'a InternalKey, &'a Bytes)>>,
}

/// Snapshot-aware merge: BTree range + `tail_idx` range (newest tail version
/// per user key). O(log n + hits) — a parked 4 MiB tail must not be walked
/// per count/scan (RFC-0041 deps_scan regression). The idx side is a single
/// CF shard in sorted order (cross-CF / unbounded queries fall back to
/// [`MemInternalMerge`] first).
pub(crate) struct MemInternalIdx<'a> {
    map: MemInternalRange<'a>,
    /// Concrete single-shard short-key range (RFC-0054 P1.3 / RFC-0149 P2.1:
    /// no Box and no per-bound `Bytes` clones — the gate in
    /// `iter_internal_iter_at` only reaches here when the bounds pin one
    /// CF shard of keys ≤ 32 bytes).
    idx: std::iter::Peekable<std::collections::btree_map::Range<'a, (u128, u128, u16), usize>>,
    tail: &'a [Version],
}

/// Map-only or map+tail merge. Returned by [`MemTable::iter_internal_range`].
pub(crate) enum MemInternalIter<'a> {
    Map(MemInternalRange<'a>),
    Merge(MemInternalMerge<'a>),
    Idx(MemInternalIdx<'a>),
}

impl<'a> Iterator for MemInternalRange<'a> {
    type Item = (&'a InternalKey, &'a Bytes);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(v) = self.cur.next() {
                return Some((&v.key, &v.value));
            }
            let (_, vers) = self.users.next()?;
            self.cur = vers.iter();
        }
    }
}

impl<'a> MemInternalRange<'a> {
    /// Next yieldable item without consuming (O(1) lookahead; may pull the
    /// next user group into the peekable — `Peekable` caches it).
    fn peek(&mut self) -> Option<(&'a InternalKey, &'a Bytes)> {
        match &self.cur {
            VersIter::One(Some(v)) => Some((&v.key, &v.value)),
            VersIter::Many(it) => it.clone().next().map(|v| (&v.key, &v.value)),
            VersIter::One(None) => {
                let (_, vers) = self.users.peek()?;
                match vers {
                    Versions::One(v) => Some((&v.key, &v.value)),
                    Versions::Many(vs) => vs.front().map(|v| (&v.key, &v.value)),
                }
            }
        }
    }

    /// Drop every remaining version of the current user group and land on
    /// the next user (RFC-0054 P1.3: count/scan `step_user` must not pop a
    /// hot user's versions one by one — the shared deps memtable holds
    /// dozens per hot key after apply). No-op when positioned elsewhere.
    fn step_user(&mut self, user: &[u8]) {
        if self
            .peek()
            .is_some_and(|(k, _)| k.user_key.as_ref() == user)
        {
            self.cur = VersIter::One(None);
            if let Some((_, vers)) = self.users.next() {
                self.cur = vers.iter();
            }
        }
    }
}

impl<'a> MemInternalMerge<'a> {
    /// Fast user skip for count/scan (see [`MemInternalRange::step_user`]);
    /// the sorted-tail side advances past the user's versions linearly.
    fn step_user(&mut self, user: &[u8]) {
        self.map.step_user(user);
        while self
            .tail
            .peek()
            .is_some_and(|(k, _)| k.user_key.as_ref() == user)
        {
            self.tail.next();
        }
    }
}

impl<'a> MemInternalIdx<'a> {
    /// Fast user skip for count/scan (see [`MemInternalRange::step_user`]);
    /// the idx side holds one entry per user.
    fn step_user(&mut self, user: &[u8]) {
        self.map.step_user(user);
        while self
            .idx
            .peek()
            .is_some_and(|(_, &i)| self.tail[i].key.user_key.as_ref() == user)
        {
            self.idx.next();
        }
    }
}

impl<'a> Iterator for MemInternalMerge<'a> {
    type Item = (&'a InternalKey, &'a Bytes);

    fn next(&mut self) -> Option<Self::Item> {
        match (self.map.peek(), self.tail.peek()) {
            (None, None) => None,
            (Some(_), None) => self.map.next(),
            (None, Some(_)) => self.tail.next(),
            (Some(m), Some(t)) => match m.0.cmp(t.0) {
                Ordering::Less => self.map.next(),
                Ordering::Greater => self.tail.next(),
                Ordering::Equal => {
                    let _ = self.map.next();
                    self.tail.next()
                }
            },
        }
    }
}

impl<'a> Iterator for MemInternalIdx<'a> {
    type Item = (&'a InternalKey, &'a Bytes);

    fn next(&mut self) -> Option<Self::Item> {
        let tail_item = self
            .idx
            .peek()
            .map(|(_, &i)| (&self.tail[i].key, &self.tail[i].value));
        match (self.map.peek(), tail_item) {
            (None, None) => None,
            (Some(_), None) => self.map.next(),
            (None, Some(_)) => {
                let (_, &i) = self.idx.next()?;
                Some((&self.tail[i].key, &self.tail[i].value))
            }
            (Some(m), Some(t)) => {
                if t.0 < m.0 {
                    let (_, &i) = self.idx.next()?;
                    Some((&self.tail[i].key, &self.tail[i].value))
                } else {
                    self.map.next()
                }
            }
        }
    }
}

impl<'a> Iterator for MemInternalIter<'a> {
    type Item = (&'a InternalKey, &'a Bytes);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Map(it) => it.next(),
            Self::Merge(it) => it.next(),
            Self::Idx(it) => it.next(),
        }
    }
}

impl<'a> MemInternalIter<'a> {
    /// Fast user skip (see [`MemInternalRange::step_user`]) — count/scan
    /// only ever consume the first visible version per user key.
    pub(crate) fn step_user(&mut self, user: &[u8]) {
        match self {
            Self::Map(it) => it.step_user(user),
            Self::Merge(it) => it.step_user(user),
            Self::Idx(it) => it.step_user(user),
        }
    }
}

/// Sorted in-memory table of versioned keys.
///
/// Keyed by user key so [`Self::get_entry`] is a borrowed `BTreeMap` lookup
/// (RFC-0035 P1.2: no `InternalKey` / `Bytes` probe alloc on the hot get).
/// Mutations are single-threaded for P0 (callers serialize writers). Reads may
/// share a reference if the outer layer uses interior mutability carefully;
/// this type itself is not synchronized.
#[derive(Debug, Default)]
pub struct MemTable {
    /// User key → versions newest-first (inline first version).
    map: BTreeMap<Bytes, Versions>,
    /// Recent inserts not yet in `map` (RFC-0041: apply Ok path is O(1) push).
    tail: Vec<Version>,
    /// Newest tail index per user key, **sharded by CF prefix** (bytes before
    /// the first `0x00` — compat `cf\\0key`; keys without NUL live in the
    /// empty prefix). Rocks gives each CF its own memtable; we emulate that
    /// on the Ok-path index so `deps_raftlog` after `deps_apply_batch` does
    /// not pay `log(N_all_cfs)` (RFC-0054).
    ///
    /// Keys ≤ 32 bytes (apply / YCSB / kvrocks) live in a Copy `(pack32, len)`
    /// tree — no `Bytes` clone per insert (RFC-0149 P2.1). Longer keys keep
    /// a full-key tree. `pack32` + len is injective for ≤ 32 bytes: zero-pad
    /// collisions (`"a"` vs `"a\\0"`) differ in `len`.
    tail_idx: BTreeMap<Bytes, TailShard>,
    /// Lazy sorted view of HashMap-only shards (`lock`/`default`/`write`) for
    /// range / last / count. Invalidated with an atomic so apply does not take
    /// the mutex. The HashMap itself is live (RFC-0154).
    point_ord: Mutex<BTreeMap<Bytes, Arc<Vec<(PackKey, usize)>>>>,
    point_ord_live: AtomicBool,
    /// Highest sequence in `tail` (fast-path guard: snapshot ≥ it ⇒ only the
    /// newest version per key can be visible).
    tail_max_seq: SequenceNumber,
    /// Cached InternalKey order of `tail`. Writers only flip
    /// `tail_ord_stale` (no mutex on the 1c put path).
    #[allow(dead_code)]
    tail_ord: Mutex<Option<Arc<Vec<u32>>>>,
    tail_ord_stale: AtomicBool,
    /// Approximate bytes for flush triggers (user key + value + trailer).
    approx_bytes: usize,
    /// Bytes per CF prefix (RFC-0065 P1.1). Keyed by `cf_prefix` (empty =
    /// default-raw). `"default\0…"` is a distinct prefix from empty.
    cf_bytes: BTreeMap<Bytes, usize>,
    /// Range-tombstone entries (full-map fallback on ranged scan when > 0).
    range_tombstones: usize,
    /// Total internal versions (not distinct user keys).
    entries: usize,
}

impl Clone for MemTable {
    fn clone(&self) -> Self {
        Self {
            map: self.map.clone(),
            tail: self.tail.clone(),
            tail_idx: self.tail_idx.clone(),
            point_ord: Mutex::new(BTreeMap::new()),
            point_ord_live: AtomicBool::new(false),
            tail_max_seq: self.tail_max_seq,
            tail_ord: Mutex::new(None),
            tail_ord_stale: AtomicBool::new(true),
            approx_bytes: self.approx_bytes,
            cf_bytes: self.cf_bytes.clone(),
            range_tombstones: self.range_tombstones,
            entries: self.entries,
        }
    }
}

/// Compat CF encoding is `cf\\0user`. Kernel keys without NUL share one shard.
pub(crate) fn cf_prefix(key: &[u8]) -> &[u8] {
    match key.iter().position(|&b| b == 0) {
        Some(i) => &key[..i],
        None => &[],
    }
}

pub use crate::cf_kernel::{cf_family_of, infer_sst_cf, key_in_cf_family};

fn family_from_prefix(prefix: &[u8]) -> String {
    if prefix.is_empty() {
        "default".into()
    } else {
        String::from_utf8_lossy(prefix).into_owned()
    }
}

/// Big-endian `(u128, u128)` of the first `min(32, len)` bytes, zero-padded
/// right. Compares as the first 32 bytes of the key would byte-wise (see
/// `MemTable::tail_idx`): a padding zero equals a real `0x00` byte, so a
/// shorter key ties only when the longer key really continues with zeros,
/// and the full-key tiebreak keeps `shorter < longer`.
fn pack32(key: &[u8]) -> (u128, u128) {
    let mut a = [0u8; 32];
    let n = key.len().min(32);
    a[..n].copy_from_slice(&key[..n]);
    let (l, r) = a.split_at(16);
    (
        u128::from_be_bytes(l.try_into().unwrap()),
        u128::from_be_bytes(r.try_into().unwrap()),
    )
}

/// `tail_idx` shard key: integer-compare fast path + full-key tiebreak
/// (oracle / keys > 32 bytes).
#[cfg(test)]
type TailIdxKey = (u128, u128, Bytes);

/// rustc-hash Fx mix. SipHash on 64k always-new MVCC keys rehashed the
/// apply mem path (11.89 → 14.52µs); reserve + this hasher is the 2× cut.
#[derive(Clone, Debug, Default)]
struct FxHasher {
    hash: u64,
}

const FX_K: u64 = 0x517c_c1b7_2722_0a95;

impl FxHasher {
    #[inline]
    fn mix(&mut self, i: u64) {
        self.hash = (self.hash.rotate_left(5) ^ i).wrapping_mul(FX_K);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }

    fn write(&mut self, mut bytes: &[u8]) {
        while bytes.len() >= 8 {
            let (chunk, rest) = bytes.split_at(8);
            self.mix(u64::from_ne_bytes(chunk.try_into().unwrap()));
            bytes = rest;
        }
        if bytes.len() >= 4 {
            let (chunk, rest) = bytes.split_at(4);
            self.mix(u32::from_ne_bytes(chunk.try_into().unwrap()) as u64);
            bytes = rest;
        }
        for &b in bytes {
            self.mix(u64::from(b));
        }
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.mix(u64::from(i));
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.mix(i);
    }

    #[inline]
    fn write_u128(&mut self, i: u128) {
        self.mix(i as u64);
        self.mix((i >> 64) as u64);
    }
}

type PointMap = HashMap<PackKey, usize, BuildHasherDefault<FxHasher>>;

/// Point shards (HashMap): `lock` / `default`. `write` stays a BTree —
/// MVCC latest is reverse-seek (RFC-0154 P1.2). Empty prefix and `raftlog`
/// stay a BTree (YCSB E / kvrocks SCAN / sequential append). P1.3
/// (empty→HashMap) **regressed** CHV 7/17→6/17 (`ycsb_c` lost 3×).
#[inline]
fn point_cf(pfx: &[u8]) -> bool {
    pfx == b"lock" || pfx == b"default"
}

#[inline]
fn point_reserve(pfx: &[u8]) -> usize {
    match pfx {
        b"default" => 1 << 17,
        b"lock" => 2048,
        _ => 0,
    }
}

/// Per-CF tail index. Short keys (≤ 32 B) are Copy `(pack32, len)` — apply
/// / SET / YCSB never clone `Bytes` into the tree (RFC-0149 P2.1).
#[derive(Clone, Debug, Default)]
struct TailShard {
    short: BTreeMap<PackKey, usize>,
    /// Point CFs (`lock` overwrites, `default` point gets). `write` is
    /// `short` so last_visible is BTree next_back, not copy+sort.
    point: PointMap,
    long: BTreeMap<Bytes, usize>,
}

fn bound_len_le_32(b: Bound<&[u8]>) -> bool {
    match b {
        Bound::Unbounded => true,
        Bound::Included(k) | Bound::Excluded(k) => k.len() <= 32,
    }
}

/// Byte bound → `(pack32, len)` for keys ≤ 32 bytes. Same total order as
/// raw bytes: `pack32` is the first 32 bytes, `len` breaks zero-pad ties.
fn packed_short_bound(b: Bound<&[u8]>) -> Bound<PackKey> {
    match b {
        Bound::Included(k) => {
            let (p0, p1) = pack32(k);
            Bound::Included((p0, p1, k.len() as u16))
        }
        Bound::Excluded(k) => {
            let (p0, p1) = pack32(k);
            Bound::Excluded((p0, p1, k.len() as u16))
        }
        Bound::Unbounded => Bound::Unbounded,
    }
}

fn pack_bound_start(ord: &[(PackKey, usize)], start: Bound<PackKey>) -> usize {
    match start {
        Bound::Unbounded => 0,
        Bound::Included(s) => ord.partition_point(|&(k, _)| k < s),
        Bound::Excluded(s) => ord.partition_point(|&(k, _)| k <= s),
    }
}

fn pack_bound_end(ord: &[(PackKey, usize)], end: Bound<PackKey>) -> usize {
    match end {
        Bound::Unbounded => ord.len(),
        Bound::Included(e) => ord.partition_point(|&(k, _)| k <= e),
        Bound::Excluded(e) => ord.partition_point(|&(k, _)| k < e),
    }
}

/// Byte-level bound → exact `(pack32, bytes)` shard bound (see `pack32`).
#[cfg(test)]
fn packed_bound(b: Bound<&[u8]>) -> Bound<TailIdxKey> {
    match b {
        Bound::Included(b) => {
            let (p0, p1) = pack32(b);
            Bound::Included((p0, p1, Bytes::copy_from_slice(b)))
        }
        Bound::Excluded(b) => {
            let (p0, p1) = pack32(b);
            Bound::Excluded((p0, p1, Bytes::copy_from_slice(b)))
        }
        Bound::Unbounded => Bound::Unbounded,
    }
}

fn bound_cf_prefix(b: Bound<&[u8]>) -> Option<&[u8]> {
    match b {
        Bound::Included(k) | Bound::Excluded(k) => Some(cf_prefix(k)),
        Bound::Unbounded => None,
    }
}

/// [`InternalKey`] order on `(seq, kind)` only (user key already equal).
fn version_newer(a: &Version, b: &Version) -> bool {
    a.key.sequence > b.key.sequence || (a.key.sequence == b.key.sequence && a.key.kind > b.key.kind)
}

fn ver_cmp(
    a_seq: SequenceNumber,
    a_kind: ValueType,
    b_seq: SequenceNumber,
    b_kind: ValueType,
) -> Ordering {
    match b_seq.cmp(&a_seq) {
        Ordering::Equal => b_kind.cmp(&a_kind),
        o => o,
    }
}

impl MemTable {
    /// Create an empty MemTable.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of internal key entries (versions), not distinct user keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries
    }

    /// Whether no entries are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries == 0
    }

    /// Approximate memory used by keys and values (for flush thresholds).
    #[must_use]
    pub fn approx_memory_usage(&self) -> usize {
        self.approx_bytes
    }

    fn bump_cf_bytes_map(map: &mut BTreeMap<Bytes, usize>, user_key: &[u8], n: usize, add: bool) {
        let p = cf_prefix(user_key);
        if let Some(slot) = map.get_mut(p) {
            *slot = if add {
                slot.saturating_add(n)
            } else {
                slot.saturating_sub(n)
            };
            return;
        }
        if add && n > 0 {
            map.insert(Bytes::copy_from_slice(p), n);
        }
    }

    fn bump_cf_bytes(&mut self, user_key: &[u8], n: usize, add: bool) {
        Self::bump_cf_bytes_map(&mut self.cf_bytes, user_key, n, add);
    }

    /// Approximate memory of one CF family (RFC-0065 P1.1).
    ///
    /// `"default"` sums the empty prefix (raw keys) and the `default\0` prefix.
    #[must_use]
    pub fn approx_memory_usage_cf(&self, family: &str) -> usize {
        if family == "default" {
            let raw = self.cf_bytes.get(&b""[..]).copied().unwrap_or(0);
            let pref = self.cf_bytes.get(&b"default"[..]).copied().unwrap_or(0);
            raw.saturating_add(pref)
        } else {
            self.cf_bytes.get(family.as_bytes()).copied().unwrap_or(0)
        }
    }

    /// Move every key of `family` into a new table (RFC-0065 P1.1).
    ///
    /// Keepers are not cloned — the map is partitioned in place so a tiny
    /// `lock` flush does not memcpy the default memtable.
    #[must_use]
    pub fn take_family(&mut self, family: &str) -> Self {
        self.spill_tail();
        let map = std::mem::take(&mut self.map);
        let mut taken_map = BTreeMap::new();
        for (k, vers) in map {
            if key_in_cf_family(k.as_ref(), family) {
                taken_map.insert(k, vers);
            } else {
                self.map.insert(k, vers);
            }
        }
        self.recount();
        let mut taken = Self::new();
        taken.map = taken_map;
        taken.recount();
        taken
    }

    fn recount(&mut self) {
        self.approx_bytes = 0;
        self.entries = 0;
        self.range_tombstones = 0;
        self.cf_bytes.clear();
        self.tail_max_seq = 0;
        self.tail_idx.clear();
        self.invalidate_tail_ord();
        let mut cf_bytes = BTreeMap::new();
        for (uk, vers) in &self.map {
            for v in vers.iter() {
                let n = uk.len() + v.value.len() + 8;
                self.approx_bytes = self.approx_bytes.saturating_add(n);
                self.entries = self.entries.saturating_add(1);
                Self::bump_cf_bytes_map(&mut cf_bytes, uk.as_ref(), n, true);
                self.tail_max_seq = self.tail_max_seq.max(v.key.sequence);
                if v.key.kind == ValueType::RangeDeletion {
                    self.range_tombstones = self.range_tombstones.saturating_add(1);
                }
            }
        }
        let tail = std::mem::take(&mut self.tail);
        for (i, v) in tail.iter().enumerate() {
            let n = v.key.user_key.len() + v.value.len() + 8;
            self.approx_bytes = self.approx_bytes.saturating_add(n);
            self.entries = self.entries.saturating_add(1);
            Self::bump_cf_bytes_map(&mut cf_bytes, v.key.user_key.as_ref(), n, true);
            self.tail_max_seq = self.tail_max_seq.max(v.key.sequence);
            if v.key.kind == ValueType::RangeDeletion {
                self.range_tombstones = self.range_tombstones.saturating_add(1);
            }
            self.tail_idx_insert(v.key.user_key.as_ref(), i);
        }
        self.tail = tail;
        self.cf_bytes = cf_bytes;
    }

    /// Whether any insert is still in the unsorted tail.
    #[must_use]
    pub fn has_tail(&self) -> bool {
        !self.tail.is_empty()
    }

    /// Length of the unsorted tail (RFC-0054).
    #[must_use]
    pub fn tail_len(&self) -> usize {
        self.tail.len()
    }

    /// Push onto the shared tail and index by CF prefix. Returns the global index.
    fn tail_append(&mut self, key: InternalKey, value: Bytes) -> usize {
        let pfx = cf_prefix(key.user_key.as_ref());
        let pfx_b = Bytes::copy_from_slice(pfx);
        let point = point_cf(pfx);
        let cap = point_reserve(pfx);
        let s = self.tail_idx.entry(pfx_b).or_insert_with(|| {
            let mut s = TailShard::default();
            if cap > 0 {
                s.point.reserve(cap);
            }
            s
        });
        let i = self.tail.len();
        if key.user_key.len() <= 32 {
            let (p0, p1) = pack32(key.user_key.as_ref());
            let sk = (p0, p1, key.user_key.len() as u16);
            if point {
                s.point.insert(sk, i);
            } else {
                s.short.insert(sk, i);
            }
        } else {
            s.long.insert(key.user_key.clone(), i);
        }
        self.tail.push(Version { key, value });
        i
    }

    fn invalidate_tail_ord(&self) {
        self.point_ord_live.store(false, AtomicOrdering::Relaxed);
        self.tail_ord_stale.store(true, AtomicOrdering::Release);
    }

    fn tail_idx_insert(&mut self, key: &[u8], i: usize) {
        self.tail_idx_insert_at(key, cf_prefix(key), i);
    }

    fn tail_idx_insert_at(&mut self, key: &[u8], pfx: &[u8], i: usize) {
        Self::tail_idx_insert_into(&mut self.tail_idx, key, pfx, i);
    }

    fn tail_idx_insert_into(
        idx: &mut BTreeMap<Bytes, TailShard>,
        key: &[u8],
        pfx: &[u8],
        i: usize,
    ) {
        if key.len() <= 32 {
            let (p0, p1) = pack32(key);
            let sk = (p0, p1, key.len() as u16);
            if point_cf(pfx) {
                if let Some(s) = idx.get_mut(pfx) {
                    s.point.insert(sk, i);
                    return;
                }
                let mut s = TailShard::default();
                let cap = point_reserve(pfx);
                if cap > 0 {
                    s.point.reserve(cap);
                }
                s.point.insert(sk, i);
                idx.insert(Bytes::copy_from_slice(pfx), s);
                return;
            }
            if let Some(s) = idx.get_mut(pfx) {
                s.short.insert(sk, i);
                return;
            }
            let p = Bytes::copy_from_slice(pfx);
            idx.entry(p).or_default().short.insert(sk, i);
        } else {
            let full = Bytes::copy_from_slice(key);
            if let Some(s) = idx.get_mut(pfx) {
                s.long.insert(full, i);
                return;
            }
            let p = Bytes::copy_from_slice(pfx);
            idx.entry(p).or_default().long.insert(full, i);
        }
    }

    fn shard_lookup(shard: &TailShard, user_key: &[u8]) -> Option<usize> {
        if user_key.len() <= 32 {
            let (p0, p1) = pack32(user_key);
            let sk = (p0, p1, user_key.len() as u16);
            shard
                .point
                .get(&sk)
                .copied()
                .or_else(|| shard.short.get(&sk).copied())
        } else {
            shard.long.get(user_key).copied()
        }
    }

    /// Sorted `(pack32, len)` of a HashMap shard. First range after apply
    /// pays the sort; apply itself never reads this (RFC-0149 P2.1).
    fn cached_point_ord(&self, pfx: &[u8]) -> Option<Arc<Vec<(PackKey, usize)>>> {
        if self.point_ord_live.load(AtomicOrdering::Relaxed) {
            if let Ok(g) = self.point_ord.lock() {
                if let Some(v) = g.get(pfx) {
                    return Some(Arc::clone(v));
                }
            }
        }
        let mut v: Vec<(PackKey, usize)> = {
            let shard = self.tail_idx.get(pfx)?;
            if shard.point.is_empty() {
                return None;
            }
            shard.point.iter().map(|(&k, &i)| (k, i)).collect()
        };
        v.sort_unstable_by_key(|&(k, _)| k);
        let arc = Arc::new(v);
        if let Ok(mut g) = self.point_ord.lock() {
            g.insert(Bytes::copy_from_slice(pfx), Arc::clone(&arc));
        }
        self.point_ord_live.store(true, AtomicOrdering::Relaxed);
        Some(arc)
    }

    #[allow(dead_code)]
    fn tail_idx_get(&self, user_key: &[u8]) -> Option<usize> {
        let shard = self.tail_idx.get(cf_prefix(user_key))?;
        Self::shard_lookup(shard, user_key)
    }

    /// Fold [`Self::tail`] into the BTree (SST write / fold / tests).
    pub fn spill_tail(&mut self) {
        self.spill_tail_with_gc(None);
    }

    /// [`Self::spill_tail`] with version GC: drops superseded versions below
    /// `floor` (Rocks-style snapshot-list GC — rust-rocksdb `Snapshot` pins
    /// are the reader contract). `None` keeps every version (core default).
    pub fn spill_tail_with_gc(&mut self, floor: Option<SequenceNumber>) {
        self.invalidate_tail_ord();
        self.tail_max_seq = 0;
        let tail = std::mem::take(&mut self.tail);
        self.tail_idx.clear();
        for v in tail {
            let entry_bytes = v.key.user_key.len() + v.value.len() + 8;
            let is_rd = v.key.kind == ValueType::RangeDeletion;
            self.entries = self.entries.saturating_sub(1);
            self.approx_bytes = self.approx_bytes.saturating_sub(entry_bytes);
            if is_rd {
                self.range_tombstones = self.range_tombstones.saturating_sub(1);
            }
            self.insert_map_gc(v.key, v.value, floor);
        }
    }

    /// Move every version from `other` into `self` (retired L0 fold).
    pub fn absorb(&mut self, other: Self) {
        self.absorb_with_floor(other, None);
    }

    /// [`Self::absorb`] with version GC under `floor` (see
    /// [`Self::spill_tail_with_gc`]). The dropped set is exactly
    /// `{seq ≤ floor} \ {newest ≤ floor}` — every read at or above the floor
    /// still sees its exact version.
    pub fn absorb_with_floor(&mut self, mut other: Self, floor: Option<SequenceNumber>) {
        self.spill_tail_with_gc(floor);
        other.spill_tail_with_gc(floor);
        if self.is_empty() {
            *self = other;
            return;
        }
        if other.is_empty() {
            return;
        }
        for (_, vers) in other.map {
            match vers {
                Versions::One(v) => self.insert_map_gc(v.key, v.value, floor),
                // Oldest-first: every incoming version is newer than the
                // versions already merged for its key, so the binary search
                // lands at index 0 — O(1) deque front inserts. Newest-first
                // would land at a growing index and shift O(k) per insert
                // (the quadratic the 2026-08-22 ycsb_a profile caught).
                Versions::Many(vs) => {
                    for v in vs.into_iter().rev() {
                        self.insert_map_gc(v.key, v.value, floor);
                    }
                }
            }
        }
    }

    /// Insert a put or deletion. Does not assign sequence numbers — caller does.
    pub fn insert(&mut self, key: InternalKey, value: Bytes) {
        let entry_bytes = key.user_key.len() + value.len() + 8;
        let is_rd = key.kind == ValueType::RangeDeletion;
        // Consecutive same-seq replace only (O(1)). apply_mc4 keys are distinct;
        // a full tail scan would be O(n²) and slower than the BTree we replaced.
        if let Some(v) = self.tail.last_mut() {
            if v.key.sequence == key.sequence
                && v.key.kind == key.kind
                && v.key.user_key == key.user_key
            {
                let old = std::mem::replace(&mut v.value, value);
                let old_n = old.len();
                let new_n = v.value.len();
                self.approx_bytes = self
                    .approx_bytes
                    .saturating_sub(old_n)
                    .saturating_add(new_n);
                if new_n >= old_n {
                    Self::bump_cf_bytes_map(
                        &mut self.cf_bytes,
                        v.key.user_key.as_ref(),
                        new_n - old_n,
                        true,
                    );
                } else {
                    Self::bump_cf_bytes_map(
                        &mut self.cf_bytes,
                        v.key.user_key.as_ref(),
                        old_n - new_n,
                        false,
                    );
                }
                return;
            }
        }
        self.entries = self.entries.saturating_add(1);
        self.approx_bytes = self.approx_bytes.saturating_add(entry_bytes);
        self.bump_cf_bytes(key.user_key.as_ref(), entry_bytes, true);
        if is_rd {
            self.range_tombstones = self.range_tombstones.saturating_add(1);
        }
        self.invalidate_tail_ord();
        self.tail_max_seq = self.tail_max_seq.max(key.sequence);
        self.tail_append(key, value);
    }

    /// Batch insert: one `tail_ord` invalidate (RFC-0044 P1.1 pipeline).
    ///
    /// RFC-0154: every key updates live `tail_idx`. hint≥16 still skips the
    /// consecutive same-seq replace scan (apply keys are distinct) — it does
    /// **not** skip the index.
    pub fn insert_many(&mut self, items: impl IntoIterator<Item = (InternalKey, Bytes)>) {
        let iter = items.into_iter();
        let hint = iter.size_hint().0;
        let skip_replace = hint >= 16;
        let mut any = false;
        for (key, value) in iter {
            let entry_bytes = key.user_key.len() + value.len() + 8;
            let is_rd = key.kind == ValueType::RangeDeletion;
            if !skip_replace {
                if let Some(v) = self.tail.last_mut() {
                    if v.key.sequence == key.sequence
                        && v.key.kind == key.kind
                        && v.key.user_key == key.user_key
                    {
                        let old = std::mem::replace(&mut v.value, value);
                        let old_n = old.len();
                        let new_n = v.value.len();
                        self.approx_bytes = self
                            .approx_bytes
                            .saturating_sub(old_n)
                            .saturating_add(new_n);
                        if new_n >= old_n {
                            Self::bump_cf_bytes_map(
                                &mut self.cf_bytes,
                                v.key.user_key.as_ref(),
                                new_n - old_n,
                                true,
                            );
                        } else {
                            Self::bump_cf_bytes_map(
                                &mut self.cf_bytes,
                                v.key.user_key.as_ref(),
                                old_n - new_n,
                                false,
                            );
                        }
                        continue;
                    }
                }
            }
            self.entries = self.entries.saturating_add(1);
            self.approx_bytes = self.approx_bytes.saturating_add(entry_bytes);
            self.bump_cf_bytes(key.user_key.as_ref(), entry_bytes, true);
            if is_rd {
                self.range_tombstones = self.range_tombstones.saturating_add(1);
            }
            self.tail_max_seq = self.tail_max_seq.max(key.sequence);
            self.tail_append(key, value);
            any = true;
        }
        if any {
            self.invalidate_tail_ord();
        }
    }

    /// `insert_map` with optional version-GC floor (see
    /// [`Self::spill_tail_with_gc`]).
    fn insert_map_gc(&mut self, key: InternalKey, value: Bytes, floor: Option<SequenceNumber>) {
        let entry_bytes = key.user_key.len() + value.len() + 8;
        let is_rd = key.kind == ValueType::RangeDeletion;
        match self.map.entry(key.user_key.clone()) {
            std::collections::btree_map::Entry::Vacant(e) => {
                e.insert(Versions::One(Version { key, value }));
                self.entries = self.entries.saturating_add(1);
                self.approx_bytes = self.approx_bytes.saturating_add(entry_bytes);
                if is_rd {
                    self.range_tombstones = self.range_tombstones.saturating_add(1);
                }
            }
            std::collections::btree_map::Entry::Occupied(mut e) => {
                let mut dropped = Dropped::default();
                if Self::insert_into(
                    e.get_mut(),
                    key,
                    value,
                    entry_bytes,
                    &mut self.approx_bytes,
                    floor,
                    &mut dropped,
                ) {
                    self.entries = self.entries.saturating_add(1);
                    if is_rd {
                        self.range_tombstones = self.range_tombstones.saturating_add(1);
                    }
                }
                self.entries = self.entries.saturating_sub(dropped.versions);
                self.approx_bytes = self.approx_bytes.saturating_sub(dropped.bytes);
            }
        }
    }

    /// Returns true when a new version was added (false on same-seq replace).
    fn insert_into(
        vers: &mut Versions,
        key: InternalKey,
        value: Bytes,
        entry_bytes: usize,
        approx_bytes: &mut usize,
        floor: Option<SequenceNumber>,
        dropped: &mut Dropped,
    ) -> bool {
        match vers {
            Versions::One(existing) => {
                if existing.key.sequence == key.sequence && existing.key.kind == key.kind {
                    let old = std::mem::replace(&mut existing.value, value);
                    *approx_bytes = approx_bytes
                        .saturating_sub(old.len())
                        .saturating_add(existing.value.len());
                    return false;
                }
                let existing_newer = ver_cmp(
                    existing.key.sequence,
                    existing.key.kind,
                    key.sequence,
                    key.kind,
                ) == Ordering::Less;
                let Versions::One(old) = std::mem::replace(vers, Versions::Many(VecDeque::new()))
                else {
                    unreachable!("just matched One");
                };
                let mut list = VecDeque::new();
                if existing_newer {
                    list.push_back(old);
                    list.push_back(Version { key, value });
                } else {
                    list.push_back(Version { key, value });
                    list.push_back(old);
                }
                *approx_bytes = approx_bytes.saturating_add(entry_bytes);
                if let Some(f) = floor {
                    Self::gc_below_floor(&mut list, f, dropped);
                }
                *vers = if list.is_empty() {
                    // Both versions fell below the floor — impossible (the
                    // inserted version is kept), but keep the shape honest.
                    Versions::Many(VecDeque::new())
                } else if list.len() == 1 {
                    let mut it = list.into_iter();
                    Versions::One(it.next().expect("len checked"))
                } else {
                    Versions::Many(list)
                };
                true
            }
            Versions::Many(slot) => {
                // Take the deque out so `vers` can be reassigned at the end
                // (a GC'd or replaced pair collapses back to `One`).
                let mut list = std::mem::take(slot);
                let mut added = true;
                // Newest-first list: the incoming version is newer than the
                // front (the common overwrite) → insert at 0, O(1) on a deque.
                let newer = |list: &VecDeque<Version>, i: usize| {
                    ver_cmp(
                        list[i].key.sequence,
                        list[i].key.kind,
                        key.sequence,
                        key.kind,
                    ) == Ordering::Less
                };
                let (mut lo, mut hi) = (0usize, list.len());
                while lo < hi {
                    let mid = lo + (hi - lo) / 2;
                    if newer(&list, mid) {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                if lo < list.len()
                    && list[lo].key.sequence == key.sequence
                    && list[lo].key.kind == key.kind
                {
                    let old = std::mem::replace(&mut list[lo].value, value);
                    *approx_bytes = approx_bytes
                        .saturating_sub(old.len())
                        .saturating_add(list[lo].value.len());
                    added = false;
                } else {
                    list.insert(lo, Version { key, value });
                    *approx_bytes = approx_bytes.saturating_add(entry_bytes);
                }
                if let Some(f) = floor {
                    Self::gc_below_floor(&mut list, f, dropped);
                }
                *vers = if list.len() == 1 {
                    let mut it = list.into_iter();
                    Versions::One(it.next().expect("len checked"))
                } else {
                    Versions::Many(list)
                };
                added
            }
        }
    }

    /// Keep `{seq > floor} ∪ {newest ≤ floor}` (plus same-seq siblings) in a
    /// newest-first version list, counting what was dropped. Reads at any
    /// sequence ≥ floor still resolve exactly; reads below fail closed via
    /// the caller's watermark ratchet (never silent-wrong).
    fn gc_below_floor(list: &mut VecDeque<Version>, floor: SequenceNumber, dropped: &mut Dropped) {
        // First index with sequence ≤ floor ([0, idx) all newer than floor).
        let (mut lo, mut hi) = (0usize, list.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if list[mid].key.sequence > floor {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo == list.len() {
            return; // every version is newer than the floor
        }
        // Keep the whole same-sequence group at the boundary (kind ordering).
        let mut keep = lo;
        while keep + 1 < list.len() && list[keep + 1].key.sequence == list[lo].key.sequence {
            keep += 1;
        }
        if keep + 1 >= list.len() {
            return;
        }
        // F200: a range tombstone hides every OLDER key in its range, not
        // just its own start key — the per-key "newest ≤ floor" rule cannot
        // decide it (older versions of other keys survive this GC, and older
        // SSTs may still hold covered data). Only a bottommost compaction
        // may drop one. Keep every `RangeDeletion`; the drained suffix was
        // oldest-first, so pushing the survivors back in iteration order
        // preserves the global newest-first ordering.
        let suffix: Vec<Version> = list.drain(keep + 1..).collect();
        for v in suffix {
            if v.key.kind == ValueType::RangeDeletion {
                list.push_back(v);
                continue;
            }
            dropped.versions += 1;
            dropped.bytes += v.key.user_key.len() + v.value.len() + 8;
        }
    }

    /// Convenience: put `user_key → value` at `sequence`.
    pub fn put(
        &mut self,
        user_key: impl Into<Bytes>,
        sequence: SequenceNumber,
        value: impl Into<Bytes>,
    ) {
        let key = InternalKey::new(user_key, sequence, ValueType::Value);
        self.insert(key, value.into());
    }

    /// Convenience: tombstone `user_key` at `sequence`.
    pub fn delete(&mut self, user_key: impl Into<Bytes>, sequence: SequenceNumber) {
        let key = InternalKey::new(user_key, sequence, ValueType::Deletion);
        self.insert(key, Bytes::new());
    }

    /// Point lookup visible at `snapshot`.
    #[must_use]
    pub fn get(&self, user_key: &[u8], snapshot: SequenceNumber) -> Lookup {
        self.get_entry(user_key, snapshot)
            .map_or(Lookup::NotFound, |(_, look)| look)
    }

    /// Like [`get`](Self::get) but also returns the winning version's sequence
    /// (for layered merge in `Db::lookup` without a full memtable walk).
    ///
    /// Borrowed user-key lookup — does not allocate an [`InternalKey`] probe.
    #[must_use]
    pub fn get_entry(
        &self,
        user_key: &[u8],
        snapshot: SequenceNumber,
    ) -> Option<(SequenceNumber, Lookup)> {
        let from_tail = self.tail_best(user_key, snapshot);
        let from_map = self
            .map
            .get(user_key)
            .and_then(|vers| vers.iter().find(|v| v.key.sequence <= snapshot));
        // Equal (seq, kind): tail was inserted later (same-seq replace after spill).
        let v = match (from_tail, from_map) {
            (Some(t), Some(m)) => {
                if version_newer(m, t) {
                    m
                } else {
                    t
                }
            }
            (Some(t), None) => t,
            (None, Some(m)) => m,
            (None, None) => return None,
        };
        debug_assert_eq!(v.key.user_key.as_ref(), user_key);
        if v.key.kind == ValueType::RangeDeletion {
            return None;
        }
        let range_hidden = v.key.kind == ValueType::Value
            && self.range_deleted(user_key, v.key.sequence, snapshot);
        let look = if crate::merge::visible_at(v.key.kind, range_hidden) {
            Lookup::Found(v.value.clone())
        } else {
            Lookup::Deleted
        };
        Some((v.key.sequence, look))
    }

    fn tail_best(&self, user_key: &[u8], snapshot: SequenceNumber) -> Option<&Version> {
        let pfx = cf_prefix(user_key);
        let shard = self.tail_idx.get(pfx)?;
        let i = Self::shard_lookup(shard, user_key)?;
        let newest = self.tail.get(i)?;
        if newest.key.sequence <= snapshot {
            return Some(newest);
        }
        let mut best: Option<&Version> = None;
        for v in &self.tail {
            if v.key.user_key.as_ref() != user_key || v.key.sequence > snapshot {
                continue;
            }
            if best.is_none_or(|b| version_newer(v, b)) {
                best = Some(v);
            }
        }
        best
    }

    /// Append range tombstones visible at `snapshot` (O(n) — only call when
    /// [`Self::has_range_tombstones`] is true).
    pub fn collect_range_tombstones(
        &self,
        snapshot: SequenceNumber,
        out: &mut Vec<crate::merge::RangeTombstone>,
    ) {
        if self.range_tombstones == 0 {
            return;
        }
        for (uk, vers) in &self.map {
            for v in vers {
                if v.key.kind != ValueType::RangeDeletion || v.key.sequence > snapshot {
                    continue;
                }
                out.push(crate::merge::RangeTombstone {
                    start: uk.clone(),
                    end: v.value.clone(),
                    sequence: v.key.sequence,
                });
            }
        }
        for v in &self.tail {
            if v.key.kind != ValueType::RangeDeletion || v.key.sequence > snapshot {
                continue;
            }
            out.push(crate::merge::RangeTombstone {
                start: v.key.user_key.clone(),
                end: v.value.clone(),
                sequence: v.key.sequence,
            });
        }
    }

    /// Insert a range tombstone covering `[start, end)` at `sequence`.
    pub fn delete_range(
        &mut self,
        start: impl Into<Bytes>,
        end: impl Into<Bytes>,
        sequence: SequenceNumber,
    ) {
        let start = start.into();
        let end = end.into();
        let key = InternalKey::new(start, sequence, ValueType::RangeDeletion);
        self.insert(key, end);
    }

    /// Whether a point at `point_seq` is covered by a range tombstone ≤ `snapshot`.
    fn range_deleted(
        &self,
        user_key: &[u8],
        point_seq: SequenceNumber,
        snapshot: SequenceNumber,
    ) -> bool {
        if self.range_tombstones == 0 {
            return false;
        }
        for (uk, vers) in &self.map {
            for v in vers {
                if v.key.kind != ValueType::RangeDeletion || v.key.sequence > snapshot {
                    continue;
                }
                if v.key.sequence > point_seq
                    && user_key >= uk.as_ref()
                    && user_key < v.value.as_ref()
                {
                    return true;
                }
            }
        }
        for v in &self.tail {
            if v.key.kind != ValueType::RangeDeletion || v.key.sequence > snapshot {
                continue;
            }
            if v.key.sequence > point_seq
                && user_key >= v.key.user_key.as_ref()
                && user_key < v.value.as_ref()
            {
                return true;
            }
        }
        false
    }

    /// Distinct CF families present in this table (RFC-0065 P0 split flush).
    ///
    /// Empty prefix / kernel keys map to `"default"`. Order is sorted.
    #[must_use]
    pub fn cf_families(&self) -> Vec<String> {
        let mut set = BTreeSet::new();
        for p in self.tail_idx.keys() {
            set.insert(family_from_prefix(p.as_ref()));
        }
        let mut last: Option<&[u8]> = None;
        for k in self.map.keys() {
            let p = cf_prefix(k.as_ref());
            if last != Some(p) {
                set.insert(family_from_prefix(p));
                last = Some(p);
            }
        }
        set.into_iter().collect()
    }

    /// All internal versions in [`InternalKey`] order (for SST flush).
    pub fn iter_internal(&self) -> impl Iterator<Item = (&InternalKey, &Bytes)> + '_ {
        self.iter_internal_iter(Bound::Unbounded, Bound::Unbounded)
    }

    /// Whether any range tombstone is stored (ranged scan must include them).
    #[must_use]
    pub fn has_range_tombstones(&self) -> bool {
        self.range_tombstones > 0
    }

    /// Internal versions with user key in `[start, end)` (`BTree` range, not a full scan).
    ///
    /// Range tombstones whose start key sits outside the interval are **not**
    /// yielded — callers that must honor covering tombstones should fall back
    /// to [`Self::iter_internal`] when [`Self::has_range_tombstones`] is true.
    pub fn iter_internal_range<'a>(
        &'a self,
        start: Bound<&'a [u8]>,
        end: Bound<&'a [u8]>,
    ) -> impl Iterator<Item = (&'a InternalKey, &'a Bytes)> + 'a {
        self.iter_internal_iter(start, end)
    }

    pub(crate) fn iter_internal_iter<'a>(
        &'a self,
        start: Bound<&'a [u8]>,
        end: Bound<&'a [u8]>,
    ) -> MemInternalIter<'a> {
        let map = self.iter_internal_range_cursor(start, end);
        if !self.has_tail() {
            return MemInternalIter::Map(map);
        }
        let mut tail: Vec<(&InternalKey, &Bytes)> = Vec::with_capacity(self.tail_len());
        for v in &self.tail {
            if crate::merge::user_key_in_range(v.key.user_key.as_ref(), start, end) {
                tail.push((&v.key, &v.value));
            }
        }
        tail.sort_unstable_by(|a, b| a.0.cmp(b.0));
        MemInternalIter::Merge(MemInternalMerge {
            map,
            tail: tail.into_iter().peekable(),
        })
    }

    /// Snapshot-aware range walk for **latest-snapshot** count/scan: the tail
    /// side comes from `tail_idx` (BTree range, newest version per user key),
    /// never a linear tail scan. Older snapshots fall back to
    /// [`Self::iter_internal_iter`] (all versions, sorted merge).
    pub(crate) fn iter_internal_iter_at<'a>(
        &'a self,
        start: Bound<&'a [u8]>,
        end: Bound<&'a [u8]>,
        snapshot: SequenceNumber,
    ) -> MemInternalIter<'a> {
        if !self.has_tail() || snapshot < self.tail_max_seq {
            return self.iter_internal_iter(start, end);
        }
        // Single CF shard: a non-empty `cf\0` prefix on BOTH bounds pins the
        // range inside that shard's key space (shard keys are exactly
        // `cf\0…`; any key of another shard sorts outside `cf\0…`).
        // Empty-prefix bounds (kernel keys without NUL) are only safe over a
        // one-shard index — a range like `["d/m/", "d/m0")` has no NUL, yet
        // admits `d/m/\0…` keys that live in the "d/m/" shard (F220).
        // RFC-0154: kvrocks/YCSB default-raw is that one empty shard.
        let shard = match (bound_cf_prefix(start), bound_cf_prefix(end)) {
            (Some(a), Some(b)) if a == b && !a.is_empty() && a.len() < 32 => self.tail_idx.get(a),
            (Some(a), Some(b)) if a == b && a.is_empty() && self.tail_idx.len() == 1 => {
                self.tail_idx.values().next()
            }
            _ if self.tail_idx.len() == 1 => self.tail_idx.values().next(),
            _ => None,
        };
        let Some(shard) = shard else {
            return self.iter_internal_iter(start, end);
        };
        // Long keys keep a Bytes tree; mixed shards fall back to the sorted
        // tail merge so total order stays exact (RFC-0149 P2.1).
        if !shard.long.is_empty()
            || shard.short.is_empty()
            || !bound_len_le_32(start)
            || !bound_len_le_32(end)
        {
            return self.iter_internal_iter(start, end);
        }
        let map = self.iter_internal_range_cursor(start, end);
        MemInternalIter::Idx(MemInternalIdx {
            map,
            idx: shard
                .short
                .range((packed_short_bound(start), packed_short_bound(end)))
                .peekable(),
            tail: &self.tail,
        })
    }

    /// Latest-snapshot count over a single CF shard sitting entirely in
    /// `tail` (deps_scan / kvrocks_scan: no SST, no map). One BTree or
    /// sorted-point range, no cursor merge (RFC-0154).
    pub(crate) fn count_latest_in_range(
        &self,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: usize,
        snapshot: SequenceNumber,
    ) -> Option<usize> {
        if snapshot < self.tail_max_seq || !self.map.is_empty() || self.has_range_tombstones() {
            return None;
        }
        if self.tail_len() == 0 {
            return Some(0);
        }
        let mut pfx_buf = [0u8; 32];
        let pfx_len = match (bound_cf_prefix(start), bound_cf_prefix(end)) {
            (Some(a), Some(b)) if a == b && !a.is_empty() && a.len() < 32 => {
                pfx_buf[..a.len()].copy_from_slice(a);
                a.len()
            }
            (Some(a), Some(b)) if a == b && a.is_empty() && self.tail_idx.len() == 1 => 0,
            _ => return None,
        };
        let pfx = &pfx_buf[..pfx_len];
        if !bound_len_le_32(start) || !bound_len_le_32(end) {
            return None;
        }
        let lo = packed_short_bound(start);
        let hi = packed_short_bound(end);
        let shard = self.tail_idx.get(pfx)?;
        let has_long = !shard.long.is_empty();
        let has_point = !shard.point.is_empty();
        let has_short = !shard.short.is_empty();
        if has_long {
            return None;
        }
        if has_point {
            let ord = self.cached_point_ord(pfx)?;
            let a = pack_bound_start(&ord, lo);
            let b = pack_bound_end(&ord, hi);
            let mut n = 0usize;
            for &(_, i) in &ord[a..b] {
                if self.tail[i].key.kind == ValueType::Value {
                    n += 1;
                    if n >= limit {
                        break;
                    }
                }
            }
            return Some(n);
        }
        if !has_short {
            return None;
        }
        let mut n = 0usize;
        for (_, &i) in shard.short.range((lo, hi)) {
            if self.tail[i].key.kind == ValueType::Value {
                n += 1;
                if n >= limit {
                    break;
                }
            }
        }
        Some(n)
    }

    #[allow(dead_code)]
    fn cached_tail_order(&self) -> Arc<Vec<u32>> {
        if !self.tail_ord_stale.load(AtomicOrdering::Acquire) {
            let g = self.tail_ord.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(arc) = g.as_ref() {
                return Arc::clone(arc);
            }
        }
        let mut g = self.tail_ord.lock().unwrap_or_else(|e| e.into_inner());
        if !self.tail_ord_stale.load(AtomicOrdering::Acquire) {
            if let Some(arc) = g.as_ref() {
                return Arc::clone(arc);
            }
        }
        let mut idx: Vec<u32> = (0..self.tail.len() as u32).collect();
        idx.sort_unstable_by(|&a, &b| self.tail[a as usize].key.cmp(&self.tail[b as usize].key));
        let arc = Arc::new(idx);
        *g = Some(Arc::clone(&arc));
        self.tail_ord_stale.store(false, AtomicOrdering::Release);
        arc
    }

    /// Concrete (no `dyn`) range cursor — count/scan hot path.
    pub(crate) fn iter_internal_range_cursor<'a>(
        &'a self,
        start: Bound<&'a [u8]>,
        end: Bound<&'a [u8]>,
    ) -> MemInternalRange<'a> {
        MemInternalRange {
            users: self.map.range::<[u8], _>((start, end)).peekable(),
            cur: VersIter::One(None),
        }
    }

    /// Largest user key in `[prefix, before)` visible at `snapshot`.
    ///
    /// Reverse user-key walk of the prefix, then the newest version ≤ snapshot
    /// (same visibility as [`get_entry`]). `before` is an exclusive upper bound
    /// inside the prefix (retry after a cross-layer tombstone). `None` means
    /// `prefix_succ`.
    ///
    /// RFC-0054 P1.1: newest-first without materializing the prefix — the
    /// candidate is the max of the per-set maxima (`map` + each `tail_idx`
    /// shard; the sets partition the keyspace, so max-of-maxes is the global
    /// max), re-seeking with the candidate as `before` only when it is not
    /// visible. O(rounds × shards × log n) — the previous collect-sort-walk
    /// cloned EVERY version of the user into a `Vec` per call, which the
    /// shared 264k-entry memtable of the deps suite turned into the mvcc
    /// gap (probe: `last` 289 ns isolated → 1649 ns full).
    #[must_use]
    pub fn last_visible_under_prefix(
        &self,
        prefix: &[u8],
        snapshot: SequenceNumber,
        before: Option<&[u8]>,
    ) -> Option<(Bytes, Bytes)> {
        let prefix_end = crate::prefix::prefix_exclusive_end(prefix);
        let mut before_owned: Option<Bytes> = before.map(Bytes::copy_from_slice);
        // RFC-0154 P1.1: a `cf\0…` prefix can only live in that CF shard.
        let pin = prefix.iter().position(|&b| b == 0).map(|i| &prefix[..i]);
        loop {
            let end_b = match (before_owned.as_deref(), prefix_end.as_deref()) {
                (Some(b), Some(p)) if b < p => Bound::Excluded(b),
                (Some(b), None) => Bound::Excluded(b),
                (_, Some(p)) => Bound::Excluded(p),
                (_, None) => Bound::Unbounded,
            };
            // [prefix, end_b) contains exactly the keys that start with
            // `prefix` (F57 contract; `before` only shrinks it inside), so
            // no post-filter is needed — the max of these disjoint sets is
            // the newest candidate.
            let mut cand: Option<Bytes> = self
                .map
                .range::<[u8], _>((Bound::Included(prefix), end_b))
                .next_back()
                .map(|(uk, _)| uk.clone());
            let lo = packed_short_bound(Bound::Included(prefix));
            let hi = packed_short_bound(end_b);
            let short_ok = prefix.len() <= 32 && bound_len_le_32(end_b);
            let mut point_pfxs: Vec<Bytes> = Vec::new();
            let consider = |pfx: &Bytes,
                            shard: &TailShard,
                            cand: &mut Option<Bytes>,
                            point_pfxs: &mut Vec<Bytes>| {
                if short_ok {
                    if let Some((_, &i)) = shard.short.range((lo, hi)).next_back() {
                        let uk = &self.tail[i].key.user_key;
                        if cand.as_ref().is_none_or(|c| uk > c) {
                            *cand = Some(uk.clone());
                        }
                    }
                }
                if let Some((uk, _)) = shard
                    .long
                    .range::<[u8], _>((Bound::Included(prefix), end_b))
                    .next_back()
                {
                    if cand.as_ref().is_none_or(|c| uk > c) {
                        *cand = Some(uk.clone());
                    }
                }
                if short_ok && !shard.point.is_empty() {
                    point_pfxs.push(pfx.clone());
                }
            };
            if let Some(only) = pin {
                if let Some((pfx, shard)) = self.tail_idx.get_key_value(only) {
                    consider(pfx, shard, &mut cand, &mut point_pfxs);
                }
            } else {
                for (pfx, shard) in &self.tail_idx {
                    consider(pfx, shard, &mut cand, &mut point_pfxs);
                }
            }
            for pfx in point_pfxs {
                let Some(ord) = self.cached_point_ord(pfx.as_ref()) else {
                    continue;
                };
                let a = pack_bound_start(&ord, lo);
                let b = pack_bound_end(&ord, hi);
                if b > a {
                    let i = ord[b - 1].1;
                    if let Some(uk) = self.tail.get(i).map(|v| v.key.user_key.clone()) {
                        if cand.as_ref().is_none_or(|c| uk > *c) {
                            cand = Some(uk);
                        }
                    }
                }
            }
            let Some(uk) = cand else {
                return None;
            };
            if let Some((_, Lookup::Found(val))) = self.get_entry(&uk, snapshot) {
                return Some((uk, val));
            }
            before_owned = Some(uk);
        }
    }

    /// Rewrite every stored value with `f` (used by value-log GC remapping).
    pub fn map_values<F>(&mut self, mut f: F)
    where
        F: FnMut(&Bytes) -> Bytes,
    {
        self.approx_bytes = 0;
        self.cf_bytes.clear();
        for (uk, vers) in &mut self.map {
            for v in vers.iter_mut() {
                v.value = f(&v.value);
                let n = uk.len() + v.value.len() + 8;
                self.approx_bytes = self.approx_bytes.saturating_add(n);
                Self::bump_cf_bytes_map(&mut self.cf_bytes, uk.as_ref(), n, true);
            }
        }
        for v in &mut self.tail {
            v.value = f(&v.value);
            let n = v.key.user_key.len() + v.value.len() + 8;
            self.approx_bytes = self.approx_bytes.saturating_add(n);
            Self::bump_cf_bytes_map(&mut self.cf_bytes, v.key.user_key.as_ref(), n, true);
        }
    }

    /// Iterate user-visible entries in user-key order at `snapshot`.
    ///
    /// Yields `(user_key, value)` for each distinct user key that has a visible
    /// non-deleted version. Internal versions and tombstones are skipped.
    pub fn iter_snapshot(
        &self,
        snapshot: SequenceNumber,
    ) -> impl Iterator<Item = (Bytes, Bytes)> + '_ {
        self.range_snapshot(Bound::Unbounded, Bound::Unbounded, snapshot)
    }

    /// Range over user keys at `snapshot` (`start` / `end` are user-key bounds).
    pub fn range_snapshot<'a>(
        &'a self,
        start: Bound<&'a [u8]>,
        end: Bound<&'a [u8]>,
        snapshot: SequenceNumber,
    ) -> impl Iterator<Item = (Bytes, Bytes)> + 'a {
        let mut out = Vec::new();
        let mut last: Option<Bytes> = None;
        for (k, v) in self.iter_internal_iter_at(start, end, snapshot) {
            if k.sequence > snapshot {
                continue;
            }
            if last.as_ref().is_some_and(|u| u == &k.user_key) {
                continue;
            }
            last = Some(k.user_key.clone());
            if k.kind == ValueType::Value && !self.range_deleted(&k.user_key, k.sequence, snapshot)
            {
                out.push((k.user_key.clone(), v.clone()));
            }
        }
        out.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Bound;

    /// RFC-0044 P2.2 micro: deps_raftlog memtable floor — `insert_many`
    /// (tail append + tail_idx index) only, no WAL/Db/publish. Run:
    /// `cargo test -p pedradb-core --lib --release mem_insert_raftlog_micro -- --ignored --nocapture`
    /// `MEM_MICRO_OPS` sets ops/batch (default 16), `MEM_MICRO_N` batches.
    #[test]
    #[ignore]
    fn mem_insert_raftlog_micro() {
        let per: usize = std::env::var("MEM_MICRO_OPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(16);
        let n: u64 = std::env::var("MEM_MICRO_N")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(100_000);
        let mut mt = MemTable::new();
        let val = Bytes::from(vec![b'r'; 100]);
        let mut seq = 0u64;
        let t0 = std::time::Instant::now();
        for _ in 0..n {
            let mut items = Vec::with_capacity(per);
            for _ in 0..per {
                seq += 1;
                items.push((
                    InternalKey::new(format!("raftlog/{seq:08}"), seq, ValueType::Value),
                    val.clone(),
                ));
            }
            mt.insert_many(items);
        }
        let el = t0.elapsed();
        println!(
            "mem micro: {n} batches x {per} ops, {el:?} ({:.3} µs/batch, {:.4} µs/op) entries={} approx_kb={}",
            el.as_secs_f64() * 1e6 / n as f64,
            el.as_secs_f64() * 1e6 / (n as f64 * per as f64),
            mt.len(),
            mt.approx_memory_usage() / 1024,
        );
    }

    /// RFC-0054 P1.4 micro: deps_apply_batch memtable shape — per op two
    /// `insert_many` commits of 64 entries (prewrite: `lock\0u/N` put +
    /// `default\0` mvcc put; commit: `write\0` mvcc put + lock delete),
    /// 32 txns over 1024 users, ever-growing ts (mvcc keys are always
    /// fresh, lock keys repeat). Run:
    /// `cargo test -p pedradb-core --lib --release mem_insert_apply_micro -- --ignored --nocapture`
    /// `MEM_MICRO_N` sets ops (default 20000).
    #[test]
    #[ignore]
    fn mem_insert_apply_micro() {
        let txns: usize = 32;
        let users: u64 = 1024;
        let n: u64 = std::env::var("MEM_MICRO_N")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(20_000);
        let mut mt = MemTable::new();
        let val = Bytes::from(vec![b'd'; 100]);
        let mut ts = 0u64;
        let mut rng = 0x5EED_0001u64;
        let mut xorshift = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let t0 = std::time::Instant::now();
        let mut ins = std::time::Duration::ZERO;
        for _ in 0..n {
            let mut pre = Vec::with_capacity(txns * 2);
            let mut com = Vec::with_capacity(txns * 2);
            for _ in 0..txns {
                let u = xorshift() % users;
                ts += 1;
                let be = ts.to_be_bytes();
                let k = format!("u/{u:06}");
                let mut mv = k.clone().into_bytes();
                mv.extend_from_slice(&be);
                pre.push((
                    InternalKey::new(Bytes::from(format!("lock\0{k}")), ts, ValueType::Value),
                    Bytes::from_static(b"l"),
                ));
                let mut dk = format!("default\0").into_bytes();
                dk.extend_from_slice(&mv);
                pre.push((
                    InternalKey::new(Bytes::from(dk), ts, ValueType::Value),
                    val.clone(),
                ));
                let mut wk = format!("write\0").into_bytes();
                wk.extend_from_slice(&mv);
                com.push((
                    InternalKey::new(Bytes::from(wk), ts, ValueType::Value),
                    Bytes::from_static(b"c"),
                ));
                com.push((
                    InternalKey::new(Bytes::from(format!("lock\0{k}")), ts, ValueType::Deletion),
                    Bytes::new(),
                ));
            }
            let ti = std::time::Instant::now();
            mt.insert_many(pre);
            mt.insert_many(com);
            ins += ti.elapsed();
        }
        let el = t0.elapsed();
        println!(
            "mem apply micro: {n} ops x 2x64 entries, total {el:?} ({:.2} µs/op), insert {ins:?} ({:.2} µs/op, {:.4} µs/entry) entries={} approx_kb={}",
            el.as_secs_f64() * 1e6 / n as f64,
            ins.as_secs_f64() * 1e6 / n as f64,
            ins.as_secs_f64() * 1e6 / (n as f64 * 128.0),
            mt.len(),
            mt.approx_memory_usage() / 1024,
        );
    }

    #[test]
    fn put_get() {
        let mut mt = MemTable::new();
        mt.put(b"a".as_slice(), 1, b"va".as_slice());
        assert_eq!(mt.get(b"a", 1), Lookup::Found(Bytes::from_static(b"va")));
        assert_eq!(mt.get(b"a", 0), Lookup::NotFound);
        assert_eq!(mt.get(b"missing", 1), Lookup::NotFound);
    }

    #[test]
    fn newer_version_wins() {
        let mut mt = MemTable::new();
        mt.put(b"k".as_slice(), 1, b"old".as_slice());
        mt.put(b"k".as_slice(), 5, b"new".as_slice());
        assert_eq!(mt.get(b"k", 10), Lookup::Found(Bytes::from_static(b"new")));
        assert_eq!(mt.get(b"k", 3), Lookup::Found(Bytes::from_static(b"old")));
    }

    #[test]
    fn delete_hides_value() {
        let mut mt = MemTable::new();
        mt.put(b"k".as_slice(), 1, b"v".as_slice());
        mt.delete(b"k".as_slice(), 2);
        assert_eq!(mt.get(b"k", 2), Lookup::Deleted);
        assert_eq!(mt.get(b"k", 1), Lookup::Found(Bytes::from_static(b"v")));
    }

    #[test]
    fn put_after_delete() {
        let mut mt = MemTable::new();
        mt.put(b"k".as_slice(), 1, b"v1".as_slice());
        mt.delete(b"k".as_slice(), 2);
        mt.put(b"k".as_slice(), 3, b"v3".as_slice());
        assert_eq!(mt.get(b"k", 3), Lookup::Found(Bytes::from_static(b"v3")));
        assert_eq!(mt.get(b"k", 2), Lookup::Deleted);
    }

    #[test]
    fn iter_snapshot_skips_tombstones_and_old_versions() {
        let mut mt = MemTable::new();
        mt.put(b"a".as_slice(), 1, b"va".as_slice());
        mt.put(b"b".as_slice(), 1, b"vb".as_slice());
        mt.delete(b"b".as_slice(), 2);
        mt.put(b"c".as_slice(), 1, b"vc".as_slice());
        mt.put(b"c".as_slice(), 3, b"vc3".as_slice());

        let items: Vec<_> = mt.iter_snapshot(10).collect();
        assert_eq!(
            items,
            vec![
                (Bytes::from_static(b"a"), Bytes::from_static(b"va")),
                (Bytes::from_static(b"c"), Bytes::from_static(b"vc3")),
            ]
        );

        let at_1: Vec<_> = mt.iter_snapshot(1).collect();
        assert_eq!(at_1.len(), 3);
        assert_eq!(
            at_1[1],
            (Bytes::from_static(b"b"), Bytes::from_static(b"vb"))
        );
    }

    #[test]
    fn range_snapshot() {
        let mut mt = MemTable::new();
        for (k, v) in [("a", "1"), ("b", "2"), ("c", "3"), ("d", "4")] {
            mt.put(k.as_bytes(), 1, v.as_bytes());
        }
        let mid: Vec<_> = mt
            .range_snapshot(Bound::Included(b"b"), Bound::Excluded(b"d"), 1)
            .map(|(k, _)| k)
            .collect();
        assert_eq!(
            mid,
            vec![Bytes::from_static(b"b"), Bytes::from_static(b"c")]
        );
    }

    #[test]
    fn iter_internal_range_skips_outside_prefix() {
        let mut mt = MemTable::new();
        for k in [b"a" as &[u8], b"b", b"c", b"d", b"e"] {
            mt.put(k, 1, b"v".as_slice());
        }
        mt.put(b"b".as_slice(), 2, b"v2".as_slice());
        let got: Vec<&[u8]> = mt
            .iter_internal_range(Bound::Included(b"b"), Bound::Excluded(b"d"))
            .map(|(k, _)| k.user_key.as_ref())
            .collect();
        assert!(got.iter().all(|u| *u == b"b" || *u == b"c"), "{got:?}");
        assert_eq!(got.iter().filter(|u| **u == b"b").count(), 2);
        assert!(!mt.has_range_tombstones());
        mt.delete_range(b"a".as_slice(), b"z".as_slice(), 3);
        assert!(mt.has_range_tombstones());
    }

    #[test]
    fn insert_many_keeps_live_idx() {
        let mut mt = MemTable::new();
        let val = Bytes::from_static(b"v");
        let mut items = Vec::with_capacity(32);
        let mut written = Vec::with_capacity(32);
        for i in 0..32u32 {
            let mut k = b"default\0u/".to_vec();
            k.extend_from_slice(&i.to_be_bytes());
            written.push(k.clone());
            items.push((
                InternalKey::new(Bytes::from(k), u64::from(i) + 1, ValueType::Value),
                val.clone(),
            ));
        }
        mt.insert_many(items);
        let def = mt
            .tail_idx
            .get(&b"default"[..])
            .expect("default shard live");
        assert_eq!(def.point.len(), 32, "insert_many indexes default");
        assert!(def.short.is_empty());
        for k in &written {
            assert_eq!(mt.get(k, 32), Lookup::Found(Bytes::from_static(b"v")));
        }
        let mut items = Vec::with_capacity(32);
        for u in 0..32u32 {
            let mut k = b"write\0u/".to_vec();
            k.extend_from_slice(format!("{u:02}").as_bytes());
            items.push((
                InternalKey::new(Bytes::from(k), 100 + u64::from(u), ValueType::Value),
                Bytes::from_static(b"c"),
            ));
        }
        mt.insert_many(items);
        let def = mt
            .tail_idx
            .get(&b"default"[..])
            .expect("default shard still live");
        let wr = mt.tail_idx.get(&b"write"[..]).expect("write shard live");
        assert_eq!(def.point.len(), 32, "write inserts leave default idx live");
        assert_eq!(wr.short.len(), 32, "write CF has its own short idx");
        let n = mt
            .count_latest_in_range(
                Bound::Included(b"write\0u/00".as_slice()),
                Bound::Excluded(b"write\0u/32".as_slice()),
                32,
                200,
            )
            .expect("live write shard count");
        assert_eq!(n, 32);
        let write = mt.tail_idx.get(&b"write"[..]).expect("write shard live");
        assert_eq!(write.short.len(), 32, "write stays ordered (RFC-0154 P1.2)");
        assert!(write.point.is_empty());
    }

    /// kvrocks/YCSB default-raw: no `cf\0`. Count uses the empty-prefix
    /// live BTree (Some). P1.3 HashMap here lost `ycsb_c` on CHV.
    #[test]
    fn insert_many_empty_prefix_count_matches_gets() {
        let mut mt = MemTable::new();
        let val = Bytes::from_static(b"x");
        let mut keys = Vec::with_capacity(64);
        let mut items = Vec::with_capacity(64);
        for i in 0..64u32 {
            let k = format!("k{i:04}").into_bytes();
            keys.push(k.clone());
            items.push((
                InternalKey::new(Bytes::from(k), u64::from(i) + 1, ValueType::Value),
                val.clone(),
            ));
        }
        mt.insert_many(items);
        assert_eq!(mt.tail_idx.len(), 1, "raw keys share the empty shard");
        let empty = mt.tail_idx.get(&b""[..]).expect("empty prefix shard");
        assert_eq!(empty.short.len(), 64);
        assert!(empty.point.is_empty());
        let start = keys[10].as_slice();
        let end = keys[35].as_slice();
        let n = mt
            .count_latest_in_range(Bound::Included(start), Bound::Excluded(end), 25, 64)
            .expect("empty-prefix live count (None would be the O(n) fallback)");
        let mut oracle = 0usize;
        for k in &keys {
            if k.as_slice() >= start && k.as_slice() < end {
                assert_eq!(mt.get(k, 64), Lookup::Found(Bytes::from_static(b"x")));
                oracle += 1;
                if oracle == 25 {
                    break;
                }
            }
        }
        assert_eq!(n, oracle);
        assert_eq!(n, 25);
    }

    #[test]
    fn last_visible_pins_write_shard_after_apply() {
        let mut mt = MemTable::new();
        let mut items = Vec::with_capacity(96);
        for u in 0..32u32 {
            let mut lock = b"lock\0u/".to_vec();
            lock.extend_from_slice(format!("{u:02}").as_bytes());
            items.push((
                InternalKey::new(Bytes::from(lock), u64::from(u) + 1, ValueType::Value),
                Bytes::from_static(b"l"),
            ));
            let mut def = b"default\0u/".to_vec();
            def.extend_from_slice(format!("{u:02}").as_bytes());
            items.push((
                InternalKey::new(Bytes::from(def), 50 + u64::from(u), ValueType::Value),
                Bytes::from_static(b"v"),
            ));
            let mut w = b"write\0u/".to_vec();
            w.extend_from_slice(format!("{u:02}").as_bytes());
            items.push((
                InternalKey::new(Bytes::from(w), 100 + u64::from(u), ValueType::Value),
                Bytes::from_static(b"c"),
            ));
        }
        mt.insert_many(items);
        assert_eq!(mt.tail_idx.len(), 3);
        let (k, v) = mt
            .last_visible_under_prefix(b"write\0u/10", 200, None)
            .expect("write prefix lives in the write shard");
        assert!(k.starts_with(b"write\0u/10"), "{k:?}");
        assert_eq!(v, Bytes::from_static(b"c"));
        assert!(mt
            .last_visible_under_prefix(b"write\0z", 200, None)
            .is_none());
        let w = mt.tail_idx.get(&b"write"[..]).expect("write");
        assert!(w.point.is_empty() && w.short.len() == 32);
    }

    /// RFC-0154 P1.2: reverse-seek on `write` uses the BTree, not a HashMap
    /// sort. Oracle = `get` of the keys we wrote.
    #[test]
    fn write_cf_ordered_last_and_count_match_gets() {
        let mut mt = MemTable::new();
        let mut items = Vec::with_capacity(64);
        let mut keys = Vec::with_capacity(64);
        for u in 0..64u32 {
            let mut k = b"write\0u/".to_vec();
            k.extend_from_slice(format!("{u:02}").as_bytes());
            keys.push(k.clone());
            items.push((
                InternalKey::new(Bytes::from(k), u64::from(u) + 1, ValueType::Value),
                Bytes::from_static(b"c"),
            ));
        }
        mt.insert_many(items);
        let w = mt.tail_idx.get(&b"write"[..]).expect("write shard");
        assert_eq!(w.short.len(), 64);
        assert!(w.point.is_empty(), "write must not be HashMap");
        let start = keys[10].as_slice();
        let end = keys[35].as_slice();
        let n = mt
            .count_latest_in_range(Bound::Included(start), Bound::Excluded(end), 25, 64)
            .expect("ordered write count");
        let mut oracle = 0usize;
        for k in &keys {
            if k.as_slice() >= start && k.as_slice() < end {
                assert_eq!(mt.get(k, 64), Lookup::Found(Bytes::from_static(b"c")));
                oracle += 1;
                if oracle == 25 {
                    break;
                }
            }
        }
        assert_eq!(n, oracle);
        let (last, val) = mt
            .last_visible_under_prefix(b"write\0u/10", 64, None)
            .expect("reverse-seek write");
        assert!(last.starts_with(b"write\0u/10"), "{last:?}");
        assert_eq!(val, Bytes::from_static(b"c"));
        assert_eq!(mt.get(&last, 64), Lookup::Found(Bytes::from_static(b"c")));
    }

    #[test]
    fn insert_many_matches_insert() {
        let mut a = MemTable::new();
        let mut b = MemTable::new();
        let items = [
            (
                InternalKey::new(Bytes::from_static(b"k0"), 1, ValueType::Value),
                Bytes::from_static(b"v0"),
            ),
            (
                InternalKey::new(Bytes::from_static(b"k1"), 2, ValueType::Value),
                Bytes::from_static(b"v1"),
            ),
            (
                InternalKey::new(Bytes::from_static(b"k2"), 3, ValueType::Value),
                Bytes::from_static(b"v2"),
            ),
        ];
        for (k, v) in items.clone() {
            a.insert(k, v);
        }
        b.insert_many(items);
        assert_eq!(a.len(), b.len());
        assert_eq!(a.get(b"k1", 3), b.get(b"k1", 3));
        assert_eq!(a.approx_memory_usage(), b.approx_memory_usage());
    }

    #[test]
    fn approx_memory_grows() {
        let mut mt = MemTable::new();
        assert_eq!(mt.approx_memory_usage(), 0);
        mt.put(b"hello".as_slice(), 1, b"world".as_slice());
        assert!(mt.approx_memory_usage() >= 5 + 5 + 8);
        assert_eq!(mt.len(), 1);
        assert!(!mt.is_empty());
    }

    #[test]
    fn absorb_moves_versions_into_one_table() {
        let mut a = MemTable::new();
        a.put(b"a".as_slice(), 1, b"va".as_slice());
        let mut b = MemTable::new();
        b.put(b"b".as_slice(), 2, b"vb".as_slice());
        a.absorb(b);
        assert_eq!(a.len(), 2);
        assert_eq!(a.get(b"a", 2), Lookup::Found(Bytes::from_static(b"va")));
        assert_eq!(a.get(b"b", 2), Lookup::Found(Bytes::from_static(b"vb")));
    }

    #[test]
    fn last_visible_under_prefix_skips_deleted_tail() {
        let mut mt = MemTable::new();
        mt.put(b"p/a".as_slice(), 1, b"va".as_slice());
        mt.put(b"p/b".as_slice(), 1, b"vb".as_slice());
        mt.put(b"p/c".as_slice(), 1, b"vc".as_slice());
        mt.delete(b"p/c".as_slice(), 2);
        let (k, v) = mt
            .last_visible_under_prefix(b"p/", 2, None)
            .expect("live key under prefix");
        assert_eq!(&k[..], b"p/b");
        assert_eq!(&v[..], b"vb");
        // Newer deletion is invisible at seq=1.
        let (k1, v1) = mt
            .last_visible_under_prefix(b"p/", 1, None)
            .expect("old snapshot");
        assert_eq!(&k1[..], b"p/c");
        assert_eq!(&v1[..], b"vc");
    }

    #[test]
    fn last_visible_under_prefix_newest_version_not_older() {
        let mut mt = MemTable::new();
        mt.put(b"u/1".as_slice(), 1, b"v1".as_slice());
        mt.put(b"u/1".as_slice(), 2, b"v2".as_slice());
        mt.put(b"u/1".as_slice(), 3, b"v3".as_slice());
        mt.put(b"u/2".as_slice(), 4, b"other".as_slice());
        let (k, v) = mt
            .last_visible_under_prefix(b"u/1", 10, None)
            .expect("latest of u/1");
        assert_eq!(&k[..], b"u/1");
        assert_eq!(&v[..], b"v3");
        let (_, mid) = mt
            .last_visible_under_prefix(b"u/1", 2, None)
            .expect("mid snapshot");
        assert_eq!(&mid[..], b"v2");
    }

    #[test]
    fn last_visible_under_prefix_respects_before() {
        let mut mt = MemTable::new();
        mt.put(b"p/a".as_slice(), 1, b"va".as_slice());
        mt.put(b"p/b".as_slice(), 1, b"vb".as_slice());
        mt.put(b"p/c".as_slice(), 1, b"vc".as_slice());
        let (k, _) = mt
            .last_visible_under_prefix(b"p/", 1, Some(b"p/c"))
            .expect("before p/c");
        assert_eq!(&k[..], b"p/b");
        assert!(mt
            .last_visible_under_prefix(b"p/", 1, Some(b"p/a"))
            .is_none());
    }

    #[test]
    fn replace_same_internal_key_updates_value() {
        let mut mt = MemTable::new();
        mt.put(b"k".as_slice(), 1, b"old".as_slice());
        mt.put(b"k".as_slice(), 1, b"new".as_slice());
        assert_eq!(mt.len(), 1);
        assert_eq!(mt.get(b"k", 1), Lookup::Found(Bytes::from_static(b"new")));
    }

    #[test]
    fn get_entry_borrowed_same_as_versions() {
        let mut mt = MemTable::new();
        mt.put(b"user/1".as_slice(), 1, b"a".as_slice());
        mt.put(b"user/1".as_slice(), 3, b"c".as_slice());
        mt.put(b"user/2".as_slice(), 2, b"b".as_slice());
        assert_eq!(mt.get_entry(b"user/1", 10).map(|(s, _)| s), Some(3));
        assert_eq!(mt.get_entry(b"user/1", 2).map(|(s, _)| s), Some(1));
        assert!(mt.get_entry(b"nope", 10).is_none());
    }

    #[test]
    fn gc_floor_keeps_exact_reads_at_or_above() {
        let mut mt = MemTable::new();
        for seq in 1..=8 {
            mt.put(b"hot".as_slice(), seq, format!("v{seq}"));
        }
        mt.spill_tail_with_gc(Some(5));
        // Keep-set: {seq > 5} ∪ {newest ≤ 5} = {6, 7, 8, 5}
        assert_eq!(mt.len(), 4);
        for (seq, want) in [(5u64, "v5"), (6, "v6"), (7, "v7"), (8, "v8"), (100, "v8")] {
            assert_eq!(
                mt.get(b"hot", seq),
                Lookup::Found(Bytes::from_static(want.as_bytes())),
                "read at {seq}"
            );
        }
        assert_eq!(mt.approx_memory_usage() > 0, true);
    }

    #[test]
    fn gc_floor_none_keeps_everything() {
        let mut mt = MemTable::new();
        for seq in 1..=8 {
            mt.put(b"hot".as_slice(), seq, format!("v{seq}"));
        }
        mt.spill_tail_with_gc(None);
        assert_eq!(mt.len(), 8);
        assert_eq!(mt.get(b"hot", 1), Lookup::Found(Bytes::from_static(b"v1")));
    }

    #[test]
    fn gc_collapses_pair_back_to_one() {
        let mut mt = MemTable::new();
        mt.put(b"k".as_slice(), 1, b"old".as_slice());
        mt.spill_tail();
        mt.put(b"k".as_slice(), 2, b"new".as_slice());
        mt.spill_tail_with_gc(Some(2));
        // newest-≤-2 = v2; v1 dropped → back to Versions::One
        assert!(matches!(
            mt.map.get(b"k".as_slice()),
            Some(Versions::One(_))
        ));
        assert_eq!(mt.len(), 1);
        assert_eq!(mt.get(b"k", 2), Lookup::Found(Bytes::from_static(b"new")));
    }

    #[test]
    fn absorb_with_floor_merges_and_gcs() {
        let mut a = MemTable::new();
        for seq in 1..=4 {
            a.put(b"k".as_slice(), seq, format!("a{seq}"));
        }
        a.spill_tail();
        let mut b = MemTable::new();
        for seq in 5..=9 {
            b.put(b"k".as_slice(), seq, format!("b{seq}"));
        }
        b.spill_tail();
        let a_len = a.len();
        a.absorb_with_floor(b, Some(7));
        // keep {> 7} ∪ {newest ≤ 7} = {8, 9, 7}
        assert_eq!(a.len(), 3, "had {a_len}");
        assert_eq!(a.get(b"k", 7), Lookup::Found(Bytes::from_static(b"b7")));
        assert_eq!(a.get(b"k", 9), Lookup::Found(Bytes::from_static(b"b9")));
        assert_eq!(a.get(b"k", 100), Lookup::Found(Bytes::from_static(b"b9")));
    }

    #[test]
    fn hot_key_overwrite_fold_is_not_quadratic() {
        // Pre-fix shape: newest-first Vec insert landed at index 0 → one
        // memmove per version (one full core of `insert_map` memmove in the
        // 2026-08-22 ycsb_a profile). 40k versions on one key must fold fast.
        let mut a = MemTable::new();
        for seq in 1..=20_000u64 {
            a.put(b"hot".as_slice(), seq, b"x".as_slice());
        }
        let mut b = MemTable::new();
        for seq in 20_001..=40_000u64 {
            b.put(b"hot".as_slice(), seq, b"x".as_slice());
        }
        let t0 = std::time::Instant::now();
        use std::time::Duration;
        a.absorb(b);
        let el = t0.elapsed();
        assert_eq!(a.len(), 40_000);
        assert_eq!(
            a.get(b"hot", 40_000),
            Lookup::Found(Bytes::from_static(b"x"))
        );
        assert!(
            el < Duration::from_millis(250),
            "absorb of 40k hot-key versions took {el:?} — front-insert regressed"
        );
    }

    #[test]
    fn first_put_stays_one_then_promotes() {
        let mut mt = MemTable::new();
        mt.put(b"k".as_slice(), 1, b"v1".as_slice());
        mt.spill_tail();
        assert!(matches!(
            mt.map.get(b"k".as_slice()),
            Some(Versions::One(_))
        ));
        mt.put(b"k".as_slice(), 3, b"v3".as_slice());
        mt.put(b"k".as_slice(), 2, b"v2".as_slice());
        mt.spill_tail();
        match mt.map.get(b"k".as_slice()) {
            Some(Versions::Many(vs)) => {
                assert_eq!(vs.len(), 3);
                assert_eq!(vs[0].key.sequence, 3);
                assert_eq!(vs[1].key.sequence, 2);
                assert_eq!(vs[2].key.sequence, 1);
            }
            other => panic!("expected Many, got {other:?}"),
        }
        assert_eq!(mt.get(b"k", 10), Lookup::Found(Bytes::from_static(b"v3")));
        assert_eq!(mt.get(b"k", 2), Lookup::Found(Bytes::from_static(b"v2")));
    }

    #[test]
    fn get_sees_tail_before_spill() {
        let mut mt = MemTable::new();
        mt.put(b"a".as_slice(), 1, b"va".as_slice());
        mt.put(b"b".as_slice(), 2, b"vb".as_slice());
        assert!(mt.has_tail());
        assert!(mt.map.is_empty());
        assert_eq!(mt.get(b"a", 2), Lookup::Found(Bytes::from_static(b"va")));
        assert_eq!(mt.get(b"b", 2), Lookup::Found(Bytes::from_static(b"vb")));
        mt.spill_tail();
        assert!(!mt.has_tail());
        assert_eq!(mt.get(b"a", 2), Lookup::Found(Bytes::from_static(b"va")));
        assert_eq!(mt.len(), 2);
    }

    #[test]
    fn tail_delete_after_spill_hides_map_put() {
        let mut mt = MemTable::new();
        mt.put(b"k".as_slice(), 1, b"v".as_slice());
        mt.spill_tail();
        mt.delete(b"k".as_slice(), 2);
        assert_eq!(mt.get(b"k", 2), Lookup::Deleted);
        let snap: Vec<_> = mt.iter_snapshot(2).collect();
        assert!(snap.is_empty(), "{snap:?}");
        let (k, v) = mt
            .last_visible_under_prefix(b"k", 2, None)
            .map_or((Bytes::new(), Bytes::new()), |x| x);
        assert!(
            mt.last_visible_under_prefix(b"k", 2, None).is_none(),
            "deleted key still visible as {k:?}={v:?}"
        );
    }

    #[test]
    fn same_seq_replace_after_spill_prefers_tail() {
        let mut mt = MemTable::new();
        mt.put(b"k".as_slice(), 1, b"old".as_slice());
        mt.spill_tail();
        mt.put(b"k".as_slice(), 1, b"new".as_slice());
        assert_eq!(mt.get(b"k", 1), Lookup::Found(Bytes::from_static(b"new")));
    }

    #[test]
    fn fat_apply_stays_in_tail() {
        let mut mt = MemTable::new();
        for i in 0..64u32 {
            mt.put(
                Bytes::copy_from_slice(&i.to_le_bytes()),
                u64::from(i) + 1,
                b"v".as_slice(),
            );
        }
        assert!(mt.has_tail());
        assert!(mt.map.is_empty());
        assert_eq!(mt.len(), 64);
        assert_eq!(
            mt.get(&1u32.to_le_bytes(), 64),
            Lookup::Found(Bytes::from_static(b"v"))
        );
    }

    #[test]
    fn tail_index_get_does_not_need_spill() {
        let mut mt = MemTable::new();
        for i in 0..2000u32 {
            mt.put(
                Bytes::copy_from_slice(&i.to_le_bytes()),
                u64::from(i) + 1,
                b"v".as_slice(),
            );
        }
        assert!(mt.has_tail());
        assert!(mt.map.is_empty());
        assert_eq!(mt.len(), 2000);
        assert_eq!(
            mt.get(&0u32.to_le_bytes(), 2000),
            Lookup::Found(Bytes::from_static(b"v"))
        );
        assert_eq!(
            mt.get(&1999u32.to_le_bytes(), 2000),
            Lookup::Found(Bytes::from_static(b"v"))
        );
        let (k, v) = mt
            .last_visible_under_prefix(&1999u32.to_le_bytes(), 2000, None)
            .expect("indexed last");
        assert_eq!(&k[..], &1999u32.to_le_bytes());
        assert_eq!(&v[..], b"v");
    }

    #[test]
    fn cf_sharded_tail_idx_isolates_lookups() {
        // RFC-0054: lock\\0* keys must not sit in the raftlog shard.
        let mut mt = MemTable::new();
        for i in 0..5000u32 {
            let mut k = b"lock\0".to_vec();
            k.extend_from_slice(&i.to_le_bytes());
            mt.put(k, u64::from(i) + 1, b"L".as_slice());
        }
        mt.put(b"raftlog\0x".as_slice(), 9000, b"R".as_slice());
        assert_eq!(
            mt.get(b"raftlog\0x", 9000),
            Lookup::Found(Bytes::from_static(b"R"))
        );
        assert_eq!(
            mt.get(b"lock\0\x00\x00\x00\x00", 9000),
            Lookup::Found(Bytes::from_static(b"L"))
        );
        assert_eq!(mt.get(b"raftlog\0missing", 9000), Lookup::NotFound);
        assert_eq!(mt.tail_idx.len(), 2, "lock + raftlog shards");
        let fams = mt.cf_families();
        assert_eq!(fams, vec!["lock".to_string(), "raftlog".to_string()]);
    }

    #[test]
    fn cf_family_of_raw_and_prefixed() {
        assert_eq!(cf_family_of(b"aaa"), "default");
        assert_eq!(cf_family_of(b"default\0k"), "default");
        assert_eq!(cf_family_of(b"lock\0k"), "lock");
        assert!(key_in_cf_family(b"aaa", "default"));
        assert!(key_in_cf_family(b"default\0k", "default"));
        assert!(!key_in_cf_family(b"lock\0k", "default"));
        assert!(key_in_cf_family(b"lock\0k", "lock"));
        assert_eq!(infer_sst_cf(Some(b"lock\0a"), Some(b"lock\0z")), "lock");
        assert_eq!(infer_sst_cf(Some(b"aaa"), Some(b"lock\0z")), "");
        assert_eq!(infer_sst_cf(Some(b"aaa"), Some(b"zzz")), "default");
    }

    #[test]
    fn take_family_keeps_other_cf_and_bytes() {
        let mut mt = MemTable::new();
        mt.put(b"lock\0a".as_slice(), 1, b"L".as_slice());
        mt.put(b"default\0a".as_slice(), 2, b"D".as_slice());
        mt.put(b"default\0b".as_slice(), 3, b"E".as_slice());
        let lock = mt.take_family("lock");
        assert_eq!(
            lock.get(b"lock\0a", 3),
            Lookup::Found(Bytes::from_static(b"L"))
        );
        assert_eq!(mt.get(b"lock\0a", 3), Lookup::NotFound);
        assert_eq!(
            mt.get(b"default\0a", 3),
            Lookup::Found(Bytes::from_static(b"D"))
        );
        assert!(mt.approx_memory_usage_cf("lock") == 0);
        assert!(mt.approx_memory_usage_cf("default") > 0);
        assert!(lock.approx_memory_usage_cf("lock") > 0);
    }

    #[test]
    fn take_family_leaves_other_cf() {
        let mut mt = MemTable::new();
        mt.insert(
            InternalKey::new(Bytes::from_static(b"lock\0a"), 1, ValueType::Value),
            Bytes::from_static(b"L"),
        );
        mt.insert(
            InternalKey::new(Bytes::from_static(b"default\0a"), 2, ValueType::Value),
            Bytes::from_static(b"D"),
        );
        assert!(mt.approx_memory_usage_cf("lock") > 0);
        assert!(mt.approx_memory_usage_cf("default") > 0);
        let lock = mt.take_family("lock");
        assert_eq!(
            lock.get(b"lock\0a", 10),
            Lookup::Found(Bytes::from_static(b"L"))
        );
        assert_eq!(
            mt.get(b"default\0a", 10),
            Lookup::Found(Bytes::from_static(b"D"))
        );
        assert_eq!(mt.get(b"lock\0a", 10), Lookup::NotFound);
        assert_eq!(lock.approx_memory_usage_cf("default"), 0);
    }

    #[test]
    fn tail_idx_range_spans_shards_for_prefix_bounds() {
        // F219: a prefix query's bounds land in two CF shards (`["u/03",
        // "u/04")`); the sharded index must not drop the prefix's tail keys.
        let mut mt = MemTable::new();
        let key = |u: u32, v: u64| {
            let mut k = format!("u/{u:02}").into_bytes();
            k.extend_from_slice(&v.to_be_bytes());
            k
        };
        for u in 0..=4u32 {
            for v in 1..=3u64 {
                mt.put(key(u, v), v, b"x".as_slice());
            }
        }
        let (k, _) = mt
            .last_visible_under_prefix(b"u/03", 99, None)
            .expect("u/03 lives in the tail");
        assert!(k.starts_with(b"u/03"), "{k:?}");
        assert_eq!(&k[k.len() - 8..], &3u64.to_be_bytes(), "newest version");
        // Point range across shards still sees only in-range keys.
        assert!(mt.last_visible_under_prefix(b"u/77", 99, None).is_none());
    }

    #[test]
    fn point_hash_write_count_and_last() {
        let mut mt = MemTable::new();
        let mut seq = 0u64;
        for u in 0..40u32 {
            let mut k = b"write\0u/".to_vec();
            k.extend_from_slice(format!("{u:02}").as_bytes());
            seq += 1;
            mt.put(k, seq, b"c".as_slice());
            let mut d = b"default\0u/".to_vec();
            d.extend_from_slice(format!("{u:02}").as_bytes());
            seq += 1;
            mt.put(d, seq, b"v".as_slice());
        }
        let write = mt.tail_idx.get(&b"write"[..]).expect("write shard");
        assert!(!write.short.is_empty() && write.point.is_empty());
        let def = mt.tail_idx.get(&b"default"[..]).expect("default shard");
        assert!(!def.point.is_empty() && def.short.is_empty());
        let start = b"write\0u/10".as_slice();
        let end = b"write\0u/35".as_slice();
        let n = mt
            .count_latest_in_range(Bound::Included(start), Bound::Excluded(end), 25, seq)
            .expect("write short-tree count");
        assert_eq!(n, 25, "u/10 .. u/34 is 25 keys");
        let (k, _) = mt
            .last_visible_under_prefix(b"write\0u/10", seq, None)
            .expect("last under write prefix");
        assert!(k.starts_with(b"write\0u/10"), "{k:?}");
        assert_eq!(
            mt.get(b"default\0u/00", seq),
            Lookup::Found(Bytes::from_static(b"v"))
        );
    }

    #[test]
    fn count_latest_in_range_matches_idx_walk() {
        let mut mt = MemTable::new();
        let mut seq = 0u64;
        for u in 0..40u32 {
            let mut k = b"write\0u/".to_vec();
            k.extend_from_slice(format!("{u:02}").as_bytes());
            seq += 1;
            mt.put(k, seq, b"c".as_slice());
        }
        let start = b"write\0u/10".as_slice();
        let end = b"write\0u/35".as_slice();
        let n = mt
            .count_latest_in_range(Bound::Included(start), Bound::Excluded(end), 25, seq)
            .expect("tail-only latest count");
        assert_eq!(n, 25, "u/10 .. u/34 is 25 keys");
        let n2 = mt
            .count_latest_in_range(Bound::Included(start), Bound::Excluded(end), 8, seq)
            .unwrap();
        assert_eq!(n2, 8);
    }

    /// RFC-0149 P2.1: `(pack32, len)` for keys ≤ 32 bytes equals raw byte
    /// order (zero-pad ties broken by len, not a `Bytes` clone).
    #[test]
    fn tail_idx_short_pack_len_order_matches_bytes() {
        let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let mut keys: Vec<Bytes> = Vec::new();
        for len in 0..=32usize {
            for variant in 0..3u64 {
                let mut k = vec![0u8; len];
                for b in k.iter_mut() {
                    *b = match variant {
                        0 => ((next() % 5) as u8).saturating_sub(1),
                        1 => (next() % 256) as u8,
                        _ => b'a' + ((next() % 3) as u8),
                    };
                }
                keys.push(Bytes::from(k));
            }
        }
        keys.sort();
        keys.dedup();
        let mut packed: BTreeMap<(u128, u128, u16), usize> = BTreeMap::new();
        let mut plain: BTreeMap<Bytes, usize> = BTreeMap::new();
        for (i, k) in keys.iter().enumerate() {
            let (p0, p1) = pack32(k);
            packed.insert((p0, p1, k.len() as u16), i);
            plain.insert(k.clone(), i);
        }
        let packed_order: Vec<usize> = packed.values().copied().collect();
        let plain_order: Vec<usize> = plain.values().copied().collect();
        assert_eq!(
            packed_order, plain_order,
            "short-key total order must match"
        );
        let mut mt = MemTable::new();
        for (i, k) in keys.iter().enumerate() {
            mt.put(k.clone(), i as u64 + 1, b"v".as_slice());
        }
        for k in &keys {
            assert_eq!(
                mt.get(k.as_ref(), 10_000),
                Lookup::Found(Bytes::from_static(b"v")),
                "short-key get {k:?}"
            );
        }
        let long = Bytes::from(vec![b'x'; 40]);
        mt.put(long.clone(), 99, b"L".as_slice());
        assert_eq!(
            mt.get(long.as_ref(), 10_000),
            Lookup::Found(Bytes::from_static(b"L"))
        );
    }

    /// RFC-0054 P1.4: `(pack32, key)` shard order must equal raw byte order
    /// for every key shape — zero-padded prefixes, embedded `0x00`, shared
    /// 32-byte prefixes, and length ties across the 32-byte boundary.
    #[test]
    fn tail_idx_packed_order_matches_bytes_oracle() {
        let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let mut keys: Vec<Bytes> = Vec::new();
        // Adversarial shapes: shared prefixes, zeros, lengths around 32.
        for len in 0..=40usize {
            for variant in 0..3u64 {
                let mut k = vec![0u8; len];
                for (i, b) in k.iter_mut().enumerate() {
                    *b = match variant {
                        0 => ((next() % 5) as u8).saturating_sub(1), // many zeros
                        1 => (next() % 256) as u8,
                        _ => b'a' + ((next() % 3) as u8),
                    };
                    let _ = i;
                }
                if variant == 2 && !k.is_empty() {
                    k[0] = b'z';
                }
                keys.push(Bytes::from(k));
            }
            // A 32-byte-boundary prefix extension of the previous key.
            if let Some(last) = keys.last().cloned() {
                let mut ext = last.to_vec();
                ext.push(0);
                keys.push(Bytes::from(ext));
            }
        }
        keys.sort();
        keys.dedup();
        let mut packed: BTreeMap<TailIdxKey, usize> = BTreeMap::new();
        let mut plain: BTreeMap<Bytes, usize> = BTreeMap::new();
        for (i, k) in keys.iter().enumerate() {
            let (p0, p1) = pack32(k);
            packed.insert((p0, p1, k.clone()), i);
            plain.insert(k.clone(), i);
        }
        let packed_order: Vec<&Bytes> = packed.keys().map(|(_, _, k)| k).collect();
        let plain_order: Vec<&Bytes> = plain.keys().collect();
        assert_eq!(packed_order, plain_order, "total order must match");
        // Range equivalence on random byte bounds (incl. bounds not in set).
        for _ in 0..200 {
            let a = &keys[(next() as usize) % keys.len()];
            let b = &keys[(next() as usize) % keys.len()];
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            let p: Vec<&Bytes> = packed
                .range((
                    packed_bound(Bound::Included(lo)),
                    packed_bound(Bound::Excluded(hi)),
                ))
                .map(|((_, _, k), _)| k)
                .collect();
            let q: Vec<&Bytes> = plain
                .range::<Bytes, _>((Bound::Included(lo), Bound::Excluded(hi)))
                .map(|(k, _)| k)
                .collect();
            assert_eq!(p, q, "range [{lo:?}, {hi:?}) must match");
        }
    }

    #[test]
    fn merge_iter_sees_map_and_tail_in_internal_order() {
        let mut mt = MemTable::new();
        for k in [b"a" as &[u8], b"b", b"c", b"d"] {
            mt.put(k, 1, b"v1".as_slice());
        }
        mt.spill_tail();
        mt.put(b"b".as_slice(), 2, b"v2".as_slice());
        mt.put(b"e".as_slice(), 3, b"v3".as_slice());
        let got: Vec<(Vec<u8>, u64)> = mt
            .iter_internal_range(Bound::Included(b"b"), Bound::Excluded(b"e"))
            .map(|(k, _)| (k.user_key.to_vec(), k.sequence))
            .collect();
        assert_eq!(
            got,
            vec![
                (b"b".to_vec(), 2),
                (b"b".to_vec(), 1),
                (b"c".to_vec(), 1),
                (b"d".to_vec(), 1),
            ]
        );
        let snap: Vec<_> = mt.iter_snapshot(10).map(|(k, v)| (k, v)).collect();
        assert_eq!(
            snap.last().map(|(k, v)| (&k[..], &v[..])),
            Some((&b"e"[..], &b"v3"[..]))
        );
    }

    #[test]
    fn idx_range_walk_matches_spilled_on_latest_snapshot() {
        let mut mt = MemTable::new();
        // Everything in the tail (parked apply table — no spill on stage).
        for i in 0..2000u32 {
            mt.put(
                Bytes::copy_from_slice(&i.to_be_bytes()),
                u64::from(i) + 1,
                b"v".as_slice(),
            );
        }
        assert!(mt.has_tail() && mt.map.is_empty());
        let got: Vec<Vec<u8>> = mt
            .range_snapshot(
                Bound::Included(&100u32.to_be_bytes()),
                Bound::Excluded(&130u32.to_be_bytes()),
                2000,
            )
            .map(|(k, _)| k.to_vec())
            .collect();
        let expect: Vec<Vec<u8>> = (100u32..130).map(|i| i.to_be_bytes().to_vec()).collect();
        assert_eq!(got, expect);
        // Snapshot below the tail max falls back and still sees old versions.
        mt.put(
            Bytes::copy_from_slice(&150u32.to_be_bytes()),
            5000,
            b"new".as_slice(),
        );
        let old: Vec<_> = mt
            .range_snapshot(
                Bound::Included(&150u32.to_be_bytes()),
                Bound::Excluded(&151u32.to_be_bytes()),
                2000,
            )
            .collect();
        assert_eq!(old.len(), 1);
        assert_eq!(&old[0].1[..], b"v");
    }
}
