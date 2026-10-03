//! RFC-0291 Fronteira 3: Não-Interferência e Hazard Pointers: Preservação de Snapshot sob GC de SSTs.
//!
//! Garante matematicamente via Separation Logic que nenhum arquivo SST registrado
//! em disco seja desvinculado (unlinked) enquanto for potencialmente visível por
//! qualquer snapshot ativo ou leitor de longa duração.

#![forbid(unsafe_code)]

use std::collections::HashMap;

/// Descritor de arquivo SST com cobertura temporal de números de sequência.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SstTemporalDescriptor {
    pub file_number: u64,
    pub min_seq: u64,
    pub max_seq: u64,
    pub level: usize,
    pub file_size: u64,
}

impl SstTemporalDescriptor {
    /// Safely constructs a temporal descriptor, validating non-zero file number and valid temporal bounds.
    pub fn try_new(
        file_number: u64,
        min_seq: u64,
        max_seq: u64,
        level: usize,
        file_size: u64,
    ) -> Result<Self, SnapshotIsolationViolation> {
        if file_number == 0 {
            return Err(SnapshotIsolationViolation::ZeroFileNumber);
        }
        if min_seq > max_seq {
            return Err(SnapshotIsolationViolation::CorruptedTemporalBounds {
                file_number,
                min_seq,
                max_seq,
            });
        }
        Ok(Self {
            file_number,
            min_seq,
            max_seq,
            level,
            file_size,
        })
    }
}

/// Registro de Snapshot ativo no motor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LiveSnapshot {
    pub snapshot_id: u64,
    pub sequence_number: u64,
}

impl LiveSnapshot {
    /// Safely constructs a live snapshot record, rejecting zero IDs and zero sequences.
    pub fn try_new(snapshot_id: u64, sequence_number: u64) -> Result<Self, SnapshotIsolationViolation> {
        if snapshot_id == 0 {
            return Err(SnapshotIsolationViolation::ZeroSnapshotId);
        }
        if sequence_number == 0 {
            return Err(SnapshotIsolationViolation::ZeroSnapshotSeq);
        }
        Ok(Self {
            snapshot_id,
            sequence_number,
        })
    }
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
    CorruptedTemporalBounds {
        file_number: u64,
        min_seq: u64,
        max_seq: u64,
    },
    ZeroFileNumber,
    ZeroSnapshotId,
    ZeroSnapshotSeq,
}

impl std::fmt::Display for SnapshotIsolationViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PrematureUnlinkUnderActiveSnapshot {
                file_number,
                snapshot_id,
                snapshot_seq,
                sst_min_seq,
                sst_max_seq,
            } => write!(
                f,
                "Premature unlink of file {file_number} under active snapshot {snapshot_id} (seq {snapshot_seq}) spanning [{sst_min_seq}, {sst_max_seq}]"
            ),
            Self::PrematureUnlinkUnderHazardPointer { file_number, active_pins } => {
                write!(f, "Premature unlink of file {file_number} with {active_pins} active hazard pointer pins")
            }
            Self::CorruptedTemporalBounds { file_number, min_seq, max_seq } => {
                write!(f, "Corrupted temporal bounds for file {file_number}: min_seq {min_seq} > max_seq {max_seq}")
            }
            Self::ZeroFileNumber => write!(f, "File number cannot be zero"),
            Self::ZeroSnapshotId => write!(f, "Snapshot ID cannot be zero"),
            Self::ZeroSnapshotSeq => write!(f, "Snapshot sequence number cannot be zero"),
        }
    }
}

impl std::error::Error for SnapshotIsolationViolation {}

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

    /// Registra um novo snapshot ativo de forma segura.
    pub fn try_register_snapshot(&mut self, snapshot: LiveSnapshot) -> Result<(), SnapshotIsolationViolation> {
        if snapshot.snapshot_id == 0 {
            return Err(SnapshotIsolationViolation::ZeroSnapshotId);
        }
        if snapshot.sequence_number == 0 {
            return Err(SnapshotIsolationViolation::ZeroSnapshotSeq);
        }
        self.live_snapshots.insert(snapshot.snapshot_id, snapshot);
        Ok(())
    }

    /// Registra um novo snapshot ativo (legado).
    pub fn register_snapshot(&mut self, snapshot: LiveSnapshot) {
        let _ = self.try_register_snapshot(snapshot);
    }

    /// Retorna o número de snapshots ativos registrados.
    #[must_use]
    pub fn active_snapshot_count(&self) -> usize {
        self.live_snapshots.len()
    }

    /// Remove um snapshot após o término da transação/leitor.
    pub fn release_snapshot(&mut self, snapshot_id: u64) {
        self.live_snapshots.remove(&snapshot_id);
    }

    /// Incrementa o hazard pointer de um arquivo por um iterador em execução de forma segura.
    pub fn try_pin_file(&mut self, file_number: u64) -> Result<(), SnapshotIsolationViolation> {
        if file_number == 0 {
            return Err(SnapshotIsolationViolation::ZeroFileNumber);
        }
        let entry = self.hazard_pins.entry(file_number).or_insert(0);
        *entry = entry.saturating_add(1);
        Ok(())
    }

    /// Incrementa o hazard pointer de um arquivo por um iterador em execução.
    pub fn pin_file(&mut self, file_number: u64) {
        let _ = self.try_pin_file(file_number);
    }

    /// Verifica se um arquivo está retido por hazard pointers ativos.
    #[must_use]
    pub fn is_pinned(&self, file_number: u64) -> bool {
        self.hazard_pins.get(&file_number).copied().unwrap_or(0) > 0
    }

    /// Retorna a contagem de hazard pins para um arquivo.
    #[must_use]
    pub fn hazard_count(&self, file_number: u64) -> usize {
        self.hazard_pins.get(&file_number).copied().unwrap_or(0)
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
        let is_corrupted = sst.min_seq > sst.max_seq;

        // 1. Verifica se algum hazard pointer ativo está fixando este descritor de arquivo
        let hazard_ref_count = self.hazard_pins.get(&sst.file_number).copied().unwrap_or(0);

        // 2. Verifica se o arquivo intercepta a linha temporal de qualquer snapshot vivo
        let retaining_snapshot_id = self
            .live_snapshots
            .values()
            .find(|snap| sst.min_seq <= snap.sequence_number)
            .map(|snap| snap.snapshot_id);

        if hazard_ref_count > 0 || retaining_snapshot_id.is_some() || is_corrupted {
            return UnlinkDecision::RetainActive {
                file_number: sst.file_number,
                retaining_snapshot_id,
                hazard_ref_count,
            };
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
            if sst.min_seq > sst.max_seq {
                return Err(SnapshotIsolationViolation::CorruptedTemporalBounds {
                    file_number: sst.file_number,
                    min_seq: sst.min_seq,
                    max_seq: sst.max_seq,
                });
            }

            match self.evaluate_unlink(sst) {
                UnlinkDecision::SafeToUnlink { .. } => continue,
                UnlinkDecision::RetainActive {
                    retaining_snapshot_id: Some(snap_id),
                    ..
                } => {
                    let snap_seq = self
                        .live_snapshots
                        .get(&snap_id)
                        .map_or(0, |s| s.sequence_number);
                    return Err(
                        SnapshotIsolationViolation::PrematureUnlinkUnderActiveSnapshot {
                            file_number: sst.file_number,
                            snapshot_id: snap_id,
                            snapshot_seq: snap_seq,
                            sst_min_seq: sst.min_seq,
                            sst_max_seq: sst.max_seq,
                        },
                    );
                }
                UnlinkDecision::RetainActive {
                    hazard_ref_count, ..
                } => {
                    if hazard_ref_count > 0 {
                        return Err(
                            SnapshotIsolationViolation::PrematureUnlinkUnderHazardPointer {
                                file_number: sst.file_number,
                                active_pins: hazard_ref_count,
                            },
                        );
                    } else {
                        return Err(SnapshotIsolationViolation::CorruptedTemporalBounds {
                            file_number: sst.file_number,
                            min_seq: sst.min_seq,
                            max_seq: sst.max_seq,
                        });
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_descriptor_validation_red_to_green() {
        assert_eq!(
            SstTemporalDescriptor::try_new(0, 10, 20, 0, 1024),
            Err(SnapshotIsolationViolation::ZeroFileNumber)
        );
        assert_eq!(
            SstTemporalDescriptor::try_new(1, 30, 20, 0, 1024),
            Err(SnapshotIsolationViolation::CorruptedTemporalBounds {
                file_number: 1,
                min_seq: 30,
                max_seq: 20
            })
        );
        let sst = SstTemporalDescriptor::try_new(1, 10, 20, 0, 1024).expect("valid");
        assert_eq!(sst.file_number, 1);
    }

    #[test]
    fn test_live_snapshot_validation_red_to_green() {
        assert_eq!(
            LiveSnapshot::try_new(0, 100),
            Err(SnapshotIsolationViolation::ZeroSnapshotId)
        );
        assert_eq!(
            LiveSnapshot::try_new(1, 0),
            Err(SnapshotIsolationViolation::ZeroSnapshotSeq)
        );
        let snap = LiveSnapshot::try_new(1, 100).expect("valid");
        assert_eq!(snap.snapshot_id, 1);
        assert_eq!(snap.sequence_number, 100);
    }

    #[test]
    fn test_hazard_gate_lifecycle_and_pinning() {
        let mut gate = SnapshotHazardSafetyGate::new();
        assert_eq!(gate.active_snapshot_count(), 0);

        let snap = LiveSnapshot::try_new(10, 50).unwrap();
        assert!(gate.try_register_snapshot(snap).is_ok());
        assert_eq!(gate.active_snapshot_count(), 1);

        assert_eq!(gate.try_pin_file(0), Err(SnapshotIsolationViolation::ZeroFileNumber));
        assert!(!gate.is_pinned(42));
        assert_eq!(gate.hazard_count(42), 0);

        gate.pin_file(42);
        assert!(gate.is_pinned(42));
        assert_eq!(gate.hazard_count(42), 1);

        gate.pin_file(42);
        assert_eq!(gate.hazard_count(42), 2);

        gate.unpin_file(42);
        assert_eq!(gate.hazard_count(42), 1);
        gate.unpin_file(42);
        assert_eq!(gate.hazard_count(42), 0);
        assert!(!gate.is_pinned(42));

        gate.release_snapshot(10);
        assert_eq!(gate.active_snapshot_count(), 0);
    }

    #[test]
    fn test_verify_unlinks_soundness_protection() {
        let mut gate = SnapshotHazardSafetyGate::new();
        let snap = LiveSnapshot::try_new(1, 100).unwrap();
        gate.register_snapshot(snap);

        // SST spanning 50..80: min_seq 50 <= snap.seq 100 -> MUST retain!
        let protected_sst = SstTemporalDescriptor::try_new(5, 50, 80, 1, 4096).unwrap();
        assert!(gate.verify_unlinks_soundness(&[protected_sst]).is_err());

        // SST spanning 120..150: min_seq 120 > snap.seq 100 -> Safe to unlink
        let free_sst = SstTemporalDescriptor::try_new(6, 120, 150, 1, 4096).unwrap();
        assert!(gate.verify_unlinks_soundness(&[free_sst]).is_ok());

        // Pinned file cannot be unlinked even if seq is beyond snapshot
        gate.pin_file(6);
        let free_sst2 = SstTemporalDescriptor::try_new(6, 120, 150, 1, 4096).unwrap();
        assert!(gate.verify_unlinks_soundness(&[free_sst2]).is_err());
    }
}

