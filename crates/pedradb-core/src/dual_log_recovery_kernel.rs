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
}

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
pub struct DualLogRecoveryGate {
    anchor: ManifestRecoveryAnchor,
    last_applied_seq: u64,
}

impl DualLogRecoveryGate {
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
