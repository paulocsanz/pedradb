//! RFC-0292: Pilar 3 - Bisimulação Causal Trans-Crash ($H_{\text{pre}} \sim_C H_{\text{post}}$).
//!
//! Formaliza e verifica a preservação estrita do poset de visibilidade e durabilidade entre o estado
//! do motor no microssegundo anterior ao corte súbito de energia e o estado reconstruído pós-reboot.

use std::collections::{HashMap, HashSet};

/// Violações da bisimulação causal trans-crash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransCrashViolation {
    /// Uma transação confirmada (ACKed) ao cliente pré-crash desapareceu após o reboot.
    AcknowledgedTransactionLost {
        /// ID da transação perdida.
        tx_id: u64,
        /// Sequence number atribuído pré-crash.
        seq: u64,
    },
    /// A ordem causal relativa entre duas transações foi invertida após a recuperação.
    CausalOrderInverted {
        /// Transação que precedia causalmente no histórico pré-crash.
        tx_prior: u64,
        /// Transação subsequente no histórico pré-crash.
        tx_subsequent: u64,
        /// Sequência pós-crash da transação anterior.
        seq_prior_post: u64,
        /// Sequência pós-crash da transação subsequente.
        seq_subsequent_post: u64,
    },
    /// Uma transação não persistida (un-synced) foi ressuscitada com sequence number inválido.
    UnpersistedTornWriteResurrected {
        /// ID da transação corrompida.
        tx_id: u64,
    },
    ZeroTransactionId,
    ZeroSequenceNumber,
    EmptyKeys,
    EmptyKey,
    DuplicateTransactionId(u64),
    SequenceRegression {
        prev_seq: u64,
        new_seq: u64,
    },
}

impl std::fmt::Display for TransCrashViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AcknowledgedTransactionLost { tx_id, seq } => {
                write!(f, "Acknowledged transaction {tx_id} (seq {seq}) was lost across crash reboot")
            }
            Self::CausalOrderInverted { tx_prior, tx_subsequent, seq_prior_post, seq_subsequent_post } => {
                write!(
                    f,
                    "Causal order inverted between tx {tx_prior} and {tx_subsequent}: post sequences {seq_prior_post} >= {seq_subsequent_post}"
                )
            }
            Self::UnpersistedTornWriteResurrected { tx_id } => {
                write!(f, "Unpersisted or torn transaction {tx_id} resurrected post-reboot")
            }
            Self::ZeroTransactionId => write!(f, "Transaction ID cannot be zero"),
            Self::ZeroSequenceNumber => write!(f, "Sequence number cannot be zero"),
            Self::EmptyKeys => write!(f, "Transaction key set cannot be empty"),
            Self::EmptyKey => write!(f, "Individual transaction key cannot be empty"),
            Self::DuplicateTransactionId(tx_id) => write!(f, "Duplicate transaction ID {tx_id} in causal history"),
            Self::SequenceRegression { prev_seq, new_seq } => {
                write!(f, "Sequence regression in causal history: {prev_seq} -> {new_seq}")
            }
        }
    }
}

impl std::error::Error for TransCrashViolation {}

/// Registro de evento transacional no motor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionRecord {
    /// Identificador único da transação.
    pub tx_id: u64,
    /// Sequence number monotônico global.
    pub seq: u64,
    /// Chaves mutadas pela transação.
    pub keys: Vec<Vec<u8>>,
    /// Flag indicando se a transação recebeu fsync e foi confirmada (ACKed) ao cliente.
    pub is_fsynced_and_acknowledged: bool,
}

impl TransactionRecord {
    /// Safely constructs a TransactionRecord, rejecting zero IDs, zero sequences, and empty key sets.
    pub fn try_new(
        tx_id: u64,
        seq: u64,
        keys: Vec<Vec<u8>>,
        is_fsynced_and_acknowledged: bool,
    ) -> Result<Self, TransCrashViolation> {
        if tx_id == 0 {
            return Err(TransCrashViolation::ZeroTransactionId);
        }
        if seq == 0 {
            return Err(TransCrashViolation::ZeroSequenceNumber);
        }
        if keys.is_empty() {
            return Err(TransCrashViolation::EmptyKeys);
        }
        for k in &keys {
            if k.is_empty() {
                return Err(TransCrashViolation::EmptyKey);
            }
        }
        Ok(Self {
            tx_id,
            seq,
            keys,
            is_fsynced_and_acknowledged,
        })
    }
}

/// Poset de histórico causal de execução.
#[derive(Debug, Clone, Default)]
pub struct CausalHistoryPoset {
    /// Transações ordenadas por sequência.
    pub records: Vec<TransactionRecord>,
    /// Relações de precedência causal direta: tx_A -> HashSet<tx_B> (onde tx_A < tx_B).
    pub direct_causal_edges: HashMap<u64, HashSet<u64>>,
}

impl CausalHistoryPoset {
    /// Adiciona um registro ao histórico de forma segura validando monotonicidade e unicidade de ID.
    pub fn try_record_transaction(&mut self, record: TransactionRecord) -> Result<(), TransCrashViolation> {
        if record.tx_id == 0 {
            return Err(TransCrashViolation::ZeroTransactionId);
        }
        if record.seq == 0 {
            return Err(TransCrashViolation::ZeroSequenceNumber);
        }
        if record.keys.is_empty() {
            return Err(TransCrashViolation::EmptyKeys);
        }
        if self.records.iter().any(|r| r.tx_id == record.tx_id) {
            return Err(TransCrashViolation::DuplicateTransactionId(record.tx_id));
        }
        if let Some(last) = self.records.last() {
            if record.seq <= last.seq {
                return Err(TransCrashViolation::SequenceRegression {
                    prev_seq: last.seq,
                    new_seq: record.seq,
                });
            }
        }
        self.record_transaction(record);
        Ok(())
    }

    /// Adiciona um registro ao histórico e atualiza as arestas causais de dependência de chaves.
    pub fn record_transaction(&mut self, record: TransactionRecord) {
        let tx_id = record.tx_id;
        let keys_set: HashSet<_> = record.keys.iter().collect();

        // Encontra transações anteriores que tocaram nas mesmas chaves
        let mut parents = HashSet::new();
        for prev in &self.records {
            if prev.keys.iter().any(|k| keys_set.contains(k)) {
                parents.insert(prev.tx_id);
            }
        }

        for parent in parents {
            self.direct_causal_edges
                .entry(parent)
                .or_default()
                .insert(tx_id);
        }

        self.records.push(record);
    }
}


/// Oráculo de verificação de bisimulação causal trans-crash.
pub struct TransCrashBisimulationOracle;

impl TransCrashBisimulationOracle {
    /// Valida formalmente a bisimulação causal entre o histórico pré-crash e pós-crash:
    /// 1. E_ack \subseteq E_post (zero perda de transações confirmadas).
    /// 2. E_post \subseteq E_pre (zero ressurreição de escritas rasgadas/fantasmas).
    /// 3. \forall e_1, e_2 \in E_ack: e_1 <_pre e_2 \iff e_1 <_post e_2 (preservação estrita de ordem).
    pub fn verify_trans_crash_bisimulation(
        pre_crash: &CausalHistoryPoset,
        post_crash: &CausalHistoryPoset,
    ) -> Result<(), TransCrashViolation> {
        let pre_map: HashMap<u64, &TransactionRecord> = pre_crash
            .records
            .iter()
            .map(|r| (r.tx_id, r))
            .collect();

        let post_map: HashMap<u64, &TransactionRecord> = post_crash
            .records
            .iter()
            .map(|r| (r.tx_id, r))
            .collect();

        // 1. Prova de contenção E_post \subseteq E_pre: nenhuma transação fantasma ou torn-write não catalogada
        for post_r in &post_crash.records {
            match pre_map.get(&post_r.tx_id) {
                None => {
                    return Err(TransCrashViolation::UnpersistedTornWriteResurrected {
                        tx_id: post_r.tx_id,
                    });
                }
                Some(pre_r) => {
                    // Se a transação não foi fsynced no pré-crash mas sobreviveu, deve ser causalmente idêntica
                    if !pre_r.is_fsynced_and_acknowledged
                        && (post_r.seq != pre_r.seq || post_r.keys != pre_r.keys)
                    {
                        return Err(TransCrashViolation::UnpersistedTornWriteResurrected {
                            tx_id: post_r.tx_id,
                        });
                    }
                }
            }
        }

        // 2. Prova de monotonicidade estrita interna do histórico recuperado pós-crash
        for i in 1..post_crash.records.len() {
            let prev = &post_crash.records[i - 1];
            let curr = &post_crash.records[i];
            if prev.seq >= curr.seq {
                return Err(TransCrashViolation::CausalOrderInverted {
                    tx_prior: prev.tx_id,
                    tx_subsequent: curr.tx_id,
                    seq_prior_post: prev.seq,
                    seq_subsequent_post: curr.seq,
                });
            }
        }

        // Subconjunto de transações confirmadas pré-crash
        let acked_pre: Vec<&TransactionRecord> = pre_crash
            .records
            .iter()
            .filter(|r| r.is_fsynced_and_acknowledged)
            .collect();

        // 3. Prova de preservação: todas as transações ACKed devem existir pós-crash
        for acked in &acked_pre {
            if !post_map.contains_key(&acked.tx_id) {
                return Err(TransCrashViolation::AcknowledgedTransactionLost {
                    tx_id: acked.tx_id,
                    seq: acked.seq,
                });
            }
        }

        // 4. Prova de preservação da ordem causal relativa entre transações ACKed
        for i in 0..acked_pre.len() {
            for j in (i + 1)..acked_pre.len() {
                let tx_a = acked_pre[i];
                let tx_b = acked_pre[j];

                let post_a = post_map[&tx_a.tx_id];
                let post_b = post_map[&tx_b.tx_id];

                // No pré-crash: tx_a.seq < tx_b.seq
                if tx_a.seq < tx_b.seq && post_a.seq >= post_b.seq {
                    return Err(TransCrashViolation::CausalOrderInverted {
                        tx_prior: tx_a.tx_id,
                        tx_subsequent: tx_b.tx_id,
                        seq_prior_post: post_a.seq,
                        seq_subsequent_post: post_b.seq,
                    });
                }
            }
        }

        // 5. Prova de preservação de dependências causais diretas
        for (&parent_id, children) in &pre_crash.direct_causal_edges {
            if let Some(post_parent) = post_map.get(&parent_id) {
                for &child_id in children {
                    if let Some(post_child) = post_map.get(&child_id) {
                        if post_parent.seq >= post_child.seq {
                            return Err(TransCrashViolation::CausalOrderInverted {
                                tx_prior: parent_id,
                                tx_subsequent: child_id,
                                seq_prior_post: post_parent.seq,
                                seq_subsequent_post: post_child.seq,
                            });
                        }
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
    fn test_transaction_record_try_new_red_to_green() {
        assert_eq!(
            TransactionRecord::try_new(0, 10, vec![b"k1".to_vec()], true),
            Err(TransCrashViolation::ZeroTransactionId)
        );
        assert_eq!(
            TransactionRecord::try_new(1, 0, vec![b"k1".to_vec()], true),
            Err(TransCrashViolation::ZeroSequenceNumber)
        );
        assert_eq!(
            TransactionRecord::try_new(1, 10, vec![], true),
            Err(TransCrashViolation::EmptyKeys)
        );
        assert_eq!(
            TransactionRecord::try_new(1, 10, vec![vec![]], true),
            Err(TransCrashViolation::EmptyKey)
        );
        let rec = TransactionRecord::try_new(1, 10, vec![b"k1".to_vec()], true).unwrap();
        assert_eq!(rec.tx_id, 1);
    }

    #[test]
    fn test_causal_history_try_record_duplicate_and_regression() {
        let mut history = CausalHistoryPoset::default();
        let r1 = TransactionRecord::try_new(1, 10, vec![b"k1".to_vec()], true).unwrap();
        assert!(history.try_record_transaction(r1.clone()).is_ok());

        // Duplicate tx_id
        assert_eq!(
            history.try_record_transaction(r1),
            Err(TransCrashViolation::DuplicateTransactionId(1))
        );

        // Sequence regression
        let r2 = TransactionRecord::try_new(2, 5, vec![b"k2".to_vec()], true).unwrap();
        assert_eq!(
            history.try_record_transaction(r2),
            Err(TransCrashViolation::SequenceRegression { prev_seq: 10, new_seq: 5 })
        );
    }
}

