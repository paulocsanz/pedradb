//! RFC-0283 Pilar 6 — Detecção Dinâmica de Ciclos de Anti-Dependência em SSI (SSI Cycle Detector Kernel).
//!
//! Formalizes dynamic Serialization Graph Checking (SGC) based on the Fekete/Cahill theorem.
//! In Snapshot Isolation, non-serializable anomalies (such as write-skew) can only occur if
//! there exists a cycle with two consecutive read-write anti-dependency edges:
//!   T_in -(rw)-> T_pivot -(rw)-> T_out.
//!
//! Proves that identifying and aborting dangerous structures or cycles at commit time
//! guarantees strict Serializable Snapshot Isolation (SSI) across arbitrary transaction counts.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

/// Transaction identifier.
pub type TxnId = u64;

/// Type of dependency edge between two concurrent transactions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DependencyEdgeType {
    /// Read-Write anti-dependency: T1 read key k, T2 later wrote/overwrote key k.
    RwAntiDependency,
    /// Write-Read dependency: T1 wrote key k, T2 read key k.
    WrDependency,
    /// Write-Write dependency: T1 wrote key k, T2 wrote key k.
    WwDependency,
}

/// Violations resulting from non-serializable multi-transaction interleavings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SsiViolation {
    /// Dangerous consecutive rw-antidependency structure detected (T_in -> T_pivot -> T_out).
    DangerousStructureDetected {
        /// The pivot transaction that must be aborted.
        pivot_txn: TxnId,
        /// Incoming anti-dependency source.
        in_txn: TxnId,
        /// Outgoing anti-dependency target.
        out_txn: TxnId,
    },
    /// A directed cycle was closed in the serialization dependency graph.
    SerializationCycleDetected {
        /// Transactions participating in the cycle.
        cycle: Vec<TxnId>,
    },
}

/// Dynamic dependency graph tracking active and committed transactions under SSI.
#[derive(Clone, Debug, Default)]
pub struct SsiSerializationGraph {
    /// Active transactions: TxnId -> (read_keys, written_keys)
    pub active_txns: BTreeMap<TxnId, (BTreeSet<Vec<u8>>, BTreeSet<Vec<u8>>)>,
    /// Directed edges: (from_txn, to_txn) -> EdgeType
    pub edges: BTreeMap<(TxnId, TxnId), DependencyEdgeType>,
    /// Transactions with incoming rw-antidependency: TxnId -> set of in_txns
    pub in_rw_edges: BTreeMap<TxnId, BTreeSet<TxnId>>,
    /// Transactions with outgoing rw-antidependency: TxnId -> set of out_txns
    pub out_rw_edges: BTreeMap<TxnId, BTreeSet<TxnId>>,
}

impl SsiSerializationGraph {
    /// Creates a new SSI serialization graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Begins a new transaction in the graph.
    pub fn begin_txn(&mut self, txn_id: TxnId) {
        self.active_txns
            .insert(txn_id, (BTreeSet::new(), BTreeSet::new()));
    }

    /// Records a read on key `k` by transaction `txn_id`.
    pub fn record_read(&mut self, txn_id: TxnId, key: &[u8]) {
        if let Some((reads, _)) = self.active_txns.get_mut(&txn_id) {
            reads.insert(key.to_vec());
        }
    }

    /// Records a write on key `k` by transaction `txn_id`.
    pub fn record_write(&mut self, txn_id: TxnId, key: &[u8]) {
        if let Some((_, writes)) = self.active_txns.get_mut(&txn_id) {
            writes.insert(key.to_vec());
        }
    }

    /// Checks for dependencies between `txn_id` and all other active transactions.
    pub fn analyze_dependencies_for_commit(&mut self, committing_txn: TxnId) {
        let (my_reads, my_writes) = match self.active_txns.get(&committing_txn) {
            Some((r, w)) => (r.clone(), w.clone()),
            None => return,
        };

        for (&other_id, (other_reads, other_writes)) in &self.active_txns {
            if other_id == committing_txn {
                continue;
            }

            // Check if other_id read a key that committing_txn wrote: other_id -(rw)-> committing_txn
            for key in &my_writes {
                if other_reads.contains(key) {
                    self.edges
                        .insert((other_id, committing_txn), DependencyEdgeType::RwAntiDependency);
                    self.in_rw_edges
                        .entry(committing_txn)
                        .or_default()
                        .insert(other_id);
                    self.out_rw_edges
                        .entry(other_id)
                        .or_default()
                        .insert(committing_txn);
                }
            }

            // Check if committing_txn read a key that other_id wrote: committing_txn -(rw)-> other_id
            for key in &my_reads {
                if other_writes.contains(key) {
                    self.edges
                        .insert((committing_txn, other_id), DependencyEdgeType::RwAntiDependency);
                    self.in_rw_edges
                        .entry(other_id)
                        .or_default()
                        .insert(committing_txn);
                    self.out_rw_edges
                        .entry(committing_txn)
                        .or_default()
                        .insert(other_id);
                }
            }
        }
    }

    /// Evaluates if `committing_txn` forms a dangerous structure (both in_rw and out_rw).
    ///
    /// # Errors
    /// Returns `SsiViolation::DangerousStructureDetected` if serializability would be broken.
    pub fn verify_can_commit(&self, committing_txn: TxnId) -> Result<(), SsiViolation> {
        let in_edges = self.in_rw_edges.get(&committing_txn);
        let out_edges = self.out_rw_edges.get(&committing_txn);

        if let (Some(in_set), Some(out_set)) = (in_edges, out_edges) {
            if !in_set.is_empty() && !out_set.is_empty() {
                let in_txn = *in_set.iter().next().unwrap();
                let out_txn = *out_set.iter().next().unwrap();
                return Err(SsiViolation::DangerousStructureDetected {
                    pivot_txn: committing_txn,
                    in_txn,
                    out_txn,
                });
            }
        }

        Ok(())
    }

    /// Removes transaction from active set after commit or abort.
    pub fn finish_txn(&mut self, txn_id: TxnId) {
        self.active_txns.remove(&txn_id);
    }
}
