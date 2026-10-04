//! RFC-0327: Causal History Bisimulation Kernel.
//!
//! Provides formal verification of transaction causal closure and order preservation
//! across crash-reboot boundaries (RFC-0278, RFC-0327). Ensures that if transaction T2
//! causally depends on T1, recovery can never recover T2 without T1.

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::fmt;

/// Errors arising from causal closure or ordering violations during recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CausalClosureViolation {
    /// A recovered transaction is missing one or more of its causal predecessors.
    MissingCausalPredecessor {
        tx_id: u64,
        missing_predecessor_id: u64,
    },
    /// Causal order inverted: predecessor sequence is greater than or equal to dependent sequence.
    CausalSequenceInversion {
        predecessor_seq: u64,
        dependent_seq: u64,
    },
    /// Transaction identifier cannot be zero.
    ZeroTransactionIdHazard,
    /// Duplicate transaction identifier in causal DAG.
    DuplicateTransactionId(u64),
}

impl fmt::Display for CausalClosureViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCausalPredecessor { tx_id, missing_predecessor_id } => {
                write!(
                    f,
                    "Causal closure broken: tx {tx_id} was recovered, but its causal predecessor {missing_predecessor_id} is missing"
                )
            }
            Self::CausalSequenceInversion { predecessor_seq, dependent_seq } => {
                write!(
                    f,
                    "Causal sequence inversion: predecessor seq {predecessor_seq} >= dependent seq {dependent_seq}"
                )
            }
            Self::ZeroTransactionIdHazard => write!(f, "Transaction ID cannot be zero in causal graph"),
            Self::DuplicateTransactionId(id) => write!(f, "Duplicate transaction ID {id} in causal DAG"),
        }
    }
}

impl std::error::Error for CausalClosureViolation {}

/// Descriptor of a transaction within the causal DAG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CausalTxNode {
    pub tx_id: u64,
    pub seq: u64,
    pub dependencies: Vec<u64>,
}

/// Verifier of causal bisimulation and closure across recovery.
#[derive(Debug, Clone, Default)]
pub struct CausalHistoryBisimulationJudge {
    nodes: HashMap<u64, CausalTxNode>,
}

impl CausalHistoryBisimulationJudge {
    /// Creates a new empty causal history judge.
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
        }
    }

    /// Records a committed transaction and its explicit causal predecessor IDs.
    pub fn try_register_tx(
        &mut self,
        tx_id: u64,
        seq: u64,
        dependencies: Vec<u64>,
    ) -> Result<(), CausalClosureViolation> {
        if tx_id == 0 {
            return Err(CausalClosureViolation::ZeroTransactionIdHazard);
        }
        if self.nodes.contains_key(&tx_id) {
            return Err(CausalClosureViolation::DuplicateTransactionId(tx_id));
        }

        // Verify sequence ordering against all recorded predecessors
        for &dep_id in &dependencies {
            if let Some(pred) = self.nodes.get(&dep_id) {
                if pred.seq >= seq {
                    return Err(CausalClosureViolation::CausalSequenceInversion {
                        predecessor_seq: pred.seq,
                        dependent_seq: seq,
                    });
                }
            }
        }

        self.nodes.insert(
            tx_id,
            CausalTxNode {
                tx_id,
                seq,
                dependencies,
            },
        );
        Ok(())
    }

    /// Verifies that a set of recovered transaction IDs satisfies Causal Closure ($T_{\text{causal}}$).
    ///
    /// Every predecessor of every recovered transaction must also be present in `recovered_ids`.
    pub fn verify_causal_closure(
        &self,
        recovered_ids: &[u64],
    ) -> Result<(), CausalClosureViolation> {
        let set: HashSet<u64> = recovered_ids.iter().copied().collect();

        for &id in recovered_ids {
            if let Some(node) = self.nodes.get(&id) {
                for &pred_id in &node.dependencies {
                    if !set.contains(&pred_id) {
                        return Err(CausalClosureViolation::MissingCausalPredecessor {
                            tx_id: id,
                            missing_predecessor_id: pred_id,
                        });
                    }
                }
            }
        }

        Ok(())
    }

    /// Total registered transaction count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the judge is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}
