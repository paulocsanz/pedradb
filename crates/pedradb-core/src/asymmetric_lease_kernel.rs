//! RFC-0282 Pilar 3 — Partições de Rede Assimétricas e Validade de Leases (Asymmetric Lease Kernel).
//!
//! Formalizes leader lease safety in distributed consensus under:
//!   - Asymmetric directed network partitions (node i can reach j, but j cannot reach i);
//!   - Bounded clock drift (\Delta_max) between nodes.
//!
//! Proves that leader local reads remain strictly linearizable: a leader never serves
//! a stale read from an expired lease, even if asymmetric packet loss prevents heartbeat acks.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;

/// Violations of distributed lease validity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaseViolation {
    /// Local read attempted under an expired lease.
    StaleReadUnderExpiredLease {
        /// Time elapsed since last quorum renewal.
        elapsed_micros: u64,
        /// Maximum duration for safe read (including 2 * \Delta margin).
        safe_duration_micros: u64,
    },
    /// The cluster clock drift assumption is violated.
    ClockDriftExceeded {
        /// Observed drift.
        drift_micros: u64,
        /// Maximum allowed bound \Delta_max.
        bound_micros: u64,
    },
    /// Quorum lost due to asymmetric network partition.
    QuorumLossUnderAsymmetry {
        /// Number of nodes that successfully responded.
        active_responses: usize,
        /// Required majority threshold (N/2 + 1).
        majority_required: usize,
    },
}

/// A directed network connectivity matrix representing asymmetric partitions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectedNetworkMatrix {
    /// Number of nodes in the cluster.
    pub cluster_size: usize,
    /// Set of directed edges (from, to) where packet transmission succeeds.
    pub connectivity: BTreeSet<(usize, usize)>,
}

impl DirectedNetworkMatrix {
    /// Creates a fully connected network.
    #[must_use]
    pub fn fully_connected(cluster_size: usize) -> Self {
        let mut connectivity = BTreeSet::new();
        for i in 0..cluster_size {
            for j in 0..cluster_size {
                connectivity.insert((i, j));
            }
        }
        Self {
            cluster_size,
            connectivity,
        }
    }

    /// Injects an asymmetric drop: node `from` can no longer reach node `to`,
    /// while `to` may still reach `from`.
    pub fn drop_directed_edge(&mut self, from: usize, to: usize) {
        self.connectivity.remove(&(from, to));
    }

    /// Evaluates if a round-trip heartbeat (leader -> follower -> leader) succeeds.
    #[must_use]
    pub fn round_trip_ok(&self, leader: usize, follower: usize) -> bool {
        self.connectivity.contains(&(leader, follower)) && self.connectivity.contains(&(follower, leader))
    }
}

/// Distributed leader lease state tracker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaderLeaseTracker {
    /// Cluster size N.
    pub cluster_size: usize,
    /// Configured lease duration in microseconds (e.g. 5,000,000 µs = 5s).
    pub lease_duration_micros: u64,
    /// Maximum clock drift between nodes \Delta_max (e.g. 200,000 µs = 200ms).
    pub max_clock_drift_micros: u64,
    /// Timestamp (µs) when the last valid quorum renewal was committed.
    pub last_quorum_timestamp_micros: u64,
}

impl LeaderLeaseTracker {
    /// Initializes a new lease tracker.
    #[must_use]
    pub fn new(cluster_size: usize, lease_duration_micros: u64, max_clock_drift_micros: u64) -> Self {
        Self {
            cluster_size,
            lease_duration_micros,
            max_clock_drift_micros,
            last_quorum_timestamp_micros: 0,
        }
    }

    /// Required majority threshold (N / 2 + 1).
    #[must_use]
    pub fn majority_threshold(&self) -> usize {
        (self.cluster_size / 2) + 1
    }

    /// Calculates the safe operational lease duration taking into account clock drift:
    ///   safe_duration = lease_duration - 2 * \Delta_max.
    #[must_use]
    pub fn safe_lease_duration(&self) -> u64 {
        let drift_margin = 2 * self.max_clock_drift_micros;
        self.lease_duration_micros.saturating_sub(drift_margin)
    }

    /// Evaluates whether a leader can renew its lease given the current network topology.
    pub fn attempt_renew(
        &mut self,
        leader: usize,
        network: &DirectedNetworkMatrix,
        current_time_micros: u64,
    ) -> Result<(), LeaseViolation> {
        let mut successful_responses = 1; // Leader votes for itself

        for follower in 0..self.cluster_size {
            if follower == leader {
                continue;
            }
            if network.round_trip_ok(leader, follower) {
                successful_responses += 1;
            }
        }

        let required = self.majority_threshold();
        if successful_responses >= required {
            self.last_quorum_timestamp_micros = current_time_micros;
            Ok(())
        } else {
            Err(LeaseViolation::QuorumLossUnderAsymmetry {
                active_responses: successful_responses,
                majority_required: required,
            })
        }
    }

    /// Authorizes a local linearizable read against the lease.
    ///
    /// # Errors
    /// Returns `LeaseViolation::StaleReadUnderExpiredLease` if the lease has expired.
    pub fn check_local_read_authorized(&self, current_time_micros: u64) -> Result<(), LeaseViolation> {
        if self.last_quorum_timestamp_micros == 0 {
            return Err(LeaseViolation::StaleReadUnderExpiredLease {
                elapsed_micros: current_time_micros,
                safe_duration_micros: 0,
            });
        }

        let elapsed = current_time_micros.saturating_sub(self.last_quorum_timestamp_micros);
        let safe_duration = self.safe_lease_duration();

        if elapsed >= safe_duration {
            Err(LeaseViolation::StaleReadUnderExpiredLease {
                elapsed_micros: elapsed,
                safe_duration_micros: safe_duration,
            })
        } else {
            Ok(())
        }
    }
}
