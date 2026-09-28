//! RFC-0287: Testes Unitários das Cinco Fronteiras Matemáticas e Estruturais do LSM Puro.
//!
//! Valida:
//! 1. Álgebra de Particionamento por Epsilon-Aproximação e Entropia de SST
//! 2. Aciclicidade de Desalocação e Liveness sob Reserva de Emergência em ENOSPC Dinâmico
//! 3. Causalidade Monótona Não-Zenoniana no MVCC sob Regressão de Relógio de Parede
//! 4. Coerência de Geração na Borda de DMA/Readahead contra Leituras Fantasmas
//! 5. Álgebra de Recuperação com Cadeia Criptográfica no WAL sob Corrupções Não-Torn

use pedradb_core::dma_generation_fence_kernel::{
    DmaGenerationFence, FenceRejection, FileGenerationToken,
};
use pedradb_core::enospc_drain_headroom_kernel::{
    AllocationOutcome, AllocationPriority, DrainHeadroomGovernor,
};
use pedradb_core::non_zeno_monotonic_clock_kernel::{
    NonZenoLogicalClock, TtlVisibilityOracle,
};
use pedradb_core::sst_topological_entropy_kernel::{
    KeySamplePoint, PartitionError, TopologicalEntropyTracker,
};
use pedradb_core::wal_crypto_chain_recovery_kernel::{
    ChainBreachReason, WalCryptoChainRecovery, WalRecoveryStatus, WAL_GENESIS_SEED,
};

#[test]
fn test_sst_topological_entropy_partition() {
    // Generate 50 keys with varied weights
    let mut samples = Vec::new();
    for i in 0..50 {
        let key = format!("user:{:05}", i).into_bytes();
        let weight = if i % 10 == 0 { 2000 } else { 500 };
        samples.push(KeySamplePoint::new(key, weight));
    }

    // Check entropy calculation
    let entropy = TopologicalEntropyTracker::calculate_prefix_entropy(&samples, 5);
    assert!(entropy >= 0.0 && entropy <= 1.0);

    // Plan with target 5000 bytes and 200 permille (20%) tolerance
    let plan = TopologicalEntropyTracker::plan_partitions(&samples, 5000, 200).expect("planning should succeed");

    assert!(plan.slice_count() > 1);
    assert!(plan.is_balanced());

    // Verify key boundaries are strictly disjoint and monotonic
    for window in plan.slices.windows(2) {
        assert!(window[0].end_key < window[1].start_key);
        assert!(window[0].start_key <= window[0].end_key);
    }

    // Verify error conditions
    assert_eq!(
        TopologicalEntropyTracker::plan_partitions(&[], 5000, 200),
        Err(PartitionError::EmptySamples)
    );
    assert_eq!(
        TopologicalEntropyTracker::plan_partitions(&samples, 0, 200),
        Err(PartitionError::InvalidTargetBytes)
    );
    assert_eq!(
        TopologicalEntropyTracker::plan_partitions(&samples, 5000, 1000),
        Err(PartitionError::InvalidEpsilonPermille(1000))
    );
}

#[test]
fn test_enospc_drain_headroom_governor() {
    // 100MB capacity, 10MB free, 4MB emergency reserve, 6MB client stall threshold
    let governor = DrainHeadroomGovernor::new(
        100 * 1024 * 1024,
        10 * 1024 * 1024,
        4 * 1024 * 1024,
        6 * 1024 * 1024,
    );

    assert!(governor.can_client_write());

    // Client requests 5MB: remaining would be 5MB (< 6MB stall threshold) -> Stalled!
    let outcome = governor.request_allocation(AllocationPriority::ClientIngest, 5 * 1024 * 1024);
    match outcome {
        AllocationOutcome::StalledClient { available_bytes, .. } => {
            assert_eq!(available_bytes, 10 * 1024 * 1024);
        }
        _ => panic!("client should be stalled before dipping below threshold"),
    }

    // Emergency Drain requests 3MB: granted because drain can dip below stall threshold!
    let drain_outcome = governor.request_allocation(AllocationPriority::EmergencyReclaimDrain, 3 * 1024 * 1024);
    match drain_outcome {
        AllocationOutcome::Granted { granted_bytes, .. } => {
            assert_eq!(granted_bytes, 3 * 1024 * 1024);
        }
        _ => panic!("emergency drain should be granted"),
    }

    assert_eq!(governor.available_bytes(), 7 * 1024 * 1024);

    // Emergency Drain requests another 2MB: dips into emergency reserve (leaving 5MB > 4MB)!
    let drain_outcome2 = governor.request_allocation(AllocationPriority::EmergencyReclaimDrain, 2 * 1024 * 1024);
    assert!(matches!(drain_outcome2, AllocationOutcome::Granted { .. }));
    assert_eq!(governor.available_bytes(), 5 * 1024 * 1024);

    // Client write is definitely stalled
    assert!(!governor.can_client_write());

    // Release ticket and reclaim 40MB net freed space
    let new_avail = governor.release_and_reclaim(2 * 1024 * 1024, 40 * 1024 * 1024);
    assert_eq!(new_avail, 47 * 1024 * 1024);
    assert!(governor.can_client_write());
}

#[test]
fn test_non_zeno_monotonic_clock_and_ttl() {
    let clock = NonZenoLogicalClock::new(1, 1_000_000);

    // Advance clock observing 1_500_000 ns
    let ts1 = clock.tick(1_500_000);
    assert_eq!(ts1.epoch, 1);
    assert_eq!(ts1.logical_seq, 1);
    assert_eq!(ts1.monotonic_tick_ns, 1_500_000);

    // NTP Step-Back: external wall clock jumps backwards to 800_000 ns!
    let ts2 = clock.tick(800_000);
    assert_eq!(ts2.epoch, 1);
    assert_eq!(ts2.logical_seq, 2);
    // Non-Zenonian advance: must be strictly > ts1.monotonic_tick_ns!
    assert!(ts2.monotonic_tick_ns > ts1.monotonic_tick_ns);
    assert_eq!(ts2.monotonic_tick_ns, 1_500_001);

    // Visibility testing
    assert!(TtlVisibilityOracle::is_visible(ts1, ts2));
    assert!(!TtlVisibilityOracle::is_visible(ts2, ts1));

    // TTL Expiration and Non-Resurrection testing
    let ttl_duration_ns = 500_000;
    // Record at ts1 (1_500_000), deadline is 2_000_000
    assert!(!TtlVisibilityOracle::is_expired(ts1, ts2, ttl_duration_ns));

    // Advance clock past expiration
    let ts3 = clock.tick(2_100_000);
    assert!(TtlVisibilityOracle::is_expired(ts1, ts3, ttl_duration_ns));

    // Even if external clock proposes past time, subsequent tick never revokes expiration
    let ts4 = clock.tick(100_000);
    assert!(ts4 > ts3);
    assert!(TtlVisibilityOracle::is_expired(ts1, ts4, ttl_duration_ns));
}

#[test]
fn test_dma_generation_fence() {
    let token = FileGenerationToken::new([0x42; 16], 10, 100);
    let payload = b"pedradb_dma_zero_ghost_block_content";

    // Seal authoritative block
    let sealed = DmaGenerationFence::seal_dma_block(&token, 5, payload);

    // Valid verification
    let verified = DmaGenerationFence::verify_dma_block(&token, 5, &sealed).expect("should verify clean block");
    assert_eq!(verified, payload);

    // Wrong block index rejection
    let err_idx = DmaGenerationFence::verify_dma_block(&token, 6, &sealed);
    assert!(matches!(err_idx, Err(FenceRejection::BlockIndexMismatch { expected: 6, actual: 5 })));

    // File generation mismatch rejection (simulating concurrent compaction recycling file)
    let stale_token = FileGenerationToken::new([0x42; 16], 11, 100);
    let err_gen = DmaGenerationFence::verify_dma_block(&stale_token, 5, &sealed);
    assert!(matches!(err_gen, Err(FenceRejection::GenerationMismatch { expected: 11, actual: 10 })));

    // File UUID mismatch rejection
    let other_file_token = FileGenerationToken::new([0x99; 16], 10, 100);
    let err_uuid = DmaGenerationFence::verify_dma_block(&other_file_token, 5, &sealed);
    assert!(matches!(err_uuid, Err(FenceRejection::FileUuidMismatch { .. })));

    // Corrupted payload rejection
    let mut corrupted = sealed.clone();
    let last = corrupted.len() - 1;
    corrupted[last] ^= 0xff;
    let err_crc = DmaGenerationFence::verify_dma_block(&token, 5, &corrupted);
    assert!(matches!(err_crc, Err(FenceRejection::ChecksumMismatch { .. })));
}

#[test]
fn test_wal_crypto_chain_recovery() {
    // Generate 4 chained WAL records
    let mut raw_wal = Vec::new();
    let mut prev_hash = WAL_GENESIS_SEED;

    let records_data = vec![
        (1u64, b"tx1_op_put_key1".to_vec()),
        (2u64, b"tx2_op_put_key2".to_vec()),
        (3u64, b"tx3_op_delete_key1".to_vec()),
        (4u64, b"tx4_op_merge_key3".to_vec()),
    ];

    for (seq, payload) in &records_data {
        let (encoded, rec_hash) = WalCryptoChainRecovery::encode_chained_record(prev_hash, *seq, payload);
        raw_wal.extend_from_slice(&encoded);
        prev_hash = rec_hash;
    }

    // 1. Recover clean WAL
    let report = WalCryptoChainRecovery::recover_longest_valid_prefix(&raw_wal, WAL_GENESIS_SEED);
    assert_eq!(report.status, WalRecoveryStatus::CleanEof);
    assert_eq!(report.recovered_records.len(), 4);
    assert_eq!(report.recovered_records[0].seq_num, 1);
    assert_eq!(report.recovered_records[3].seq_num, 4);

    // 2. Corrupt record 3 (bytes in the middle), leaving record 4 untouched (out-of-order NVMe write fault)
    let (rec0_enc, _) = WalCryptoChainRecovery::encode_chained_record(WAL_GENESIS_SEED, 1, &records_data[0].1);
    let (_rec1_enc, _h1) = WalCryptoChainRecovery::encode_chained_record(
        crc32c::crc32c_append(WAL_GENESIS_SEED, &1u64.to_le_bytes()),
        2,
        &records_data[1].1,
    );
    // Find offset of record 2 in the raw buffer
    let mut faulty_wal = raw_wal.clone();
    // Tamper with record 3's header hash
    let offset_rec3 = rec0_enc.len() + 10; // offset inside record 2
    faulty_wal[offset_rec3] ^= 0xaa;

    let faulty_report = WalCryptoChainRecovery::recover_longest_valid_prefix(&faulty_wal, WAL_GENESIS_SEED);
    // Must stop deterministically at maximum contiguous prefix, never recovering record 4!
    match faulty_report.status {
        WalRecoveryStatus::DeterministicFailStop { record_index, reason, .. } => {
            assert!(record_index <= 2);
            assert!(matches!(
                reason,
                ChainBreachReason::RecordChecksumMismatch { .. }
                    | ChainBreachReason::HashChainDiscontinuity { .. }
            ));
        }
        _ => panic!("recovery must fail-stop on corrupted intermediate block"),
    }
}
