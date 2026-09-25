//! RFC-0282 Test Suite:
//! Os Dez Pilares Fundamentais de Verificação Avançada de Sistemas de Armazenamento:
//! - Pilar 1: Semântica Formal de Memória Fraca (RC11 / ARM64)
//! - Pilar 2: Cura de Escrita Fracionária de Setor Físico (Dual-Boundary Envelope)
//! - Pilar 3: Partições de Rede Assimétricas e Validade de Leases
//! - Pilar 4: Linearizabilidade Estrita de Iteradores de Range
//! - Pilar 5: Barreira de GC no VLog com Marcação Tricolor
//! - Pilar 6: Terminação de Compactação por Métrica de Lyapunov
//! - Pilar 7: Não-Interferência e Zeroização Criptográfica
//! - Pilar 8: Agendador de Tempo Denso Contínuo no DST
//! - Pilar 9: Confluência de 2PC sob Queda Concorrente
//! - Pilar 10: Homomorfismo de Esquema e Preservação Semântica

use std::collections::{BTreeMap, BTreeSet};

use pedradb_core::asymmetric_lease_kernel::{
    DirectedNetworkMatrix, LeaderLeaseTracker, LeaseViolation,
};
use pedradb_core::compaction_lyapunov_kernel::{
    CompactionTerminationViolation, LevelDebtProfile, LyapunovCompactionVerifier,
};
use pedradb_core::dense_time_scheduler_kernel::{DenseTimeScheduler, ScheduledEventType};
use pedradb_core::range_scan_linear_kernel::{
    RangeScanItem, RangeScanLinearityOracle, RangeScanViolation,
};
use pedradb_core::rc11_relaxed_memory_kernel::{
    MemoryEvent, MemoryOrder, Rc11ConsistencyViolation, Rc11ExecutionGraph,
};
use pedradb_core::schema_homomorphism_kernel::{
    RecordV1, RecordV2, SchemaEvolutionViolation, SchemaHomomorphismVerifier,
};
use pedradb_core::torn_sector_heal_kernel::{PageEnvelope, TornSectorViolation, LOGICAL_PAGE_SIZE};
use pedradb_core::twopc_confluence_kernel::{
    CoordinatorState, ParticipantState, TwoPcDecision, TwoPcDivergenceViolation,
    TwoPcConfluenceEngine,
};
use pedradb_core::vlog_gc_barrier_kernel::{
    BlobCoordinate, MarkColor, VlogGcBarrierOracle, VlogGcViolation,
};
use pedradb_core::zeroize_entropy_kernel::{
    SecureZeroizeBuffer, ZeroizeEntropyOracle, ZeroizeViolation,
};

#[test]
fn test_pilar1_rc11_relaxed_memory_causality() {
    let mut graph = Rc11ExecutionGraph::default();

    // Thread 0: Writes payload (Relaxed), then writes seq (Release)
    let w_data = graph.add_event(MemoryEvent::Write {
        thread: 0,
        addr: 0x1000,
        val: 42,
        order: MemoryOrder::Relaxed,
    });
    let w_seq = graph.add_event(MemoryEvent::Write {
        thread: 0,
        addr: 0x2000,
        val: 100,
        order: MemoryOrder::Release,
    });

    // Thread 1: Reads seq (Acquire), then reads payload (Relaxed)
    let r_seq = graph.add_event(MemoryEvent::Read {
        thread: 1,
        addr: 0x2000,
        val: 100,
        order: MemoryOrder::Acquire,
    });
    let r_data = graph.add_event(MemoryEvent::Read {
        thread: 1,
        addr: 0x1000,
        val: 42,
        order: MemoryOrder::Relaxed,
    });

    // Establish Reads-From edges
    graph.add_reads_from(w_seq, r_seq);
    graph.add_reads_from(w_data, r_data);

    // Verify consistency: Release-Acquire synchronizes-with, making w_data happen-before r_data
    assert!(
        graph.verify_rc11_consistency().is_ok(),
        "Proper Release-Acquire pair must be sound under RC11"
    );

    // Test failure case: Reader reads seq with Relaxed (missing Acquire barrier)
    let mut broken_graph = Rc11ExecutionGraph::default();
    let b_w_data = broken_graph.add_event(MemoryEvent::Write {
        thread: 0,
        addr: 0x1000,
        val: 42,
        order: MemoryOrder::Relaxed,
    });
    let b_w_seq = broken_graph.add_event(MemoryEvent::Write {
        thread: 0,
        addr: 0x2000,
        val: 100,
        order: MemoryOrder::Release,
    });
    let b_r_seq = broken_graph.add_event(MemoryEvent::Read {
        thread: 1,
        addr: 0x2000,
        val: 100,
        order: MemoryOrder::Relaxed, // Bug: Relaxed instead of Acquire!
    });
    let b_r_data = broken_graph.add_event(MemoryEvent::Read {
        thread: 1,
        addr: 0x1000,
        val: 42,
        order: MemoryOrder::Relaxed,
    });
    broken_graph.add_reads_from(b_w_seq, b_r_seq);
    broken_graph.add_reads_from(b_w_data, b_r_data);

    assert!(
        matches!(
            broken_graph.verify_rc11_consistency(),
            Err(Rc11ConsistencyViolation::UnsynchronizedRead { .. })
        ),
        "Unsynchronized cross-thread read must be flagged under weak memory"
    );
}

#[test]
fn test_pilar2_torn_sector_dual_boundary_heal() {
    let mut page_buffer = [0u8; LOGICAL_PAGE_SIZE];
    let payload = b"critical_database_payload_bytes_alpha";

    // 1. Encode valid page
    PageEnvelope::encode_page(42, 1001, payload, &mut page_buffer);

    // 2. Verify clean page
    let res = PageEnvelope::verify_page(&page_buffer);
    assert_eq!(res, Ok((42, 1001)));

    // 3. Inject a torn write: power cut after Sector 0..3 were written, Sector 7 has stale generation
    let mut torn_page = page_buffer;
    torn_page[4092..4096].copy_from_slice(&999u32.to_le_bytes()); // Stale generation in tail
    assert!(
        matches!(
            PageEnvelope::verify_page(&torn_page),
            Err(TornSectorViolation::BoundaryGenerationMismatch { .. })
        ),
        "Torn write must be caught by dual-boundary generation mismatch"
    );

    // 4. Inject payload corruption in middle sector
    let mut corrupt_payload_page = page_buffer;
    corrupt_payload_page[100] ^= 0x01; // Bit flip in Sector 0 data area
    assert!(
        matches!(
            PageEnvelope::verify_page(&corrupt_payload_page),
            Err(TornSectorViolation::PagePayloadChecksumCorrupted { .. })
        ),
        "Sector payload bit corruption must be caught by CRC"
    );
}

#[test]
fn test_pilar3_asymmetric_network_lease() {
    let mut net = DirectedNetworkMatrix::fully_connected(3);
    let mut tracker = LeaderLeaseTracker::new(3, 5_000_000, 200_000); // 5s lease, 200ms drift

    // 1. Leader 0 renews at t = 1,000,000 µs (1s)
    assert!(tracker.attempt_renew(0, &net, 1_000_000).is_ok());

    // 2. Read at t = 2,000,000 µs (1s elapsed < 4.6s safe duration): authorized
    assert!(tracker.check_local_read_authorized(2_000_000).is_ok());

    // 3. Read at t = 6,000,000 µs (5s elapsed >= 4.6s safe duration): rejected
    assert!(
        matches!(
            tracker.check_local_read_authorized(6_000_000),
            Err(LeaseViolation::StaleReadUnderExpiredLease { .. })
        ),
        "Expired lease must reject local reads fail-closed"
    );

    // 4. Asymmetric network drop: leader 0's packets to follower 1 are dropped
    // Node 0 can talk to node 2, but not node 1
    net.drop_directed_edge(0, 1);
    // Round trip to node 2 is still ok -> 2 out of 3 responses -> quorum still holds
    assert!(tracker.attempt_renew(0, &net, 7_000_000).is_ok());

    // Now drop round-trip with node 2 as well:
    net.drop_directed_edge(2, 0); // node 2 cannot reply to node 0
    assert!(
        matches!(
            tracker.attempt_renew(0, &net, 8_000_000),
            Err(LeaseViolation::QuorumLossUnderAsymmetry { .. })
        ),
        "Quorum loss under asymmetric partition must prevent lease renewal"
    );
}

#[test]
fn test_pilar4_range_scan_linearizability() {
    let mut atomic_txns = BTreeMap::new();
    let mut txn1_keys = BTreeSet::new();
    txn1_keys.insert(b"user:001:profile".to_vec());
    txn1_keys.insert(b"user:001:settings".to_vec());
    atomic_txns.insert(100, txn1_keys);

    // 1. Valid range scan stream
    let valid_stream = vec![
        RangeScanItem {
            key: b"user:001:profile".to_vec(),
            val: b"Alice".to_vec(),
            seq: 100,
            txn_id: Some(100),
        },
        RangeScanItem {
            key: b"user:001:settings".to_vec(),
            val: b"DarkTheme".to_vec(),
            seq: 100,
            txn_id: Some(100),
        },
        RangeScanItem {
            key: b"user:002:profile".to_vec(),
            val: b"Bob".to_vec(),
            seq: 150,
            txn_id: None,
        },
    ];

    assert!(RangeScanLinearityOracle::verify_range_scan_stream(
        &valid_stream,
        200, // Snapshot seq
        &atomic_txns,
    )
    .is_ok());

    // 2. Snapshot leak: record with seq 250 leaks into snapshot seq 200
    let leaked_stream = vec![RangeScanItem {
        key: b"user:003:profile".to_vec(),
        val: b"Charlie".to_vec(),
        seq: 250, // > 200!
        txn_id: None,
    }];
    assert!(matches!(
        RangeScanLinearityOracle::verify_range_scan_stream(&leaked_stream, 200, &atomic_txns),
        Err(RangeScanViolation::SnapshotCutoffLeaked { .. })
    ));

    // 3. Intra-scan time-tearing: txn 100 wrote profile and settings, but scan only observed profile
    let torn_stream = vec![RangeScanItem {
        key: b"user:001:profile".to_vec(),
        val: b"Alice".to_vec(),
        seq: 100,
        txn_id: Some(100),
    }];
    assert!(matches!(
        RangeScanLinearityOracle::verify_range_scan_stream(&torn_stream, 200, &atomic_txns),
        Err(RangeScanViolation::IntraScanTimeTearing { .. })
    ));
}

#[test]
fn test_pilar5_vlog_gc_tri_color_barrier() {
    let mut oracle = VlogGcBarrierOracle::new();

    let blob_active = BlobCoordinate {
        file_num: 1,
        offset: 1024,
    };
    let blob_inflight = BlobCoordinate {
        file_num: 1,
        offset: 2048,
    };
    let blob_staged = BlobCoordinate {
        file_num: 2,
        offset: 0,
    };
    let blob_dead = BlobCoordinate {
        file_num: 1,
        offset: 4096,
    };

    oracle.add_active_lsm_blob(blob_active);
    oracle.add_inflight_txn_blob(blob_inflight);
    oracle.add_staged_compaction_blob(blob_staged);

    // Classification
    assert_eq!(oracle.classify_blob(&blob_active), MarkColor::Black);
    assert_eq!(oracle.classify_blob(&blob_inflight), MarkColor::Black);
    assert_eq!(oracle.classify_blob(&blob_staged), MarkColor::Black);
    assert_eq!(oracle.classify_blob(&blob_dead), MarkColor::White);

    // Reclaiming dead blob is safe
    assert!(oracle.verify_reclamation_safety(&[blob_dead]).is_ok());

    // Reclaiming inflight blob fails
    assert!(matches!(
        oracle.verify_reclamation_safety(&[blob_inflight]),
        Err(VlogGcViolation::InFlightTransactionBlobReclaimed { .. })
    ));

    // Releasing inflight transaction allows safe transition once unreferenced
    oracle.remove_inflight_txn_blob(&blob_inflight);
    assert_eq!(oracle.classify_blob(&blob_inflight), MarkColor::White);
    assert!(oracle.verify_reclamation_safety(&[blob_inflight]).is_ok());
}

#[test]
fn test_pilar6_compaction_lyapunov_quiescence() {
    let levels_before = vec![
        LevelDebtProfile {
            level_idx: 0,
            current_bytes: 20_000,
            target_capacity_bytes: 10_000,
            overlapping_runs: 3,
        },
        LevelDebtProfile {
            level_idx: 1,
            current_bytes: 50_000,
            target_capacity_bytes: 40_000,
            overlapping_runs: 1,
        },
    ];

    let levels_after_progress = vec![
        LevelDebtProfile {
            level_idx: 0,
            current_bytes: 5_000, // Under target!
            target_capacity_bytes: 10_000,
            overlapping_runs: 1,
        },
        LevelDebtProfile {
            level_idx: 1,
            current_bytes: 55_000, // Slightly more, but weight is much lower than L0
            target_capacity_bytes: 40_000,
            overlapping_runs: 1,
        },
    ];

    // Verify progress strictly reduces system energy
    let step_res =
        LyapunovCompactionVerifier::verify_compaction_step(&levels_before, &levels_after_progress);
    assert!(step_res.is_ok(), "Compaction step must reduce Lyapunov energy");

    // Zero progress step
    assert!(matches!(
        LyapunovCompactionVerifier::verify_compaction_step(&levels_before, &levels_before),
        Err(CompactionTerminationViolation::ZeroProgressStep { .. })
    ));

    // Quiescent state check
    let quiescent_levels = vec![
        LevelDebtProfile {
            level_idx: 0,
            current_bytes: 8_000,
            target_capacity_bytes: 10_000,
            overlapping_runs: 1,
        },
        LevelDebtProfile {
            level_idx: 1,
            current_bytes: 35_000,
            target_capacity_bytes: 40_000,
            overlapping_runs: 1,
        },
    ];
    assert!(LyapunovCompactionVerifier::is_quiescent(&quiescent_levels));
}

#[test]
fn test_pilar7_zeroize_entropy_destruction() {
    let secret = b"my_super_secret_master_encryption_key_2026";
    let mut buf = SecureZeroizeBuffer::new(secret);

    // Initial entropy is high (> 3.0 bits)
    assert!(ZeroizeEntropyOracle::calculate_shannon_entropy(buf.as_slice()) > 3.0);

    // Perform secure zeroization
    buf.zeroize();

    // Verify that every byte is 0 and entropy is 0.0
    assert!(ZeroizeEntropyOracle::verify_zeroized_entropy(buf.as_slice()).is_ok());
    assert_eq!(
        ZeroizeEntropyOracle::calculate_shannon_entropy(buf.as_slice()),
        0.0
    );

    // Inject residual byte
    buf.as_mut_slice()[5] = 0x7F;
    assert!(matches!(
        ZeroizeEntropyOracle::verify_zeroized_entropy(buf.as_slice()),
        Err(ZeroizeViolation::ResidualEntropyDetected { offset: 5, leaked_byte: 0x7F })
    ));
}

#[test]
fn test_pilar8_dense_time_scheduler_epsilon() {
    let mut scheduler = DenseTimeScheduler::new(5_000); // 5,000 ns epsilon window

    // Event 1: Lease timeout at 100,000 ns
    scheduler.schedule_event(
        1,
        100_000,
        ScheduledEventType::TimerExpiry { timer_id: 42 },
    );
    // Event 2: I/O completion at 103,000 ns (delta = 3,000 ns <= 5,000 ns)
    scheduler.schedule_event(
        2,
        103_000,
        ScheduledEventType::IoCompletion { io_id: 10 },
    );
    // Event 3: Far away event at 200,000 ns
    scheduler.schedule_event(
        3,
        200_000,
        ScheduledEventType::NetworkPacket {
            sender: 1,
            msg_id: 99,
        },
    );

    let racing_pairs = scheduler.find_epsilon_racing_pairs();
    assert_eq!(racing_pairs.len(), 1);
    assert_eq!(racing_pairs[0], (1, 2, 3_000));

    let (trace_ab, trace_ba) = scheduler.generate_perturbed_schedules(1, 2);
    assert_eq!(trace_ab, vec![1, 2, 3]);
    assert_eq!(trace_ba, vec![2, 1, 3]);
}

#[test]
fn test_pilar9_twopc_recovery_confluence() {
    let mut participants = BTreeMap::new();
    participants.insert(1, ParticipantState::PreparedDurable);
    participants.insert(2, ParticipantState::CommittedDurable);

    // 1. Coordinator crashed after persisting Committed
    let decision = TwoPcConfluenceEngine::recover_and_verify(
        CoordinatorState::CommittedDurable,
        &participants,
    );
    assert_eq!(decision, Ok(TwoPcDecision::Commit));

    // 2. Coordinator crashed in Preparing (before committing)
    let mut prep_participants = BTreeMap::new();
    prep_participants.insert(1, ParticipantState::PreparedDurable);
    prep_participants.insert(2, ParticipantState::PreparedDurable);
    let abort_decision = TwoPcConfluenceEngine::recover_and_verify(
        CoordinatorState::Preparing,
        &prep_participants,
    );
    assert_eq!(abort_decision, Ok(TwoPcDecision::Abort));

    // 3. Spontaneous commit without coordinator approval
    let mut illegal_participants = BTreeMap::new();
    illegal_participants.insert(1, ParticipantState::CommittedDurable);
    let violation = TwoPcConfluenceEngine::recover_and_verify(
        CoordinatorState::AbortedDurable,
        &illegal_participants,
    );
    assert!(matches!(
        violation,
        Err(TwoPcDivergenceViolation::SpontaneousCommitWithoutQuorum { .. })
    ));
}

#[test]
fn test_pilar10_schema_homomorphism_cross_version() {
    // 1. Encode V1 record
    let v1 = RecordV1 {
        id: 1042,
        key: b"config:max_connections".to_vec(),
        value: b"10000".to_vec(),
    };

    // 2. Verify categorical homomorphism
    assert!(SchemaHomomorphismVerifier::verify_homomorphism(&v1).is_ok());

    // 3. Wire decode using V2 decoder
    let encoded_v1 = SchemaHomomorphismVerifier::encode_v1(&v1);
    let decoded_v2 = SchemaHomomorphismVerifier::decode_v2(&encoded_v1).unwrap();

    assert_eq!(decoded_v2.id, 1042);
    assert_eq!(decoded_v2.key, b"config:max_connections");
    assert_eq!(decoded_v2.value, b"10000");
    assert_eq!(decoded_v2.ttl_seconds, None); // Canonical default

    // 4. Native V2 record with TTL
    let v2_native = RecordV2 {
        id: 999999999,
        key: b"session:auth:token".to_vec(),
        value: b"jwt_payload".to_vec(),
        ttl_seconds: Some(3600),
    };
    let encoded_v2 = SchemaHomomorphismVerifier::encode_v2(&v2_native);
    let decoded_native = SchemaHomomorphismVerifier::decode_v2(&encoded_v2).unwrap();
    assert_eq!(v2_native, decoded_native);

    // 5. Corrupt version tag
    let corrupt_bytes = vec![0x99, 0x99, 0x01, 0x02];
    assert!(matches!(
        SchemaHomomorphismVerifier::decode_v2(&corrupt_bytes),
        Err(SchemaEvolutionViolation::UnknownVersionHeader { .. })
    ));
}
