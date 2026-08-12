//! Merge MemTable ∪ SST layers for point/range reads with MVCC visibility.
//!
//! Given versioned entries ordered by [`InternalKey`] (user key ascending,
//! sequence descending), emit one live value per user key at a snapshot.

use std::collections::BTreeMap;
use std::ops::Bound;

use bytes::Bytes;

use crate::key::{InternalKey, SequenceNumber, ValueType};

/// One user-visible key/value after MVCC filtering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleKv {
    /// User key.
    pub key: Bytes,
    /// Value at snapshot.
    pub value: Bytes,
}

/// Whether `user_key` falls within `[start, end)` style bounds.
#[must_use]
pub fn user_key_in_range(user_key: &[u8], start: Bound<&[u8]>, end: Bound<&[u8]>) -> bool {
    let after_start = match start {
        Bound::Unbounded => true,
        Bound::Included(s) => user_key >= s,
        Bound::Excluded(s) => user_key > s,
    };
    let before_end = match end {
        Bound::Unbounded => true,
        Bound::Included(e) => user_key <= e,
        Bound::Excluded(e) => user_key < e,
    };
    after_start && before_end
}

/// Merge version streams and return visible puts in user-key order.
///
/// `entries` must be iterable in any order; they are sorted via [`BTreeMap`].
/// For each user key, the newest version with `sequence <= snapshot` wins;
/// deletions hide the key.
pub fn visible_range(
    entries: impl IntoIterator<Item = (InternalKey, Bytes)>,
    snapshot: SequenceNumber,
    start: Bound<&[u8]>,
    end: Bound<&[u8]>,
) -> Vec<VisibleKv> {
    let mut map: BTreeMap<InternalKey, Bytes> = BTreeMap::new();
    for (ikey, value) in entries {
        if !user_key_in_range(ikey.user_key.as_ref(), start, end) {
            continue;
        }
        if ikey.sequence > snapshot {
            continue;
        }
        map.insert(ikey, value);
    }

    let mut out = Vec::new();
    let mut iter = map.into_iter().peekable();
    while let Some((ikey, value)) = iter.next() {
        let user_key = ikey.user_key.clone();
        let live = match ikey.kind {
            ValueType::Value => Some(VisibleKv {
                key: user_key.clone(),
                value,
            }),
            ValueType::Deletion => None,
        };
        // Skip older versions of the same user key (map order = newest first).
        while let Some((next, _)) = iter.peek() {
            if next.user_key == user_key {
                iter.next();
            } else {
                break;
            }
        }
        if let Some(kv) = live {
            out.push(kv);
        }
    }
    out
}

/// Options for version GC during compaction (RFC-0009 P1.3).
#[derive(Debug, Clone, Copy, Default)]
pub struct CompactGcOptions {
    /// Drop any version with `sequence < min_sequence`.
    pub min_sequence: SequenceNumber,
    /// If true, keep only the newest remaining version per user key
    /// (and drop a lone tombstone so the key disappears).
    pub keep_only_latest: bool,
}

impl CompactGcOptions {
    /// Aggressive GC for single-writer DBs with no long-lived snapshots:
    /// keep only the newest version of each user key.
    #[must_use]
    pub fn latest_only() -> Self {
        Self {
            min_sequence: 0,
            keep_only_latest: true,
        }
    }
}

/// Filter/merge versions for a compacted SST.
///
/// Input may be unsorted; output is sorted by [`InternalKey`].
#[must_use]
pub fn gc_compact_entries(
    entries: impl IntoIterator<Item = (InternalKey, Bytes)>,
    gc: CompactGcOptions,
) -> Vec<(InternalKey, Bytes)> {
    let mut map: BTreeMap<InternalKey, Bytes> = BTreeMap::new();
    for (ikey, value) in entries {
        if ikey.sequence < gc.min_sequence {
            continue;
        }
        map.insert(ikey, value);
    }

    if !gc.keep_only_latest {
        return map.into_iter().collect();
    }

    // Newest first per user key (InternalKey order).
    let mut out = Vec::new();
    let mut iter = map.into_iter().peekable();
    while let Some((ikey, value)) = iter.next() {
        let user_key = ikey.user_key.clone();
        let keep = match ikey.kind {
            ValueType::Value => Some((ikey, value)),
            // Tombstone as newest → key gone from compacted file.
            ValueType::Deletion => None,
        };
        while let Some((next, _)) = iter.peek() {
            if next.user_key == user_key {
                iter.next();
            } else {
                break;
            }
        }
        if let Some(pair) = keep {
            out.push(pair);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::ValueType;

    fn ik(user: &[u8], seq: u64, kind: ValueType) -> InternalKey {
        InternalKey::new(Bytes::copy_from_slice(user), seq, kind)
    }

    #[test]
    fn range_mvcc_newest_and_tombstone() {
        let entries = vec![
            (ik(b"a", 1, ValueType::Value), Bytes::from_static(b"a1")),
            (ik(b"b", 2, ValueType::Value), Bytes::from_static(b"b2")),
            (ik(b"b", 3, ValueType::Value), Bytes::from_static(b"b3")),
            (ik(b"c", 4, ValueType::Value), Bytes::from_static(b"c4")),
            (ik(b"c", 5, ValueType::Deletion), Bytes::new()),
            (ik(b"d", 6, ValueType::Value), Bytes::from_static(b"d6")),
        ];
        let got = visible_range(
            entries,
            10,
            Bound::Included(b"b".as_ref()),
            Bound::Excluded(b"d".as_ref()),
        );
        assert_eq!(
            got,
            vec![VisibleKv {
                key: Bytes::from_static(b"b"),
                value: Bytes::from_static(b"b3"),
            }]
        );
    }

    #[test]
    fn snapshot_hides_newer_versions() {
        let entries = vec![
            (ik(b"k", 1, ValueType::Value), Bytes::from_static(b"old")),
            (ik(b"k", 5, ValueType::Value), Bytes::from_static(b"new")),
        ];
        let at_3 = visible_range(entries.clone(), 3, Bound::Unbounded, Bound::Unbounded);
        assert_eq!(at_3[0].value.as_ref(), b"old");
        let at_10 = visible_range(entries, 10, Bound::Unbounded, Bound::Unbounded);
        assert_eq!(at_10[0].value.as_ref(), b"new");
    }

    #[test]
    fn gc_latest_only_drops_history_and_tombstones() {
        let entries = vec![
            (ik(b"a", 1, ValueType::Value), Bytes::from_static(b"a1")),
            (ik(b"a", 3, ValueType::Value), Bytes::from_static(b"a3")),
            (ik(b"b", 2, ValueType::Value), Bytes::from_static(b"b2")),
            (ik(b"b", 4, ValueType::Deletion), Bytes::new()),
        ];
        let out = gc_compact_entries(entries, CompactGcOptions::latest_only());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0.user_key.as_ref(), b"a");
        assert_eq!(out[0].1.as_ref(), b"a3");
    }

    #[test]
    fn gc_min_sequence_drops_old() {
        let entries = vec![
            (ik(b"a", 1, ValueType::Value), Bytes::from_static(b"a1")),
            (ik(b"a", 5, ValueType::Value), Bytes::from_static(b"a5")),
        ];
        let out = gc_compact_entries(
            entries,
            CompactGcOptions {
                min_sequence: 5,
                keep_only_latest: false,
            },
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1.as_ref(), b"a5");
    }
}
