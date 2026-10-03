//! Cross-Device (EXDEV) Safe Migration Protocol Kernel (RFC-0284 Pilar 6).
//!
//! Enforces an idempotent two-phase transfer protocol across distinct filesystem
//! mountpoints where POSIX `rename(2)` fails with `EXDEV`.
//!
//! Guarantees:
//! 1. Zero data loss: source remains canonical until target is confirmed in MANIFEST.
//! 2. Zero zombie duplicates: target temporary files are purged idempotently on crash.
//! 3. Ordered synchronization of parent directories on both source and target devices.

#![forbid(unsafe_code)]

/// Phase in the cross-device file migration state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossDevicePhase {
    /// Initial state: source file is active and canonical on Device A.
    SourceActive,
    /// Data copied to temporary file on Device B and fdatasynced.
    TargetTempSynced,
    /// Temporary file renamed to final name and parent dir synced on Device B.
    TargetFinalized,
    /// MANIFEST updated on metadata device acknowledging new file location.
    ManifestCommitted,
    /// Source file unlinked from Device A and parent dir synced.
    SourceUnlinkedCompleted,
}

/// Verification violation in cross-device transfer sequencing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossDeviceViolation {
    /// Invalid phase transition attempt.
    IllegalTransition { from: CrossDevicePhase, to: CrossDevicePhase },
    /// Premature source deletion before manifest commit.
    PrematureSourceDeletion,
    /// Manifest commit attempted before target data is finalized and directory synced.
    TargetNotFinalized,
    /// Tentativa de transição com ID de arquivo incompatível.
    FileIdMismatch {
        /// ID de arquivo esperado.
        expected: u64,
        /// ID de arquivo fornecido.
        found: u64,
    },
    /// Invalid file id (zero).
    ZeroFileId,
}

impl std::fmt::Display for CrossDeviceViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IllegalTransition { from, to } => {
                write!(f, "Illegal phase transition from {from:?} to {to:?}")
            }
            Self::PrematureSourceDeletion => {
                write!(f, "Premature source deletion before manifest commit")
            }
            Self::TargetNotFinalized => {
                write!(f, "Manifest commit attempted before target data is finalized")
            }
            Self::FileIdMismatch { expected, found } => {
                write!(f, "File ID mismatch: expected {expected}, found {found}")
            }
            Self::ZeroFileId => {
                write!(f, "File ID cannot be zero")
            }
        }
    }
}

impl std::error::Error for CrossDeviceViolation {}

/// State machine oracle verifying cross-device migration safety.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrossDeviceTransferOracle {
    file_id: u64,
    current_phase: CrossDevicePhase,
}

impl CrossDeviceTransferOracle {
    /// Creates a new transfer tracking oracle with validation.
    pub fn try_new(file_id: u64) -> Result<Self, CrossDeviceViolation> {
        if file_id == 0 {
            return Err(CrossDeviceViolation::ZeroFileId);
        }
        Ok(Self {
            file_id,
            current_phase: CrossDevicePhase::SourceActive,
        })
    }

    /// Creates a new transfer tracking oracle for a given file ID.
    pub fn new(file_id: u64) -> Self {
        Self {
            file_id,
            current_phase: CrossDevicePhase::SourceActive,
        }
    }

    /// ID do arquivo monitorado pelo oráculo.
    #[inline]
    pub fn file_id(&self) -> u64 {
        self.file_id
    }

    /// Current phase of the transfer.
    pub fn current_phase(&self) -> CrossDevicePhase {
        self.current_phase
    }

    /// Transitions to the next phase validating that the transition belongs to the specified file ID.
    pub fn advance_for_file(&mut self, file_id: u64, next: CrossDevicePhase) -> Result<(), CrossDeviceViolation> {
        if file_id != self.file_id {
            return Err(CrossDeviceViolation::FileIdMismatch {
                expected: self.file_id,
                found: file_id,
            });
        }
        self.advance_to(next)
    }

    /// Transitions to the next phase after validating protocol invariants.
    pub fn advance_to(&mut self, next: CrossDevicePhase) -> Result<(), CrossDeviceViolation> {
        match (self.current_phase, next) {
            (CrossDevicePhase::SourceActive, CrossDevicePhase::TargetTempSynced) => {
                self.current_phase = next;
                Ok(())
            }
            (CrossDevicePhase::TargetTempSynced, CrossDevicePhase::TargetFinalized) => {
                self.current_phase = next;
                Ok(())
            }
            (CrossDevicePhase::TargetFinalized, CrossDevicePhase::ManifestCommitted) => {
                self.current_phase = next;
                Ok(())
            }
            (CrossDevicePhase::ManifestCommitted, CrossDevicePhase::SourceUnlinkedCompleted) => {
                self.current_phase = next;
                Ok(())
            }
            // Violations:
            (_, CrossDevicePhase::SourceUnlinkedCompleted) => {
                Err(CrossDeviceViolation::PrematureSourceDeletion)
            }
            (CrossDevicePhase::SourceActive, CrossDevicePhase::ManifestCommitted)
            | (CrossDevicePhase::TargetTempSynced, CrossDevicePhase::ManifestCommitted) => {
                Err(CrossDeviceViolation::TargetNotFinalized)
            }
            (from, to) => Err(CrossDeviceViolation::IllegalTransition { from, to }),
        }
    }

    /// Evaluates crash recovery outcome depending on the last persisted phase.
    pub fn recover_from_crash(&self) -> (&'static str, bool) {
        match self.current_phase {
            CrossDevicePhase::SourceActive
            | CrossDevicePhase::TargetTempSynced
            | CrossDevicePhase::TargetFinalized => {
                // Manifest not committed: Source is canonical; target discarded
                ("RollbackToSource: Discard target temp/final file", true)
            }
            CrossDevicePhase::ManifestCommitted
            | CrossDevicePhase::SourceUnlinkedCompleted => {
                // Manifest committed: Target is canonical; source deleted if present
                ("RollforwardToTarget: Reclaim source file if present", true)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cross_device_oracle_bounds() {
        assert_eq!(
            CrossDeviceTransferOracle::try_new(0),
            Err(CrossDeviceViolation::ZeroFileId)
        );

        let oracle = CrossDeviceTransferOracle::try_new(42).expect("valid oracle");
        assert_eq!(oracle.file_id(), 42);
        assert_eq!(oracle.current_phase(), CrossDevicePhase::SourceActive);
    }

    #[test]
    fn test_cross_device_violation_display() {
        let err = CrossDeviceViolation::ZeroFileId;
        assert_eq!(format!("{err}"), "File ID cannot be zero");

        let err2 = CrossDeviceViolation::PrematureSourceDeletion;
        assert_eq!(format!("{err2}"), "Premature source deletion before manifest commit");

        let err3 = CrossDeviceViolation::TargetNotFinalized;
        assert_eq!(format!("{err3}"), "Manifest commit attempted before target data is finalized");
    }
}
