//! RFC-0294: Pilar 3 - Cobertura Temporal Fechada e Visibilidade Estável de Snapshot sob Compactação.
//!
//! Formaliza a blindagem de iteradores de snapshot de longa duração contra purga precoce de
//! tombstones em compactações de níveis inferiores, provando a invariância de visibilidade do scan.


/// Violações de estabilidade de snapshot sob compactações concorrentes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotCompactionViolation {
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

/// Descritor de um snapshot ativo mantido por um cliente ou iterador de longa duração.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveSnapshotDescriptor {
    /// ID unívoco do snapshot.
    pub snapshot_id: u64,
    /// Sequence number temporal no momento da abertura do snapshot.
    pub snapshot_seq: u64,
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

            // Para cada versão no grupo, verifica se precisa ser retida
            for record in key_group {
                if record.is_tombstone && is_bottommost_level {
                    if Self::can_purge_tombstone(record.seq, live_snapshots) {
                        // Purga segura do tombstone no nível mais baixo
                        continue;
                    }
                }
                compacted.push(record);
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
