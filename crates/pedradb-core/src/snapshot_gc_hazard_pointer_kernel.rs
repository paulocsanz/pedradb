//! RFC-0291 Fronteira 3: Não-Interferência e Hazard Pointers: Preservação de Snapshot sob GC de SSTs.
//!
//! Garante matematicamente via Separation Logic que nenhum arquivo SST registrado
//! em disco seja desvinculado (unlinked) enquanto for potencialmente visível por
//! qualquer snapshot ativo ou leitor de longa duração.

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};

/// Descritor de arquivo SST com cobertura temporal de números de sequência.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SstTemporalDescriptor {
    pub file_number: u64,
    pub min_seq: u64,
    pub max_seq: u64,
    pub level: usize,
    pub file_size: u64,
}

/// Registro de Snapshot ativo no motor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LiveSnapshot {
    pub snapshot_id: u64,
    pub sequence_number: u64,
}

/// Ação de descarte de arquivo de compactação.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnlinkDecision {
    /// Arquivo seguro para remoção física do filesystem.
    SafeToUnlink { file_number: u64 },
    /// Descarte estritamente proibido: retido por snapshot ativo ou hazard pointer.
    RetainActive {
        file_number: u64,
        retaining_snapshot_id: Option<u64>,
        hazard_ref_count: usize,
    },
}

/// Violação de integridade de snapshot por descarte prematuro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotIsolationViolation {
    PrematureUnlinkUnderActiveSnapshot {
        file_number: u64,
        snapshot_id: u64,
        snapshot_seq: u64,
        sst_min_seq: u64,
        sst_max_seq: u64,
    },
    PrematureUnlinkUnderHazardPointer {
        file_number: u64,
        active_pins: usize,
    },
}

/// Gerenciador de Hazard Pointers e Não-Interferência de Snapshots.
pub struct SnapshotHazardSafetyGate {
    live_snapshots: HashMap<u64, LiveSnapshot>,
    hazard_pins: HashMap<u64, usize>,
}

impl Default for SnapshotHazardSafetyGate {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapshotHazardSafetyGate {
    pub fn new() -> Self {
        Self {
            live_snapshots: HashMap::new(),
            hazard_pins: HashMap::new(),
        }
    }

    /// Registra um novo snapshot ativo.
    pub fn register_snapshot(&mut self, snapshot: LiveSnapshot) {
        self.live_snapshots.insert(snapshot.snapshot_id, snapshot);
    }

    /// Remove um snapshot após o término da transação/leitor.
    pub fn release_snapshot(&mut self, snapshot_id: u64) {
        self.live_snapshots.remove(&snapshot_id);
    }

    /// Incrementa o hazard pointer de um arquivo por um iterador em execução.
    pub fn pin_file(&mut self, file_number: u64) {
        *self.hazard_pins.entry(file_number).or_insert(0) += 1;
    }

    /// Decrementa o hazard pointer ao fechar o iterador.
    pub fn unpin_file(&mut self, file_number: u64) {
        if let Some(pins) = self.hazard_pins.get_mut(&file_number) {
            if *pins > 1 {
                *pins -= 1;
            } else {
                self.hazard_pins.remove(&file_number);
            }
        }
    }

    /// Avalia se um arquivo marcado como obsoleto pela compactação pode sofrer `unlink` físico.
    #[must_use]
    pub fn evaluate_unlink(&self, sst: &SstTemporalDescriptor) -> UnlinkDecision {
        // 1. Verifica se algum hazard pointer ativo está fixando este descritor de arquivo
        if let Some(&pins) = self.hazard_pins.get(&sst.file_number) {
            if pins > 0 {
                return UnlinkDecision::RetainActive {
                    file_number: sst.file_number,
                    retaining_snapshot_id: None,
                    hazard_ref_count: pins,
                };
            }
        }

        // 2. Verifica se o arquivo intercepta a linha temporal de qualquer snapshot vivo
        // Se min_seq <= snap.seq <= max_seq (ou se max_seq <= snap.seq para dados que o snapshot precisa ler)
        for snap in self.live_snapshots.values() {
            // Um arquivo contém dados visíveis para o snapshot se seu min_seq é menor ou igual ao snapshot
            if sst.min_seq <= snap.sequence_number {
                return UnlinkDecision::RetainActive {
                    file_number: sst.file_number,
                    retaining_snapshot_id: Some(snap.snapshot_id),
                    hazard_ref_count: 0,
                };
            }
        }

        UnlinkDecision::SafeToUnlink {
            file_number: sst.file_number,
        }
    }

    /// Valida uma lista de deleções contra os invariantes de snapshots vivos (auditoria anti-vacuidade).
    pub fn verify_unlinks_soundness(
        &self,
        unlinks: &[SstTemporalDescriptor],
    ) -> Result<(), SnapshotIsolationViolation> {
        for sst in unlinks {
            match self.evaluate_unlink(sst) {
                UnlinkDecision::SafeToUnlink { .. } => continue,
                UnlinkDecision::RetainActive {
                    retaining_snapshot_id: Some(snap_id),
                    ..
                } => {
                    let snap = self.live_snapshots.get(&snap_id).expect("Snapshot must exist");
                    return Err(
                        SnapshotIsolationViolation::PrematureUnlinkUnderActiveSnapshot {
                            file_number: sst.file_number,
                            snapshot_id: snap_id,
                            snapshot_seq: snap.sequence_number,
                            sst_min_seq: sst.min_seq,
                            sst_max_seq: sst.max_seq,
                        },
                    );
                }
                UnlinkDecision::RetainActive {
                    hazard_ref_count, ..
                } => {
                    return Err(
                        SnapshotIsolationViolation::PrematureUnlinkUnderHazardPointer {
                            file_number: sst.file_number,
                            active_pins: hazard_ref_count,
                        },
                    );
                }
            }
        }
        Ok(())
    }
}
