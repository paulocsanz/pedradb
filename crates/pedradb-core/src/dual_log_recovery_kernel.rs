//! RFC-0281 P0.2 — Dual-Log Recovery & Anti-Zombie Resurrection Kernel.
//!
//! Formalizes the coupled replay invariant between the MANIFEST and WAL logs:
//!   - Segment Lower Bound: log_number < min_log_number must be rejected/skipped;
//!   - Sequence Lower Bound: record_seq <= earliest_readable_seq has already been
//!     consolidated into SSTables and must not overwrite newer database state.
//!
//! Guarantees that no obsolete mutation from un-garbage-collected WAL files or
//! pre-flush sequences can resurrect "zombie" records post-crash.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

/// Violations of the dual-log crash-recovery coupling invariant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DualLogRecoveryViolation {
    /// A WAL record with a sequence number already flushed to SSTable was incorrectly applied.
    ZombieResurrectionAttempted {
        /// Sequence number of the zombie record.
        seq: u64,
        /// MANIFEST cutoff threshold.
        earliest_readable_seq: u64,
        /// The resurrected key.
        key: Vec<u8>,
    },
    /// A WAL segment older than the MANIFEST minimum log number was admitted.
    ObsoleteSegmentAdmitted {
        /// Segment ID of the obsolete file.
        admitted_segment: u64,
        /// Minimum allowed segment in MANIFEST.
        min_log_number: u64,
    },
    /// Sequence numbers in the WAL went backwards within the replay phase.
    NonMonotonicWalSequence {
        /// Previous sequence number observed.
        prev_seq: u64,
        /// Current sequence number observed.
        curr_seq: u64,
    },
    /// Minimum log number cannot be zero.
    ZeroLogNumber,
    /// Manifest head sequence cannot be lower than earliest readable sequence.
    InvalidHeadSeq {
        /// Recorded manifest head sequence.
        head_seq: u64,
        /// Earliest readable sequence.
        earliest_readable: u64,
    },
    /// WAL segment ID cannot be zero.
    ZeroSegmentId,
    /// Sequence number cannot be zero.
    ZeroSeq,
    /// Record key cannot be empty.
    EmptyKey,
}

impl std::fmt::Display for DualLogRecoveryViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZombieResurrectionAttempted { seq, earliest_readable_seq, .. } => {
                write!(f, "Zombie resurrection attempted: seq {seq} <= cutoff {earliest_readable_seq}")
            }
            Self::ObsoleteSegmentAdmitted { admitted_segment, min_log_number } => {
                write!(f, "Obsolete segment {admitted_segment} admitted < min log {min_log_number}")
            }
            Self::NonMonotonicWalSequence { prev_seq, curr_seq } => {
                write!(f, "Non-monotonic WAL sequence: prev {prev_seq}, curr {curr_seq}")
            }
            Self::ZeroLogNumber => {
                write!(f, "Minimum log number cannot be zero")
            }
            Self::InvalidHeadSeq { head_seq, earliest_readable } => {
                write!(f, "Manifest head seq {head_seq} < earliest readable seq {earliest_readable}")
            }
            Self::ZeroSegmentId => {
                write!(f, "WAL segment ID cannot be zero")
            }
            Self::ZeroSeq => {
                write!(f, "Sequence number cannot be zero")
            }
            Self::EmptyKey => {
                write!(f, "Record key cannot be empty")
            }
        }
    }
}

impl std::error::Error for DualLogRecoveryViolation {}

/// The durable anchor established by the MANIFEST before WAL replay begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ManifestRecoveryAnchor {
    /// Oldest WAL segment needed for crash recovery. Any segment < this is dead.
    pub min_log_number: u64,
    /// Highest sequence number already guaranteed to be durable in SSTables.
    pub earliest_readable_seq: u64,
    /// Highest sequence number recorded in the MANIFEST version edit.
    pub manifest_head_seq: u64,
}

impl ManifestRecoveryAnchor {
    /// Creates a validated manifest recovery anchor.
    pub fn try_new(
        min_log_number: u64,
        earliest_readable_seq: u64,
        manifest_head_seq: u64,
    ) -> Result<Self, DualLogRecoveryViolation> {
        if min_log_number == 0 {
            return Err(DualLogRecoveryViolation::ZeroLogNumber);
        }
        if manifest_head_seq < earliest_readable_seq {
            return Err(DualLogRecoveryViolation::InvalidHeadSeq {
                head_seq: manifest_head_seq,
                earliest_readable: earliest_readable_seq,
            });
        }
        Ok(Self {
            min_log_number,
            earliest_readable_seq,
            manifest_head_seq,
        })
    }
}

/// A physical entry read from a WAL segment during recovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalRecoveryRecord {
    /// WAL segment / file number.
    pub segment_id: u64,
    /// Monotonically increasing sequence number.
    pub seq: u64,
    /// Key payload.
    pub key: Vec<u8>,
    /// Value payload (`None` indicates a tombstone deletion).
    pub value: Option<Vec<u8>>,
}

impl WalRecoveryRecord {
    /// Creates a validated WAL recovery record.
    pub fn try_new(
        segment_id: u64,
        seq: u64,
        key: Vec<u8>,
        value: Option<Vec<u8>>,
    ) -> Result<Self, DualLogRecoveryViolation> {
        if segment_id == 0 {
            return Err(DualLogRecoveryViolation::ZeroSegmentId);
        }
        if seq == 0 {
            return Err(DualLogRecoveryViolation::ZeroSeq);
        }
        if key.is_empty() {
            return Err(DualLogRecoveryViolation::EmptyKey);
        }
        Ok(Self {
            segment_id,
            seq,
            key,
            value,
        })
    }
}

/// Action decided by the recovery gate for each individual WAL record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalReplayDecision {
    /// Skip because the WAL segment is obsolete (older than MANIFEST `min_log_number`).
    SkipObsoleteSegment,
    /// Skip because sequence number is already incorporated in SSTable (`seq <= earliest_readable_seq`).
    SkipConsolidatedSeq,
    /// Apply to the recovered MemTable (`seq > earliest_readable_seq`).
    ApplyToMemTable,
}

/// Replay gate enforcing the dual-log recovery invariant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DualLogRecoveryGate {
    anchor: ManifestRecoveryAnchor,
    last_applied_seq: u64,
}

impl DualLogRecoveryGate {
    /// Initializes the recovery gate with validation of the anchor.
    pub fn try_new(anchor: ManifestRecoveryAnchor) -> Result<Self, DualLogRecoveryViolation> {
        if anchor.min_log_number == 0 {
            return Err(DualLogRecoveryViolation::ZeroLogNumber);
        }
        if anchor.manifest_head_seq < anchor.earliest_readable_seq {
            return Err(DualLogRecoveryViolation::InvalidHeadSeq {
                head_seq: anchor.manifest_head_seq,
                earliest_readable: anchor.earliest_readable_seq,
            });
        }
        Ok(Self::new(anchor))
    }

    /// Initializes the recovery gate with the validated MANIFEST anchor.
    #[must_use]
    pub fn new(anchor: ManifestRecoveryAnchor) -> Self {
        let last_applied_seq = anchor.earliest_readable_seq;
        Self {
            anchor,
            last_applied_seq,
        }
    }

    /// Evaluates a WAL record against the MANIFEST anchor and classifies the action.
    #[must_use]
    pub fn decide(&self, record: &WalRecoveryRecord) -> WalReplayDecision {
        if record.segment_id < self.anchor.min_log_number {
            WalReplayDecision::SkipObsoleteSegment
        } else if record.seq <= self.anchor.earliest_readable_seq {
            WalReplayDecision::SkipConsolidatedSeq
        } else {
            WalReplayDecision::ApplyToMemTable
        }
    }

    /// Replays a stream of WAL records into an in-memory recovered state,
    /// formally verifying that no zombie resurrection or monotonicity violation occurs.
    ///
    /// # Errors
    /// Returns `DualLogRecoveryViolation` if any record violates recovery coupling rules.
    pub fn execute_verified_replay(
        &mut self,
        records: &[WalRecoveryRecord],
    ) -> Result<BTreeMap<Vec<u8>, Option<Vec<u8>>>, DualLogRecoveryViolation> {
        let mut recovered_state = BTreeMap::new();
        let mut highest_observed_seq = self.anchor.earliest_readable_seq;

        for record in records {
            let decision = self.decide(record);
            match decision {
                WalReplayDecision::SkipObsoleteSegment => {
                    // Correctly ignored
                }
                WalReplayDecision::SkipConsolidatedSeq => {
                    // Correctly ignored to avoid overwriting SSTable state
                }
                WalReplayDecision::ApplyToMemTable => {
                    // Verify monotonicity
                    if record.seq <= highest_observed_seq {
                        return Err(DualLogRecoveryViolation::NonMonotonicWalSequence {
                            prev_seq: highest_observed_seq,
                            curr_seq: record.seq,
                        });
                    }
                    highest_observed_seq = record.seq;
                    self.last_applied_seq = record.seq;
                    recovered_state.insert(record.key.clone(), record.value.clone());
                }
            }
        }

        Ok(recovered_state)
    }

    /// Returns the highest sequence number applied during recovery.
    #[must_use]
    pub fn last_applied_seq(&self) -> u64 {
        self.last_applied_seq
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dual_log_anchor_and_record_bounds() {
        assert_eq!(
            ManifestRecoveryAnchor::try_new(0, 10, 20),
            Err(DualLogRecoveryViolation::ZeroLogNumber)
        );

        assert_eq!(
            ManifestRecoveryAnchor::try_new(1, 20, 10),
            Err(DualLogRecoveryViolation::InvalidHeadSeq {
                head_seq: 10,
                earliest_readable: 20,
            })
        );

        let anchor = ManifestRecoveryAnchor::try_new(1, 10, 20).expect("valid anchor");
        let gate = DualLogRecoveryGate::try_new(anchor).expect("valid gate");
        assert_eq!(gate.last_applied_seq(), 10);

        assert_eq!(
            WalRecoveryRecord::try_new(0, 1, vec![1], None),
            Err(DualLogRecoveryViolation::ZeroSegmentId)
        );

        assert_eq!(
            WalRecoveryRecord::try_new(1, 0, vec![1], None),
            Err(DualLogRecoveryViolation::ZeroSeq)
        );

        assert_eq!(
            WalRecoveryRecord::try_new(1, 1, vec![], None),
            Err(DualLogRecoveryViolation::EmptyKey)
        );
    }

    #[test]
    fn test_dual_log_violation_display() {
        let err = DualLogRecoveryViolation::ZeroLogNumber;
        assert_eq!(format!("{err}"), "Minimum log number cannot be zero");

        let err2 = DualLogRecoveryViolation::ZeroSeq;
        assert_eq!(format!("{err2}"), "Sequence number cannot be zero");

        let err3 = DualLogRecoveryViolation::EmptyKey;
        assert_eq!(format!("{err3}"), "Record key cannot be empty");
    }
}

