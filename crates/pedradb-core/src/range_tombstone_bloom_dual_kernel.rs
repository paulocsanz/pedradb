//! RFC-0290: Dual Filter Pruning and Range Tombstone Bloom Blindspot Invariant Kernel.
//!
//! Enforces dual evaluation of point Bloom filters and active range tombstones:
//! PruneSST(SST, k) <=> BloomMiss(SST, k) and not RangeCover(SST, k).
//! Mathematically guarantees that a Bloom miss never suppresses an SST containing
//! an active range deletion covering k, preventing resurrecting stale keys from lower levels.

#![forbid(unsafe_code)]

use std::fmt;

/// Errors associated with malformed or invalid range tombstones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeTombstoneError {
    /// Range tombstone start_key is strictly greater than end_key.
    InvertedBounds {
        start_key: Vec<u8>,
        end_key: Vec<u8>,
    },
    /// Range tombstone start_key equals end_key, which covers 0 keys.
    EmptyRange {
        key: Vec<u8>,
    },
    /// Sequence number cannot be 0 for an active committed tombstone.
    ZeroSequenceNumber,
    /// Query key cannot be empty.
    EmptyQueryKey,
}

impl fmt::Display for RangeTombstoneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvertedBounds { start_key, end_key } => {
                write!(f, "Range tombstone start_key {start_key:?} > end_key {end_key:?}")
            }
            Self::EmptyRange { key } => {
                write!(f, "Range tombstone start_key equals end_key {key:?}, covering empty range")
            }
            Self::ZeroSequenceNumber => {
                write!(f, "Range tombstone sequence number cannot be 0")
            }
            Self::EmptyQueryKey => {
                write!(f, "Query key cannot be empty")
            }
        }
    }
}

impl std::error::Error for RangeTombstoneError {}

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
    /// Validates span bounds and sequence number without panicking.
    pub fn try_new(start_key: Vec<u8>, end_key: Vec<u8>, sequence_number: u64) -> Result<Self, RangeTombstoneError> {
        if sequence_number == 0 {
            return Err(RangeTombstoneError::ZeroSequenceNumber);
        }
        if start_key > end_key {
            return Err(RangeTombstoneError::InvertedBounds { start_key, end_key });
        }
        if start_key == end_key {
            return Err(RangeTombstoneError::EmptyRange { key: start_key });
        }
        Ok(Self {
            start_key,
            end_key,
            sequence_number,
        })
    }

    /// Creates a new range tombstone span (panics if invalid, for backwards compatibility).
    #[must_use]
    pub fn new(start_key: Vec<u8>, end_key: Vec<u8>, sequence_number: u64) -> Self {
        Self::try_new(start_key, end_key, sequence_number).expect("Invalid range tombstone span parameters")
    }

    /// Validates an existing span.
    pub fn validate(&self) -> Result<(), RangeTombstoneError> {
        if self.sequence_number == 0 {
            return Err(RangeTombstoneError::ZeroSequenceNumber);
        }
        if self.start_key > self.end_key {
            return Err(RangeTombstoneError::InvertedBounds {
                start_key: self.start_key.clone(),
                end_key: self.end_key.clone(),
            });
        }
        if self.start_key == self.end_key {
            return Err(RangeTombstoneError::EmptyRange {
                key: self.start_key.clone(),
            });
        }
        Ok(())
    }

    /// Checks if a point key falls strictly within `[start_key, end_key)`.
    #[must_use]
    pub fn covers_key(&self, key: &[u8]) -> bool {
        key >= self.start_key.as_slice() && key < self.end_key.as_slice()
    }

    /// Checks if this span overlaps with another range tombstone span.
    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        self.start_key.as_slice() < other.end_key.as_slice()
            && self.end_key.as_slice() > other.start_key.as_slice()
    }

    /// Checks if this span completely encloses another range tombstone span.
    #[must_use]
    pub fn contains_span(&self, other: &Self) -> bool {
        self.start_key.as_slice() <= other.start_key.as_slice()
            && self.end_key.as_slice() >= other.end_key.as_slice()
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
    /// Must inspect SSTable for both candidate point key and covering range tombstone.
    InspectDualHit {
        /// Sequence number of the highest applicable range tombstone covering the key.
        tombstone_seq: u64,
    },
}

/// Dual filter evaluator eliminating the range tombstone Bloom blindspot.
pub struct RangeTombstoneBloomDualFilter;

impl RangeTombstoneBloomDualFilter {
    /// Evaluates the dual filter condition for a query key `k`, validating tombstone integrity.
    pub fn evaluate_checked(
        bloom_matches_point: bool,
        range_tombstones: &[RangeTombstoneSpan],
        key: &[u8],
        read_snapshot_seq: u64,
    ) -> Result<PruneDecision, RangeTombstoneError> {
        if read_snapshot_seq == 0 {
            return Err(RangeTombstoneError::ZeroSequenceNumber);
        }
        if key.is_empty() {
            return Err(RangeTombstoneError::EmptyQueryKey);
        }
        for tombstone in range_tombstones {
            tombstone.validate()?;
        }

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
            if bloom_matches_point {
                Ok(PruneDecision::InspectDualHit { tombstone_seq })
            } else {
                Ok(PruneDecision::InspectRangeTombstoneHit { tombstone_seq })
            }
        } else if bloom_matches_point {
            Ok(PruneDecision::InspectPointHit)
        } else {
            Ok(PruneDecision::SafeToPrune)
        }
    }

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
            if bloom_matches_point {
                PruneDecision::InspectDualHit { tombstone_seq }
            } else {
                PruneDecision::InspectRangeTombstoneHit { tombstone_seq }
            }
        } else if bloom_matches_point {
            PruneDecision::InspectPointHit
        } else {
            PruneDecision::SafeToPrune
        }
    }

    /// Determines whether the SSTable can be completely skipped (pruned).
    #[must_use]
    pub fn should_prune(
        bloom_matches_point: bool,
        range_tombstones: &[RangeTombstoneSpan],
        key: &[u8],
        read_snapshot_seq: u64,
    ) -> bool {
        Self::evaluate(bloom_matches_point, range_tombstones, key, read_snapshot_seq)
            == PruneDecision::SafeToPrune
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_snapshot_seq_rejected() {
        let tombstones = [RangeTombstoneSpan::new(b"a".to_vec(), b"z".to_vec(), 10)];
        let err = RangeTombstoneBloomDualFilter::evaluate_checked(false, &tombstones, b"m", 0).unwrap_err();
        assert_eq!(err, RangeTombstoneError::ZeroSequenceNumber);
    }

    #[test]
    fn test_empty_query_key_rejected() {
        let tombstones = [RangeTombstoneSpan::new(b"a".to_vec(), b"z".to_vec(), 10)];
        let err = RangeTombstoneBloomDualFilter::evaluate_checked(false, &tombstones, b"", 10).unwrap_err();
        assert_eq!(err, RangeTombstoneError::EmptyQueryKey);
    }

    #[test]
    fn test_span_overlaps_and_contains() {
        let s1 = RangeTombstoneSpan::new(b"10".to_vec(), b"50".to_vec(), 1);
        let s2 = RangeTombstoneSpan::new(b"20".to_vec(), b"40".to_vec(), 1);
        let s3 = RangeTombstoneSpan::new(b"45".to_vec(), b"70".to_vec(), 1);
        let s4 = RangeTombstoneSpan::new(b"60".to_vec(), b"80".to_vec(), 1);

        assert!(s1.contains_span(&s2));
        assert!(!s2.contains_span(&s1));

        assert!(s1.overlaps(&s3));
        assert!(!s1.overlaps(&s4));
    }

    #[test]
    fn test_should_prune_helper() {
        let tombstones = [RangeTombstoneSpan::new(b"10".to_vec(), b"50".to_vec(), 10)];
        // Outside range and bloom miss -> should prune
        assert!(RangeTombstoneBloomDualFilter::should_prune(false, &tombstones, b"05", 10));
        // Inside range -> must not prune
        assert!(!RangeTombstoneBloomDualFilter::should_prune(false, &tombstones, b"25", 10));
        // Bloom hit -> must not prune
        assert!(!RangeTombstoneBloomDualFilter::should_prune(true, &tombstones, b"05", 10));
    }
}

