//! Pilar 3: Bisimulação e Continuidade Estrita de Cursores Federados (RFC-0285).
//!
//! Garante a equivalência semântica e ausência de ressurreição zumbi na transição
//! entre snapshots atômicos (`sync_caixote_state_atomic`) e fluxos de deltas (`sync_caixote_state_delta`).

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt;
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
    EmptyKeyInSnapshot,
    EmptyKeyInDelta { sequence: u64 },
    InvalidDeltaSequence { sequence: u64 },
    SequenceOverflow,
}

impl fmt::Display for ContinuityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SequenceGapDetected { expected, got } => {
                write!(f, "Sequence gap detected in federated cursor: expected {expected}, got {got}")
            }
            Self::SnapshotOutdated { current_seq, snapshot_seq } => {
                write!(f, "Snapshot sequence {snapshot_seq} is older than current sequence {current_seq}")
            }
            Self::CorruptSnapshotSequence => {
                write!(f, "Corrupted snapshot sequence 0 on non-empty dataset")
            }
            Self::EmptyKeyInSnapshot => {
                write!(f, "Empty key in snapshot is structurally forbidden")
            }
            Self::EmptyKeyInDelta { sequence } => {
                write!(f, "Empty key in delta at sequence {sequence}")
            }
            Self::InvalidDeltaSequence { sequence } => {
                write!(f, "Invalid delta sequence {sequence} (cannot be 0)")
            }
            Self::SequenceOverflow => {
                write!(f, "Sequence number overflow beyond u64::MAX")
            }
        }
    }
}

impl std::error::Error for ContinuityError {}

impl FederatedFoldState {
    /// Inicializa ou substitui o estado com um snapshot atômico completo.
    pub fn apply_atomic_snapshot(
        &mut self,
        snapshot_seq: u64,
        data: impl IntoIterator<Item = (Bytes, Bytes)>,
    ) -> Result<(), ContinuityError> {
        let entries_vec: Vec<(Bytes, Bytes)> = data.into_iter().collect();
        if snapshot_seq == 0 && (!entries_vec.is_empty() || !self.entries.is_empty()) {
            return Err(ContinuityError::CorruptSnapshotSequence);
        }
        for (k, _) in &entries_vec {
            if k.is_empty() {
                return Err(ContinuityError::EmptyKeyInSnapshot);
            }
        }
        // Snapshots mais antigos que o estado atual são rejeitados fail-closed
        if snapshot_seq < self.last_sequence {
            return Err(ContinuityError::SnapshotOutdated {
                current_seq: self.last_sequence,
                snapshot_seq,
            });
        }
        self.entries.clear();
        for (k, v) in entries_vec {
            self.entries.insert(k, v);
        }
        self.last_sequence = snapshot_seq;
        Ok(())
    }

    /// Aplica um lote sequencial de deltas com garantia estrita de continuidade e atomicidade (rollback total em falha).
    pub fn apply_deltas(&mut self, deltas: &[SequencedDelta]) -> Result<usize, ContinuityError> {
        let mut sim_last_seq = self.last_sequence;
        let mut staged_ops = Vec::new();

        for delta in deltas {
            if delta.sequence == 0 {
                return Err(ContinuityError::InvalidDeltaSequence { sequence: 0 });
            }
            let key = match &delta.op {
                DeltaOp::Put { key, .. } => key,
                DeltaOp::Delete { key } => key,
            };
            if key.is_empty() {
                return Err(ContinuityError::EmptyKeyInDelta { sequence: delta.sequence });
            }

            if delta.sequence <= sim_last_seq {
                continue;
            }

            let expected = sim_last_seq.checked_add(1).ok_or(ContinuityError::SequenceOverflow)?;
            if delta.sequence != expected {
                return Err(ContinuityError::SequenceGapDetected {
                    expected,
                    got: delta.sequence,
                });
            }

            sim_last_seq = delta.sequence;
            staged_ops.push((delta.sequence, &delta.op));
        }

        let applied = staged_ops.len();
        for (seq, op) in staged_ops {
            match op {
                DeltaOp::Put { key, value } => {
                    self.entries.insert(key.clone(), value.clone());
                }
                DeltaOp::Delete { key } => {
                    self.entries.remove(key);
                }
            }
            self.last_sequence = seq;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_federated_cursor_continuity_structural_invariants_red_to_green() {
        let mut state = FederatedFoldState::default();

        // 1. Empty key in snapshot rejected
        let bad_snap = vec![(Bytes::new(), Bytes::from_static(b"val"))];
        assert_eq!(
            state.apply_atomic_snapshot(1, bad_snap),
            Err(ContinuityError::EmptyKeyInSnapshot)
        );

        // 2. Corrupt snapshot sequence 0 on non-empty dataset
        let non_empty = vec![(Bytes::from_static(b"key1"), Bytes::from_static(b"val1"))];
        assert_eq!(
            state.apply_atomic_snapshot(0, non_empty.clone()),
            Err(ContinuityError::CorruptSnapshotSequence)
        );

        // 3. Valid snapshot works
        assert_eq!(state.apply_atomic_snapshot(10, non_empty), Ok(()));
        assert_eq!(state.last_sequence, 10);

        // 4. Stale snapshot rejected
        assert_eq!(
            state.apply_atomic_snapshot(5, vec![]),
            Err(ContinuityError::SnapshotOutdated {
                current_seq: 10,
                snapshot_seq: 5,
            })
        );

        // 5. Delta sequence gap rejected
        let gap_deltas = vec![SequencedDelta {
            sequence: 12, // expected 11
            op: DeltaOp::Put {
                key: Bytes::from_static(b"key2"),
                value: Bytes::from_static(b"val2"),
            },
        }];
        assert_eq!(
            state.apply_deltas(&gap_deltas),
            Err(ContinuityError::SequenceGapDetected {
                expected: 11,
                got: 12,
            })
        );
    }
}

