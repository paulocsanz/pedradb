//! RFC-0282 Pilar 5 — Barreira de GC no VLog com Marcação Tricolor (VLog GC Barrier Kernel).
//!
//! Formalizes the tri-color reachability oracle for concurrent Value Log (VLog) garbage collection.
//! Proves that the reclamation set is strictly disjoint from all reachable blob references:
//!   ReclaimedBlobs ∩ (ActiveLSM ∪ InFlightTransactions ∪ StagedVersionEdits) == ∅.
//!
//! Guarantees zero false-positive collections (never reclaims a blob whose pointer
//! has been allocated or is pending publication in a concurrent compaction or transaction).

#![forbid(unsafe_code)]

use std::collections::BTreeSet;

/// Violations resulting from unsafe garbage collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VlogGcViolation {
    /// Attempted to reclaim a blob actively referenced in an LSM table.
    ActiveLsmBlobReclaimed {
        /// File number.
        file_num: u64,
        /// Blob byte offset.
        offset: u64,
    },
    /// Attempted to reclaim a blob allocated by an in-flight transaction.
    InFlightTransactionBlobReclaimed {
        /// File number.
        file_num: u64,
        /// Blob byte offset.
        offset: u64,
    },
    /// Attempted to reclaim a blob referenced in a staged compaction VersionEdit.
    StagedVersionEditBlobReclaimed {
        /// File number.
        file_num: u64,
        /// Blob byte offset.
        offset: u64,
    },
}

/// A unique coordinate identifying an external blob on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BlobCoordinate {
    /// VLog file number.
    pub file_num: u64,
    /// Byte offset within the file.
    pub offset: u64,
}

/// Tri-color mark state for garbage collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkColor {
    /// White: Candidate for deletion.
    White,
    /// Grey: In-process discovery.
    Grey,
    /// Black: Definitively reachable and protected from deletion.
    Black,
}

/// Oracle managing the tri-color reachability barrier for VLog Garbage Collection.
#[derive(Clone, Debug, Default)]
pub struct VlogGcBarrierOracle {
    /// Active blob references in the published LSM index.
    pub active_lsm_blobs: BTreeSet<BlobCoordinate>,
    /// Blob references allocated by uncommitted in-flight write batches.
    pub inflight_txn_blobs: BTreeSet<BlobCoordinate>,
    /// Blob references staged in uncommitted compaction VersionEdits.
    pub staged_compaction_blobs: BTreeSet<BlobCoordinate>,
}

impl VlogGcBarrierOracle {
    /// Creates a new barrier oracle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a published LSM blob reference.
    pub fn add_active_lsm_blob(&mut self, blob: BlobCoordinate) {
        self.active_lsm_blobs.insert(blob);
    }

    /// Registers an in-flight transaction blob reference.
    pub fn add_inflight_txn_blob(&mut self, blob: BlobCoordinate) {
        self.inflight_txn_blobs.insert(blob);
    }

    /// Releases an in-flight transaction blob reference (on commit or abort).
    pub fn remove_inflight_txn_blob(&mut self, blob: &BlobCoordinate) {
        self.inflight_txn_blobs.remove(blob);
    }

    /// Registers a staged compaction blob reference.
    pub fn add_staged_compaction_blob(&mut self, blob: BlobCoordinate) {
        self.staged_compaction_blobs.insert(blob);
    }

    /// Determines the tri-color mark for a blob.
    #[must_use]
    pub fn classify_blob(&self, blob: &BlobCoordinate) -> MarkColor {
        if self.active_lsm_blobs.contains(blob)
            || self.inflight_txn_blobs.contains(blob)
            || self.staged_compaction_blobs.contains(blob)
        {
            MarkColor::Black
        } else {
            MarkColor::White
        }
    }

    /// Verifies whether a candidate set of blobs can be safely unlinked/reclaimed.
    ///
    /// # Errors
    /// Returns `VlogGcViolation` if any reachable blob is in the reclamation set.
    pub fn verify_reclamation_safety(
        &self,
        blobs_to_reclaim: &[BlobCoordinate],
    ) -> Result<(), VlogGcViolation> {
        for blob in blobs_to_reclaim {
            if self.active_lsm_blobs.contains(blob) {
                return Err(VlogGcViolation::ActiveLsmBlobReclaimed {
                    file_num: blob.file_num,
                    offset: blob.offset,
                });
            }
            if self.inflight_txn_blobs.contains(blob) {
                return Err(VlogGcViolation::InFlightTransactionBlobReclaimed {
                    file_num: blob.file_num,
                    offset: blob.offset,
                });
            }
            if self.staged_compaction_blobs.contains(blob) {
                return Err(VlogGcViolation::StagedVersionEditBlobReclaimed {
                    file_num: blob.file_num,
                    offset: blob.offset,
                });
            }
        }
        Ok(())
    }
}
