//! Pilar 4: Ordem POSIX de Diretório-Pai e Idempotência sob Falha de Energia no Metal (RFC-0285).
//!
//! Modela e impõe a máquina de estados rigorosa para persistência atômica de arquivos no storage metal:
//! Write(tmp) -> fdatasync(tmp) -> fsync(parent_dir) -> rename(tmp, dest) -> fsync(parent_dir).

/// Passos atômicos na persistência segura de arquivos em filesystem POSIX.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FsyncPhase {
    Uninitialized = 0,
    DataWritten = 1,
    FileSynced = 2,
    PreRenameDirSynced = 3,
    Renamed = 4,
    PostRenameDirSynced = 5,
}

/// Estado do envelope de persistência de arquivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsyncBarrierTracker {
    pub current_phase: FsyncPhase,
    pub tmp_path: String,
    pub dest_path: String,
}

/// Erro de violação de protocolo de barreira fsync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsyncViolation {
    InvalidPhaseTransition { from: FsyncPhase, attempted: FsyncPhase },
    CrashBeforeCommit { phase_at_crash: FsyncPhase },
}

impl FsyncBarrierTracker {
    pub fn new(tmp_path: &str, dest_path: &str) -> Self {
        Self {
            current_phase: FsyncPhase::Uninitialized,
            tmp_path: tmp_path.to_string(),
            dest_path: dest_path.to_string(),
        }
    }

    /// Registra escrita de dados no arquivo temporário.
    pub fn step_write_data(&mut self) -> Result<(), FsyncViolation> {
        if self.current_phase != FsyncPhase::Uninitialized {
            return Err(FsyncViolation::InvalidPhaseTransition {
                from: self.current_phase,
                attempted: FsyncPhase::DataWritten,
            });
        }
        self.current_phase = FsyncPhase::DataWritten;
        Ok(())
    }

    /// Registra flush de dados (`fdatasync`) no arquivo temporário.
    pub fn step_fdatasync_file(&mut self) -> Result<(), FsyncViolation> {
        if self.current_phase != FsyncPhase::DataWritten {
            return Err(FsyncViolation::InvalidPhaseTransition {
                from: self.current_phase,
                attempted: FsyncPhase::FileSynced,
            });
        }
        self.current_phase = FsyncPhase::FileSynced;
        Ok(())
    }

    /// Registra sincronização do diretório pai antes do rename (garante criação do tmp visível).
    pub fn step_fsync_parent_pre_rename(&mut self) -> Result<(), FsyncViolation> {
        if self.current_phase != FsyncPhase::FileSynced {
            return Err(FsyncViolation::InvalidPhaseTransition {
                from: self.current_phase,
                attempted: FsyncPhase::PreRenameDirSynced,
            });
        }
        self.current_phase = FsyncPhase::PreRenameDirSynced;
        Ok(())
    }

    /// Registra a substituição atômica (`rename`).
    pub fn step_rename(&mut self) -> Result<(), FsyncViolation> {
        if self.current_phase != FsyncPhase::PreRenameDirSynced {
            return Err(FsyncViolation::InvalidPhaseTransition {
                from: self.current_phase,
                attempted: FsyncPhase::Renamed,
            });
        }
        self.current_phase = FsyncPhase::Renamed;
        Ok(())
    }

    /// Registra sincronização final do diretório pai (torna a entrada de diretório persistente).
    pub fn step_fsync_parent_post_rename(&mut self) -> Result<(), FsyncViolation> {
        if self.current_phase != FsyncPhase::Renamed {
            return Err(FsyncViolation::InvalidPhaseTransition {
                from: self.current_phase,
                attempted: FsyncPhase::PostRenameDirSynced,
            });
        }
        self.current_phase = FsyncPhase::PostRenameDirSynced;
        Ok(())
    }

    /// Verifica se uma quebra de energia (crash) neste ponto é recuperável com integridade.
    pub fn assert_crash_soundness(&self) -> Result<bool, FsyncViolation> {
        // Se caiu antes do rename, o arquivo dest ainda aponta para a versão anterior íntegra;
        // o tmp_path é considerado órfão descartável.
        if self.current_phase < FsyncPhase::Renamed {
            return Ok(false); // Operação abortada, estado anterior mantido
        }
        // Se caiu entre rename e o fsync do diretório pós-rename, em filesystems POSIX journaling
        // pode ocorrer perda de metadados da entrada de diretório dependendo das opções de montagem.
        if self.current_phase == FsyncPhase::Renamed {
            // Zona perigosa sem fsync pós-rename
            return Err(FsyncViolation::CrashBeforeCommit {
                phase_at_crash: self.current_phase,
            });
        }
        Ok(true) // Totalmente seguro
    }
}

/// Mutante degenerado (AS-IS): omite os fsyncs do diretório-pai, pulando de fdatasync direto para rename.
pub fn step_unsafe_fast_rename_as_is(tracker: &mut FsyncBarrierTracker) {
    tracker.current_phase = FsyncPhase::Renamed;
}
