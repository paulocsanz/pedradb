//! In-memory versioned key-value buffer (MemTable).
//!
//! Holds puts and deletions as [`InternalKey`] entries until flushed to SST
//! (later) or rebuilt from the WAL on recovery. Point lookups and ranges honor
//! a **snapshot sequence**: only entries with `sequence <= snapshot` are visible;
//! the newest such entry wins (delete tombstone hides the key).

use std::cmp::Ordering;
use std::collections::{BTreeMap, VecDeque};
use std::ops::Bound;
use std::sync::{Arc, Mutex};

use bytes::Bytes;

use crate::key::{InternalKey, SequenceNumber, ValueType};

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
    users: std::collections::btree_map::Range<'a, Bytes, Versions>,
    cur: VersIter<'a>,
}

/// Merge of the sorted BTree with a **sorted tail** (O(tail log tail), not
/// O((map+tail) log) — YCSB E/scan must not sort the whole memtable).
pub(crate) struct MemInternalMerge<'a> {
    map: std::iter::Peekable<MemInternalRange<'a>>,
    tail: std::iter::Peekable<std::vec::IntoIter<(&'a InternalKey, &'a Bytes)>>,
}

/// Snapshot-aware merge: BTree range + `tail_idx` range (newest tail version
/// per user key). O(log n + hits) — a parked 4 MiB tail must not be walked
/// per count/scan (RFC-0041 deps_scan regression).
pub(crate) struct MemInternalIdx<'a> {
    map: std::iter::Peekable<MemInternalRange<'a>>,
    idx: std::iter::Peekable<std::collections::btree_map::Range<'a, Bytes, usize>>,
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
    tail_idx: BTreeMap<Bytes, BTreeMap<Bytes, usize>>,
    /// Always empty — missing-shard `range` needs a `btree_map::Range`.
    empty_idx: BTreeMap<Bytes, usize>,
    /// Highest sequence in `tail` (fast-path guard: snapshot ≥ it ⇒ only the
    /// newest version per key can be visible).
    tail_max_seq: SequenceNumber,
    /// Cached InternalKey order of `tail` (invalidated on insert).
    tail_ord: Mutex<Option<Arc<Vec<u32>>>>,
    /// Approximate bytes for flush triggers (user key + value + trailer).
    approx_bytes: usize,
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
            empty_idx: BTreeMap::new(),
            tail_max_seq: self.tail_max_seq,
            tail_ord: Mutex::new(None),
            approx_bytes: self.approx_bytes,
            range_tombstones: self.range_tombstones,
            entries: self.entries,
        }
    }
}

/// Compat CF encoding is `cf\\0user`. Kernel keys without NUL share one shard.
fn cf_prefix(key: &[u8]) -> &[u8] {
    match key.iter().position(|&b| b == 0) {
        Some(i) => &key[..i],
        None => &[],
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

    fn invalidate_tail_ord(&self) {
        if let Ok(mut g) = self.tail_ord.lock() {
            *g = None;
        }
    }

    fn tail_idx_insert(&mut self, key: Bytes, i: usize) {
        let p = Bytes::copy_from_slice(cf_prefix(key.as_ref()));
        self.tail_idx.entry(p).or_default().insert(key, i);
    }

    fn tail_idx_get(&self, user_key: &[u8]) -> Option<&usize> {
        self.tail_idx
            .get(cf_prefix(user_key))
            .and_then(|m| m.get(user_key))
    }

    fn tail_idx_range<'a>(
        &'a self,
        start: Bound<&'a [u8]>,
        end: Bound<&'a [u8]>,
    ) -> std::collections::btree_map::Range<'a, Bytes, usize> {
        let p = match (bound_cf_prefix(start), bound_cf_prefix(end)) {
            (Some(a), Some(b)) if a == b => a,
            (Some(a), None) => a,
            (None, Some(b)) => b,
            _ => {
                // Unbounded / cross-CF: one shard only → that shard; else empty
                // (caller should have fallen back to the linear merge).
                if self.tail_idx.len() == 1 {
                    return self
                        .tail_idx
                        .values()
                        .next()
                        .expect("len==1")
                        .range::<[u8], _>((start, end));
                }
                return self.empty_idx.range::<[u8], _>((start, end));
            }
        };
        match self.tail_idx.get(p) {
            Some(m) => m.range::<[u8], _>((start, end)),
            None => self.empty_idx.range::<[u8], _>((start, end)),
        }
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
        self.tail_idx.clear();
        // shards dropped; empty_idx stays empty
        self.tail_max_seq = 0;
        let tail = std::mem::take(&mut self.tail);
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
                self.approx_bytes = self
                    .approx_bytes
                    .saturating_sub(old.len())
                    .saturating_add(v.value.len());
                return;
            }
        }
        self.entries = self.entries.saturating_add(1);
        self.approx_bytes = self.approx_bytes.saturating_add(entry_bytes);
        if is_rd {
            self.range_tombstones = self.range_tombstones.saturating_add(1);
        }
        self.invalidate_tail_ord();
        self.tail_max_seq = self.tail_max_seq.max(key.sequence);
        self.tail_idx_insert(key.user_key.clone(), self.tail.len());
        self.tail.push(Version { key, value });
    }

    /// Batch insert: one `tail_ord` invalidate (RFC-0044 P1.1 pipeline).
    pub fn insert_many(&mut self, items: impl IntoIterator<Item = (InternalKey, Bytes)>) {
        let iter = items.into_iter();
        self.tail.reserve(iter.size_hint().0);
        let mut any = false;
        for (key, value) in iter {
            let entry_bytes = key.user_key.len() + value.len() + 8;
            let is_rd = key.kind == ValueType::RangeDeletion;
            if let Some(v) = self.tail.last_mut() {
                if v.key.sequence == key.sequence
                    && v.key.kind == key.kind
                    && v.key.user_key == key.user_key
                {
                    let old = std::mem::replace(&mut v.value, value);
                    self.approx_bytes = self
                        .approx_bytes
                        .saturating_sub(old.len())
                        .saturating_add(v.value.len());
                    continue;
                }
            }
            self.entries = self.entries.saturating_add(1);
            self.approx_bytes = self.approx_bytes.saturating_add(entry_bytes);
            if is_rd {
                self.range_tombstones = self.range_tombstones.saturating_add(1);
            }
            self.tail_max_seq = self.tail_max_seq.max(key.sequence);
            self.tail_idx_insert(key.user_key.clone(), self.tail.len());
            self.tail.push(Version { key, value });
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
    fn gc_below_floor(
        list: &mut VecDeque<Version>,
        floor: SequenceNumber,
        dropped: &mut Dropped,
    ) {
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
        let look = match v.key.kind {
            ValueType::Deletion => Lookup::Deleted,
            ValueType::Value => {
                if self.range_deleted(user_key, v.key.sequence, snapshot) {
                    Lookup::Deleted
                } else {
                    Lookup::Found(v.value.clone())
                }
            }
            ValueType::RangeDeletion => return None,
        };
        Some((v.key.sequence, look))
    }

    fn tail_best(&self, user_key: &[u8], snapshot: SequenceNumber) -> Option<&Version> {
        let Some(&i) = self.tail_idx_get(user_key) else {
            return None;
        };
        let newest = &self.tail[i];
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
        if self.tail.is_empty() {
            return MemInternalIter::Map(map);
        }
        let order = self.cached_tail_order();
        let tail: Vec<(&InternalKey, &Bytes)> = order
            .iter()
            .filter_map(|&i| {
                let v = &self.tail[i as usize];
                if crate::merge::user_key_in_range(v.key.user_key.as_ref(), start, end) {
                    Some((&v.key, &v.value))
                } else {
                    None
                }
            })
            .collect();
        MemInternalIter::Merge(MemInternalMerge {
            map: map.peekable(),
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
        if self.tail.is_empty() || snapshot < self.tail_max_seq {
            return self.iter_internal_iter(start, end);
        }
        // Cross-CF / unbounded range cannot use a single shard cursor.
        match (bound_cf_prefix(start), bound_cf_prefix(end)) {
            (Some(a), Some(b)) if a != b => return self.iter_internal_iter(start, end),
            (None, _) | (_, None) if self.tail_idx.len() > 1 => {
                return self.iter_internal_iter(start, end);
            }
            _ => {}
        }
        let map = self.iter_internal_range_cursor(start, end);
        MemInternalIter::Idx(MemInternalIdx {
            map: map.peekable(),
            idx: self.tail_idx_range(start, end).peekable(),
            tail: &self.tail,
        })
    }

    fn cached_tail_order(&self) -> Arc<Vec<u32>> {
        let mut g = self.tail_ord.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(arc) = g.as_ref() {
            return Arc::clone(arc);
        }
        let mut idx: Vec<u32> = (0..self.tail.len() as u32).collect();
        idx.sort_unstable_by(|&a, &b| self.tail[a as usize].key.cmp(&self.tail[b as usize].key));
        let arc = Arc::new(idx);
        *g = Some(Arc::clone(&arc));
        arc
    }

    /// Concrete (no `dyn`) range cursor — count/scan hot path.
    pub(crate) fn iter_internal_range_cursor<'a>(
        &'a self,
        start: Bound<&'a [u8]>,
        end: Bound<&'a [u8]>,
    ) -> MemInternalRange<'a> {
        MemInternalRange {
            users: self.map.range::<[u8], _>((start, end)),
            cur: VersIter::One(None),
        }
    }

    /// Largest user key in `[prefix, before)` visible at `snapshot`.
    ///
    /// Reverse user-key walk of the prefix, then the newest version ≤ snapshot
    /// (same visibility as [`get_entry`]). `before` is an exclusive upper bound
    /// inside the prefix (retry after a cross-layer tombstone). `None` means
    /// `prefix_succ`.
    #[must_use]
    pub fn last_visible_under_prefix(
        &self,
        prefix: &[u8],
        snapshot: SequenceNumber,
        before: Option<&[u8]>,
    ) -> Option<(Bytes, Bytes)> {
        let prefix_end = crate::prefix::prefix_exclusive_end(prefix);
        let end_b = match (before, prefix_end.as_deref()) {
            (Some(b), Some(p)) if b < p => Bound::Excluded(b),
            (Some(b), None) => Bound::Excluded(b),
            (_, Some(p)) => Bound::Excluded(p),
            (_, None) => Bound::Unbounded,
        };
        let mut keys: Vec<Bytes> = self
            .map
            .range::<[u8], _>((Bound::Included(prefix), end_b))
            .filter(|(uk, _)| prefix.is_empty() || uk.starts_with(prefix))
            .map(|(uk, _)| uk.clone())
            .collect();
        keys.extend(
            self.tail_idx_range(Bound::Included(prefix), end_b)
                .filter(|(uk, _)| prefix.is_empty() || uk.starts_with(prefix))
                .map(|(uk, _)| uk.clone()),
        );
        keys.sort();
        keys.dedup();
        for uk in keys.into_iter().rev() {
            if let Some((_, Lookup::Found(val))) = self.get_entry(&uk, snapshot) {
                return Some((uk, val));
            }
        }
        None
    }

    /// Rewrite every stored value with `f` (used by value-log GC remapping).
    pub fn map_values<F>(&mut self, mut f: F)
    where
        F: FnMut(&Bytes) -> Bytes,
    {
        self.approx_bytes = 0;
        for (uk, vers) in &mut self.map {
            for v in vers.iter_mut() {
                v.value = f(&v.value);
                self.approx_bytes = self
                    .approx_bytes
                    .saturating_add(uk.len() + v.value.len() + 8);
            }
        }
        for v in &mut self.tail {
            v.value = f(&v.value);
            self.approx_bytes = self
                .approx_bytes
                .saturating_add(v.key.user_key.len() + v.value.len() + 8);
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
            if k.kind == ValueType::Value
                && !self.range_deleted(&k.user_key, k.sequence, snapshot)
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
        for (seq, want) in [
            (5u64, "v5"),
            (6, "v6"),
            (7, "v7"),
            (8, "v8"),
            (100, "v8"),
        ] {
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
        assert!(matches!(mt.map.get(b"k".as_slice()), Some(Versions::One(_))));
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
        assert_eq!(a.get(b"hot", 40_000), Lookup::Found(Bytes::from_static(b"x")));
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
