//! Pre-Manifest Orphan SST Recovery and Idempotent Cleanup Kernel (RFC-0284 Pilar 8).
//!
//! Reconciles disk files against MANIFEST state during post-crash initialization,
//! safely scrubbing uncommitted orphan SST files without risking active data.
//!
//! Guarantees:
//! 1. Partitioning: files on disk are partitioned into `Active`, `Orphan`, or `CorruptedMissing`.
//! 2. Invariant: `OrphanFileNumber < Manifest.NextFileNumber` (no conflict with future files).
//! 3. Fail-closed: missing manifest-referenced files immediately trigger recovery abortion.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;

/// Error detected during post-crash directory and manifest reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogReconciliationError {
    /// A file recorded as active in MANIFEST is missing from disk (data loss).
    MissingActiveFile { file_number: u64 },
    /// An orphan file has a number >= Manifest.NextFileNumber (watermark corruption).
    OrphanViolatesWatermark { file_number: u64, next_file_number: u64 },
}

/// Recovery plan produced by the orphan scrubber.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryScrubPlan {
    /// Files confirmed active and kept open by the storage engine.
    pub active_files: BTreeSet<u64>,
    /// Uncommitted orphan files to be safely unlinked.
    pub orphan_files_to_delete: BTreeSet<u64>,
}

/// Scrubber that audits disk files against canonical manifest records.
pub struct PreManifestOrphanScrubber;

impl PreManifestOrphanScrubber {
    /// Computes reconciliation plan between disk files and manifest state.
    pub fn reconcile(
        manifest_active_files: &BTreeSet<u64>,
        manifest_next_file_number: u64,
        disk_files: &BTreeSet<u64>,
    ) -> Result<RecoveryScrubPlan, CatalogReconciliationError> {
        // 1. Verify every active file in manifest exists on disk
        for active_fn in manifest_active_files {
            if !disk_files.contains(active_fn) {
                return Err(CatalogReconciliationError::MissingActiveFile {
                    file_number: *active_fn,
                });
            }
        }

        // 2. Partition disk files
        let mut active_files = BTreeSet::new();
        let mut orphan_files_to_delete = BTreeSet::new();

        for disk_fn in disk_files {
            if manifest_active_files.contains(disk_fn) {
                active_files.insert(*disk_fn);
            } else {
                // Orphan candidate: verify it does not violate future allocation watermark
                if *disk_fn >= manifest_next_file_number {
                    return Err(CatalogReconciliationError::OrphanViolatesWatermark {
                        file_number: *disk_fn,
                        next_file_number: manifest_next_file_number,
                    });
                }
                orphan_files_to_delete.insert(*disk_fn);
            }
        }

        Ok(RecoveryScrubPlan {
            active_files,
            orphan_files_to_delete,
        })
    }
}
