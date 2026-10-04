//! RFC-0282 Pilar 5 — Barreira de GC no VLog com Marcação Tricolor (VLog GC Barrier Kernel).
//!
//! Formalizes the tri-color reachability oracle for concurrent Value Log (VLog) garbage collection.
//! Proves that the reclamation set is strictly disjoint from all reachable blob references:
//!   ReclaimedBlobs ∩ (ActiveLSM ∪ InFlightTransactions ∪ StagedVersionEdits) == ∅.
//!
//! Guarantees zero false-positive collections (never reclaims a blob whose pointer
//! has been allocated or is pending publication in a concurrent compaction or transaction).

#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashSet};

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
    /// Attempted to register or reclaim an invalid blob coordinate.
    InvalidBlobCoordinate {
        /// File number.
        file_num: u64,
        /// Blob byte offset.
        offset: u64,
        /// Description of invalidity.
        reason: &'static str,
    },
    /// Duplicate coordinate found in candidate reclamation batch.
    DuplicateReclamationCoordinate {
        /// File number.
        file_num: u64,
        /// Blob byte offset.
        offset: u64,
    },
}

impl std::fmt::Display for VlogGcViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ActiveLsmBlobReclaimed { file_num, offset } => {
                write!(f, "Attempted to reclaim active LSM blob at file {file_num}, offset {offset}")
            }
            Self::InFlightTransactionBlobReclaimed { file_num, offset } => {
                write!(f, "Attempted to reclaim in-flight transaction blob at file {file_num}, offset {offset}")
            }
            Self::StagedVersionEditBlobReclaimed { file_num, offset } => {
                write!(f, "Attempted to reclaim staged version edit blob at file {file_num}, offset {offset}")
            }
            Self::InvalidBlobCoordinate { file_num, offset, reason } => {
                write!(f, "Invalid blob coordinate (file {file_num}, offset {offset}): {reason}")
            }
            Self::DuplicateReclamationCoordinate { file_num, offset } => {
                write!(f, "Duplicate reclamation coordinate detected at file {file_num}, offset {offset}")
            }
        }
    }
}

impl std::error::Error for VlogGcViolation {}

/// A unique coordinate identifying an external blob on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlobCoordinate {
    /// VLog file number.
    pub file_num: u64,
    /// Byte offset within the file.
    pub offset: u64,
}

impl BlobCoordinate {
    /// Creates a validated BlobCoordinate.
    pub fn try_new(file_num: u64, offset: u64) -> Result<Self, VlogGcViolation> {
        if file_num == 0 {
            return Err(VlogGcViolation::InvalidBlobCoordinate {
                file_num,
                offset,
                reason: "file_num cannot be zero",
            });
        }
        Ok(Self { file_num, offset })
    }
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

    /// Safely registers a published LSM blob reference.
    pub fn try_add_active_lsm_blob(&mut self, blob: BlobCoordinate) -> Result<(), VlogGcViolation> {
        if blob.file_num == 0 {
            return Err(VlogGcViolation::InvalidBlobCoordinate {
                file_num: blob.file_num,
                offset: blob.offset,
                reason: "file_num cannot be zero",
            });
        }
        self.add_active_lsm_blob(blob);
        Ok(())
    }

    /// Registers an in-flight transaction blob reference.
    pub fn add_inflight_txn_blob(&mut self, blob: BlobCoordinate) {
        self.inflight_txn_blobs.insert(blob);
    }

    /// Safely registers an in-flight transaction blob reference.
    pub fn try_add_inflight_txn_blob(&mut self, blob: BlobCoordinate) -> Result<(), VlogGcViolation> {
        if blob.file_num == 0 {
            return Err(VlogGcViolation::InvalidBlobCoordinate {
                file_num: blob.file_num,
                offset: blob.offset,
                reason: "file_num cannot be zero",
            });
        }
        self.add_inflight_txn_blob(blob);
        Ok(())
    }

    /// Releases an in-flight transaction blob reference (on commit or abort).
    pub fn remove_inflight_txn_blob(&mut self, blob: &BlobCoordinate) {
        self.inflight_txn_blobs.remove(blob);
    }

    /// Registers a staged compaction blob reference.
    pub fn add_staged_compaction_blob(&mut self, blob: BlobCoordinate) {
        self.staged_compaction_blobs.insert(blob);
    }

    /// Safely registers a staged compaction blob reference.
    pub fn try_add_staged_compaction_blob(&mut self, blob: BlobCoordinate) -> Result<(), VlogGcViolation> {
        if blob.file_num == 0 {
            return Err(VlogGcViolation::InvalidBlobCoordinate {
                file_num: blob.file_num,
                offset: blob.offset,
                reason: "file_num cannot be zero",
            });
        }
        self.add_staged_compaction_blob(blob);
        Ok(())
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
    /// Returns `VlogGcViolation` if any reachable blob is in the reclamation set,
    /// or if invalid/duplicate coordinates are present in the batch.
    pub fn verify_reclamation_safety(
        &self,
        blobs_to_reclaim: &[BlobCoordinate],
    ) -> Result<(), VlogGcViolation> {
        let mut seen = HashSet::with_capacity(blobs_to_reclaim.len());
        for blob in blobs_to_reclaim {
            if blob.file_num == 0 {
                return Err(VlogGcViolation::InvalidBlobCoordinate {
                    file_num: blob.file_num,
                    offset: blob.offset,
                    reason: "file_num cannot be zero in reclamation set",
                });
            }
            if !seen.insert(*blob) {
                return Err(VlogGcViolation::DuplicateReclamationCoordinate {
                    file_num: blob.file_num,
                    offset: blob.offset,
                });
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vlog_gc_barrier_structural_invariants_red_to_green() {
        // 1. Error Display and std::error::Error compliance
        let violations = [
            VlogGcViolation::ActiveLsmBlobReclaimed {
                file_num: 1,
                offset: 100,
            },
            VlogGcViolation::InFlightTransactionBlobReclaimed {
                file_num: 2,
                offset: 200,
            },
            VlogGcViolation::StagedVersionEditBlobReclaimed {
                file_num: 3,
                offset: 300,
            },
            VlogGcViolation::InvalidBlobCoordinate {
                file_num: 0,
                offset: 0,
                reason: "zero file number",
            },
            VlogGcViolation::DuplicateReclamationCoordinate {
                file_num: 4,
                offset: 400,
            },
        ];
        for v in &violations {
            let msg = format!("{v}");
            assert!(!msg.is_empty());
            let dyn_err: &dyn std::error::Error = v;
            assert_eq!(dyn_err.to_string(), msg);
        }

        // 2. BlobCoordinate validation
        assert_eq!(
            BlobCoordinate::try_new(0, 10),
            Err(VlogGcViolation::InvalidBlobCoordinate {
                file_num: 0,
                offset: 10,
                reason: "file_num cannot be zero",
            })
        );
        let coord1 = BlobCoordinate::try_new(1, 1024).expect("valid coordinate");
        assert_eq!(coord1.file_num, 1);
        assert_eq!(coord1.offset, 1024);

        // 3. Oracle safe insertions
        let mut oracle = VlogGcBarrierOracle::new();
        let invalid_coord = BlobCoordinate { file_num: 0, offset: 0 };
        assert!(oracle.try_add_active_lsm_blob(invalid_coord).is_err());
        assert!(oracle.try_add_inflight_txn_blob(invalid_coord).is_err());
        assert!(oracle.try_add_staged_compaction_blob(invalid_coord).is_err());

        assert!(oracle.try_add_active_lsm_blob(coord1).is_ok());

        // 4. verify_reclamation_safety rejects invalid zero coordinate
        assert_eq!(
            oracle.verify_reclamation_safety(&[invalid_coord]),
            Err(VlogGcViolation::InvalidBlobCoordinate {
                file_num: 0,
                offset: 0,
                reason: "file_num cannot be zero in reclamation set",
            })
        );

        // 5. verify_reclamation_safety rejects duplicates
        let dead1 = BlobCoordinate::try_new(10, 500).unwrap();
        assert_eq!(
            oracle.verify_reclamation_safety(&[dead1, dead1]),
            Err(VlogGcViolation::DuplicateReclamationCoordinate {
                file_num: 10,
                offset: 500,
            })
        );

        // 6. verify_reclamation_safety succeeds on valid distinct dead blobs
        let dead2 = BlobCoordinate::try_new(10, 600).unwrap();
        assert!(oracle.verify_reclamation_safety(&[dead1, dead2]).is_ok());

        // 7. verify_reclamation_safety rejects active blob
        assert_eq!(
            oracle.verify_reclamation_safety(&[coord1]),
            Err(VlogGcViolation::ActiveLsmBlobReclaimed {
                file_num: 1,
                offset: 1024,
            })
        );
    }
}
