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
        self.active_memtable_readers.fetch_add(1, Ordering::SeqCst);
        Ok(())
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
