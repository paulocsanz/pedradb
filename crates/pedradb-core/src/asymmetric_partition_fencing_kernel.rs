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
    ) -> Result<Self, &'static str> {
        if total_nodes == 0 {
            return Err("total_nodes must be positive");
        }
        if local_node_id == 0 {
            return Err("local_node_id must be non-zero");
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
    /// Returns an error if `peer_id == local_node_id`.
    pub fn record_peer_heartbeat(
        &mut self,
        peer_id: u64,
        peer_seq: u64,
        ack_of_our_seq: u64,
        tick: u64,
    ) -> Result<(), &'static str> {
        if peer_id == self.local_node_id {
            return Err("cannot record self as peer");
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
