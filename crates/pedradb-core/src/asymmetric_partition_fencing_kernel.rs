//! RFC-0322: Asymmetric Partition Fencing Kernel.
//!
//! Enforces two-way symmetric reachability across cloud cluster nodes.
//! In cloud networks (SDN / vSwitch packet filter rules), asymmetric partitions
//! occur when Node A can receive packets from Node B, but Node B drops Node A's
//! return packets. Standard heartbeat detectors mistakenly assume quorum, causing
//! dual-primary split-brain write corruption.
//!
//! This kernel tracks both incoming sequence numbers and the peer's acknowledgment
//! of local sequence numbers. If a majority of the cluster does not confirm
//! two-way reachability within `max_ack_lag`, write admission is revoked.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt;

/// Invariant violations and errors for asymmetric partition fencing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionFencingError {
    /// Total nodes must be strictly positive.
    ZeroTotalNodes,
    /// Local node ID must be non-zero.
    ZeroLocalNodeId,
    /// Peer ID cannot be zero.
    InvalidPeerId,
    /// Cannot record local node as peer.
    CannotRecordSelfAsPeer,
    /// Peer sequence number must be non-zero.
    ZeroPeerSequence,
    /// Number of active peers exceeds cluster ceiling (total_nodes - 1).
    PeerCountExceedsClusterLimit { active: u32, max_allowed: u32 },
}

impl fmt::Display for PartitionFencingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroTotalNodes => write!(f, "total_nodes must be positive"),
            Self::ZeroLocalNodeId => write!(f, "local_node_id must be non-zero"),
            Self::InvalidPeerId => write!(f, "peer_id cannot be zero"),
            Self::CannotRecordSelfAsPeer => write!(f, "cannot record self as peer"),
            Self::ZeroPeerSequence => write!(f, "peer_seq must be non-zero"),
            Self::PeerCountExceedsClusterLimit { active, max_allowed } => {
                write!(
                    f,
                    "peer count exceeds cluster limit: {active} > {max_allowed}"
                )
            }
        }
    }
}

impl std::error::Error for PartitionFencingError {}

/// Health classification of a cluster peer connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerReachability {
    /// Both incoming and outgoing directions are healthy within `max_ack_lag`.
    SymmetricHealthy,
    /// Local node receives peer packets, but peer does not acknowledge our packets.
    AsymmetricOutboundBlackhole,
    /// No packets received from peer within the timeout window.
    Unreachable,
}

/// Node fencing state determined by global two-way quorum analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FencingState {
    /// Quorum confirmed with full two-way reachability. Writes permitted.
    NormalQuorum,
    /// Two-way quorum lost due to asymmetric partitions or drops. Writes denied.
    FencedSplitBrainRisk,
}

/// Peer link observation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerLinkState {
    /// Highest sequence number received from the peer.
    pub last_received_seq: u64,
    /// Highest local sequence number confirmed received by the peer.
    pub last_ack_of_our_seq: u64,
    /// Monotonic tick when the last heartbeat was processed.
    pub last_seen_tick: u64,
}

/// Deterministic kernel evaluating two-way quorum connectivity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsymmetricPartitionFencingKernel {
    local_node_id: u64,
    total_nodes: u32,
    quorum_size: u32,
    max_ack_lag: u64,
    heartbeat_timeout_ticks: u64,
    our_seq: u64,
    current_tick: u64,
    peers: BTreeMap<u64, PeerLinkState>,
}

impl AsymmetricPartitionFencingKernel {
    /// Construct a new asymmetric partition fencing kernel.
    ///
    /// # Errors
    /// Returns an error if `total_nodes == 0` or `local_node_id == 0`.
    pub fn new(
        local_node_id: u64,
        total_nodes: u32,
        max_ack_lag: u64,
        heartbeat_timeout_ticks: u64,
    ) -> Result<Self, PartitionFencingError> {
        if total_nodes == 0 {
            return Err(PartitionFencingError::ZeroTotalNodes);
        }
        if local_node_id == 0 {
            return Err(PartitionFencingError::ZeroLocalNodeId);
        }
        let quorum_size = (total_nodes / 2) + 1;
        Ok(Self {
            local_node_id,
            total_nodes,
            quorum_size,
            max_ack_lag,
            heartbeat_timeout_ticks,
            our_seq: 1,
            current_tick: 0,
            peers: BTreeMap::new(),
        })
    }

    /// Advance local sequence number (e.g., when emitting an outbound heartbeat or batch).
    pub fn advance_our_seq(&mut self) -> u64 {
        self.our_seq = self.our_seq.saturating_add(1);
        self.our_seq
    }

    /// Advance local monotonic tick.
    pub fn advance_tick(&mut self, delta: u64) {
        self.current_tick = self.current_tick.saturating_add(delta);
    }

    /// Record a heartbeat received from a peer node.
    ///
    /// # Errors
    /// Returns an error if `peer_id == local_node_id`, `peer_id == 0`, `peer_seq == 0`,
    /// or if recording this peer would exceed cluster capacity (`total_nodes - 1`).
    pub fn record_peer_heartbeat(
        &mut self,
        peer_id: u64,
        peer_seq: u64,
        ack_of_our_seq: u64,
        tick: u64,
    ) -> Result<(), PartitionFencingError> {
        if peer_id == 0 {
            return Err(PartitionFencingError::InvalidPeerId);
        }
        if peer_id == self.local_node_id {
            return Err(PartitionFencingError::CannotRecordSelfAsPeer);
        }
        if peer_seq == 0 {
            return Err(PartitionFencingError::ZeroPeerSequence);
        }
        let max_peers = self.total_nodes.saturating_sub(1);
        if !self.peers.contains_key(&peer_id) && self.peers.len() >= max_peers as usize {
            return Err(PartitionFencingError::PeerCountExceedsClusterLimit {
                active: (self.peers.len() + 1) as u32,
                max_allowed: max_peers,
            });
        }
        if tick > self.current_tick {
            self.current_tick = tick;
        }

        let entry = self.peers.entry(peer_id).or_insert(PeerLinkState {
            last_received_seq: peer_seq,
            last_ack_of_our_seq: ack_of_our_seq,
            last_seen_tick: tick,
        });

        if peer_seq > entry.last_received_seq {
            entry.last_received_seq = peer_seq;
        }
        if ack_of_our_seq > entry.last_ack_of_our_seq {
            entry.last_ack_of_our_seq = ack_of_our_seq;
        }
        if tick > entry.last_seen_tick {
            entry.last_seen_tick = tick;
        }

        Ok(())
    }

    /// Check reachability status for a given peer.
    pub fn peer_reachability(&self, peer_id: u64) -> PeerReachability {
        let Some(link) = self.peers.get(&peer_id) else {
            return PeerReachability::Unreachable;
        };

        let tick_age = self.current_tick.saturating_sub(link.last_seen_tick);
        if tick_age > self.heartbeat_timeout_ticks {
            return PeerReachability::Unreachable;
        }

        let ack_lag = self.our_seq.saturating_sub(link.last_ack_of_our_seq);
        if ack_lag > self.max_ack_lag {
            return PeerReachability::AsymmetricOutboundBlackhole;
        }

        PeerReachability::SymmetricHealthy
    }

    /// Count how many remote peers are currently two-way symmetric healthy.
    pub fn active_symmetric_peers(&self) -> u32 {
        let mut count = 0;
        for &peer_id in self.peers.keys() {
            if self.peer_reachability(peer_id) == PeerReachability::SymmetricHealthy {
                count += 1;
            }
        }
        count
    }

    /// Evaluate overall cluster fencing state.
    ///
    /// Requires that `(1 + active_symmetric_peers) >= quorum_size`.
    pub fn evaluate_fencing_state(&self) -> FencingState {
        let effective_participants = 1 + self.active_symmetric_peers();
        if effective_participants >= self.quorum_size {
            FencingState::NormalQuorum
        } else {
            FencingState::FencedSplitBrainRisk
        }
    }

    /// Returns `true` if writes can be admitted safely.
    pub fn is_write_admitted(&self) -> bool {
        self.evaluate_fencing_state() == FencingState::NormalQuorum
    }

    /// Get current local sequence number.
    pub fn our_seq(&self) -> u64 {
        self.our_seq
    }

    /// Get required quorum size.
    pub fn quorum_size(&self) -> u32 {
        self.quorum_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asymmetric_partition_fencing_structural_invariants_red_to_green() {
        // Invariant 1: Total nodes == 0 must fail-closed with ZeroTotalNodes
        assert_eq!(
            AsymmetricPartitionFencingKernel::new(1, 0, 5, 20),
            Err(PartitionFencingError::ZeroTotalNodes)
        );

        // Invariant 2: Local node ID == 0 must fail-closed with ZeroLocalNodeId
        assert_eq!(
            AsymmetricPartitionFencingKernel::new(0, 3, 5, 20),
            Err(PartitionFencingError::ZeroLocalNodeId)
        );

        let mut kernel = AsymmetricPartitionFencingKernel::new(1, 3, 5, 20).unwrap();

        // Invariant 3: Peer ID == 0 must fail-closed with InvalidPeerId
        assert_eq!(
            kernel.record_peer_heartbeat(0, 10, 1, 1),
            Err(PartitionFencingError::InvalidPeerId)
        );

        // Invariant 4: Recording self as peer must fail-closed with CannotRecordSelfAsPeer
        assert_eq!(
            kernel.record_peer_heartbeat(1, 10, 1, 1),
            Err(PartitionFencingError::CannotRecordSelfAsPeer)
        );

        // Invariant 5: Peer sequence == 0 must fail-closed with ZeroPeerSequence
        assert_eq!(
            kernel.record_peer_heartbeat(2, 0, 1, 1),
            Err(PartitionFencingError::ZeroPeerSequence)
        );

        // Normal heartbeats for valid peers (peers 2 and 3 in a 3-node cluster)
        assert!(kernel.record_peer_heartbeat(2, 10, 1, 1).is_ok());
        assert!(kernel.record_peer_heartbeat(3, 10, 1, 1).is_ok());

        // Invariant 6: Sybil inflation: attempting to record peer 4 in a 3-node cluster (max remote peers = 2)
        assert_eq!(
            kernel.record_peer_heartbeat(4, 10, 1, 1),
            Err(PartitionFencingError::PeerCountExceedsClusterLimit {
                active: 3,
                max_allowed: 2
            })
        );
    }
}

