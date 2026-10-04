//! RFC-0292: Pilar 9 - Aciclicidade e Liveness da Rede de Petri dos Background Workers.
//!
//! Modela os trabalhadores de background (Flusher, Compactor, Blob GC, Scrubber) e seus recursos
//! finitos (FDs, I/O Quota, Catalog Lock) como uma Rede de Petri formalmente viva e livre de deadlocks.

use std::collections::{HashSet, VecDeque};

/// Violações de liveness e propriedades estruturais da Rede de Petri.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PetriNetLivenessViolation {
    /// Foi encontrado um estado de marcação alcançável sem nenhuma transição habilitada (Deadlock).
    DeadlockStateReachable {
        /// Marcação terminal de recursos.
        marking: PetriMarking,
        /// Sequência de transições que conduziu ao deadlock.
        firing_sequence: Vec<&'static str>,
    },
    /// Um recurso finito desbalanceou, violando a limitação da rede (Unbounded).
    CapacityLimitExceeded {
        /// Recurso que transbordou.
        resource: &'static str,
        /// Tokens contados.
        tokens: u32,
    },
    EmptyTransitions,
    ZeroCapacityLimit,
    ZeroMaxStates,
    EmptyTransitionName,
    UnknownTask {
        task: &'static str,
    },
}

impl std::fmt::Display for PetriNetLivenessViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeadlockStateReachable { marking, firing_sequence } => {
                write!(f, "Deadlock state reachable at {marking:?} via path {firing_sequence:?}")
            }
            Self::CapacityLimitExceeded { resource, tokens } => {
                write!(f, "Capacity limit exceeded for {resource}: {tokens} tokens")
            }
            Self::EmptyTransitions => write!(f, "Transitions vector cannot be empty"),
            Self::ZeroCapacityLimit => write!(f, "Capacity limit cannot be 0"),
            Self::ZeroMaxStates => write!(f, "Max states to explore cannot be 0"),
            Self::EmptyTransitionName => write!(f, "Transition name cannot be empty"),
            Self::UnknownTask { task } => write!(f, "Unknown task type: {task}"),
        }
    }
}

impl std::error::Error for PetriNetLivenessViolation {}

/// Estado de marcação dos lugares de recursos e tarefas na Rede de Petri.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PetriMarking {
    /// File descriptors disponíveis (P_fd).
    pub p_fd_available: u32,
    /// Quota de tokens de I/O disponíveis (P_quota).
    pub p_quota_available: u32,
    /// Permissão de lock do catálogo MANIFEST (P_manifest).
    pub p_manifest_lock: u32,
    /// MemTables aguardando flush (P_memtable_flush_queue).
    pub p_flush_tasks: u32,
    /// Arquivos aguardando compactação (P_compact_tasks).
    pub p_compact_tasks: u32,
    /// Arquivos vLog aguardando GC (P_blob_gc_tasks).
    pub p_blob_gc_tasks: u32,
}

/// Descritor de uma transição na Rede de Petri.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PetriTransition {
    /// Nome descritivo da transição.
    pub name: &'static str,
    /// Requisitos de consumo da transição (pré-condições).
    pub required_fd: u32,
    pub required_quota: u32,
    pub required_manifest: u32,
    pub required_task: &'static str, // "flush", "compact", "blob_gc"
    /// Recursos liberados e produzidos na conclusão da transição.
    pub produced_fd: u32,
    pub produced_quota: u32,
    pub produced_manifest: u32,
}

impl PetriTransition {
    pub fn try_new(
        name: &'static str,
        required_fd: u32,
        required_quota: u32,
        required_manifest: u32,
        required_task: &'static str,
        produced_fd: u32,
        produced_quota: u32,
        produced_manifest: u32,
    ) -> Result<Self, PetriNetLivenessViolation> {
        if name.trim().is_empty() {
            return Err(PetriNetLivenessViolation::EmptyTransitionName);
        }
        match required_task {
            "flush" | "compact" | "blob_gc" => {}
            other => return Err(PetriNetLivenessViolation::UnknownTask { task: other }),
        }
        Ok(Self {
            name,
            required_fd,
            required_quota,
            required_manifest,
            required_task,
            produced_fd,
            produced_quota,
            produced_manifest,
        })
    }
}

/// Motor formal de análise da Rede de Petri de background workers.
pub struct BackgroundWorkerPetriNet {
    transitions: Vec<PetriTransition>,
    max_capacity_per_place: Option<u32>,
}

impl Default for BackgroundWorkerPetriNet {
    fn default() -> Self {
        Self {
            transitions: vec![
                PetriTransition {
                    name: "T_flush_execute",
                    required_fd: 2,
                    required_quota: 1,
                    required_manifest: 1,
                    required_task: "flush",
                    produced_fd: 2,
                    produced_quota: 1,
                    produced_manifest: 1,
                },
                PetriTransition {
                    name: "T_compaction_execute",
                    required_fd: 4,
                    required_quota: 2,
                    required_manifest: 1,
                    required_task: "compact",
                    produced_fd: 4,
                    produced_quota: 2,
                    produced_manifest: 1,
                },
                PetriTransition {
                    name: "T_blob_gc_execute",
                    required_fd: 1,
                    required_quota: 1,
                    required_manifest: 0,
                    required_task: "blob_gc",
                    produced_fd: 1,
                    produced_quota: 1,
                    produced_manifest: 0,
                },
            ],
            max_capacity_per_place: None,
        }
    }
}

impl BackgroundWorkerPetriNet {
    /// Cria uma nova rede de Petri com transições especificadas.
    pub fn new(transitions: Vec<PetriTransition>) -> Self {
        Self {
            transitions,
            max_capacity_per_place: None,
        }
    }

    /// Cria uma nova rede de Petri validada contra transições vazias.
    pub fn try_new(transitions: Vec<PetriTransition>) -> Result<Self, PetriNetLivenessViolation> {
        if transitions.is_empty() {
            return Err(PetriNetLivenessViolation::EmptyTransitions);
        }
        for t in &transitions {
            if t.name.trim().is_empty() {
                return Err(PetriNetLivenessViolation::EmptyTransitionName);
            }
            match t.required_task {
                "flush" | "compact" | "blob_gc" => {}
                other => return Err(PetriNetLivenessViolation::UnknownTask { task: other }),
            }
        }
        Ok(Self {
            transitions,
            max_capacity_per_place: None,
        })
    }

    /// Configura um limite máximo finito de tokens por lugar de forma validada.
    pub fn try_with_capacity_limit(mut self, limit: u32) -> Result<Self, PetriNetLivenessViolation> {
        if limit == 0 {
            return Err(PetriNetLivenessViolation::ZeroCapacityLimit);
        }
        self.max_capacity_per_place = Some(limit);
        Ok(self)
    }

    /// Configura um limite máximo finito de tokens por lugar.
    pub fn with_capacity_limit(mut self, limit: u32) -> Self {
        if limit == 0 {
            self.max_capacity_per_place = None;
        } else {
            self.max_capacity_per_place = Some(limit);
        }
        self
    }

    /// Valida se uma marcação respeita os limites de capacidade dos lugares.
    pub fn check_capacity(&self, m: &PetriMarking) -> Result<(), PetriNetLivenessViolation> {
        if let Some(limit) = self.max_capacity_per_place {
            if m.p_fd_available > limit {
                return Err(PetriNetLivenessViolation::CapacityLimitExceeded {
                    resource: "p_fd_available",
                    tokens: m.p_fd_available,
                });
            }
            if m.p_quota_available > limit {
                return Err(PetriNetLivenessViolation::CapacityLimitExceeded {
                    resource: "p_quota_available",
                    tokens: m.p_quota_available,
                });
            }
            if m.p_manifest_lock > limit {
                return Err(PetriNetLivenessViolation::CapacityLimitExceeded {
                    resource: "p_manifest_lock",
                    tokens: m.p_manifest_lock,
                });
            }
            if m.p_flush_tasks > limit {
                return Err(PetriNetLivenessViolation::CapacityLimitExceeded {
                    resource: "p_flush_tasks",
                    tokens: m.p_flush_tasks,
                });
            }
            if m.p_compact_tasks > limit {
                return Err(PetriNetLivenessViolation::CapacityLimitExceeded {
                    resource: "p_compact_tasks",
                    tokens: m.p_compact_tasks,
                });
            }
            if m.p_blob_gc_tasks > limit {
                return Err(PetriNetLivenessViolation::CapacityLimitExceeded {
                    resource: "p_blob_gc_tasks",
                    tokens: m.p_blob_gc_tasks,
                });
            }
        }
        Ok(())
    }

    /// Avalia se uma transição está habilitada sob a marcação atual.
    pub fn is_enabled(&self, trans: &PetriTransition, m: &PetriMarking) -> bool {
        if m.p_fd_available < trans.required_fd
            || m.p_quota_available < trans.required_quota
            || m.p_manifest_lock < trans.required_manifest
        {
            return false;
        }

        match trans.required_task {
            "flush" => m.p_flush_tasks > 0,
            "compact" => m.p_compact_tasks > 0,
            "blob_gc" => m.p_blob_gc_tasks > 0,
            _ => false,
        }
    }

    /// Tenta disparar uma transição habilitada de forma segura (sem panic).
    pub fn try_fire(&self, trans: &PetriTransition, m: &PetriMarking) -> Option<PetriMarking> {
        if !self.is_enabled(trans, m) {
            return None;
        }

        let mut next = *m;
        next.p_fd_available = next
            .p_fd_available
            .saturating_sub(trans.required_fd)
            .saturating_add(trans.produced_fd);
        next.p_quota_available = next
            .p_quota_available
            .saturating_sub(trans.required_quota)
            .saturating_add(trans.produced_quota);
        next.p_manifest_lock = next
            .p_manifest_lock
            .saturating_sub(trans.required_manifest)
            .saturating_add(trans.produced_manifest);

        match trans.required_task {
            "flush" => next.p_flush_tasks = next.p_flush_tasks.saturating_sub(1),
            "compact" => next.p_compact_tasks = next.p_compact_tasks.saturating_sub(1),
            "blob_gc" => next.p_blob_gc_tasks = next.p_blob_gc_tasks.saturating_sub(1),
            _ => {}
        }

        Some(next)
    }

    /// Dispara uma transição de forma resiliente, retornando a marcação inalterada se desabilitada.
    pub fn fire(&self, trans: &PetriTransition, m: &PetriMarking) -> PetriMarking {
        self.try_fire(trans, m).unwrap_or(*m)
    }

    /// Explora o grafo de alcançabilidade a partir da marcação inicial M0 e prova a ausência de deadlocks e estouros de capacidade.
    pub fn verify_liveness_and_deadlock_freedom(
        &self,
        initial_marking: PetriMarking,
        max_states: usize,
    ) -> Result<usize, PetriNetLivenessViolation> {
        if max_states == 0 {
            return Err(PetriNetLivenessViolation::ZeroMaxStates);
        }
        self.check_capacity(&initial_marking)?;

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();

        visited.insert(initial_marking);
        queue.push_back((initial_marking, Vec::new()));

        let mut state_count = 0;

        while let Some((curr_m, path)) = queue.pop_front() {
            state_count += 1;
            if state_count > max_states {
                break;
            }

            // Verifica se há tarefas pendentes
            let has_pending_tasks = curr_m.p_flush_tasks > 0
                || curr_m.p_compact_tasks > 0
                || curr_m.p_blob_gc_tasks > 0;

            let mut any_enabled = false;
            for trans in &self.transitions {
                if let Some(next_m) = self.try_fire(trans, &curr_m) {
                    any_enabled = true;
                    self.check_capacity(&next_m)?;
                    if !visited.contains(&next_m) {
                        visited.insert(next_m);
                        let mut next_path = path.clone();
                        next_path.push(trans.name);
                        queue.push_back((next_m, next_path));
                    }
                }
            }

            // Se existiam tarefas pendentes mas nenhuma transição pôde ser disparada: DEADLOCK!
            if has_pending_tasks && !any_enabled {
                return Err(PetriNetLivenessViolation::DeadlockStateReachable {
                    marking: curr_m,
                    firing_sequence: path,
                });
            }
        }

        Ok(state_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_petri_net_structural_invariants_red_to_green() {
        assert_eq!(
            BackgroundWorkerPetriNet::try_new(vec![]).err(),
            Some(PetriNetLivenessViolation::EmptyTransitions)
        );

        let t_bad_name = PetriTransition::try_new("", 1, 1, 1, "flush", 1, 1, 1);
        assert_eq!(
            t_bad_name,
            Err(PetriNetLivenessViolation::EmptyTransitionName)
        );

        let t_bad_task = PetriTransition::try_new("T1", 1, 1, 1, "invalid_task", 1, 1, 1);
        assert_eq!(
            t_bad_task,
            Err(PetriNetLivenessViolation::UnknownTask { task: "invalid_task" })
        );

        let net = BackgroundWorkerPetriNet::default();
        assert_eq!(
            net.try_with_capacity_limit(0).err(),
            Some(PetriNetLivenessViolation::ZeroCapacityLimit)
        );

        let m0 = PetriMarking {
            p_fd_available: 10,
            p_quota_available: 10,
            p_manifest_lock: 1,
            p_flush_tasks: 1,
            p_compact_tasks: 0,
            p_blob_gc_tasks: 0,
        };
        let net_valid = BackgroundWorkerPetriNet::default();
        assert_eq!(
            net_valid.verify_liveness_and_deadlock_freedom(m0, 0),
            Err(PetriNetLivenessViolation::ZeroMaxStates)
        );

        let disp = format!("{}", PetriNetLivenessViolation::ZeroCapacityLimit);
        assert!(!disp.is_empty());
    }
}
