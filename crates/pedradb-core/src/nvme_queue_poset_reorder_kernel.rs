//! RFC-0291 Fronteira 8: Desordenação de Comandos NVMe em Filas Paralelas (Poset vs. Hardware FUA/Barriers).
//!
//! Modela formalmente a ordem parcial (poset) de comandos submetidos a múltiplas
//! Submission Queues (SQ) paralelas de controladoras NVMe, provando que barreiras de
//! hardware (FUA / NVMe Flush) impedem inversões causais sob falhas de energia.

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};

/// Identificador de fila de submissão NVMe (0..64k).
pub type NvmeQueueId = u16;

/// Comando I/O atômico despachado para a controladora NVMe.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NvmeIoCommand {
    pub command_id: u64,
    pub queue_id: NvmeQueueId,
    pub lba_offset: u64,
    pub block_count: u32,
    pub payload_crc: u32,
    /// Se true, o comando porta o bit Force Unit Access (gravação não-volátil garantida antes do ack).
    pub is_fua: bool,
    /// Relação causal: este comando depende estritamente da persistência dos comandos listados.
    pub causal_dependencies: Vec<u64>,
}

/// Barreira explícita de sincronização NVMe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NvmeFlushBarrier {
    pub barrier_id: u64,
    /// Máscara de filas cobertas pela barreira de flush.
    pub queues_flushed: HashSet<NvmeQueueId>,
}

/// Violação de causalidade por reordenação inter-filas em hardware NVMe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NvmeCausalReorderViolation {
    MissingHardwareBarrier {
        dependent_cmd_id: u64,
        dependency_cmd_id: u64,
        dep_queue: NvmeQueueId,
        target_queue: NvmeQueueId,
    },
    TornPersistenceUnderPowerLoss {
        persisted_child_cmd: u64,
        lost_parent_cmd: u64,
    },
}

/// Simulador e verificador de Poset de execução e persistência NVMe.
pub struct NvmeQueuePosetVerifier {
    submitted_commands: HashMap<u64, NvmeIoCommand>,
    persisted_commands: HashSet<u64>,
    last_barrier_per_queue: HashMap<NvmeQueueId, u64>,
    current_barrier_epoch: u64,
}

impl Default for NvmeQueuePosetVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl NvmeQueuePosetVerifier {
    pub fn new() -> Self {
        Self {
            submitted_commands: HashMap::new(),
            persisted_commands: HashSet::new(),
            last_barrier_per_queue: HashMap::new(),
            current_barrier_epoch: 0,
        }
    }

    /// Emite uma barreira física NVMe Flush em um conjunto de Submission Queues.
    pub fn emit_flush_barrier(&mut self, barrier: NvmeFlushBarrier) {
        self.current_barrier_epoch += 1;
        for q in barrier.queues_flushed {
            self.last_barrier_per_queue.insert(q, self.current_barrier_epoch);
        }
        // Todos os comandos pendentes nas filas da barreira são promovidos à mídia não-volátil
        for (&id, cmd) in &self.submitted_commands {
            if self.last_barrier_per_queue.contains_key(&cmd.queue_id) {
                self.persisted_commands.insert(id);
            }
        }
    }

    /// Submete um comando I/O à fila especificada, validando que suas dependências causais
    /// estão garantidas ou por FUA local ou por barreira física intermediária.
    pub fn submit_command(&mut self, cmd: NvmeIoCommand) -> Result<(), NvmeCausalReorderViolation> {
        // Valida que qualquer dependência foi persistida antes da submissão deste comando dependente
        for &dep_id in &cmd.causal_dependencies {
            let dep = self.submitted_commands.get(&dep_id).expect("Dependency must have been submitted");

            // Se for na mesma fila, a controladora NVMe garante ordem FIFO se não houver reordenação out-of-order
            let same_queue = dep.queue_id == cmd.queue_id;
            let is_persisted = self.persisted_commands.contains(&dep_id) || dep.is_fua;

            if !same_queue && !is_persisted {
                // Violação Crítica: submeter um comando dependente em OUTRA fila sem barreira
                // permite que a controladora NVMe persista o filho antes do pai em caso de crash!
                return Err(NvmeCausalReorderViolation::MissingHardwareBarrier {
                    dependent_cmd_id: cmd.command_id,
                    dependency_cmd_id: dep_id,
                    dep_queue: dep.queue_id,
                    target_queue: cmd.queue_id,
                });
            }
        }

        let cmd_id = cmd.command_id;
        if cmd.is_fua {
            self.persisted_commands.insert(cmd_id);
        }
        self.submitted_commands.insert(cmd_id, cmd);
        Ok(())
    }

    /// Simula um corte súbito de energia e valida a consistência de prefixo das transações.
    pub fn simulate_crash_and_verify(&self) -> Result<(), NvmeCausalReorderViolation> {
        // Qualquer comando persistido cujas dependências NÃO foram persistidas é uma violação causal
        for &persisted_id in &self.persisted_commands {
            if let Some(cmd) = self.submitted_commands.get(&persisted_id) {
                for &dep_id in &cmd.causal_dependencies {
                    if !self.persisted_commands.contains(&dep_id) {
                        return Err(NvmeCausalReorderViolation::TornPersistenceUnderPowerLoss {
                            persisted_child_cmd: persisted_id,
                            lost_parent_cmd: dep_id,
                        });
                    }
                }
            }
        }
        Ok(())
    }
}
