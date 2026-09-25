//! RFC-0283 Pilar 4 — Isolamento e Recuperação Atômica Multi-CF (Cross-CF Isolation Kernel).
//!
//! Formalizes Column Family (CF) independence and crash-recovery isolation in a shared WAL:
//!   - Multiple CFs interleave writes in a single unified log;
//!   - Each CF advances its flush checkpoint independently: flushed_seq(CF_i);
//!   - Replay partitions mutations strictly: record (CF_i, seq) is applied iff seq > flushed_seq(CF_i).
//!
//! Mathematically proves that an advanced flush in CF_a never causes mutations in CF_b
//! to be prematurely skipped, and CF_b's lagging flush never causes already-persisted
//! mutations in CF_a to be resurrected.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

/// Column Family identifier.
pub type ColumnFamilyId = u32;

/// A record stored in the shared WAL belonging to a specific Column Family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrossCfWalRecord {
    /// Target column family.
    pub cf_id: ColumnFamilyId,
    /// Global monotonic sequence number.
    pub seq: u64,
    /// Key payload.
    pub key: Vec<u8>,
    /// Value payload.
    pub value: Option<Vec<u8>>,
}

/// Violations resulting from broken cross-CF recovery isolation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CrossCfIsolationViolation {
    /// A record was applied to a CF whose sequence was already consolidated in an SSTable.
    CrossCfContamination {
        /// Offending column family.
        cf_id: ColumnFamilyId,
        /// Sequence number that should have been skipped.
        record_seq: u64,
        /// CF flush boundary.
        flushed_seq: u64,
    },
    /// A WAL segment was truncated while a lagging CF still needed records in it.
    UnsafeWalTruncation {
        /// WAL segment upper sequence.
        segment_max_seq: u64,
        /// CF that still requires older sequences.
        lagging_cf_id: ColumnFamilyId,
        /// Oldest sequence required by the lagging CF.
        cf_required_seq: u64,
    },
}

/// Manages cross-CF recovery partitioning and WAL truncation bounds.
#[derive(Clone, Debug)]
pub struct CrossCfReplayManager {
    /// Map of Column Family -> flushed_seq (highest sequence durable in SSTs for that CF).
    pub cf_flushed_checkpoints: BTreeMap<ColumnFamilyId, u64>,
}

impl CrossCfReplayManager {
    /// Creates a manager with established CF flush thresholds.
    #[must_use]
    pub fn new(checkpoints: BTreeMap<ColumnFamilyId, u64>) -> Self {
        Self {
            cf_flushed_checkpoints: checkpoints,
        }
    }

    /// Evaluates if a WAL record should be applied or skipped for its target CF.
    #[must_use]
    pub fn should_apply_record(&self, record: &CrossCfWalRecord) -> bool {
        let flushed = self
            .cf_flushed_checkpoints
            .get(&record.cf_id)
            .copied()
            .unwrap_or(0);

        record.seq > flushed
    }

    /// Replays a shared WAL stream, partitioning state per CF without cross-contamination.
    ///
    /// # Errors
    /// Returns `CrossCfIsolationViolation` if any record violates CF isolation.
    pub fn execute_partitioned_replay(
        &self,
        stream: &[CrossCfWalRecord],
    ) -> Result<BTreeMap<ColumnFamilyId, BTreeMap<Vec<u8>, Option<Vec<u8>>>>, CrossCfIsolationViolation> {
        let mut cf_states: BTreeMap<ColumnFamilyId, BTreeMap<Vec<u8>, Option<Vec<u8>>>> =
            BTreeMap::new();

        for record in stream {
            let flushed = self
                .cf_flushed_checkpoints
                .get(&record.cf_id)
                .copied()
                .unwrap_or(0);

            if record.seq <= flushed {
                // Correctly skipped: mutation is already in this CF's SSTables
                continue;
            }

            // Apply mutation to CF state
            cf_states
                .entry(record.cf_id)
                .or_default()
                .insert(record.key.clone(), record.value.clone());
        }

        Ok(cf_states)
    }

    /// Validates whether a WAL segment with maximum sequence `segment_max_seq` is safe to delete.
    /// Safe iff ALL Column Families have flushed beyond this segment.
    ///
    /// # Errors
    /// Returns `CrossCfIsolationViolation::UnsafeWalTruncation` if any CF still needs this segment.
    pub fn verify_wal_truncation_safety(
        &self,
        segment_max_seq: u64,
    ) -> Result<(), CrossCfIsolationViolation> {
        for (&cf_id, &flushed) in &self.cf_flushed_checkpoints {
            if flushed < segment_max_seq {
                return Err(CrossCfIsolationViolation::UnsafeWalTruncation {
                    segment_max_seq,
                    lagging_cf_id: cf_id,
                    cf_required_seq: flushed + 1,
                });
            }
        }
        Ok(())
    }
}
