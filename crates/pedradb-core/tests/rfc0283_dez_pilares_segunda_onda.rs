//! RFC-0283 Test Suite:
//! Os Dez Pilares da Segunda Onda de Verificação Avançada:
//! - Pilar 1: Ausência de Inanição e Limite Estrito de Ultrapassagem (Starvation Freedom)
//! - Pilar 2: Teorema da Inversão Bijetiva de Codecs de Compressão (Codec Inversion)
//! - Pilar 3: Soundness de Delta-Encoding por Prefixo e Pontos de Reinício (Prefix Delta)
//! - Pilar 4: Isolamento e Recuperação Atômica Multi-Column Family (Cross-CF)
//! - Pilar 5: Imunidade a Envenenamento de Cache em DRAM (Cache Canary Sentinel)
//! - Pilar 6: Detecção Dinâmica de Ciclos de Anti-Dependência em SSI (SSI Cycle Detector)
//! - Pilar 7: Prevenção de Esgotamento de Espaço por Snapshots Abandonados (Snapshot Epoch Lease)
//! - Pilar 8: Imunidade a Inversão de Prioridade no Commit (Priority Inversion Freedom)
//! - Pilar 9: Persistência Estrita do Diretório-Pai POSIX (POSIX Dir Sync)
//! - Pilar 10: Continuidade Monotônica Snapshot-para-Log na Replicação (Replication Boundary)

use std::collections::BTreeMap;

use pedradb_core::cache_canary_sentinel_kernel::{
    CacheIntegrityViolation, CachedBlockSentinel,
};
use pedradb_core::codec_inversion_kernel::{BlockCodec, CodecError};
use pedradb_core::cross_cf_isolation_kernel::{
    CrossCfIsolationViolation, CrossCfReplayManager, CrossCfWalRecord,
};
use pedradb_core::posix_dir_sync_kernel::{
    FilePersistencePhase, PosixDirSyncOrderOracle, PosixDirSyncViolation,
};
use pedradb_core::prefix_delta_restart_kernel::{
    BlockKvEntry, PrefixDeltaBlock, PrefixDeltaViolation,
};
use pedradb_core::priority_inversion_freedom_kernel::{
    AdmissionTask, PriorityAdmissionEngine, PriorityLevel,
};
use pedradb_core::replication_catchup_boundary_kernel::{
    ReplicatedLogEntry, ReplicationBoundaryReconciler, ReplicationBoundaryViolation,
};
use pedradb_core::snapshot_epoch_lease_kernel::{
    SnapshotLeaseManager, SnapshotLeaseViolation,
};
use pedradb_core::ssi_cycle_detector_kernel::{SsiSerializationGraph, SsiViolation};
use pedradb_core::starvation_freedom_kernel::BoundedOvertakingQueue;

#[test]
fn test_pilar1_starvation_freedom_k_bounded_overtaking() {
    let mut queue = BoundedOvertakingQueue::new(2); // K = 2 bypasses max

    let t1 = queue.enqueue(); // Ticket 1
    let t2 = queue.enqueue(); // Ticket 2
    let t3 = queue.enqueue(); // Ticket 3

    // Thread 2 is admitted (bypassing t1 once)
    assert!(queue.can_admit(t2));
    assert!(queue.admit(t2).is_ok());

    // Thread 3 is admitted (bypassing t1 twice = K reached)
    assert!(queue.can_admit(t3));
    assert!(queue.admit(t3).is_ok());

    // Thread 4 arrives
    let t4 = queue.enqueue();

    // Thread 4 CANNOT be admitted because t1 has already reached K=2 bypasses!
    assert!(!queue.can_admit(t4));

    // Must service t1 first
    assert!(queue.can_admit(t1));
    assert!(queue.admit(t1).is_ok());

    // Now thread 4 can be admitted
    assert!(queue.can_admit(t4));
    assert!(queue.admit(t4).is_ok());
    assert_eq!(queue.pending_count(), 0);
}

#[test]
fn test_pilar2_codec_inversion_roundtrip_and_safety() {
    // 1. Repeating pattern (tests backreferences)
    let pattern_data = b"PedraDB_PedraDB_PedraDB_ZeroTwin_ZeroTwin_ZeroTwin_1234567890".repeat(20);
    assert!(BlockCodec::verify_inversion_roundtrip(&pattern_data));

    // 2. High-entropy random-like pattern (tests literal runs)
    let pseudo_random: Vec<u8> = (0..5000).map(|i| ((i * 73 + 19) % 256) as u8).collect();
    assert!(BlockCodec::verify_inversion_roundtrip(&pseudo_random));

    // 3. Small literal
    assert!(BlockCodec::verify_inversion_roundtrip(b"tiny"));

    // 4. Corrupt stream safety: truncated stream fails closed
    let compressed = BlockCodec::compress(&pattern_data);
    let truncated = &compressed[..compressed.len() - 5];
    assert!(matches!(
        BlockCodec::decompress(truncated),
        Err(CodecError::UnexpectedEndOfStream) | Err(CodecError::LengthMismatch { .. })
    ));
}

#[test]
fn test_pilar3_prefix_delta_restart_points() {
    // Generate 50 sorted keys with strong prefix sharing
    let mut entries = Vec::new();
    for i in 0..50 {
        let key = format!("pedra:tenant:0042:record:{i:04}").into_bytes();
        let val = format!("payload_value_{i:04}").into_bytes();
        entries.push(BlockKvEntry { key, val });
    }

    // 1. Encode with restart interval = 4
    let block = PrefixDeltaBlock::encode_block(&entries, 4);

    // 2. Decode and verify exact inductive equality
    let decoded = PrefixDeltaBlock::decode_and_verify(&block, 4).expect("Block decode must succeed");
    assert_eq!(decoded, entries, "Decoded block entries must exactly match original sorted sequence");

    // 3. Corrupt restart point trailer (too short)
    assert!(matches!(
        PrefixDeltaBlock::decode_and_verify(&block[..10], 4),
        Err(PrefixDeltaViolation::CorruptTrailer)
    ));
}

#[test]
fn test_pilar4_cross_cf_isolation_and_replay() {
    let mut checkpoints = BTreeMap::new();
    checkpoints.insert(0, 100); // CF 0 ("default") flushed to seq 100
    checkpoints.insert(1, 50);  // CF 1 ("metadata") flushed to seq 50

    let manager = CrossCfReplayManager::new(checkpoints);

    let wal_stream = vec![
        // CF 0, seq 80: already flushed, must be skipped
        CrossCfWalRecord {
            cf_id: 0,
            seq: 80,
            key: b"k0_old".to_vec(),
            value: Some(b"v0_old".to_vec()),
        },
        // CF 1, seq 80: NOT flushed in CF 1 (flushed was 50), MUST be applied!
        CrossCfWalRecord {
            cf_id: 1,
            seq: 80,
            key: b"k1_live".to_vec(),
            value: Some(b"v1_live".to_vec()),
        },
        // CF 0, seq 120: live mutation for CF 0, MUST be applied
        CrossCfWalRecord {
            cf_id: 0,
            seq: 120,
            key: b"k0_new".to_vec(),
            value: Some(b"v0_new".to_vec()),
        },
    ];

    let replayed = manager
        .execute_partitioned_replay(&wal_stream)
        .expect("Cross-CF replay must succeed");

    // CF 0 state has only k0_new (k0_old was skipped)
    let cf0_state = replayed.get(&0).unwrap();
    assert!(!cf0_state.contains_key(&b"k0_old"[..]));
    assert!(cf0_state.contains_key(&b"k0_new"[..]));

    // CF 1 state has k1_live
    let cf1_state = replayed.get(&1).unwrap();
    assert!(cf1_state.contains_key(&b"k1_live"[..]));

    // Truncation safety: segment with max_seq 70 cannot be deleted because CF 1 still needs seq > 50
    assert!(matches!(
        manager.verify_wal_truncation_safety(70),
        Err(CrossCfIsolationViolation::UnsafeWalTruncation { lagging_cf_id: 1, .. })
    ));
}

#[test]
fn test_pilar5_cache_canary_sentinel() {
    let payload = b"uncompressed_sst_block_data_in_ram_cache".to_vec();
    let mut sentinel = CachedBlockSentinel::new(payload.clone());

    // 1. Initial valid read
    assert_eq!(sentinel.get_verified_payload(), Ok(&payload[..]));

    // 2. In-RAM bit flip corruption
    sentinel.inject_bit_flip(5);
    assert!(matches!(
        sentinel.get_verified_payload(),
        Err(CacheIntegrityViolation::InRamBitFlipDetected { .. })
    ));

    // Reset payload
    sentinel = CachedBlockSentinel::new(payload);

    // 3. Head canary corruption (e.g. buffer underflow in memory)
    sentinel.inject_head_canary_corrupt();
    assert!(matches!(
        sentinel.get_verified_payload(),
        Err(CacheIntegrityViolation::HeadCanaryCorrupted { .. })
    ));
}

#[test]
fn test_pilar6_ssi_dangerous_structure_detection() {
    let mut sgc = SsiSerializationGraph::new();

    // Start 3 transactions: T1, T2, T3
    sgc.begin_txn(1);
    sgc.begin_txn(2);
    sgc.begin_txn(3);

    // T1 reads "account_a"
    sgc.record_read(1, b"account_a");
    // T2 writes "account_a" -> creates T1 -(rw)-> T2
    sgc.record_write(2, b"account_a");

    // T2 reads "account_b"
    sgc.record_read(2, b"account_b");
    // T3 writes "account_b" -> creates T2 -(rw)-> T3
    sgc.record_write(3, b"account_b");

    // Analyze dependencies
    sgc.analyze_dependencies_for_commit(2);

    // T2 has in_rw from T1 and out_rw to T3: dangerous structure!
    assert!(matches!(
        sgc.verify_can_commit(2),
        Err(SsiViolation::DangerousStructureDetected { pivot_txn: 2, in_txn: 1, out_txn: 3 })
    ));

    // T1 has only out_rw, no in_rw: safe to commit
    assert!(sgc.verify_can_commit(1).is_ok());
}

#[test]
fn test_pilar7_snapshot_epoch_lease_revocation() {
    let mut manager = SnapshotLeaseManager::new(10); // Epoch 10

    // Acquire snapshot 100 with duration 5 epochs (expires at 15)
    let lease = manager.acquire_snapshot(100, 500, 5);
    assert_eq!(lease.expiration_epoch(), 15);

    // At epoch 12: read is valid
    manager.current_epoch = 12;
    assert_eq!(manager.verify_read_access(100), Ok(500));
    assert_eq!(manager.min_active_unexpired_seq(), Some(500));

    // Advance to epoch 16: lease has expired!
    manager.current_epoch = 16;
    assert!(matches!(
        manager.verify_read_access(100),
        Err(SnapshotLeaseViolation::SnapshotLeaseExpired { snapshot_id: 100, .. })
    ));

    // Expired snapshot is excluded from min_active_unexpired_seq (returns None, unblocking purge)
    assert_eq!(manager.min_active_unexpired_seq(), None);
}

#[test]
fn test_pilar8_priority_inversion_freedom() {
    let mut engine = PriorityAdmissionEngine::new(0); // Strict priority

    let bg_compaction = AdmissionTask {
        task_id: 1,
        priority: PriorityLevel::BackgroundMaintenance,
        cost_units: 100_000,
    };
    let client_put = AdmissionTask {
        task_id: 2,
        priority: PriorityLevel::InteractiveClientWrite,
        cost_units: 1,
    };

    // Submit background first, then client
    engine.submit_task(bg_compaction.clone());
    engine.submit_task(client_put.clone());

    // Client write MUST be admitted first despite being submitted second!
    let next_task = engine.admit_next().unwrap();
    assert_eq!(next_task.priority, PriorityLevel::InteractiveClientWrite);
    assert_eq!(next_task.task_id, 2);

    // Maintenance task admitted next
    let bg_task = engine.admit_next().unwrap();
    assert_eq!(bg_task.priority, PriorityLevel::BackgroundMaintenance);
    assert_eq!(bg_task.task_id, 1);

    // Verify execution trace
    let trace = vec![client_put, bg_compaction];
    assert!(PriorityAdmissionEngine::verify_execution_trace(&trace, 0).is_ok());
}

#[test]
fn test_pilar9_posix_dir_sync_order() {
    let file_num = 42;

    // 1. Valid lifecycle progression
    assert!(PosixDirSyncOrderOracle::verify_transition(
        file_num,
        FilePersistencePhase::DataWritten,
        FilePersistencePhase::FileDataSynced,
    )
    .is_ok());

    assert!(PosixDirSyncOrderOracle::verify_transition(
        file_num,
        FilePersistencePhase::FileDataSynced,
        FilePersistencePhase::AtomicallyRenamed,
    )
    .is_ok());

    assert!(PosixDirSyncOrderOracle::verify_transition(
        file_num,
        FilePersistencePhase::AtomicallyRenamed,
        FilePersistencePhase::ParentDirectorySynced,
    )
    .is_ok());

    assert!(PosixDirSyncOrderOracle::verify_transition(
        file_num,
        FilePersistencePhase::ParentDirectorySynced,
        FilePersistencePhase::ManifestCommitted,
    )
    .is_ok());

    // 2. Ordering violation: renaming before fdatasync
    assert!(matches!(
        PosixDirSyncOrderOracle::verify_transition(
            file_num,
            FilePersistencePhase::DataWritten,
            FilePersistencePhase::AtomicallyRenamed,
        ),
        Err(PosixDirSyncViolation::RenameBeforeDataSync { .. })
    ));

    // 3. Manifest commit before parent dir sync
    let mut oracle = PosixDirSyncOrderOracle::new();
    assert!(matches!(
        oracle.authorize_manifest_commit(file_num),
        Err(PosixDirSyncViolation::ManifestCommittedBeforeDirSync { .. })
    ));

    // Confirming dir sync authorizes manifest commit
    oracle.note_dir_synced(file_num);
    assert!(oracle.authorize_manifest_commit(file_num).is_ok());
}

#[test]
fn test_pilar10_replication_catchup_boundary() {
    let snapshot_seq = 1000;

    // 1. Exact seamless stitching: stream starts at 1001
    let clean_stream = vec![
        ReplicatedLogEntry {
            seq: 1001,
            key: b"k1".to_vec(),
            value: Some(b"v1".to_vec()),
        },
        ReplicatedLogEntry {
            seq: 1002,
            key: b"k2".to_vec(),
            value: Some(b"v2".to_vec()),
        },
    ];
    let reconciled = ReplicationBoundaryReconciler::verify_boundary_continuity(
        snapshot_seq,
        &clean_stream,
    )
    .expect("Replication boundary should stitch cleanly");
    assert_eq!(reconciled.len(), 2);

    // 2. Gap detection: stream starts at 1005 (missing 1001..1004)
    let gap_stream = vec![ReplicatedLogEntry {
        seq: 1005,
        key: b"k5".to_vec(),
        value: Some(b"v5".to_vec()),
    }];
    assert!(matches!(
        ReplicationBoundaryReconciler::verify_boundary_continuity(snapshot_seq, &gap_stream),
        Err(ReplicationBoundaryViolation::ReplicationGapDetected { gap_size: 4, .. })
    ));

    // 3. Overlapping stream: stream contains 1000, 1001 (seq 1000 idempotently skipped)
    let overlap_stream = vec![
        ReplicatedLogEntry {
            seq: 1000,
            key: b"k0".to_vec(),
            value: Some(b"v0".to_vec()),
        },
        ReplicatedLogEntry {
            seq: 1001,
            key: b"k1".to_vec(),
            value: Some(b"v1".to_vec()),
        },
    ];
    let filtered = ReplicationBoundaryReconciler::verify_boundary_continuity(
        snapshot_seq,
        &overlap_stream,
    )
    .expect("Overlapping prefix should be safely skipped");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].seq, 1001);
}
