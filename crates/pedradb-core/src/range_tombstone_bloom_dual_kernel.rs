//! RFC-0290: Dual Filter Pruning and Range Tombstone Bloom Blindspot Invariant Kernel.
//!
//! Enforces dual evaluation of point Bloom filters and active range tombstones:
//! PruneSST(SST, k) <=> BloomMiss(SST, k) and not RangeCover(SST, k).
//! Mathematically guarantees that a Bloom miss never suppresses an SST containing
//! an active range deletion covering k, preventing resurrecting stale keys from lower levels.

#![forbid(unsafe_code)]

/// An active range tombstone covering `[start_key, end_key)` at a specific sequence number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeTombstoneSpan {
    /// Inclusive lower bound of the range deletion.
    pub start_key: Vec<u8>,
    /// Exclusive upper bound of the range deletion.
    pub end_key: Vec<u8>,
    /// Sequence number when the range tombstone was committed.
    pub sequence_number: u64,
}

impl RangeTombstoneSpan {
    /// Creates a new range tombstone span.
    #[must_use]
    pub fn new(start_key: Vec<u8>, end_key: Vec<u8>, sequence_number: u64) -> Self {
        assert!(start_key <= end_key, "start_key must be <= end_key");
        Self {
            start_key,
            end_key,
            sequence_number,
        }
    }

    /// Checks if a point key falls strictly within `[start_key, end_key)`.
    #[must_use]
    pub fn covers_key(&self, key: &[u8]) -> bool {
        key >= self.start_key.as_slice() && key < self.end_key.as_slice()
    }
}

/// Decision outcome for whether an SSTable can be safely bypassed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PruneDecision {
    /// Safe to skip reading the SSTable (no point key and no covering range deletion).
    SafeToPrune,
    /// Must inspect SSTable because Bloom filter matched a potential point key.
    InspectPointHit,
    /// Must inspect SSTable because a Range Tombstone covers the key, preventing lower-level resurrection.
    InspectRangeTombstoneHit {
        /// Sequence number of the highest applicable range tombstone covering the key.
        tombstone_seq: u64,
    },
}

/// Dual filter evaluator eliminating the range tombstone Bloom blindspot.
pub struct RangeTombstoneBloomDualFilter;

impl RangeTombstoneBloomDualFilter {
    /// Evaluates the dual filter condition for a query key `k`.
    ///
    /// Mathematical Invariant:
    /// An SSTable is pruned IF AND ONLY IF Bloom filter reports absence AND
    /// no active range tombstone in the SST covers `k`.
    #[must_use]
    pub fn evaluate(
        bloom_matches_point: bool,
        range_tombstones: &[RangeTombstoneSpan],
        key: &[u8],
        read_snapshot_seq: u64,
    ) -> PruneDecision {
        // Find highest sequence number range tombstone visible to snapshot covering the key
        let mut covering_tombstone_seq: Option<u64> = None;

        for tombstone in range_tombstones {
            if tombstone.sequence_number <= read_snapshot_seq && tombstone.covers_key(key) {
                covering_tombstone_seq = match covering_tombstone_seq {
                    Some(prev) => Some(prev.max(tombstone.sequence_number)),
                    None => Some(tombstone.sequence_number),
                };
            }
        }

        if let Some(tombstone_seq) = covering_tombstone_seq {
            // Invariant: Even if Bloom filter missed, range tombstone must be applied!
            PruneDecision::InspectRangeTombstoneHit { tombstone_seq }
        } else if bloom_matches_point {
            PruneDecision::InspectPointHit
        } else {
            PruneDecision::SafeToPrune
        }
    }
}
