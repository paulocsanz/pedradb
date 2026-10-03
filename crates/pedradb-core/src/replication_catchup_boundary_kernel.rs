//! RFC-0283 Pilar 10 — Continuidade Monotônica Snapshot-para-Log na Replicação (Replication Catch-Up Boundary Kernel).
//!
//! Formalizes the boundary transition between bulk SSTable snapshot installation
//! and live streaming WAL replication on lagging distributed replicas.
//! Proves that the transition boundary is hermetically sealed:
//!   S_first_stream == S_snapshot + 1  ⇒  Gap == ∅ ∧ Overlap == ∅.
//!
//! Guarantees that the follower's reconstructed state is strictly identical to an
//! unbroken linear log replay without dropped mutations or duplicate execution.

#![forbid(unsafe_code)]

use std::fmt;

/// Replicated mutation entry received from the leader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicatedLogEntry {
    /// Global monotonic sequence number.
    pub seq: u64,
    /// Key payload.
    pub key: Vec<u8>,
    /// Value payload.
    pub value: Option<Vec<u8>>,
}

impl ReplicatedLogEntry {
    /// Safely constructs a validated log entry, rejecting zero sequence and empty keys.
    pub fn try_new(
        seq: u64,
        key: Vec<u8>,
        value: Option<Vec<u8>>,
    ) -> Result<Self, ReplicationBoundaryViolation> {
        if seq == 0 {
            return Err(ReplicationBoundaryViolation::ZeroSequenceNumber);
        }
        if key.is_empty() {
            return Err(ReplicationBoundaryViolation::EmptyKey);
        }
        Ok(Self { seq, key, value })
    }
}

/// Violations resulting from broken snapshot-to-log replication continuity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplicationBoundaryViolation {
    /// A sequence gap was detected (mutations between snapshot and log stream were dropped).
    ReplicationGapDetected {
        /// Sequence number of the snapshot cutoff.
        snapshot_cutoff_seq: u64,
        /// First sequence number in the streaming log.
        stream_first_seq: u64,
        /// Missing sequence count.
        gap_size: u64,
    },
    /// Log stream started before snapshot cutoff without idempotency protection.
    UncheckedStreamOverlap {
        /// Sequence number of the snapshot cutoff.
        snapshot_cutoff_seq: u64,
        /// First sequence number in the streaming log.
        stream_first_seq: u64,
    },
    /// Log stream sequences went backwards during replication catch-up.
    NonMonotonicStream {
        /// Previous sequence.
        prev_seq: u64,
        /// Current sequence.
        curr_seq: u64,
    },
    /// Snapshot cutoff sequence or entry sequence is zero.
    ZeroSequenceNumber,
    /// Key payload is empty.
    EmptyKey,
    /// Sequence addition overflowed u64::MAX.
    SequenceOverflow {
        /// Offending sequence number.
        seq: u64,
    },
}

impl fmt::Display for ReplicationBoundaryViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReplicationGapDetected { snapshot_cutoff_seq, stream_first_seq, gap_size } => {
                write!(f, "Replication gap of {} entries detected between cutoff {} and stream {}", gap_size, snapshot_cutoff_seq, stream_first_seq)
            }
            Self::UncheckedStreamOverlap { snapshot_cutoff_seq, stream_first_seq } => {
                write!(f, "Unchecked stream overlap: stream starts at {} before cutoff {}", stream_first_seq, snapshot_cutoff_seq)
            }
            Self::NonMonotonicStream { prev_seq, curr_seq } => {
                write!(f, "Replication stream non-monotonic: curr {} <= prev {}", curr_seq, prev_seq)
            }
            Self::ZeroSequenceNumber => write!(f, "Sequence number cannot be zero"),
            Self::EmptyKey => write!(f, "Replicated log key cannot be empty"),
            Self::SequenceOverflow { seq } => write!(f, "Sequence number {} overflowed u64::MAX", seq),
        }
    }
}

impl std::error::Error for ReplicationBoundaryViolation {}

/// Reconciler managing the boundary between snapshot ingestion and live streaming catch-up.
pub struct ReplicationBoundaryReconciler;

impl ReplicationBoundaryReconciler {
    /// Verifies that a streaming WAL log correctly stitches onto an installed snapshot.
    ///
    /// # Errors
    /// Returns `ReplicationBoundaryViolation` if a gap or non-monotonic sequence is found.
    pub fn verify_boundary_continuity(
        snapshot_cutoff_seq: u64,
        stream: &[ReplicatedLogEntry],
    ) -> Result<Vec<ReplicatedLogEntry>, ReplicationBoundaryViolation> {
        if stream.is_empty() {
            // Empty stream after snapshot is valid (up to date)
            return Ok(Vec::new());
        }

        let first_seq = stream[0].seq;

        // Condition 1: Check for gap safely with checked arithmetic
        let expected_first = match snapshot_cutoff_seq.checked_add(1) {
            Some(s) => s,
            None => {
                return Err(ReplicationBoundaryViolation::SequenceOverflow {
                    seq: snapshot_cutoff_seq,
                });
            }
        };

        if first_seq > expected_first {
            return Err(ReplicationBoundaryViolation::ReplicationGapDetected {
                snapshot_cutoff_seq,
                stream_first_seq: first_seq,
                gap_size: first_seq - expected_first,
            });
        }

        let mut filtered_entries = Vec::new();
        let mut last_seq = snapshot_cutoff_seq;

        for entry in stream {
            if entry.seq <= snapshot_cutoff_seq {
                // Idempotently skip entries already incorporated into the physical snapshot
                continue;
            }

            // Verify strict monotonic progression: entry.seq == last_seq + 1
            if entry.seq <= last_seq {
                return Err(ReplicationBoundaryViolation::NonMonotonicStream {
                    prev_seq: last_seq,
                    curr_seq: entry.seq,
                });
            }

            let next_expected = match last_seq.checked_add(1) {
                Some(s) => s,
                None => {
                    return Err(ReplicationBoundaryViolation::SequenceOverflow { seq: last_seq });
                }
            };

            if entry.seq > next_expected {
                return Err(ReplicationBoundaryViolation::ReplicationGapDetected {
                    snapshot_cutoff_seq: last_seq,
                    stream_first_seq: entry.seq,
                    gap_size: entry.seq - next_expected,
                });
            }

            last_seq = entry.seq;
            filtered_entries.push(entry.clone());
        }

        Ok(filtered_entries)
    }

    /// Verifies boundary continuity under strict non-overlapping policy,
    /// rejecting any prefix overlaps with `UncheckedStreamOverlap`.
    pub fn verify_strict_boundary_continuity(
        snapshot_cutoff_seq: u64,
        stream: &[ReplicatedLogEntry],
    ) -> Result<Vec<ReplicatedLogEntry>, ReplicationBoundaryViolation> {
        if stream.is_empty() {
            return Ok(Vec::new());
        }

        if stream[0].seq <= snapshot_cutoff_seq {
            return Err(ReplicationBoundaryViolation::UncheckedStreamOverlap {
                snapshot_cutoff_seq,
                stream_first_seq: stream[0].seq,
            });
        }

        Self::verify_boundary_continuity(snapshot_cutoff_seq, stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replication_boundary_hardening_red_to_green() {
        // 1. Rejeita entrada com seq zero
        assert_eq!(
            ReplicatedLogEntry::try_new(0, b"key".to_vec(), None),
            Err(ReplicationBoundaryViolation::ZeroSequenceNumber)
        );

        // 2. Rejeita chave vazia
        assert_eq!(
            ReplicatedLogEntry::try_new(10, vec![], None),
            Err(ReplicationBoundaryViolation::EmptyKey)
        );

        // 3. Overflow em u64::MAX é tratado sem panic
        let stream = vec![ReplicatedLogEntry {
            seq: u64::MAX,
            key: b"k".to_vec(),
            value: None,
        }];
        assert_eq!(
            ReplicationBoundaryReconciler::verify_boundary_continuity(u64::MAX, &stream),
            Err(ReplicationBoundaryViolation::SequenceOverflow { seq: u64::MAX })
        );

        // 4. Strict mode rejeita overlap
        let overlap = vec![ReplicatedLogEntry {
            seq: 100,
            key: b"k".to_vec(),
            value: None,
        }];
        assert_eq!(
            ReplicationBoundaryReconciler::verify_strict_boundary_continuity(100, &overlap),
            Err(ReplicationBoundaryViolation::UncheckedStreamOverlap {
                snapshot_cutoff_seq: 100,
                stream_first_seq: 100,
            })
        );
    }
}

