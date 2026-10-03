//! RFC-0294: Pilar 3 - Cobertura Temporal Fechada e Visibilidade Estável de Snapshot sob Compactação.
//!
//! Formaliza a blindagem de iteradores de snapshot de longa duração contra purga precoce de
//! tombstones em compactações de níveis inferiores, provando a invariância de visibilidade do scan.


/// Violações de estabilidade de snapshot sob compactações concorrentes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotCompactionViolation {
    /// Chave fornecida é vazia.
    EmptyKey,
    /// Sequence number não pode ser zero.
    ZeroSequenceNumber,
    /// ID do snapshot não pode ser zero.
    InvalidSnapshotId,
    /// Registro do tipo put exige um valor não-vazio.
    MissingPutValue,
    /// Tombstone não pode conter payload de valor.
    UnexpectedTombstoneValue,
    /// Um tombstone necessário para mascarar dados antigos em um snapshot ativo foi prematuramente purgado.
    TombstonePrematurelyPurged {
        /// Chave afetada.
        key: Vec<u8>,
        /// Sequence number do tombstone purgado.
        tombstone_seq: u64,
        /// Sequence number do snapshot ativo afetado.
        snapshot_seq: u64,
    },
    /// Uma chave deletada ressuscitou na visão do iterador de snapshot.
    ResurrectedKeyInSnapshotView {
        /// Chave ressuscitada.
        key: Vec<u8>,
        /// Snapshot sequence.
        snapshot_seq: u64,
    },
}

impl std::fmt::Display for SnapshotCompactionViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyKey => write!(f, "SST record key cannot be empty"),
            Self::ZeroSequenceNumber => write!(f, "sequence number cannot be zero"),
            Self::InvalidSnapshotId => write!(f, "snapshot ID cannot be zero"),
            Self::MissingPutValue => write!(f, "put record requires a present value"),
            Self::UnexpectedTombstoneValue => write!(f, "tombstone record cannot carry a value payload"),
            Self::TombstonePrematurelyPurged { key, tombstone_seq, snapshot_seq } => {
                write!(
                    f,
                    "tombstone with seq {tombstone_seq} for key {:?} prematurely purged while snapshot {snapshot_seq} is active",
                    key
                )
            }
            Self::ResurrectedKeyInSnapshotView { key, snapshot_seq } => {
                write!(
                    f,
                    "deleted key {:?} resurrected in active snapshot view {snapshot_seq}",
                    key
                )
            }
        }
    }
}

impl std::error::Error for SnapshotCompactionViolation {}

/// Descritor de um snapshot ativo mantido por um cliente ou iterador de longa duração.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveSnapshotDescriptor {
    /// ID unívoco do snapshot.
    pub snapshot_id: u64,
    /// Sequence number temporal no momento da abertura do snapshot.
    pub snapshot_seq: u64,
}

impl ActiveSnapshotDescriptor {
    /// Cria um descritor de snapshot validado.
    pub fn try_new(snapshot_id: u64, snapshot_seq: u64) -> Result<Self, SnapshotCompactionViolation> {
        if snapshot_id == 0 {
            return Err(SnapshotCompactionViolation::InvalidSnapshotId);
        }
        if snapshot_seq == 0 {
            return Err(SnapshotCompactionViolation::ZeroSequenceNumber);
        }
        Ok(Self {
            snapshot_id,
            snapshot_seq,
        })
    }
}

/// Registro de dados ou tombstone em um arquivo SST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SstRecordEntry {
    /// Chave do registro.
    pub key: Vec<u8>,
    /// Sequence number da mutação.
    pub seq: u64,
    /// Flag indicando se é um tombstone (delete).
    pub is_tombstone: bool,
    /// Valor (se não for tombstone).
    pub value: Option<Vec<u8>>,
}

impl SstRecordEntry {
    /// Cria uma mutação Put validada.
    pub fn try_new_put(key: Vec<u8>, seq: u64, value: Vec<u8>) -> Result<Self, SnapshotCompactionViolation> {
        if key.is_empty() {
            return Err(SnapshotCompactionViolation::EmptyKey);
        }
        if seq == 0 {
            return Err(SnapshotCompactionViolation::ZeroSequenceNumber);
        }
        Ok(Self {
            key,
            seq,
            is_tombstone: false,
            value: Some(value),
        })
    }

    /// Cria uma mutação Tombstone (Delete) validada.
    pub fn try_new_delete(key: Vec<u8>, seq: u64) -> Result<Self, SnapshotCompactionViolation> {
        if key.is_empty() {
            return Err(SnapshotCompactionViolation::EmptyKey);
        }
        if seq == 0 {
            return Err(SnapshotCompactionViolation::ZeroSequenceNumber);
        }
        Ok(Self {
            key,
            seq,
            is_tombstone: true,
            value: None,
        })
    }
}

/// Oráculo de verificação da estabilidade de snapshot sob compactação.
pub struct SnapshotCompactionStabilityOracle;

impl SnapshotCompactionStabilityOracle {
    /// Determina se um tombstone pode ser purgado com segurança durante a compactação para o nível mais baixo (L_max).
    ///
    /// Regra formal de segurança:
    /// Um tombstone só pode ser purgado se NÃO existir nenhum snapshot vivo cujo `snapshot_seq >= tombstone.seq`.
    /// Se houver qualquer snapshot vivo com `snapshot_seq >= tombstone.seq`, o tombstone deve ser mantido
    /// para continuar mascarando eventuais versões mais antigas da chave que estejam visíveis sob esse snapshot.
    pub fn can_purge_tombstone(
        tombstone_seq: u64,
        live_snapshots: &[ActiveSnapshotDescriptor],
    ) -> bool {
        if live_snapshots.is_empty() {
            return true;
        }

        // Se houver qualquer snapshot vivo com snapshot_seq >= tombstone_seq,
        // o tombstone não pode ser purgado pois ainda é necessário para mascarar a chave.
        !live_snapshots.iter().any(|s| s.snapshot_seq >= tombstone_seq)
    }

    /// Executa uma compactação segura em um conjunto de registros considerando os snapshots vivos.
    pub fn compact_entries(
        records: &[SstRecordEntry],
        live_snapshots: &[ActiveSnapshotDescriptor],
        is_bottommost_level: bool,
    ) -> Vec<SstRecordEntry> {
        let mut sorted = records.to_vec();
        // Ordena por chave crescente, e para a mesma chave por seq decrescente (mais recente primeiro)
        sorted.sort_by(|a, b| {
            a.key.cmp(&b.key).then_with(|| b.seq.cmp(&a.seq))
        });

        let mut compacted = Vec::new();
        let mut i = 0;

        while i < sorted.len() {
            let curr = &sorted[i];
            let key = &curr.key;

            // Agrupa todas as versões da mesma chave
            let mut key_group = Vec::new();
            while i < sorted.len() && sorted[i].key == *key {
                key_group.push(sorted[i].clone());
                i += 1;
            }

            // Identifica quais versões são necessárias para leituras atuais ou snapshots ativos
            let mut retained_indices = std::collections::BTreeSet::new();

            // Versão mais recente é visível para leitores do estado atual
            retained_indices.insert(0);

            // Para cada snapshot vivo, localiza a versão mais recente visível (seq <= snapshot_seq)
            for snap in live_snapshots {
                if let Some(pos) = key_group.iter().position(|r| r.seq <= snap.snapshot_seq) {
                    retained_indices.insert(pos);
                }
            }

            // Filtra os registros retidos, avaliando purga de tombstones no nível mais baixo
            for &idx in &retained_indices {
                let record = &key_group[idx];
                if record.is_tombstone && is_bottommost_level {
                    if Self::can_purge_tombstone(record.seq, live_snapshots) {
                        // Purga segura do tombstone no nível mais baixo
                        continue;
                    }
                }
                compacted.push(record.clone());
            }
        }

        compacted
    }

    /// Valida que um leitor de snapshot enxerga o mesmíssimo valor antes e depois da compactação.
    pub fn verify_snapshot_invariance(
        key: &[u8],
        snapshot: ActiveSnapshotDescriptor,
        pre_compaction_records: &[SstRecordEntry],
        post_compaction_records: &[SstRecordEntry],
    ) -> Result<(), SnapshotCompactionViolation> {
        if key.is_empty() {
            return Err(SnapshotCompactionViolation::EmptyKey);
        }
        if snapshot.snapshot_id == 0 {
            return Err(SnapshotCompactionViolation::InvalidSnapshotId);
        }
        if snapshot.snapshot_seq == 0 {
            return Err(SnapshotCompactionViolation::ZeroSequenceNumber);
        }

        let eval = |recs: &[SstRecordEntry]| -> Option<Vec<u8>> {
            // Busca a mutação mais recente visível sob o snapshot (seq <= snapshot.snapshot_seq)
            let mut visible: Vec<&SstRecordEntry> = recs
                .iter()
                .filter(|r| r.key == key && r.seq <= snapshot.snapshot_seq)
                .collect();
            visible.sort_by_key(|r| std::cmp::Reverse(r.seq));

            if let Some(first) = visible.first() {
                if first.is_tombstone {
                    None
                } else {
                    first.value.clone()
                }
            } else {
                None
            }
        };

        let pre_val = eval(pre_compaction_records);
        let post_val = eval(post_compaction_records);

        if pre_val != post_val {
            if pre_val.is_none() && post_val.is_some() {
                return Err(SnapshotCompactionViolation::ResurrectedKeyInSnapshotView {
                    key: key.to_vec(),
                    snapshot_seq: snapshot.snapshot_seq,
                });
            } else {
                return Err(SnapshotCompactionViolation::TombstonePrematurelyPurged {
                    key: key.to_vec(),
                    tombstone_seq: snapshot.snapshot_seq,
                    snapshot_seq: snapshot.snapshot_seq,
                });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_compaction_stability_structural_invariants_red_to_green() {
        let key = b"user_key".to_vec();

        // 1. Valid snapshot descriptor and entries
        let snap = ActiveSnapshotDescriptor::try_new(1, 10).expect("valid snap");
        assert_eq!(snap.snapshot_id, 1);
        assert_eq!(snap.snapshot_seq, 10);

        let put1 = SstRecordEntry::try_new_put(key.clone(), 5, b"v1".to_vec()).expect("put1");
        let del1 = SstRecordEntry::try_new_delete(key.clone(), 8).expect("del1");

        // 2. Reject zero sequence numbers and invalid snapshot IDs
        let err_snap_id = ActiveSnapshotDescriptor::try_new(0, 10);
        assert_eq!(err_snap_id, Err(SnapshotCompactionViolation::InvalidSnapshotId));

        let err_snap_seq = ActiveSnapshotDescriptor::try_new(1, 0);
        assert_eq!(err_snap_seq, Err(SnapshotCompactionViolation::ZeroSequenceNumber));

        let err_put_empty_key = SstRecordEntry::try_new_put(vec![], 5, b"v1".to_vec());
        assert_eq!(err_put_empty_key, Err(SnapshotCompactionViolation::EmptyKey));

        let err_del_zero_seq = SstRecordEntry::try_new_delete(key.clone(), 0);
        assert_eq!(err_del_zero_seq, Err(SnapshotCompactionViolation::ZeroSequenceNumber));

        // 3. Compact entries preserves snapshot invariance
        let records = vec![put1.clone(), del1.clone()];
        let snapshots = vec![snap];
        let compacted = SnapshotCompactionStabilityOracle::compact_entries(&records, &snapshots, true);

        // del1 (seq 8) cannot be purged because snapshot_seq (10) >= tombstone.seq (8)
        assert!(compacted.iter().any(|r| r.is_tombstone && r.seq == 8));

        // Invariance verified
        assert!(SnapshotCompactionStabilityOracle::verify_snapshot_invariance(
            &key,
            snap,
            &records,
            &compacted,
        ).is_ok());

        // 4. Invariance fails if tombstone is prematurely purged
        let leaked_records = vec![put1];
        let err_resurrect = SnapshotCompactionStabilityOracle::verify_snapshot_invariance(
            &key,
            snap,
            &records,
            &leaked_records,
        );
        assert_eq!(
            err_resurrect,
            Err(SnapshotCompactionViolation::ResurrectedKeyInSnapshotView {
                key: key.clone(),
                snapshot_seq: 10,
            })
        );

        // 5. Display & Error implementations
        let d = format!("{}", SnapshotCompactionViolation::EmptyKey);
        assert!(d.contains("SST record key cannot be empty"));
    }
}
