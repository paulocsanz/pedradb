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

/// Borrowed walk of `BTreeMap` user-key range, newest-first versions per key.
/// Concrete so count/scan do not `Box<dyn Iterator>` on every refill.
pub(crate) struct MemInternalRange<'a> {
    users: std::collections::btree_map::Range<'a, Bytes, Vec<Version>>,
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
    /// User key → versions newest-first.
    map: BTreeMap<Bytes, Vec<Version>>,
    /// Approximate bytes for flush triggers (user key + value + trailer).
    approx_bytes: usize,
    /// Range-tombstone entries (full-map fallback on ranged scan when > 0).
    range_tombstones: usize,
    /// Total internal versions (not distinct user keys).
    entries: usize,
}

/// [`InternalKey`] order on `(seq, kind)` only (user key already equal).
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

    /// Insert a put or deletion. Does not assign sequence numbers — caller does.
    pub fn insert(&mut self, key: InternalKey, value: Bytes) {
        let entry_bytes = key.user_key.len() + value.len() + 8;
        let is_rd = key.kind == ValueType::RangeDeletion;
        let vers = self.map.entry(key.user_key.clone()).or_default();
        if vers.is_empty() {
            vers.push(Version { key, value });
            self.entries = self.entries.saturating_add(1);
            self.approx_bytes = self.approx_bytes.saturating_add(entry_bytes);
            if is_rd {
                self.range_tombstones = self.range_tombstones.saturating_add(1);
            }
            return;
        }
        let pos = vers.partition_point(|v| {
            ver_cmp(v.key.sequence, v.key.kind, key.sequence, key.kind) == Ordering::Less
        });
        if pos < vers.len()
            && vers[pos].key.sequence == key.sequence
            && vers[pos].key.kind == key.kind
        {
            let old = std::mem::replace(&mut vers[pos].value, value);
            self.approx_bytes = self
                .approx_bytes
                .saturating_sub(old.len())
                .saturating_add(vers[pos].value.len());
            return;
        }
        vers.insert(pos, Version { key, value });
        self.entries = self.entries.saturating_add(1);
        self.approx_bytes = self.approx_bytes.saturating_add(entry_bytes);
        if is_rd {
            self.range_tombstones = self.range_tombstones.saturating_add(1);
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
        let vers = self.map.get(user_key)?;
        let v = vers.iter().find(|v| v.key.sequence <= snapshot)?;
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
        false
    }

    /// All internal versions in [`InternalKey`] order (for SST flush).
    pub fn iter_internal(&self) -> impl Iterator<Item = (&InternalKey, &Bytes)> + '_ {
        self.map
            .values()
            .flat_map(|vers| vers.iter().map(|v| (&v.key, &v.value)))
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
        self.iter_internal_range_cursor(start, end)
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
        for (uk, vers) in self
            .map
            .range::<[u8], _>((Bound::Included(prefix), end_b))
            .rev()
        {
            if !prefix.is_empty() && !uk.starts_with(prefix) {
                continue;
            }
            let Some(v) = vers.iter().find(|v| v.key.sequence <= snapshot) else {
                continue;
            };
            match v.key.kind {
                ValueType::Value if !self.range_deleted(uk.as_ref(), v.key.sequence, snapshot) => {
                    return Some((uk.clone(), v.value.clone()));
                }
                ValueType::Value | ValueType::Deletion | ValueType::RangeDeletion => {}
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
            for v in vers {
                v.value = f(&v.value);
                self.approx_bytes = self
                    .approx_bytes
                    .saturating_add(uk.len() + v.value.len() + 8);
            }
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
        self.map
            .range::<[u8], _>((start, end))
            .filter_map(move |(uk, vers)| {
                let v = vers.iter().find(|v| v.key.sequence <= snapshot)?;
                match v.key.kind {
                    ValueType::Value => Some((uk.clone(), v.value.clone())),
                    ValueType::Deletion | ValueType::RangeDeletion => None,
                }
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
}
