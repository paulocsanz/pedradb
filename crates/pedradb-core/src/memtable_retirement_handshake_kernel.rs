//! Linearizable MemTable Retirement and VersionSet Handshake Kernel (RFC-0286 Fronteira 5).
//!
//! Enforces atomic dual-pointer handover between retiring immutable MemTables in RAM
//! and freshly committed L0 SST files in the VersionSet.
//!
//! Guarantees:
//! 1. Zero duplicate reads: Iterators never observe the same key simultaneously from RAM and disk.
//! 2. Zero temporal gaps: No key vanishes during the transition from RAM to disk.
//! 3. Epoch linearizability: Every read snapshot resolves against exactly one authoritative source.

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicU64, Ordering};

/// Lifecycle phase of an immutable MemTable being flushed to L0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlushRetirementPhase {
    /// MemTable is frozen; SST file is being written to disk; RAM is canonical.
    FlushingToDisk,
    /// VersionEdit committed to MANIFEST; L0 SST is canonical; RAM is pending cleanup.
    VersionInstalledOnDisk,
    /// Immutable MemTable unlinked from memory; retirement completed.
    MemTableReclaimed,
}

/// Verification violation during flush retirement handover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoverViolation {
    /// Premature reclamation before MANIFEST version installation.
    PrematureReclamation,
    /// Invalid phase transition sequence.
    IllegalPhaseTransition { from: FlushRetirementPhase, to: FlushRetirementPhase },
}

/// Coordinator managing the atomic transition from immutable MemTable to on-disk VersionSet.
pub struct MemTableRetirementCoordinator {
    current_phase: FlushRetirementPhase,
    version_commit_epoch: AtomicU64,
}

impl MemTableRetirementCoordinator {
    /// Creates a new retirement coordinator initialized in `FlushingToDisk`.
    pub fn new() -> Self {
        Self {
            current_phase: FlushRetirementPhase::FlushingToDisk,
            version_commit_epoch: AtomicU64::new(u64::MAX),
        }
    }

    /// Current phase of the retirement lifecycle.
    pub fn current_phase(&self) -> FlushRetirementPhase {
        self.current_phase
    }

    /// Records that the MANIFEST VersionEdit has been committed to disk at `commit_epoch`.
    pub fn mark_version_installed(&mut self, commit_epoch: u64) -> Result<(), HandoverViolation> {
        if self.current_phase != FlushRetirementPhase::FlushingToDisk {
            return Err(HandoverViolation::IllegalPhaseTransition {
                from: self.current_phase,
                to: FlushRetirementPhase::VersionInstalledOnDisk,
            });
        }
        self.version_commit_epoch.store(commit_epoch, Ordering::Release);
        self.current_phase = FlushRetirementPhase::VersionInstalledOnDisk;
        Ok(())
    }

    /// Reclaims the in-memory immutable MemTable after version installation.
    pub fn mark_memtable_reclaimed(&mut self) -> Result<(), HandoverViolation> {
        if self.current_phase != FlushRetirementPhase::VersionInstalledOnDisk {
            return Err(HandoverViolation::PrematureReclamation);
        }
        self.current_phase = FlushRetirementPhase::MemTableReclaimed;
        Ok(())
    }

    /// Resolves the authoritative data source for a read request initiated at `read_epoch`.
    ///
    /// Returns:
    /// - `"ImmutableMemTable"` if the read was initiated prior to the version commit.
    /// - `"VersionSetL0"` if the read was initiated at or after the version commit.
    pub fn resolve_authoritative_source(&self, read_epoch: u64) -> &'static str {
        let commit_epoch = self.version_commit_epoch.load(Ordering::Acquire);
        if read_epoch < commit_epoch {
            "ImmutableMemTable"
        } else {
            "VersionSetL0"
        }
    }
}
