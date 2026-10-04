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

use std::fmt;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Fonte autoritativa canônica para resolução de leituras MVCC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthoritativeSource {
    /// Leitura atendida pela MemTable imutável em RAM.
    ImmutableMemTable,
    /// Leitura atendida pelo VersionSet persistido em L0.
    VersionSetL0,
}

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
    /// Existem leitores concorrentes ativos na MemTable impedindo a reciclagem.
    ActiveReadersPending { count: usize },
    /// A fonte de dados solicitada pelo snapshot antigo já foi destruída da memória.
    ReclaimedSourceUnavailable { read_epoch: u64, commit_epoch: u64 },
    /// Invalid phase transition sequence.
    IllegalPhaseTransition { from: FlushRetirementPhase, to: FlushRetirementPhase },
    /// Não existem leitores ativos na MemTable para desregistrar (prevenção de underflow).
    NoActiveReadersToUnpin,
    /// Commit epoch inválido (não pode ser 0 nem u64::MAX).
    InvalidCommitEpoch { epoch: u64 },
    /// Read epoch 0 é inválido / não inicializado.
    ZeroReadEpochHazard,
    /// Limite máximo de leitores ativos simultâneos atingido.
    MaxReadersExceeded { limit: usize },
    /// Tentativa redundante de pinar a MemTable para um epoch já comitado em disco no L0.
    RedundantPinForDiskCommittedEpoch { read_epoch: u64, commit_epoch: u64 },
}

impl fmt::Display for HandoverViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PrematureReclamation => write!(f, "Premature reclamation before version installation"),
            Self::ActiveReadersPending { count } => {
                write!(f, "{count} active readers still pending on retiring MemTable")
            }
            Self::ReclaimedSourceUnavailable { read_epoch, commit_epoch } => {
                write!(
                    f,
                    "Reclaimed MemTable source unavailable for read epoch {read_epoch} (committed at {commit_epoch})"
                )
            }
            Self::IllegalPhaseTransition { from, to } => {
                write!(f, "Illegal phase transition from {from:?} to {to:?}")
            }
            Self::NoActiveReadersToUnpin => {
                write!(f, "Attempted to unpin reader when active readers count is 0")
            }
            Self::InvalidCommitEpoch { epoch } => {
                write!(f, "Invalid commit epoch {epoch} (must be > 0 and < u64::MAX)")
            }
            Self::ZeroReadEpochHazard => {
                write!(f, "Read epoch 0 is invalid (uninitialized read hazard)")
            }
            Self::MaxReadersExceeded { limit } => {
                write!(f, "Active memtable readers exceeded hard limit of {limit}")
            }
            Self::RedundantPinForDiskCommittedEpoch { read_epoch, commit_epoch } => {
                write!(
                    f,
                    "Redundant memtable pin for read epoch {read_epoch} >= commit epoch {commit_epoch} (L0 is authoritative)"
                )
            }
        }
    }
}

impl std::error::Error for HandoverViolation {}

/// RAII guard representing an active pinned reader on the immutable MemTable.
pub struct MemTableReaderGuard<'a> {
    coordinator: &'a MemTableRetirementCoordinator,
}

impl<'a> Drop for MemTableReaderGuard<'a> {
    fn drop(&mut self) {
        let _ = self.coordinator.try_unpin_memtable_reader();
    }
}

/// Coordinator managing the atomic transition from immutable MemTable to on-disk VersionSet.
pub struct MemTableRetirementCoordinator {
    current_phase: FlushRetirementPhase,
    version_commit_epoch: AtomicU64,
    active_memtable_readers: AtomicUsize,
}

impl Default for MemTableRetirementCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl MemTableRetirementCoordinator {
    /// Limite máximo de leitores simultâneos na MemTable para proteção contra exaustão de recursos.
    pub const MAX_ACTIVE_READERS: usize = 1_000_000;

    /// Creates a new retirement coordinator initialized in `FlushingToDisk`.
    pub fn new() -> Self {
        Self {
            current_phase: FlushRetirementPhase::FlushingToDisk,
            version_commit_epoch: AtomicU64::new(u64::MAX),
            active_memtable_readers: AtomicUsize::new(0),
        }
    }

    /// Current phase of the retirement lifecycle.
    pub fn current_phase(&self) -> FlushRetirementPhase {
        self.current_phase
    }

    /// Registra um leitor ativo na MemTable imutável.
    pub fn pin_memtable_reader(&self) -> Result<(), HandoverViolation> {
        if self.current_phase == FlushRetirementPhase::MemTableReclaimed {
            let commit_epoch = self.version_commit_epoch.load(Ordering::Acquire);
            return Err(HandoverViolation::ReclaimedSourceUnavailable {
                read_epoch: 0,
                commit_epoch,
            });
        }
        let mut current = self.active_memtable_readers.load(Ordering::SeqCst);
        loop {
            if current >= Self::MAX_ACTIVE_READERS {
                return Err(HandoverViolation::MaxReadersExceeded {
                    limit: Self::MAX_ACTIVE_READERS,
                });
            }
            match self.active_memtable_readers.compare_exchange_weak(
                current,
                current + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return Ok(()),
                Err(actual) => current = actual,
            }
        }
    }

    /// Registra um leitor ativo na MemTable imutável associado a um read epoch específico.
    pub fn pin_memtable_reader_for_epoch(&self, read_epoch: u64) -> Result<(), HandoverViolation> {
        if read_epoch == 0 {
            return Err(HandoverViolation::ZeroReadEpochHazard);
        }
        let commit_epoch = self.version_commit_epoch.load(Ordering::Acquire);
        if self.current_phase == FlushRetirementPhase::MemTableReclaimed {
            return Err(HandoverViolation::ReclaimedSourceUnavailable {
                read_epoch,
                commit_epoch,
            });
        }
        if self.current_phase == FlushRetirementPhase::VersionInstalledOnDisk && read_epoch >= commit_epoch {
            return Err(HandoverViolation::RedundantPinForDiskCommittedEpoch {
                read_epoch,
                commit_epoch,
            });
        }
        self.pin_memtable_reader()
    }

    /// Adquire um RAII guard para um leitor ativo na MemTable imutável.
    pub fn acquire_reader_guard(&self) -> Result<MemTableReaderGuard<'_>, HandoverViolation> {
        self.pin_memtable_reader()?;
        Ok(MemTableReaderGuard { coordinator: self })
    }

    /// Desregistra com segurança um leitor da MemTable imutável, retornando erro se count == 0.
    pub fn try_unpin_memtable_reader(&self) -> Result<(), HandoverViolation> {
        let mut current = self.active_memtable_readers.load(Ordering::SeqCst);
        loop {
            if current == 0 {
                return Err(HandoverViolation::NoActiveReadersToUnpin);
            }
            match self.active_memtable_readers.compare_exchange_weak(
                current,
                current - 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return Ok(()),
                Err(actual) => current = actual,
            }
        }
    }

    /// Desregistra um leitor da MemTable imutável de forma saturante (sem underflow).
    pub fn unpin_memtable_reader(&self) {
        let _ = self.try_unpin_memtable_reader();
    }

    /// Contagem de leitores ativos na MemTable imutável.
    pub fn active_readers(&self) -> usize {
        self.active_memtable_readers.load(Ordering::SeqCst)
    }

    /// Records that the MANIFEST VersionEdit has been committed to disk at `commit_epoch`.
    pub fn mark_version_installed(&mut self, commit_epoch: u64) -> Result<(), HandoverViolation> {
        if commit_epoch == 0 || commit_epoch == u64::MAX {
            return Err(HandoverViolation::InvalidCommitEpoch { epoch: commit_epoch });
        }
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

    /// Reclaims the in-memory immutable MemTable after version installation and when no readers are active.
    pub fn mark_memtable_reclaimed(&mut self) -> Result<(), HandoverViolation> {
        if self.current_phase == FlushRetirementPhase::FlushingToDisk {
            return Err(HandoverViolation::PrematureReclamation);
        }
        if self.current_phase == FlushRetirementPhase::MemTableReclaimed {
            return Err(HandoverViolation::IllegalPhaseTransition {
                from: FlushRetirementPhase::MemTableReclaimed,
                to: FlushRetirementPhase::MemTableReclaimed,
            });
        }
        let active = self.active_memtable_readers.load(Ordering::SeqCst);
        if active > 0 {
            return Err(HandoverViolation::ActiveReadersPending { count: active });
        }
        self.current_phase = FlushRetirementPhase::MemTableReclaimed;
        Ok(())
    }

    /// Resolve a fonte autoritativa de dados de forma estrita e segura.
    pub fn resolve_source(&self, read_epoch: u64) -> Result<AuthoritativeSource, HandoverViolation> {
        if read_epoch == 0 {
            return Err(HandoverViolation::ZeroReadEpochHazard);
        }
        let commit_epoch = self.version_commit_epoch.load(Ordering::Acquire);
        if read_epoch < commit_epoch {
            if self.current_phase == FlushRetirementPhase::MemTableReclaimed {
                return Err(HandoverViolation::ReclaimedSourceUnavailable {
                    read_epoch,
                    commit_epoch,
                });
            }
            Ok(AuthoritativeSource::ImmutableMemTable)
        } else {
            Ok(AuthoritativeSource::VersionSetL0)
        }
    }

    /// Resolves the authoritative data source for a read request initiated at `read_epoch`.
    pub fn resolve_authoritative_source(&self, read_epoch: u64) -> &'static str {
        match self.resolve_source(read_epoch) {
            Ok(AuthoritativeSource::ImmutableMemTable) => "ImmutableMemTable",
            Ok(AuthoritativeSource::VersionSetL0) => "VersionSetL0",
            Err(_) => "ReclaimedSourceUnavailable",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memtable_retirement_handshake_structural_invariants_red_to_green() {
        let mut coordinator = MemTableRetirementCoordinator::new();

        // 1. Zero read epoch hazard in resolve_source
        assert_eq!(
            coordinator.resolve_source(0),
            Err(HandoverViolation::ZeroReadEpochHazard)
        );

        // 2. Zero read epoch hazard in pin_memtable_reader_for_epoch
        assert_eq!(
            coordinator.pin_memtable_reader_for_epoch(0),
            Err(HandoverViolation::ZeroReadEpochHazard)
        );

        // 3. Valid epoch pinning while flushing
        assert!(coordinator.pin_memtable_reader_for_epoch(50).is_ok());
        assert_eq!(coordinator.active_readers(), 1);
        coordinator.unpin_memtable_reader();
        assert_eq!(coordinator.active_readers(), 0);

        // 4. Install version on disk at commit epoch 100
        assert!(coordinator.mark_version_installed(100).is_ok());

        // 5. Stale read epoch (< 100) can still pin the retiring memtable
        assert!(coordinator.pin_memtable_reader_for_epoch(80).is_ok());
        assert_eq!(coordinator.active_readers(), 1);
        coordinator.unpin_memtable_reader();

        // 6. Contemporary or newer read epoch (>= 100) must NOT pin the retiring memtable (L0 is authoritative)
        let red_err = coordinator.pin_memtable_reader_for_epoch(100);
        assert_eq!(
            red_err,
            Err(HandoverViolation::RedundantPinForDiskCommittedEpoch {
                read_epoch: 100,
                commit_epoch: 100,
            })
        );
        let red_err_newer = coordinator.pin_memtable_reader_for_epoch(150);
        assert_eq!(
            red_err_newer,
            Err(HandoverViolation::RedundantPinForDiskCommittedEpoch {
                read_epoch: 150,
                commit_epoch: 100,
            })
        );

        // 7. Max readers limit
        coordinator.active_memtable_readers.store(
            MemTableRetirementCoordinator::MAX_ACTIVE_READERS,
            Ordering::SeqCst,
        );
        assert_eq!(
            coordinator.pin_memtable_reader(),
            Err(HandoverViolation::MaxReadersExceeded {
                limit: MemTableRetirementCoordinator::MAX_ACTIVE_READERS,
            })
        );
    }
}
