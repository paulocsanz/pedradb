//! RFC-0322: Asymmetric Partition Fencing Verification Suite.

use pedradb_core::asymmetric_partition_fencing_kernel::{
    AsymmetricPartitionFencingKernel, FencingState, PeerReachability,
};

#[test]
fn test_symmetric_quorum_healthy_progress() {
    // 3-node cluster: local=1, peers=2, 3. Quorum size = (3/2)+1 = 2.
    let mut kernel = AsymmetricPartitionFencingKernel::new(1, 3, 5, 20).unwrap();
    assert_eq!(kernel.quorum_size(), 2);
    assert_eq!(kernel.our_seq(), 1);

    // Initial state: no heartbeats yet -> unreachable peers -> fenced.
    assert_eq!(kernel.evaluate_fencing_state(), FencingState::FencedSplitBrainRisk);
    assert!(!kernel.is_write_admitted());

    // Peer 2 sends heartbeat acknowledging our seq 1.
    kernel.record_peer_heartbeat(2, 10, 1, 1).unwrap();
    assert_eq!(kernel.peer_reachability(2), PeerReachability::SymmetricHealthy);
    assert_eq!(kernel.active_symmetric_peers(), 1);

    // With local node (1) + peer 2 (1) = 2 nodes >= quorum 2 -> NormalQuorum.
    assert_eq!(kernel.evaluate_fencing_state(), FencingState::NormalQuorum);
    assert!(kernel.is_write_admitted());
}

#[test]
fn test_asymmetric_outbound_blackhole_detection() {
    // 3-node cluster, max_ack_lag = 3.
    let mut kernel = AsymmetricPartitionFencingKernel::new(1, 3, 3, 50).unwrap();

    // Peer 2 and 3 both active initially.
    kernel.record_peer_heartbeat(2, 100, 1, 10).unwrap();
    kernel.record_peer_heartbeat(3, 100, 1, 10).unwrap();
    assert_eq!(kernel.evaluate_fencing_state(), FencingState::NormalQuorum);

    // Local node advances sequence heavily (e.g. up to seq 10).
    for _ in 0..9 {
        kernel.advance_our_seq();
    }
    assert_eq!(kernel.our_seq(), 10);

    // Peer 2 continues sending heartbeats, but ack_of_our_seq remains 1 (lag = 9 > 3).
    // This simulates an asymmetric partition: node 1 hears node 2, but node 2 dropped node 1's packets.
    kernel.record_peer_heartbeat(2, 120, 1, 15).unwrap();
    assert_eq!(
        kernel.peer_reachability(2),
        PeerReachability::AsymmetricOutboundBlackhole
    );

    // Peer 3 also exhibits lag (remains at ack 1).
    kernel.record_peer_heartbeat(3, 120, 1, 15).unwrap();
    assert_eq!(
        kernel.peer_reachability(3),
        PeerReachability::AsymmetricOutboundBlackhole
    );

    // Active symmetric peers drops to 0. Quorum lost!
    assert_eq!(kernel.active_symmetric_peers(), 0);
    assert_eq!(kernel.evaluate_fencing_state(), FencingState::FencedSplitBrainRisk);
    assert!(!kernel.is_write_admitted());
}

#[test]
fn test_heartbeat_timeout_drop() {
    let mut kernel = AsymmetricPartitionFencingKernel::new(1, 3, 5, 25).unwrap();
    kernel.record_peer_heartbeat(2, 50, 1, 10).unwrap();
    assert_eq!(kernel.peer_reachability(2), PeerReachability::SymmetricHealthy);

    // Monotonic tick advances by 30 ticks (exceeding timeout 25).
    kernel.advance_tick(30);
    assert_eq!(kernel.peer_reachability(2), PeerReachability::Unreachable);
    assert_eq!(kernel.evaluate_fencing_state(), FencingState::FencedSplitBrainRisk);
}
