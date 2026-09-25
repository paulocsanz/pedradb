//! Pilar 3: Bisimulação e Continuidade Estrita de Cursores Federados (RFC-0285).
//!
//! Garante a equivalência semântica e ausência de ressurreição zumbi na transição
//! entre snapshots atômicos (`sync_caixote_state_atomic`) e fluxos de deltas (`sync_caixote_state_delta`).

use std::collections::BTreeMap;
use bytes::Bytes;

/// Operação atômica de mutação em delta federado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaOp {
    Put { key: Bytes, value: Bytes },
    Delete { key: Bytes },
}

/// Evento de log delta federado sequenciado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequencedDelta {
    pub sequence: u64,
    pub op: DeltaOp,
}

/// Estado materializado do fold federado.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FederatedFoldState {
    pub last_sequence: u64,
    pub entries: BTreeMap<Bytes, Bytes>,
}

/// Erro de continuidade no cursor federado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContinuityError {
    SequenceGapDetected { expected: u64, got: u64 },
    SnapshotOutdated { current_seq: u64, snapshot_seq: u64 },
    CorruptSnapshotSequence,
}

impl FederatedFoldState {
    /// Inicializa ou substitui o estado com um snapshot atômico completo.
    pub fn apply_atomic_snapshot(
        &mut self,
        snapshot_seq: u64,
        data: impl IntoIterator<Item = (Bytes, Bytes)>,
    ) -> Result<(), ContinuityError> {
        if snapshot_seq == 0 && !self.entries.is_empty() {
            return Err(ContinuityError::CorruptSnapshotSequence);
        }
        // Snapshots mais antigos que o estado atual são rejeitados fail-closed
        if snapshot_seq < self.last_sequence {
            return Err(ContinuityError::SnapshotOutdated {
                current_seq: self.last_sequence,
                snapshot_seq,
            });
        }
        self.entries.clear();
        for (k, v) in data {
            self.entries.insert(k, v);
        }
        self.last_sequence = snapshot_seq;
        Ok(())
    }

    /// Aplica um lote sequencial de deltas com garantia estrita de continuidade.
    pub fn apply_deltas(&mut self, deltas: &[SequencedDelta]) -> Result<usize, ContinuityError> {
        let mut applied = 0;
        for delta in deltas {
            // Se o delta é anterior ou igual ao snapshot/última seq, descarta idempotentemente
            if delta.sequence <= self.last_sequence {
                continue;
            }
            // Se há um buraco (gap) na sequência, interrompe imediatamente (fail-closed)
            let expected = self.last_sequence + 1;
            if delta.sequence != expected {
                return Err(ContinuityError::SequenceGapDetected {
                    expected,
                    got: delta.sequence,
                });
            }
            match &delta.op {
                DeltaOp::Put { key, value } => {
                    self.entries.insert(key.clone(), value.clone());
                }
                DeltaOp::Delete { key } => {
                    self.entries.remove(key);
                }
            }
            self.last_sequence = delta.sequence;
            applied += 1;
        }
        Ok(applied)
    }
}

/// Mutante degenerado (AS-IS): ignora gaps de sequência silenciosamente, aplicando deltas desordenados.
pub fn apply_deltas_as_is_ignore_gaps(state: &mut FederatedFoldState, deltas: &[SequencedDelta]) {
    for delta in deltas {
        match &delta.op {
            DeltaOp::Put { key, value } => {
                state.entries.insert(key.clone(), value.clone());
            }
            DeltaOp::Delete { key } => {
                state.entries.remove(key);
            }
        }
        state.last_sequence = state.last_sequence.max(delta.sequence);
    }
}
