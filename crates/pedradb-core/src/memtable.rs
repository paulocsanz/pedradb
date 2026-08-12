//! In-memory versioned key-value buffer (MemTable).
//!
//! Holds puts and deletions as [`InternalKey`] entries until flushed to SST
//! (later) or rebuilt from the WAL on recovery. Point lookups and ranges honor
//! a **snapshot sequence**: only entries with `sequence <= snapshot` are visible;
//! the newest such entry wins (delete tombstone hides the key).

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

/// Sorted in-memory table of versioned keys.
///
/// Mutations are single-threaded for P0 (callers serialize writers). Reads may
/// share a reference if the outer layer uses interior mutability carefully;
/// this type itself is not synchronized.
#[derive(Debug, Default, Clone)]
pub struct MemTable {
    /// Internal key → value (empty for deletions).
    map: BTreeMap<InternalKey, Bytes>,
    /// Approximate bytes for flush triggers (user key + value + trailer).
    approx_bytes: usize,
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
        self.map.len()
    }

    /// Whether no entries are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Approximate memory used by keys and values (for flush thresholds).
    #[must_use]
    pub fn approx_memory_usage(&self) -> usize {
        self.approx_bytes
    }

    /// Insert a put or deletion. Does not assign sequence numbers — caller does.
    pub fn insert(&mut self, key: InternalKey, value: Bytes) {
        let entry_bytes = key.user_key.len() + value.len() + 8;
        if let Some(old) = self.map.insert(key, value) {
            // Replaced an identical internal key (unusual); adjust estimate.
            self.approx_bytes = self.approx_bytes.saturating_sub(old.len());
        } else {
            self.approx_bytes = self.approx_bytes.saturating_add(entry_bytes);
        }
    }

    /// Convenience: put `user_key → value` at `sequence`.
    pub fn put(&mut self, user_key: impl Into<Bytes>, sequence: SequenceNumber, value: impl Into<Bytes>) {
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
        let probe = InternalKey::for_lookup(Bytes::copy_from_slice(user_key), snapshot);
        let Some((ikey, value)) = self.map.range(probe..).next() else {
            return Lookup::NotFound;
        };
        if ikey.user_key.as_ref() != user_key {
            return Lookup::NotFound;
        }
        // Range starts at first key >= probe; ordering guarantees sequence <= snapshot
        // for the same user key when we landed on this user key.
        debug_assert!(ikey.sequence <= snapshot);
        match ikey.kind {
            ValueType::Deletion => Lookup::Deleted,
            ValueType::Value => {
                // Check covering range tombstones with higher sequence.
                if self.range_deleted(user_key, ikey.sequence, snapshot) {
                    Lookup::Deleted
                } else {
                    Lookup::Found(value.clone())
                }
            }
            ValueType::RangeDeletion => Lookup::NotFound,
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
        for (ikey, end) in &self.map {
            if ikey.kind != ValueType::RangeDeletion || ikey.sequence > snapshot {
                continue;
            }
            if ikey.sequence > point_seq
                && user_key >= ikey.user_key.as_ref()
                && user_key < end.as_ref()
            {
                return true;
            }
        }
        false
    }

    /// All internal versions in [`InternalKey`] order (for SST flush).
    pub fn iter_internal(&self) -> impl Iterator<Item = (&InternalKey, &Bytes)> + '_ {
        self.map.iter()
    }

    /// Rewrite every stored value with `f` (used by value-log GC remapping).
    pub fn map_values<F>(&mut self, mut f: F)
    where
        F: FnMut(&Bytes) -> Bytes,
    {
        let old = std::mem::take(&mut self.map);
        self.approx_bytes = 0;
        for (k, v) in old {
            let new_v = f(&v);
            let entry_bytes = k.user_key.len() + new_v.len() + 8;
            self.approx_bytes = self.approx_bytes.saturating_add(entry_bytes);
            self.map.insert(k, new_v);
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
        SnapshotIter {
            inner: self.map.iter().peekable(),
            snapshot,
        }
    }

    /// Range over user keys at `snapshot` (`start` / `end` are user-key bounds).
    pub fn range_snapshot<'a>(
        &'a self,
        start: Bound<&'a [u8]>,
        end: Bound<&'a [u8]>,
        snapshot: SequenceNumber,
    ) -> impl Iterator<Item = (Bytes, Bytes)> + 'a {
        self.iter_snapshot(snapshot).filter(move |(uk, _)| {
            let after_start = match start {
                Bound::Unbounded => true,
                Bound::Included(s) => uk.as_ref() >= s,
                Bound::Excluded(s) => uk.as_ref() > s,
            };
            let before_end = match end {
                Bound::Unbounded => true,
                Bound::Included(e) => uk.as_ref() <= e,
                Bound::Excluded(e) => uk.as_ref() < e,
            };
            after_start && before_end
        })
    }
}

/// Walk internal keys in order; emit one visible put per user key.
struct SnapshotIter<'a> {
    inner: std::iter::Peekable<std::collections::btree_map::Iter<'a, InternalKey, Bytes>>,
    snapshot: SequenceNumber,
}

impl Iterator for SnapshotIter<'_> {
    type Item = (Bytes, Bytes);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((ikey, value)) = self.inner.next() {
            if ikey.sequence > self.snapshot {
                continue;
            }
            // Newest visible version for this user key (map order = newest first).
            let user_key = ikey.user_key.clone();
            let result = match ikey.kind {
                ValueType::Value => {
                    // SnapshotIter does not apply range dels; Db merge path does.
                    Some((user_key.clone(), value.clone()))
                }
                ValueType::Deletion | ValueType::RangeDeletion => None,
            };
            // Skip remaining versions of the same user key.
            while let Some((next_key, _)) = self.inner.peek() {
                if next_key.user_key == user_key {
                    self.inner.next();
                } else {
                    break;
                }
            }
            if let Some(item) = result {
                return Some(item);
            }
        }
        None
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
        assert_eq!(at_1[1], (Bytes::from_static(b"b"), Bytes::from_static(b"vb")));
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
        assert_eq!(mid, vec![Bytes::from_static(b"b"), Bytes::from_static(b"c")]);
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
}
