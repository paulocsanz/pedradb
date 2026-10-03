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
    AllocationOutcome, AllocationPriority, DrainHeadroomGovernor, EnospcConfigError,
};
use pedradb_core::non_zeno_monotonic_clock_kernel::{
    NonZenoLogicalClock, TtlPolicy, TtlVisibilityOracle,
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
fn test_sst_topological_entropy_discrepancy_and_slice_routing_red() {
    // 1. Rejeição de amostras com peso zero
    let samples_with_zero = vec![
        KeySamplePoint::new(b"key:01".to_vec(), 100),
        KeySamplePoint::new(b"key:02".to_vec(), 0), // zero weight
    ];
    let zero_res = TopologicalEntropyTracker::plan_partitions_verified(&samples_with_zero, 5000, 200);
    assert_eq!(zero_res, Err(PartitionError::ZeroWeightSample));

    // 2. Detecção de violação de discrepância por skew violento
    // Dois itens minúsculos (100b cada = 200b) seguidos de um item grande (6000b)
    // Limites para 5000 com 200 permille: [4000, 6000]
    // A primeira fatia terá 2 chaves com peso 200b (< 4000b)
    let skewed_samples = vec![
        KeySamplePoint::new(b"key:01".to_vec(), 100),
        KeySamplePoint::new(b"key:02".to_vec(), 100),
        KeySamplePoint::new(b"key:03".to_vec(), 6000),
        KeySamplePoint::new(b"key:04".to_vec(), 5000),
    ];
    let skew_res = TopologicalEntropyTracker::plan_partitions_verified(&skewed_samples, 5000, 200);
    assert!(
        matches!(skew_res, Err(PartitionError::DiscrepancyViolation { .. })),
        "Plano com discrepância patológica deve ser rejeitado fail-closed: {:?}",
        skew_res
    );

    // 3. Roteamento de chaves e busca em plano equilibrado
    let valid_samples = vec![
        KeySamplePoint::new(b"k:10".to_vec(), 2500),
        KeySamplePoint::new(b"k:20".to_vec(), 2500), // slice 0: [k:10, k:20], total 5000
        KeySamplePoint::new(b"k:30".to_vec(), 2500),
        KeySamplePoint::new(b"k:40".to_vec(), 2500), // slice 1: [k:30, k:40], total 5000
    ];
    let plan = TopologicalEntropyTracker::plan_partitions_verified(&valid_samples, 5000, 200).unwrap();
    assert_eq!(plan.slice_count(), 2);
    assert_eq!(plan.find_slice(b"k:10"), Some(0));
    assert_eq!(plan.find_slice(b"k:20"), Some(0));
    assert_eq!(plan.find_slice(b"k:30"), Some(1));
    assert_eq!(plan.find_slice(b"k:40"), Some(1));
    assert_eq!(plan.find_slice(b"k:25"), None); // Chave exata não contida em nenhuma fatia direta

    // route_key mapeia chaves intermediárias para as fatias corretas
    assert_eq!(plan.route_key(b"k:05"), Some(0)); // antes do primeiro
    assert_eq!(plan.route_key(b"k:25"), Some(0)); // entre k:20 e k:30
    assert_eq!(plan.route_key(b"k:35"), Some(1));
    assert_eq!(plan.route_key(b"k:99"), Some(1)); // após o último
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
fn test_enospc_drain_headroom_raii_lease_and_config_validation_red() {
    // 1. Validação de configuração via try_new (sem pânico)
    let bad_reserve = DrainHeadroomGovernor::try_new(100, 50, 150, 160);
    assert_eq!(
        bad_reserve.err(),
        Some(EnospcConfigError::ReserveExceedsCapacity { reserve: 150, capacity: 100 })
    );

    let bad_thresh = DrainHeadroomGovernor::try_new(100, 50, 30, 20);
    assert_eq!(
        bad_thresh.err(),
        Some(EnospcConfigError::StallThresholdBelowReserve { threshold: 20, reserve: 30 })
    );

    // 2. Lease RAII: se o processo abortar/falhar sem commit, espaço é devolvido automaticamente no Drop
    let governor = DrainHeadroomGovernor::new(
        100 * 1024 * 1024,
        10 * 1024 * 1024,
        4 * 1024 * 1024,
        6 * 1024 * 1024,
    );
    assert_eq!(governor.available_bytes(), 10 * 1024 * 1024);

    {
        let lease = governor
            .allocate_lease(AllocationPriority::EmergencyReclaimDrain, 2 * 1024 * 1024)
            .expect("deve alocar lease de 2MB");
        assert_eq!(governor.available_bytes(), 8 * 1024 * 1024);
        assert_eq!(lease.granted_bytes(), 2 * 1024 * 1024);
        // O lease é dropado sem commit_reclaim (simulando falha de I/O / panic)
    }
    // Espaço alocado foi 100% restaurado pelo Drop guard!
    assert_eq!(governor.available_bytes(), 10 * 1024 * 1024, "Drop deve restaurar espaço não-comitado");

    // 3. Commit de Lease com espaço líquido liberado
    {
        let lease = governor
            .allocate_lease(AllocationPriority::EmergencyReclaimDrain, 2 * 1024 * 1024)
            .expect("lease 2MB");
        assert_eq!(governor.available_bytes(), 8 * 1024 * 1024);

        // Commit com 20MB liberados na compactação
        let final_avail = lease.commit_reclaim(20 * 1024 * 1024);
        assert_eq!(final_avail, 30 * 1024 * 1024);
    }
    assert_eq!(governor.available_bytes(), 30 * 1024 * 1024);

    // 4. Alocação de zero bytes não queima tickets nem altera espaço
    let zero_outcome = governor.request_allocation(AllocationPriority::ClientIngest, 0);
    assert_eq!(
        zero_outcome,
        AllocationOutcome::Granted {
            granted_bytes: 0,
            ticket_id: 0,
        }
    );
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
fn test_non_zeno_clock_epoch_advance_and_cross_epoch_ttl_green() {
    let clock = NonZenoLogicalClock::new(1, 1_000_000);
    let record_ts = clock.tick(1_500_000); // Epoch 1, tick 1_500_000

    // 1. remaining_ttl_ns
    let fresh_clock = clock.tick(1_600_000);
    // TTL de 500_000ns: deadline é 2_000_000ns. Em 1_600_000ns restam 400_000ns.
    assert_eq!(
        TtlVisibilityOracle::remaining_ttl_ns(record_ts, fresh_clock, 500_000),
        Some(400_000)
    );

    // 2. Em 2_500_000ns já expirou, retorna None
    let expired_clock = clock.tick(2_500_000);
    assert_eq!(
        TtlVisibilityOracle::remaining_ttl_ns(record_ts, expired_clock, 500_000),
        None
    );

    // 3. TtlPolicy::NoExpiration nunca expira mesmo após TTL ter passado
    assert!(!TtlVisibilityOracle::is_expired_policy(record_ts, expired_clock, TtlPolicy::NoExpiration));

    // 4. Avanço atômico de epoch no relógio
    clock.advance_epoch(2, 2_600_000);
    assert_eq!(clock.epoch(), 2);
    let epoch2_ts = clock.tick(2_700_000);
    assert_eq!(epoch2_ts.epoch, 2);

    // 5. TTL através de épocas: com tempo contínuo mantido no cluster, chave não deve morrer prematuramente
    // record_ts @ 1_500_000 com TTL de 2_000_000ns (expira em 3_500_000ns)
    // No epoch 2 @ 2_700_000ns ainda faltam 800_000ns
    assert!(
        !TtlVisibilityOracle::is_expired_cross_epoch(record_ts, epoch2_ts, 2_000_000, true),
        "Chave válida com tempo contínuo entre reinicializações não deve expirar prematuramente"
    );
    // Em 4_000_000ns no epoch 2, agora sim expira
    let epoch2_late = clock.tick(4_000_000);
    assert!(
        TtlVisibilityOracle::is_expired_cross_epoch(record_ts, epoch2_late, 2_000_000, true)
    );

    // 6. NoExpiration continua válido mesmo no futuro distante e no epoch 2
    let far_future_ts = clock.tick(999_999_999);
    assert!(!TtlVisibilityOracle::is_expired_policy(record_ts, far_future_ts, TtlPolicy::NoExpiration));
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
fn test_dma_generation_fence_zero_ghost_hole_and_slice_verification_green() {
    let token = FileGenerationToken::new([0x42; 16], 10, 100);
    let payload = b"critical_column_family_index_block_payload";

    // 1. Zero-copy slice verification sem alocação de Vec
    let sealed = DmaGenerationFence::seal_dma_block(&token, 3, payload);
    let slice_verified = DmaGenerationFence::verify_dma_block_slice(&token, 3, &sealed)
        .expect("deve verificar fatia zero-copy");
    assert_eq!(slice_verified, payload);

    // 2. Buffer Direct I/O alinhado a setor (ex: 4096 bytes com padding de zeros ao final)
    let mut direct_io_sector = vec![0u8; 4096];
    direct_io_sector[..sealed.len()].copy_from_slice(&sealed);
    // Deve verificar com comprimento exato do payload sem falhar com ChecksumMismatch no padding
    let padded_verified = DmaGenerationFence::verify_dma_block_with_len(
        &token,
        3,
        &direct_io_sector,
        payload.len(),
    ).expect("deve validar bloco com padding de setor Direct I/O");
    assert_eq!(padded_verified, payload);

    // 3. Furo de NVMe (bloco inteiramente zerado) deve ser rejeitado como ZeroGhostHeader,
    // mesmo se consultado com token inválido/zerado e bloco 0
    let zero_hole = vec![0u8; 4096];
    let zero_token = FileGenerationToken::new([0u8; 16], 0, 0);
    let err_hole = DmaGenerationFence::verify_dma_block(&zero_token, 0, &zero_hole);
    assert_eq!(err_hole, Err(FenceRejection::InvalidToken));

    // Com token válido, bloco zerado falha com ZeroGhostHeader
    let err_valid_token_hole = DmaGenerationFence::verify_dma_block(&token, 3, &zero_hole);
    assert_eq!(err_valid_token_hole, Err(FenceRejection::ZeroGhostHeader));

    // 4. Payload vazio é rejeitado
    let empty_sealed = DmaGenerationFence::seal_dma_block(&token, 4, &[]);
    let err_empty = DmaGenerationFence::verify_dma_block(&token, 4, &empty_sealed);
    assert_eq!(err_empty, Err(FenceRejection::EmptyPayload));
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

#[test]
fn test_wal_crypto_chain_zero_hole_with_trailing_records_fails_stop() {
    let mut raw_wal = Vec::new();
    let prev_hash = WAL_GENESIS_SEED;

    let (rec0_enc, rec0_hash) = WalCryptoChainRecovery::encode_chained_record(prev_hash, 1, b"first_record");
    raw_wal.extend_from_slice(&rec0_enc);

    // Hole: 20 zero bytes
    raw_wal.extend_from_slice(&[0u8; 20]);

    // Subsequent live record after hole
    let (rec1_enc, _) = WalCryptoChainRecovery::encode_chained_record(rec0_hash, 2, b"second_record");
    raw_wal.extend_from_slice(&rec1_enc);

    let report = WalCryptoChainRecovery::recover_longest_valid_prefix(&raw_wal, WAL_GENESIS_SEED);
    match report.status {
        WalRecoveryStatus::DeterministicFailStop { reason, .. } => {
            assert!(
                matches!(reason, ChainBreachReason::ZeroHeaderWithTrailingBytes { .. }),
                "expected ZeroHeaderWithTrailingBytes, got {:?}",
                reason
            );
        }
        WalRecoveryStatus::CleanEof => {
            panic!("Zero hole with trailing live records must NOT be treated as CleanEof! This silently drops committed records!");
        }
    }
}

#[test]
fn test_wal_crypto_chain_max_seq_and_zero_copy_slices_green() {
    // 1. Início de recuperação com min_expected_seq customizado (ex: segmento rotacionado WAL-00042)
    let (rec, rec_hash) = WalCryptoChainRecovery::encode_chained_record(
        WAL_GENESIS_SEED,
        100_000,
        b"rotated_segment_first_record",
    );
    let report_custom = WalCryptoChainRecovery::recover_longest_valid_prefix_from(
        &rec,
        WAL_GENESIS_SEED,
        100_000,
    );
    assert_eq!(report_custom.status, WalRecoveryStatus::CleanEof);
    assert_eq!(report_custom.recovered_records.len(), 1);
    assert_eq!(report_custom.recovered_records[0].seq_num, 100_000);

    // 2. Proteção contra overflow aritmético em seq_num == u64::MAX
    let (max_rec, _) = WalCryptoChainRecovery::encode_chained_record(
        rec_hash,
        u64::MAX,
        b"boundary_record_max_u64",
    );
    let mut chained_buf = rec.clone();
    chained_buf.extend_from_slice(&max_rec);
    let report_overflow = WalCryptoChainRecovery::recover_longest_valid_prefix_from(
        &chained_buf,
        WAL_GENESIS_SEED,
        100_000,
    );
    assert_eq!(report_overflow.status, WalRecoveryStatus::CleanEof);
    assert_eq!(report_overflow.recovered_records.len(), 2);
    assert_eq!(report_overflow.recovered_records[1].seq_num, u64::MAX);

    // 3. Zero-copy slices recovery (sem clones no heap de Vec<u8> para cada payload)
    let report_slices = WalCryptoChainRecovery::recover_longest_valid_prefix_slices(
        &chained_buf,
        WAL_GENESIS_SEED,
        100_000,
    );
    assert_eq!(report_slices.status, WalRecoveryStatus::CleanEof);
    assert_eq!(report_slices.recovered_records.len(), 2);
    assert_eq!(
        report_slices.recovered_records[0].payload,
        b"rotated_segment_first_record"
    );
    assert_eq!(
        report_slices.recovered_records[1].payload,
        b"boundary_record_max_u64"
    );
}

#[test]
fn test_wal_crypto_chain_red_invariants() {
    // 1. try_encode_chained_record rejects sequence 0
    assert_eq!(
        WalCryptoChainRecovery::try_encode_chained_record(WAL_GENESIS_SEED, 0, b"zero_seq").err(),
        Some(ChainBreachReason::SequenceNumberRegression { expected_min: 1, got_seq: 0 })
    );

    // 2. try_encode_chained_record rejects payload > MAX_WAL_RECORD_PAYLOAD_LEN
    let oversized = vec![0x42u8; 65 * 1024 * 1024]; // 65MB > 64MB limit
    assert_eq!(
        WalCryptoChainRecovery::try_encode_chained_record(WAL_GENESIS_SEED, 1, &oversized).err(),
        Some(ChainBreachReason::PayloadLengthExceeded {
            payload_len: 65 * 1024 * 1024,
            remaining: 64 * 1024 * 1024,
        })
    );

    // 3. Strict sequence monotonicity: duplicate u64::MAX is rejected!
    let (rec1, h1) = WalCryptoChainRecovery::try_encode_chained_record(WAL_GENESIS_SEED, u64::MAX, b"rec1").unwrap();
    let (rec2, _) = WalCryptoChainRecovery::try_encode_chained_record(h1, u64::MAX, b"rec2").unwrap();
    let mut buf = rec1;
    buf.extend_from_slice(&rec2);

    let report = WalCryptoChainRecovery::recover_longest_valid_prefix(&buf, WAL_GENESIS_SEED);
    match report.status {
        WalRecoveryStatus::DeterministicFailStop { reason, record_index, .. } => {
            assert_eq!(record_index, 1);
            assert!(matches!(reason, ChainBreachReason::SequenceNumberRegression { .. }));
        }
        WalRecoveryStatus::CleanEof => panic!("Duplicate u64::MAX record must NOT be accepted as CleanEof!"),
    }
}

