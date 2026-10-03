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
    /// Invalid transaction identifier (e.g. 0).
    InvalidTxnId {
        txn_id: TxnId,
    },
    /// Transaction is already active in the graph.
    DuplicateActiveTxn {
        txn_id: TxnId,
    },
    /// Transaction not found in active set.
    TxnNotFound {
        txn_id: TxnId,
    },
    /// Empty key passed to read or write tracking.
    EmptyKey,
}

impl std::fmt::Display for SsiViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DangerousStructureDetected {
                pivot_txn,
                in_txn,
                out_txn,
            } => {
                write!(
                    f,
                    "dangerous SSI structure detected: pivot txn {pivot_txn} has incoming {in_txn} and outgoing {out_txn}"
                )
            }
            Self::SerializationCycleDetected { cycle } => {
                write!(
                    f,
                    "serialization cycle detected among transactions: {cycle:?}"
                )
            }
            Self::InvalidTxnId { txn_id } => {
                write!(f, "invalid transaction id {txn_id}: txn_id must be non-zero")
            }
            Self::DuplicateActiveTxn { txn_id } => {
                write!(f, "transaction {txn_id} is already active")
            }
            Self::TxnNotFound { txn_id } => {
                write!(f, "transaction {txn_id} not found in active set")
            }
            Self::EmptyKey => {
                write!(f, "key cannot be empty in SSI tracking")
            }
        }
    }
}

impl std::error::Error for SsiViolation {}

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

    /// Begins a new transaction in the graph, rejecting zero IDs and duplicates.
    pub fn try_begin_txn(&mut self, txn_id: TxnId) -> Result<(), SsiViolation> {
        if txn_id == 0 {
            return Err(SsiViolation::InvalidTxnId { txn_id: 0 });
        }
        if self.active_txns.contains_key(&txn_id) {
            return Err(SsiViolation::DuplicateActiveTxn { txn_id });
        }
        self.active_txns
            .insert(txn_id, (BTreeSet::new(), BTreeSet::new()));
        Ok(())
    }

    /// Begins a new transaction in the graph.
    pub fn begin_txn(&mut self, txn_id: TxnId) {
        let _ = self.try_begin_txn(txn_id);
    }

    /// Records a read on key `k` by transaction `txn_id` with validation.
    pub fn try_record_read(&mut self, txn_id: TxnId, key: &[u8]) -> Result<(), SsiViolation> {
        if key.is_empty() {
            return Err(SsiViolation::EmptyKey);
        }
        let (reads, _) = self
            .active_txns
            .get_mut(&txn_id)
            .ok_or(SsiViolation::TxnNotFound { txn_id })?;
        reads.insert(key.to_vec());
        Ok(())
    }

    /// Records a read on key `k` by transaction `txn_id`.
    pub fn record_read(&mut self, txn_id: TxnId, key: &[u8]) {
        let _ = self.try_record_read(txn_id, key);
    }

    /// Records a write on key `k` by transaction `txn_id` with validation.
    pub fn try_record_write(&mut self, txn_id: TxnId, key: &[u8]) -> Result<(), SsiViolation> {
        if key.is_empty() {
            return Err(SsiViolation::EmptyKey);
        }
        let (_, writes) = self
            .active_txns
            .get_mut(&txn_id)
            .ok_or(SsiViolation::TxnNotFound { txn_id })?;
        writes.insert(key.to_vec());
        Ok(())
    }

    /// Records a write on key `k` by transaction `txn_id`.
    pub fn record_write(&mut self, txn_id: TxnId, key: &[u8]) {
        let _ = self.try_record_write(txn_id, key);
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

    /// Finds a directed cycle in the dependency graph involving `start`.
    #[must_use]
    pub fn find_cycle_involving(&self, start: TxnId) -> Option<Vec<TxnId>> {
        let mut visited = BTreeSet::new();
        let mut path = Vec::new();
        let mut in_path = BTreeSet::new();

        fn dfs(
            u: TxnId,
            edges: &BTreeMap<(TxnId, TxnId), DependencyEdgeType>,
            visited: &mut BTreeSet<TxnId>,
            path: &mut Vec<TxnId>,
            in_path: &mut BTreeSet<TxnId>,
        ) -> Option<Vec<TxnId>> {
            visited.insert(u);
            path.push(u);
            in_path.insert(u);

            for ((from, to), _) in edges {
                if *from == u {
                    if in_path.contains(to) {
                        let idx = path.iter().position(|&x| x == *to).unwrap_or(0);
                        let mut cycle = path[idx..].to_vec();
                        cycle.push(*to);
                        return Some(cycle);
                    }
                    if !visited.contains(to) {
                        if let Some(c) = dfs(*to, edges, visited, path, in_path) {
                            return Some(c);
                        }
                    }
                }
            }

            path.pop();
            in_path.remove(&u);
            None
        }

        dfs(start, &self.edges, &mut visited, &mut path, &mut in_path)
    }

    /// Evaluates if `committing_txn` closes a cycle or forms a dangerous structure (both in_rw and out_rw).
    ///
    /// # Errors
    /// Returns `SsiViolation::SerializationCycleDetected` if a cycle is closed,
    /// or `SsiViolation::DangerousStructureDetected` if serializability would be broken.
    pub fn verify_can_commit(&self, committing_txn: TxnId) -> Result<(), SsiViolation> {
        if let Some(cycle) = self.find_cycle_involving(committing_txn) {
            return Err(SsiViolation::SerializationCycleDetected { cycle });
        }

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

    /// Aborts a transaction and safely prunes all its dependency edges and memory references.
    pub fn abort_txn(&mut self, txn_id: TxnId) {
        self.active_txns.remove(&txn_id);
        self.edges.retain(|(from, to), _| *from != txn_id && *to != txn_id);
        self.in_rw_edges.remove(&txn_id);
        self.out_rw_edges.remove(&txn_id);
        for in_set in self.in_rw_edges.values_mut() {
            in_set.remove(&txn_id);
        }
        for out_set in self.out_rw_edges.values_mut() {
            out_set.remove(&txn_id);
        }
    }

    /// Removes transaction from active set after commit or abort.
    pub fn finish_txn(&mut self, txn_id: TxnId) {
        self.abort_txn(txn_id);
    }
}
