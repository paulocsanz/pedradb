//! Asymmetric Partition Quorum and Bipartite Confluence Kernel (RFC-0285 Pilar 7).
//!
//! Enforces that consensus votes and quorum certificates require proven mutual
//! reachability (bidirectional connectivity). Asymmetric (directed / half-open) network
//! partitions are explicitly detected and filtered out to prevent split-brain.
//!
//! Axiom:
//! A quorum member pair (i, j) is admitted iff:
//! CanSend(i -> j) AND CanSend(j -> i) == true.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

/// Quorum error under asymmetric or partitioned network topology.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsymmetricQuorumError {
    /// Insufficient bidirectional votes to reach strict majority.
    MajorityNotReached { valid_votes: usize, required: usize },
    /// Asymmetric directed link detected between node and proposed voter.
    AsymmetricLinkDetected { from_node: u64, to_node: u64 },
    /// Split brain vulnerability detected (multiple disjoint majorities possible).
    SplitBrainRisk { cluster_size: usize, active_group: usize },
    /// Cluster node set cannot be empty.
    EmptyCluster,
    /// Local node ID is not a member of the cluster.
    LocalNodeNotInCluster { node_id: u64 },
    /// Self-directed link is disallowed in directional matrix.
    SelfLinkDisallowed { node_id: u64 },
    /// Node is not part of the configured cluster topology.
    UnknownClusterNode { node_id: u64 },
}

impl std::fmt::Display for AsymmetricQuorumError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MajorityNotReached { valid_votes, required } => write!(
                f,
                "Majority quorum not reached: valid votes {valid_votes} < required {required}"
            ),
            Self::AsymmetricLinkDetected { from_node, to_node } => write!(
                f,
                "Asymmetric directed link detected between {from_node} and {to_node}"
            ),
            Self::SplitBrainRisk { cluster_size, active_group } => write!(
                f,
                "Split brain risk: cluster size {cluster_size}, active group {active_group}"
            ),
            Self::EmptyCluster => write!(f, "Cluster node set cannot be empty"),
            Self::LocalNodeNotInCluster { node_id } => write!(
                f,
                "Local node {node_id} is not a member of the configured cluster"
            ),
            Self::SelfLinkDisallowed { node_id } => write!(
                f,
                "Self-directed link observation disallowed for node {node_id}"
            ),
            Self::UnknownClusterNode { node_id } => write!(
                f,
                "Node {node_id} is not part of the configured cluster topology"
            ),
        }
    }
}

impl std::error::Error for AsymmetricQuorumError {}

/// Link health state between two nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectedLinkState {
    /// Whether node A can send to node B.
    pub a_to_b: bool,
    /// Whether node B can send to node A.
    pub b_to_a: bool,
    /// Round-trip time in milliseconds.
    pub rtt_ms: u32,
}

impl DirectedLinkState {
    /// Whether link is fully symmetrical and healthy.
    pub fn is_bidirectional(&self) -> bool {
        self.a_to_b && self.b_to_a
    }
}

/// Mesh topology coordinator for strict quorum evaluation.
#[derive(Debug, PartialEq, Eq)]
pub struct AsymmetricQuorumGuard {
    local_node_id: u64,
    cluster_nodes: BTreeSet<u64>,
    link_matrix: BTreeMap<(u64, u64), DirectedLinkState>,
}

impl AsymmetricQuorumGuard {
    /// Creates a quorum guard with strict bounds validation.
    pub fn try_new(local_node_id: u64, nodes: impl IntoIterator<Item = u64>) -> Result<Self, AsymmetricQuorumError> {
        let cluster_nodes: BTreeSet<u64> = nodes.into_iter().collect();
        if cluster_nodes.is_empty() {
            return Err(AsymmetricQuorumError::EmptyCluster);
        }
        if !cluster_nodes.contains(&local_node_id) {
            return Err(AsymmetricQuorumError::LocalNodeNotInCluster { node_id: local_node_id });
        }
        Ok(Self {
            local_node_id,
            cluster_nodes,
            link_matrix: BTreeMap::new(),
        })
    }

    /// Creates a quorum guard for a known cluster topology.
    pub fn new(local_node_id: u64, nodes: impl IntoIterator<Item = u64>) -> Self {
        let mut cluster_nodes: BTreeSet<u64> = nodes.into_iter().collect();
        cluster_nodes.insert(local_node_id);
        Self {
            local_node_id,
            cluster_nodes,
            link_matrix: BTreeMap::new(),
        }
    }

    /// Records connectivity observation between two nodes with membership check.
    pub fn try_record_directed_link(
        &mut self,
        from: u64,
        to: u64,
        can_send: bool,
        rtt_ms: u32,
    ) -> Result<(), AsymmetricQuorumError> {
        if from == to {
            return Err(AsymmetricQuorumError::SelfLinkDisallowed { node_id: from });
        }
        if !self.cluster_nodes.contains(&from) {
            return Err(AsymmetricQuorumError::UnknownClusterNode { node_id: from });
        }
        if !self.cluster_nodes.contains(&to) {
            return Err(AsymmetricQuorumError::UnknownClusterNode { node_id: to });
        }
        self.record_directed_link(from, to, can_send, rtt_ms);
        Ok(())
    }

    /// Records connectivity observation between two nodes.
    pub fn record_directed_link(&mut self, from: u64, to: u64, can_send: bool, rtt_ms: u32) {
        let key = if from < to { (from, to) } else { (to, from) };
        let entry = self.link_matrix.entry(key).or_insert(DirectedLinkState {
            a_to_b: false,
            b_to_a: false,
            rtt_ms,
        });

        if from < to {
            entry.a_to_b = can_send;
        } else {
            entry.b_to_a = can_send;
        }
        entry.rtt_ms = rtt_ms;
    }

    /// Evaluates if a set of voting nodes forms a valid majority quorum.
    /// Strictly filters out any node that lacks proven bidirectional connectivity.
    pub fn evaluate_quorum(
        &self,
        voters: &[u64],
    ) -> Result<BTreeSet<u64>, AsymmetricQuorumError> {
        let total_nodes = self.cluster_nodes.len();
        let required_majority = (total_nodes / 2) + 1;

        let mut validated_voters = BTreeSet::new();

        // Local node always self-connected if it is a cluster member
        if self.cluster_nodes.contains(&self.local_node_id) && voters.contains(&self.local_node_id) {
            validated_voters.insert(self.local_node_id);
        }

        for &voter in voters {
            if voter == self.local_node_id {
                continue;
            }

            if !self.cluster_nodes.contains(&voter) {
                continue;
            }

            let key = if self.local_node_id < voter {
                (self.local_node_id, voter)
            } else {
                (voter, self.local_node_id)
            };

            if let Some(link) = self.link_matrix.get(&key) {
                if link.is_bidirectional() {
                    validated_voters.insert(voter);
                } else {
                    return Err(AsymmetricQuorumError::AsymmetricLinkDetected {
                        from_node: self.local_node_id,
                        to_node: voter,
                    });
                }
            } else {
                // Link state unknown; reject unverified voter
                continue;
            }
        }

        if validated_voters.len() < required_majority {
            return Err(AsymmetricQuorumError::MajorityNotReached {
                valid_votes: validated_voters.len(),
                required: required_majority,
            });
        }

        Ok(validated_voters)
    }

    /// Cluster node count.
    pub fn cluster_size(&self) -> usize {
        self.cluster_nodes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asymmetric_quorum_fail_closed_red_to_green() {
        assert_eq!(
            AsymmetricQuorumGuard::try_new(1, vec![]),
            Err(AsymmetricQuorumError::EmptyCluster)
        );
        assert_eq!(
            AsymmetricQuorumGuard::try_new(99, vec![1, 2, 3]),
            Err(AsymmetricQuorumError::LocalNodeNotInCluster { node_id: 99 })
        );

        let mut guard = AsymmetricQuorumGuard::try_new(1, vec![1, 2, 3]).expect("valid guard");
        assert_eq!(
            guard.try_record_directed_link(1, 1, true, 5),
            Err(AsymmetricQuorumError::SelfLinkDisallowed { node_id: 1 })
        );
        assert_eq!(
            guard.try_record_directed_link(1, 42, true, 5),
            Err(AsymmetricQuorumError::UnknownClusterNode { node_id: 42 })
        );
    }
}
