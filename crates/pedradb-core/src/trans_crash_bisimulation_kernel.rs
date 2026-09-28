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
}

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

/// Poset de histórico causal de execução.
#[derive(Debug, Clone, Default)]
pub struct CausalHistoryPoset {
    /// Transações ordenadas por sequência.
    pub records: Vec<TransactionRecord>,
    /// Relações de precedência causal direta: tx_A -> HashSet<tx_B> (onde tx_A < tx_B).
    pub direct_causal_edges: HashMap<u64, HashSet<u64>>,
}

impl CausalHistoryPoset {
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
    /// 2. \forall e_1, e_2 \in E_ack: e_1 <_pre e_2 \iff e_1 <_post e_2 (preservação estrita de ordem).
    pub fn verify_trans_crash_bisimulation(
        pre_crash: &CausalHistoryPoset,
        post_crash: &CausalHistoryPoset,
    ) -> Result<(), TransCrashViolation> {
        let post_map: HashMap<u64, &TransactionRecord> = post_crash
            .records
            .iter()
            .map(|r| (r.tx_id, r))
            .collect();

        // Subconjunto de transações confirmadas pré-crash
        let acked_pre: Vec<&TransactionRecord> = pre_crash
            .records
            .iter()
            .filter(|r| r.is_fsynced_and_acknowledged)
            .collect();

        // 1. Prova de preservação: todas as transações ACKed devem existir pós-crash
        for acked in &acked_pre {
            if !post_map.contains_key(&acked.tx_id) {
                return Err(TransCrashViolation::AcknowledgedTransactionLost {
                    tx_id: acked.tx_id,
                    seq: acked.seq,
                });
            }
        }

        // 2. Prova de preservação da ordem causal relativa entre transações ACKed
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

        // 3. Prova de preservação de dependências causais diretas
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
