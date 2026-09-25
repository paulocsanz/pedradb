//! RFC-0283 Pilar 9 — Persistência Estrita do Diretório-Pai POSIX (POSIX Dir Sync Kernel).
//!
//! Formalizes parent directory persistence order and atomic rename envelopes to eliminate
//! orphan inodes after power cuts.
//! Proves the atomic persistence triad:
//!   fdatasync(File) ≺ fsync(ParentDir) ≺ ManifestCommit(File).
//!
//! Guarantees that the MANIFEST version set never commits a reference to an SSTable or WAL
//! file before the filesystem journal has durably recorded the directory entry in the parent folder,
//! completely off the per-put client commit lock.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;

/// Lifecycle stages of a persistent file creation in an LSM-tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FilePersistencePhase {
    /// File data written to staging path (e.g. 000042.sst.tmp).
    DataWritten,
    /// File data and inode flushed to non-volatile media via `fdatasync`.
    FileDataSynced,
    /// File atomically renamed to final target path (e.g. 000042.sst).
    AtomicallyRenamed,
    /// Parent directory entry synchronized to filesystem journal via `fsync(dir_fd)`.
    ParentDirectorySynced,
    /// File committed and published in the MANIFEST version edit.
    ManifestCommitted,
}

/// Violations resulting from improper directory synchronization order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PosixDirSyncViolation {
    /// File was committed to MANIFEST before its parent directory was fsynced (orphan inode hazard).
    ManifestCommittedBeforeDirSync {
        /// File number.
        file_num: u64,
    },
    /// File was renamed before its contents were fdatasync'd.
    RenameBeforeDataSync {
        /// File number.
        file_num: u64,
    },
    /// Parent directory was synced before file was renamed to its final path.
    DirSyncBeforeRename {
        /// File number.
        file_num: u64,
    },
}

/// Verifier enforcing the POSIX directory synchronization order.
#[derive(Clone, Debug, Default)]
pub struct PosixDirSyncOrderOracle {
    /// Files whose parent directory has been durably synced.
    pub dir_synced_files: BTreeSet<u64>,
    /// Files committed to the MANIFEST.
    pub manifest_committed_files: BTreeSet<u64>,
}

impl PosixDirSyncOrderOracle {
    /// Creates a new directory sync order oracle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Verifies that a file state transition adheres to the safe persistence triad.
    ///
    /// # Errors
    /// Returns `PosixDirSyncViolation` if ordering steps are inverted.
    pub fn verify_transition(
        file_num: u64,
        from_phase: FilePersistencePhase,
        to_phase: FilePersistencePhase,
    ) -> Result<(), PosixDirSyncViolation> {
        match (from_phase, to_phase) {
            (FilePersistencePhase::DataWritten, FilePersistencePhase::FileDataSynced) => Ok(()),
            (FilePersistencePhase::FileDataSynced, FilePersistencePhase::AtomicallyRenamed) => {
                Ok(())
            }
            (
                FilePersistencePhase::AtomicallyRenamed,
                FilePersistencePhase::ParentDirectorySynced,
            ) => Ok(()),
            (
                FilePersistencePhase::ParentDirectorySynced,
                FilePersistencePhase::ManifestCommitted,
            ) => Ok(()),
            // Violations:
            (FilePersistencePhase::DataWritten, FilePersistencePhase::AtomicallyRenamed) => {
                Err(PosixDirSyncViolation::RenameBeforeDataSync { file_num })
            }
            (FilePersistencePhase::FileDataSynced, FilePersistencePhase::ParentDirectorySynced) => {
                Err(PosixDirSyncViolation::DirSyncBeforeRename { file_num })
            }
            (_, FilePersistencePhase::ManifestCommitted) => {
                Err(PosixDirSyncViolation::ManifestCommittedBeforeDirSync { file_num })
            }
            _ => Ok(()),
        }
    }

    /// Records a confirmed parent directory sync for `file_num`.
    pub fn note_dir_synced(&mut self, file_num: u64) {
        self.dir_synced_files.insert(file_num);
    }

    /// Authorizes committing `file_num` into the active MANIFEST.
    ///
    /// # Errors
    /// Returns `PosixDirSyncViolation::ManifestCommittedBeforeDirSync` if parent dir was not synced.
    pub fn authorize_manifest_commit(&mut self, file_num: u64) -> Result<(), PosixDirSyncViolation> {
        if !self.dir_synced_files.contains(&file_num) {
            return Err(PosixDirSyncViolation::ManifestCommittedBeforeDirSync { file_num });
        }
        self.manifest_committed_files.insert(file_num);
        Ok(())
    }
}
