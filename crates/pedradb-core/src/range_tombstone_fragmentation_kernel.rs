//! Range Tombstone Fragmentation and Point Coverage Invariant Kernel (RFC-0286 Fronteira 1).
//!
//! Provides canonical interval decomposition and MVCC containment checks for range tombstones
//! `[start, end) @ seq`, guaranteeing zero phantom resurrection and zero retroactive annihilation.
//!
//! Guarantees:
//! 1. Canonical decomposition: overlapping range deletions partition into disjoint intervals.
//! 2. Exact point coverage: `IsDeleted(k, t) <=> exists [s, e) @ t_tomb: s <= k < e and t_tomb > t`.
//! 3. Strict boundary consistency: point keys equal to `end` are strictly non-inclusive.

#![forbid(unsafe_code)]

/// Individual range deletion tombstone covering keys in half-open interval `[start, end)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeTombstone {
    /// Start key (inclusive).
    pub start: Vec<u8>,
    /// End key (exclusive).
    pub end: Vec<u8>,
    /// Commit sequence number of the deletion.
    pub seq_num: u64,
}

impl RangeTombstone {
    /// Creates a new range tombstone.
    pub fn new(start: Vec<u8>, end: Vec<u8>, seq_num: u64) -> Result<Self, &'static str> {
        if start >= end {
            return Err("Range tombstone start must be strictly less than end");
        }
        Ok(Self { start, end, seq_num })
    }

    /// Validates that the range bounds are well-formed (`start < end`).
    pub fn is_valid(&self) -> bool {
        self.start < self.end
    }

    /// Checks if a given point key falls within the half-open interval `[start, end)`.
    pub fn contains_key(&self, key: &[u8]) -> bool {
        self.is_valid() && key >= self.start.as_slice() && key < self.end.as_slice()
    }

    /// Checks if this tombstone overlaps with another half-open interval `[other_start, other_end)`.
    pub fn overlaps(&self, other_start: &[u8], other_end: &[u8]) -> bool {
        if !self.is_valid() || other_start >= other_end {
            return false;
        }
        self.start.as_slice() < other_end && other_start < self.end.as_slice()
    }
}

/// Collection of range tombstones with canonical point coverage resolution.
pub struct RangeTombstoneSet {
    tombstones: Vec<RangeTombstone>,
}

impl RangeTombstoneSet {
    /// Creates an empty set of range tombstones.
    pub fn new() -> Self {
        Self {
            tombstones: Vec::new(),
        }
    }

    /// Adds a range tombstone without validation.
    pub fn add(&mut self, tombstone: RangeTombstone) {
        self.tombstones.push(tombstone);
    }

    /// Adds a range tombstone with strict validity check.
    pub fn add_checked(&mut self, tombstone: RangeTombstone) -> Result<(), &'static str> {
        if !tombstone.is_valid() {
            return Err("Range tombstone start must be strictly less than end");
        }
        self.tombstones.push(tombstone);
        Ok(())
    }

    /// Checks if a point key at sequence `point_seq` is shadowed by any range tombstone.
    ///
    /// A point key is shadowed if and only if there is a tombstone covering `key`
    /// with `tombstone.seq_num > point_seq`.
    pub fn is_key_shadowed(&self, key: &[u8], point_seq: u64) -> bool {
        for tomb in &self.tombstones {
            if tomb.contains_key(key) && tomb.seq_num > point_seq {
                return true;
            }
        }
        false
    }

    /// Computes the effective sequence number of the newest range tombstone covering `key`.
    pub fn max_covering_seq(&self, key: &[u8]) -> Option<u64> {
        let mut max_seq = None;
        for tomb in &self.tombstones {
            if tomb.contains_key(key) {
                max_seq = Some(max_seq.map_or(tomb.seq_num, |curr| std::cmp::max(curr, tomb.seq_num)));
            }
        }
        max_seq
    }

    /// Checks if any range tombstone in the set overlaps with `[start, end)`.
    pub fn overlaps_range(&self, start: &[u8], end: &[u8]) -> bool {
        if start >= end {
            return false;
        }
        for tomb in &self.tombstones {
            if tomb.overlaps(start, end) {
                return true;
            }
        }
        false
    }

    /// Computes the maximum covering sequence number among all tombstones overlapping `[start, end)`.
    pub fn max_covering_seq_for_range(&self, start: &[u8], end: &[u8]) -> Option<u64> {
        if start >= end {
            return None;
        }
        let mut max_seq = None;
        for tomb in &self.tombstones {
            if tomb.overlaps(start, end) {
                max_seq = Some(max_seq.map_or(tomb.seq_num, |curr| std::cmp::max(curr, tomb.seq_num)));
            }
        }
        max_seq
    }

    /// Evaluates if the entire half-open interval `[start, end)` is contiguously shadowed
    /// by tombstones with sequence numbers strictly greater than `point_seq`.
    pub fn is_range_fully_shadowed(&self, start: &[u8], end: &[u8], point_seq: u64) -> bool {
        if start >= end {
            return false;
        }
        let fragmented = self.fragment();
        let mut current_pos = start;
        for tomb in &fragmented {
            if tomb.seq_num <= point_seq {
                continue;
            }
            if tomb.start.as_slice() <= current_pos && tomb.end.as_slice() > current_pos {
                current_pos = tomb.end.as_slice();
                if current_pos >= end {
                    return true;
                }
            }
        }
        false
    }

    /// Performs binary search lookup on a canonical fragmented set in O(log K).
    pub fn seek_fragmented(&self, key: &[u8]) -> Option<&RangeTombstone> {
        let idx = self.tombstones.binary_search_by(|t| {
            if key < t.start.as_slice() {
                std::cmp::Ordering::Greater
            } else if key >= t.end.as_slice() {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        });
        idx.ok().map(|i| &self.tombstones[i])
    }

    /// Total count of active tombstones.
    pub fn count(&self) -> usize {
        self.tombstones.len()
    }

    /// Returns true if the set contains no range tombstones.
    pub fn is_empty(&self) -> bool {
        self.tombstones.is_empty()
    }

    /// Checks if a point key at sequence `point_seq` is shadowed by any range tombstone
    /// visible at `snapshot_seq`.
    pub fn is_key_shadowed_at_snapshot(
        &self,
        key: &[u8],
        point_seq: u64,
        snapshot_seq: u64,
    ) -> bool {
        if point_seq > snapshot_seq {
            return false;
        }
        for tomb in &self.tombstones {
            if tomb.contains_key(key) && tomb.seq_num <= snapshot_seq && tomb.seq_num > point_seq {
                return true;
            }
        }
        false
    }

    /// Returns a new set containing only the range tombstones visible at `snapshot_seq`.
    pub fn filter_visible_at_snapshot(&self, snapshot_seq: u64) -> Self {
        Self {
            tombstones: self
                .tombstones
                .iter()
                .filter(|t| t.seq_num <= snapshot_seq)
                .cloned()
                .collect(),
        }
    }

    /// Decomposes overlapping range tombstones into canonical disjoint non-overlapping intervals (RFC-0286 Fronteira 1).
    ///
    /// Each returned interval `[s_i, e_i)` is disjoint from all others, sorted by start key,
    /// and carries the maximum covering sequence number across all overlapping tombstones.
    /// Adjacent intervals with identical sequence numbers are coalesced. Invalid tombstones are filtered.
    pub fn fragment(&self) -> Vec<RangeTombstone> {
        let valid_tombs: Vec<&RangeTombstone> = self.tombstones.iter().filter(|t| t.is_valid()).collect();
        if valid_tombs.is_empty() {
            return Vec::new();
        }

        // 1. Collect all distinct boundary endpoints (start and end).
        let mut boundaries: std::collections::BTreeSet<&[u8]> = std::collections::BTreeSet::new();
        for tomb in &valid_tombs {
            boundaries.insert(tomb.start.as_slice());
            boundaries.insert(tomb.end.as_slice());
        }

        let bounds: Vec<&[u8]> = boundaries.into_iter().collect();
        let mut uncoalesced: Vec<RangeTombstone> = Vec::new();

        // 2. Evaluate point coverage for each elementary adjacent slice [bounds[i], bounds[i+1])
        for window in bounds.windows(2) {
            let s = window[0];
            let e = window[1];

            let mut max_seq: Option<u64> = None;
            for tomb in &valid_tombs {
                // A tombstone covers the interval [s, e) iff tomb.start <= s and tomb.end >= e.
                if tomb.start.as_slice() <= s && tomb.end.as_slice() >= e {
                    max_seq = Some(max_seq.map_or(tomb.seq_num, |curr| std::cmp::max(curr, tomb.seq_num)));
                }
            }

            if let Some(seq) = max_seq {
                uncoalesced.push(RangeTombstone {
                    start: s.to_vec(),
                    end: e.to_vec(),
                    seq_num: seq,
                });
            }
        }

        // 3. Coalesce contiguous adjacent intervals sharing the exact same sequence number.
        let mut coalesced: Vec<RangeTombstone> = Vec::new();
        for interval in uncoalesced {
            if let Some(last) = coalesced.last_mut() {
                if last.end == interval.start && last.seq_num == interval.seq_num {
                    last.end = interval.end;
                    continue;
                }
            }
            coalesced.push(interval);
        }

        coalesced
    }

    /// Returns a new `RangeTombstoneSet` holding the canonical disjoint fragmented tombstones.
    pub fn into_fragmented(self) -> Self {
        Self {
            tombstones: self.fragment(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_range_tombstone_fragmentation() {
        let mut set = RangeTombstoneSet::new();
        // Overlapping tombstones: [a, z) @ 100, [d, m) @ 200
        set.add(RangeTombstone::new(b"a".to_vec(), b"z".to_vec(), 100).unwrap());
        set.add(RangeTombstone::new(b"d".to_vec(), b"m".to_vec(), 200).unwrap());

        let fragmented = set.fragment();
        // Expected disjoint partitions:
        // [a, d) @ 100
        // [d, m) @ 200
        // [m, z) @ 100
        assert_eq!(fragmented.len(), 3);
        assert_eq!(fragmented[0], RangeTombstone::new(b"a".to_vec(), b"d".to_vec(), 100).unwrap());
        assert_eq!(fragmented[1], RangeTombstone::new(b"d".to_vec(), b"m".to_vec(), 200).unwrap());
        assert_eq!(fragmented[2], RangeTombstone::new(b"m".to_vec(), b"z".to_vec(), 100).unwrap());

        // Equivalence: every key in the space has identical point coverage
        for k in [b"a", b"b", b"c", b"d", b"e", b"l", b"m", b"n", b"y"] {
            let max_orig = set.max_covering_seq(k.as_slice());
            let max_frag = fragmented.iter().find(|t| t.contains_key(k.as_slice())).map(|t| t.seq_num);
            assert_eq!(max_orig, max_frag);
        }
    }
}
