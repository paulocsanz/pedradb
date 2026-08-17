//! In-memory versioned key-value buffer (MemTable).
//!
//! Holds puts and deletions as [`InternalKey`] entries until flushed to SST
//! (later) or rebuilt from the WAL on recovery. Point lookups and ranges honor
//! a **snapshot sequence**: only entries with `sequence <= snapshot` are visible;
//! the newest such entry wins (delete tombstone hides the key).

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ops::Bound;

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
#[derive(Debug, Clone)]
enum Versions {
    One(Version),
    Many(Vec<Version>),
}

impl Versions {
    fn as_slice(&self) -> &[Version] {
        match self {
            Self::One(v) => std::slice::from_ref(v),
            Self::Many(vs) => vs.as_slice(),
        }
    }

    fn as_mut_slice(&mut self) -> &mut [Version] {
        match self {
            Self::One(v) => std::slice::from_mut(v),
            Self::Many(vs) => vs.as_mut_slice(),
        }
    }

    fn iter(&self) -> std::slice::Iter<'_, Version> {
        self.as_slice().iter()
    }
}

impl<'a> IntoIterator for &'a Versions {
    type Item = &'a Version;
    type IntoIter = std::slice::Iter<'a, Version>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Borrowed walk of `BTreeMap` user-key range, newest-first versions per key.
/// Concrete so count/scan do not `Box<dyn Iterator>` on every refill.
pub(crate) struct MemInternalRange<'a> {
    users: std::collections::btree_map::Range<'a, Bytes, Versions>,
    cur: std::slice::Iter<'a, Version>,
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

/// Sorted in-memory table of versioned keys.
///
/// Keyed by user key so [`Self::get_entry`] is a borrowed `BTreeMap` lookup
/// (RFC-0035 P1.2: no `InternalKey` / `Bytes` probe alloc on the hot get).
/// Mutations are single-threaded for P0 (callers serialize writers). Reads may
/// share a reference if the outer layer uses interior mutability carefully;
/// this type itself is not synchronized.
#[derive(Debug, Default, Clone)]
pub struct MemTable {
    /// User key → versions newest-first (inline first version).
    map: BTreeMap<Bytes, Versions>,
    /// Recent inserts not yet in `map` (RFC-0041: apply Ok path is O(1) push;
    /// [`Self::spill_tail`] runs at stage/park, off the per-write BTree).
    tail: Vec<Version>,
    /// Approximate bytes for flush triggers (user key + value + trailer).
    approx_bytes: usize,
    /// Range-tombstone entries (full-map fallback on ranged scan when > 0).
    range_tombstones: usize,
    /// Total internal versions (not distinct user keys).
    entries: usize,
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

    /// Spill when the tail would make point/scan walks linear (YCSB A/F).
    /// Fat apply batches stay under this so Ok stays O(1) push per op.
    const TAIL_SPILL: usize = 512;

    /// Fold [`Self::tail`] into the BTree (stage/park/scan-prep).
    pub fn spill_tail(&mut self) {
        let tail = std::mem::take(&mut self.tail);
        for v in tail {
            let entry_bytes = v.key.user_key.len() + v.value.len() + 8;
            let is_rd = v.key.kind == ValueType::RangeDeletion;
            self.entries = self.entries.saturating_sub(1);
            self.approx_bytes = self.approx_bytes.saturating_sub(entry_bytes);
            if is_rd {
                self.range_tombstones = self.range_tombstones.saturating_sub(1);
            }
            self.insert_map(v.key, v.value);
        }
    }

    /// Move every version from `other` into `self` (retired L0 fold).
    pub fn absorb(&mut self, mut other: Self) {
        self.spill_tail();
        other.spill_tail();
        if self.is_empty() {
            *self = other;
            return;
        }
        if other.is_empty() {
            return;
        }
        for (_, vers) in other.map {
            match vers {
                Versions::One(v) => self.insert_map(v.key, v.value),
                Versions::Many(vs) => {
                    for v in vs {
                        self.insert_map(v.key, v.value);
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
        self.tail.push(Version { key, value });
        if self.tail.len() >= Self::TAIL_SPILL {
            self.spill_tail();
        }
    }

    fn insert_map(&mut self, key: InternalKey, value: Bytes) {
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
                if Self::insert_into(e.get_mut(), key, value, entry_bytes, &mut self.approx_bytes) {
                    self.entries = self.entries.saturating_add(1);
                    if is_rd {
                        self.range_tombstones = self.range_tombstones.saturating_add(1);
                    }
                }
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
                let older_first = ver_cmp(
                    existing.key.sequence,
                    existing.key.kind,
                    key.sequence,
                    key.kind,
                ) == Ordering::Less;
                let Versions::One(old) = std::mem::replace(vers, Versions::Many(Vec::new())) else {
                    unreachable!("just matched One");
                };
                let newer = Version { key, value };
                *vers = Versions::Many(if older_first {
                    vec![old, newer]
                } else {
                    vec![newer, old]
                });
                *approx_bytes = approx_bytes.saturating_add(entry_bytes);
                true
            }
            Versions::Many(list) => {
                let pos = list.partition_point(|v| {
                    ver_cmp(v.key.sequence, v.key.kind, key.sequence, key.kind) == Ordering::Less
                });
                if pos < list.len()
                    && list[pos].key.sequence == key.sequence
                    && list[pos].key.kind == key.kind
                {
                    let old = std::mem::replace(&mut list[pos].value, value);
                    *approx_bytes = approx_bytes
                        .saturating_sub(old.len())
                        .saturating_add(list[pos].value.len());
                    return false;
                }
                list.insert(pos, Version { key, value });
                *approx_bytes = approx_bytes.saturating_add(entry_bytes);
                true
            }
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
        let mut best: Option<&Version> = None;
        for v in &self.tail {
            if v.key.user_key.as_ref() != user_key || v.key.sequence > snapshot {
                continue;
            }
            let better = best.is_none_or(|b| version_newer(v, b));
            if better {
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
        if self.tail.is_empty() {
            return Box::new(
                self.map
                    .values()
                    .flat_map(|vers| vers.iter().map(|v| (&v.key, &v.value))),
            ) as Box<dyn Iterator<Item = (&InternalKey, &Bytes)> + '_>;
        }
        let mut items: Vec<_> = self
            .map
            .values()
            .flat_map(|vers| vers.iter().map(|v| (&v.key, &v.value)))
            .chain(self.tail.iter().map(|v| (&v.key, &v.value)))
            .collect();
        items.sort_by(|a, b| a.0.cmp(b.0));
        Box::new(items.into_iter()) as Box<dyn Iterator<Item = (&InternalKey, &Bytes)> + '_>
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
        if self.tail.is_empty() {
            return Box::new(self.iter_internal_range_cursor(start, end))
                as Box<dyn Iterator<Item = (&'a InternalKey, &'a Bytes)> + 'a>;
        }
        Box::new(
            self.iter_internal().filter(move |(k, _)| {
                crate::merge::user_key_in_range(k.user_key.as_ref(), start, end)
            }),
        ) as Box<dyn Iterator<Item = (&'a InternalKey, &'a Bytes)> + 'a>
    }

    /// Concrete (no `dyn`) range cursor — count/scan hot path.
    pub(crate) fn iter_internal_range_cursor<'a>(
        &'a self,
        start: Bound<&'a [u8]>,
        end: Bound<&'a [u8]>,
    ) -> MemInternalRange<'a> {
        MemInternalRange {
            users: self.map.range::<[u8], _>((start, end)),
            cur: [].iter(),
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
        for v in &self.tail {
            let uk = v.key.user_key.as_ref();
            if !prefix.is_empty() && !uk.starts_with(prefix) {
                continue;
            }
            let in_lo = uk >= prefix;
            let in_hi = match end_b {
                Bound::Included(h) => uk <= h,
                Bound::Excluded(h) => uk < h,
                Bound::Unbounded => true,
            };
            if in_lo && in_hi {
                keys.push(v.key.user_key.clone());
            }
        }
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
            for v in vers.as_mut_slice() {
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
        let mut keys: Vec<Bytes> = self
            .map
            .range::<[u8], _>((start, end))
            .map(|(uk, _)| uk.clone())
            .collect();
        if !self.tail.is_empty() {
            for v in &self.tail {
                if crate::merge::user_key_in_range(v.key.user_key.as_ref(), start, end) {
                    keys.push(v.key.user_key.clone());
                }
            }
            keys.sort();
            keys.dedup();
        }
        keys.into_iter()
            .filter_map(move |uk| match self.get_entry(&uk, snapshot) {
                Some((_, Lookup::Found(val))) => Some((uk, val)),
                _ => None,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Bound;

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
    fn tail_auto_spills_at_threshold() {
        let mut mt = MemTable::new();
        for i in 0..MemTable::TAIL_SPILL {
            mt.put(
                Bytes::copy_from_slice(&(i as u64).to_le_bytes()),
                i as u64 + 1,
                b"v".as_slice(),
            );
        }
        assert!(!mt.has_tail());
        assert_eq!(mt.len(), MemTable::TAIL_SPILL);
        assert_eq!(
            mt.get(&0u64.to_le_bytes(), MemTable::TAIL_SPILL as u64),
            Lookup::Found(Bytes::from_static(b"v"))
        );
    }
}
