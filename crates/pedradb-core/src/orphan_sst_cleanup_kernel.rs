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

use std::path::{Path, PathBuf};

/// Detailed error when physical files on disk fail the strict catalog invariant (Barreira 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskResourceInvariantError {
    /// A committed SST file in manifest is missing from disk.
    MissingSstFile { file_number: u64, path: PathBuf },
    /// An uncommitted / untracked SST file exists on disk (disk leak / orphan).
    OrphanSstFile { file_number: u64, path: PathBuf },
    /// A temporary (.tmp) file was abandoned on disk.
    AbandonedTmpFile { path: PathBuf },
    /// Disk file violates the allocator watermark.
    WatermarkViolation { file_number: u64, next_file_number: u64 },
    /// I/O error reading directory.
    Io(String),
}

impl std::fmt::Display for DiskResourceInvariantError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingSstFile { file_number, path } => {
                write!(f, "Missing active SST {file_number} at {}", path.display())
            }
            Self::OrphanSstFile { file_number, path } => {
                write!(f, "Orphan / uncommitted SST {file_number} found at {}", path.display())
            }
            Self::AbandonedTmpFile { path } => {
                write!(f, "Abandoned temporary file found at {}", path.display())
            }
            Self::WatermarkViolation { file_number, next_file_number } => {
                write!(f, "File number {file_number} >= next_file_number {next_file_number}")
            }
            Self::Io(msg) => write!(f, "I/O error during disk invariant check: {msg}"),
        }
    }
}

impl std::error::Error for DiskResourceInvariantError {}

/// Enforces physical disk resource invariant (Barreira 2):
/// Every .sst file on disk MUST strictly belong to the engine's active inventory,
/// zero .tmp files may be abandoned, and all manifest files must be present on disk.
pub fn assert_no_orphan_files_invariant(
    db_dir: &Path,
    active_sst_nums: &[u64],
    next_file_num: u64,
) -> Result<(), DiskResourceInvariantError> {
    let manifest_set: BTreeSet<u64> = active_sst_nums.iter().copied().collect();
    let entries = match std::fs::read_dir(db_dir) {
        Ok(e) => e,
        Err(e) => return Err(DiskResourceInvariantError::Io(e.to_string())),
    };

    let mut disk_ssts = BTreeSet::new();

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => return Err(DiskResourceInvariantError::Io(e.to_string())),
        };
        let path = entry.path();
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();

        // 1. Check for abandoned temporary files (.tmp)
        if name_str.ends_with(".tmp") {
            return Err(DiskResourceInvariantError::AbandonedTmpFile { path });
        }

        // 2. Identify SST files
        if name_str.ends_with(".sst") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if let Ok(num) = stem.parse::<u64>() {
                    disk_ssts.insert((num, path.clone()));
                }
            }
        }
    }

    // 3. Verify that every active file in manifest exists on disk
    for &active_num in &manifest_set {
        let sst_name = format!("{active_num:06}.sst");
        let expected_path = db_dir.join(&sst_name);
        if !disk_ssts.iter().any(|(num, _)| *num == active_num) {
            return Err(DiskResourceInvariantError::MissingSstFile {
                file_number: active_num,
                path: expected_path,
            });
        }
    }

    // 4. Verify that NO file violates allocator watermark, and NO orphan SST exists on disk
    for (num, path) in &disk_ssts {
        if *num >= next_file_num {
            return Err(DiskResourceInvariantError::WatermarkViolation {
                file_number: *num,
                next_file_number: next_file_num,
            });
        }
        if !manifest_set.contains(num) {
            return Err(DiskResourceInvariantError::OrphanSstFile {
                file_number: *num,
                path: path.clone(),
            });
        }
    }

    Ok(())
}

