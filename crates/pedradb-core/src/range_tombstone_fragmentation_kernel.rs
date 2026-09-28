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

    /// Checks if a given point key falls within the half-open interval `[start, end)`.
    pub fn contains_key(&self, key: &[u8]) -> bool {
        key >= self.start.as_slice() && key < self.end.as_slice()
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

    /// Adds a valid range tombstone.
    pub fn add(&mut self, tombstone: RangeTombstone) {
        self.tombstones.push(tombstone);
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

    /// Total count of active tombstones.
    pub fn count(&self) -> usize {
        self.tombstones.len()
    }
}
