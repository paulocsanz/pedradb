//! RFC-0292: Suíte de Verificação das Dez Fronteiras Matemáticas, Físicas e Estruturais do LSM PedraDB.
//!
//! Cobre exaustivamente:
//! 1. Homomorfismo Topológico de Normalização Canônica e Espaço Quociente
//! 2. Estabilidade de Foster-Lyapunov sob Ruído Estocástico de Lévy/Pareto
//! 3. Bisimulação Causal Trans-Crash sob Reconstrução MANIFEST + WAL
//! 4. Semianel com Aniquilador de Range Deletes e Merges Concorrentes
//! 5. Invariante de Bounded Extent Dispersal no VFS
//! 6. Quiescência Atômica de Geração em MemTables Concorrentes
//! 7. Coerência Release-Acquire Trans-Thread na Publicação de Buffers
//! 8. Homomorfismo de Medidas em Metadados de SST
//! 9. Aciclicidade e Liveness da Rede de Petri dos Background Workers
//! 10. Invariante Anti-Amnésia contra Reversão de Blocos da FTL NVMe

use pedradb_core::atomic_generation_quiescence_kernel::{
    GenerationQuiescenceOracle, GenerationState, MemTableGeneration,
};
use pedradb_core::extent_dispersal_vfs_kernel::VfsExtentManager;
use pedradb_core::foster_lyapunov_stochastic_stall_kernel::{
    FosterLyapunovConfig, FosterLyapunovController, LsmDynamicState,
};
use pedradb_core::ftl_anti_amnesia_token_kernel::{
    FtlAmnesiaViolation, FtlAntiAmnesiaOracle, FtlAntiAmnesiaToken, PhysicalMediaBlock,
};
use pedradb_core::key_space_quotient_kernel::{
    AsciiCaseInsensitiveNormalizer, QuotientBlockIndex, QuotientBlockMeta,
    QuotientBloomFilter,
};
use pedradb_core::petri_net_background_liveness_kernel::{
    BackgroundWorkerPetriNet, PetriMarking,
};
use pedradb_core::range_delete_merge_semiring_kernel::{
    PointMutation, RangeMergeSemiringEvaluator, RangeTombstone,
};
use pedradb_core::release_acquire_cache_coherence_kernel::{
    BlockCacheSlot, DecompressedBlockEntry, ReleaseAcquireCoherenceOracle,
};
use pedradb_core::sst_metadata_measure_monoid_kernel::{
    SstMeasureMonoidOracle, SstMetadataMeasure,
};
use pedradb_core::trans_crash_bisimulation_kernel::{
    CausalHistoryPoset, TransactionRecord, TransCrashBisimulationOracle, TransCrashViolation,
};

// -----------------------------------------------------------------------------
// 1. Homomorfismo Topológico de Normalização Canônica e Espaço Quociente
// -----------------------------------------------------------------------------
#[test]
fn test_key_space_quotient_bloom_and_partition_homomorphism() {
    let normalizer = AsciiCaseInsensitiveNormalizer;
    let mut bloom = QuotientBloomFilter::new(512, 4);

    // Inserção da chave com espaços e maiúsculas
    let raw1 = b" User:JohnDoe@Example.com ";
    let raw2 = b"user:johndoe@example.com";
    bloom.insert(&normalizer, raw1);

    // Ambas as chaves equivalentes devem ser encontradas com o mesmo hash canônico
    assert!(bloom.may_contain(&normalizer, raw1));
    assert!(bloom.may_contain(&normalizer, raw2));

    // Particionamento de blocos respeitando as fronteiras canônicas
    let blocks = vec![
        QuotientBlockMeta {
            block_index: 0,
            min_canonical_key: b"user:a".to_vec(),
            max_canonical_key: b"user:m".to_vec(),
            record_count: 50,
        },
        QuotientBlockMeta {
            block_index: 1,
            min_canonical_key: b"user:n".to_vec(),
            max_canonical_key: b"user:z".to_vec(),
            record_count: 50,
        },
    ];

    let index = QuotientBlockIndex::build(blocks);
    let sample_pairs: &[(&[u8], &[u8])] = &[
        (b" USER:ALICE ", b"user:alice"),
        (b" User:JohnDoe@Example.com ", b"user:johndoe@example.com"),
    ];

    let verify_res = index.verify_quotient_invariants(&normalizer, sample_pairs);
    assert!(verify_res.is_ok(), "Invariantes quocientes devem ser preservadas");

    // Roteamento de chave em qualquer variação de caixa/espaço
    let routed = index.route(&normalizer, b" USER:BOB ");
    assert_eq!(routed, Some(0));
}

// -----------------------------------------------------------------------------
// 2. Estabilidade de Foster-Lyapunov sob Ruído Estocástico de Lévy/Pareto
// -----------------------------------------------------------------------------
#[test]
fn test_foster_lyapunov_stochastic_stall_stability() {
    let config = FosterLyapunovConfig {
        compact_memtable_bytes: 32 * 1024 * 1024,
        compact_l0_files: 4,
        gamma: 0.5,
        drift_epsilon: 0.02,
        drain_capacity_bytes: 16 * 1024 * 1024,
        drain_capacity_l0: 1,
    };
    let controller = FosterLyapunovController::new(config);

    // Estado sob pressão extrema (fora do conjunto compacto)
    let overloaded_state = LsmDynamicState {
        memtable_bytes: 96 * 1024 * 1024,
        l0_files: 10,
    };
    assert!(!controller.is_in_compact_set(overloaded_state));

    let decision = controller.evaluate_admission(overloaded_state);
    assert!(decision.admission_fraction < 1.0, "Admissão deve ser estrangulada");

    // Amostras de rajadas com cauda pesada (Pareto-like bursts)
    let arrival_bursts = vec![
        1 * 1024 * 1024,
        2 * 1024 * 1024,
        50 * 1024 * 1024, // Rajada extrema de cauda
        4 * 1024 * 1024,
    ];

    let drift = controller
        .verify_step_lyapunov(overloaded_state, &arrival_bursts)
        .expect("A deriva de Foster-Lyapunov deve ser estritamente negativa");
    assert!(drift < 0.0, "Deriva estocástica deve atrair o sistema de volta para o conjunto compacto C");
}

// -----------------------------------------------------------------------------
// 3. Bisimulação Causal Trans-Crash sob Reconstrução MANIFEST + WAL
// -----------------------------------------------------------------------------
#[test]
fn test_trans_crash_causal_bisimulation_preservation() {
    let mut pre_crash = CausalHistoryPoset::default();
    pre_crash.record_transaction(TransactionRecord {
        tx_id: 1,
        seq: 100,
        keys: vec![b"keyA".to_vec()],
        is_fsynced_and_acknowledged: true,
    });
    pre_crash.record_transaction(TransactionRecord {
        tx_id: 2,
        seq: 101,
        keys: vec![b"keyA".to_vec(), b"keyB".to_vec()],
        is_fsynced_and_acknowledged: true,
    });
    // Transação não confirmada que sofreu torn-write no crash
    pre_crash.record_transaction(TransactionRecord {
        tx_id: 3,
        seq: 102,
        keys: vec![b"keyC".to_vec()],
        is_fsynced_and_acknowledged: false,
    });

    // Pós-crash: tx 1 e 2 foram recuperadas perfeitamente; tx 3 (un-synced) foi truncada
    let mut post_crash = CausalHistoryPoset::default();
    post_crash.record_transaction(TransactionRecord {
        tx_id: 1,
        seq: 100,
        keys: vec![b"keyA".to_vec()],
        is_fsynced_and_acknowledged: true,
    });
    post_crash.record_transaction(TransactionRecord {
        tx_id: 2,
        seq: 101,
        keys: vec![b"keyA".to_vec(), b"keyB".to_vec()],
        is_fsynced_and_acknowledged: true,
    });

    let res = TransCrashBisimulationOracle::verify_trans_crash_bisimulation(&pre_crash, &post_crash);
    assert!(res.is_ok(), "Bisimulação causal deve passar para transações confirmadas");

    // Cenário de defeito: transação ACKed perdida pós-crash
    let empty_post = CausalHistoryPoset::default();
    let fail_res = TransCrashBisimulationOracle::verify_trans_crash_bisimulation(&pre_crash, &empty_post);
    assert!(matches!(fail_res, Err(TransCrashViolation::AcknowledgedTransactionLost { .. })));
}

// -----------------------------------------------------------------------------
// 4. Semianel com Aniquilador de Range Deletes e Merges Concorrentes
// -----------------------------------------------------------------------------
#[test]
fn test_range_delete_merge_semiring_confluence() {
    let key = b"counter:user_42";
    let snapshot_seq = 200;

    // Mutação base em L1: Put @ 50, Merge(+5) @ 60, Merge(+10) @ 70
    let l1_mutations = vec![
        PointMutation::Put { seq: 50, value: b"init".to_vec() },
        PointMutation::Merge { seq: 60, operand: b"m1".to_vec() },
        PointMutation::Merge { seq: 70, operand: b"m2".to_vec() },
    ];

    // Range tombstone em L0 cobrindo a chave @ 100: deve aniquilar 50, 60, 70!
    let l0_range_tombstones = vec![RangeTombstone {
        start_key: b"counter:".to_vec(),
        end_key: b"counter;".to_vec(),
        seq: 100,
    }];

    // Mutações posteriores ao range tombstone em L0: Merge(+20) @ 120
    let l0_mutations = vec![
        PointMutation::Merge { seq: 120, operand: b"m3".to_vec() },
    ];

    let conf_res = RangeMergeSemiringEvaluator::verify_range_merge_confluence(
        key,
        snapshot_seq,
        &l0_mutations,
        &l0_range_tombstones,
        &l1_mutations,
    );
    assert!(conf_res.is_ok(), "Semianel de range delete com merges deve ser confluente");

    // Leitura direta: o valor deve ser apenas "m3" (o anterior foi aniquilado pelo range delete)
    let read_val = RangeMergeSemiringEvaluator::evaluate_read_time(
        key,
        snapshot_seq,
        &[PointMutation::Merge { seq: 120, operand: b"m3".to_vec() }],
        &l0_range_tombstones,
    );
    assert_eq!(read_val, Some(b"m3".to_vec()));
}

// -----------------------------------------------------------------------------
// 5. Invariante de Bounded Extent Dispersal no VFS
// -----------------------------------------------------------------------------
#[test]
fn test_extent_dispersal_vfs_coalescence_and_bounds() {
    let total_bytes = 16 * 1024 * 1024; // 16 MiB
    let min_chunk = 2 * 1024 * 1024;   // 2 MiB
    let mut vfs = VfsExtentManager::new(total_bytes, min_chunk);

    // Punch hole de 2 MiB no meio: [4 MiB, 6 MiB)
    vfs.punch_hole(4 * 1024 * 1024, 2 * 1024 * 1024);
    assert_eq!(vfs.extents().len(), 3); // Dados, Buraco, Dados

    // Punch hole adjacente: [6 MiB, 8 MiB) -> deve coalescer com o anterior!
    vfs.punch_hole(6 * 1024 * 1024, 2 * 1024 * 1024);
    assert_eq!(vfs.extents().len(), 3, "Buracos adjacentes devem coalescer");

    let verify_res = vfs.verify_extent_dispersal_invariants();
    assert!(verify_res.is_ok(), "Invariante de dispersão de extents deve ser preservada");
}

// -----------------------------------------------------------------------------
// 6. Quiescência Atômica de Geração em MemTables Concorrentes
// -----------------------------------------------------------------------------
#[test]
fn test_atomic_generation_quiescence_pipeline() {
    let mut gen = MemTableGeneration::new(1);

    // 2 escritores adquirem tickets na geração 1
    let ticket1 = gen.acquire_write_ticket().unwrap();
    let ticket2 = gen.acquire_write_ticket().unwrap();

    // Inicia processo de congelamento
    gen.begin_quiescence();
    assert_eq!(gen.state, GenerationState::Quiescing);
    assert!(!gen.try_quiesce(), "Não pode transicionar enquanto houver tickets ativos");

    // Escritor 1 conclui
    gen.record_insertion(b"key1".to_vec(), 10);
    drop(ticket1);
    assert!(!gen.try_quiesce());

    // Escritor 2 conclui
    gen.record_insertion(b"key2".to_vec(), 11);
    drop(ticket2);
    assert!(gen.try_quiesce(), "Deve quiescer imediatamente quando o contador atinge zero");
    assert_eq!(gen.state, GenerationState::Quiesced);

    // Validação de entrega para o flusher
    let sst_keys = vec![(b"key1".to_vec(), 10), (b"key2".to_vec(), 11)];
    let res = GenerationQuiescenceOracle::verify_flush_safety(&gen, &sst_keys);
    assert!(res.is_ok(), "Todas as chaves da geração devem estar no SST");
}

// -----------------------------------------------------------------------------
// 7. Coerência Release-Acquire Trans-Thread na Publicação de Buffers
// -----------------------------------------------------------------------------
#[test]
fn test_release_acquire_cache_coherence_contract() {
    let mut slot = BlockCacheSlot::empty(42);
    assert!(slot.try_acquire_entry().is_none(), "Slot vazio não deve retornar dados");

    let payload = vec![0xca, 0xfe, 0xba, 0xbe];
    let entry = DecompressedBlockEntry {
        block_id: 42,
        payload: payload.clone(),
        crc32c: 0x12345678,
    };

    // Publica com Release
    slot.publish_entry(entry, 100);

    // Leitor obtém com Acquire
    let res = ReleaseAcquireCoherenceOracle::verify_coherence_contract(&slot, payload.len());
    assert!(res.is_ok(), "Contrato de memória fraca Release-Acquire deve ser satisfeito");

    let (acquired, epoch) = slot.try_acquire_entry().unwrap();
    assert_eq!(acquired.payload, payload);
    assert_eq!(epoch, 100);
}

// -----------------------------------------------------------------------------
// 8. Homomorfismo de Medidas em Metadados de SST
// -----------------------------------------------------------------------------
#[test]
fn test_sst_metadata_measure_monoid_and_slice_conservation() {
    let m1 = SstMetadataMeasure {
        raw_data_bytes: 4096,
        record_count: 100,
        tombstone_count: 5,
        obsolete_bytes: 512,
    };
    let m2 = SstMetadataMeasure {
        raw_data_bytes: 8192,
        record_count: 200,
        tombstone_count: 10,
        obsolete_bytes: 1024,
    };
    let m3 = SstMetadataMeasure {
        raw_data_bytes: 16384,
        record_count: 400,
        tombstone_count: 20,
        obsolete_bytes: 2048,
    };

    // Axiomas de monoide
    let monoid_res = SstMeasureMonoidOracle::verify_monoid_axioms(m1, m2, m3);
    assert!(monoid_res.is_ok(), "Monoide de medidas deve satisfazer axiomas de adição");

    // Conservação de fatias de sub-compactação
    let original = m1.combine(m2).combine(m3);
    let slices = vec![m1, m2, m3];
    let slice_res = SstMeasureMonoidOracle::verify_slice_conservation(original, &slices);
    assert!(slice_res.is_ok(), "A soma das fatias deve ser rigorosamente igual ao total original");
}

// -----------------------------------------------------------------------------
// 9. Aciclicidade e Liveness da Rede de Petri dos Background Workers
// -----------------------------------------------------------------------------
#[test]
fn test_petri_net_background_worker_liveness_and_deadlock_freedom() {
    let petri_net = BackgroundWorkerPetriNet::default();

    // Marcação inicial M0 com recursos suficientes e tarefas pendentes
    let initial_marking = PetriMarking {
        p_fd_available: 8,
        p_quota_available: 4,
        p_manifest_lock: 1,
        p_flush_tasks: 2,
        p_compact_tasks: 1,
        p_blob_gc_tasks: 2,
    };

    let explored_states = petri_net
        .verify_liveness_and_deadlock_freedom(initial_marking, 1000)
        .expect("A rede de Petri de tarefas de background deve ser livre de deadlocks");
    assert!(explored_states > 0, "Estados alcançáveis devem ser explorados");
}

// -----------------------------------------------------------------------------
// 10. Invariante Anti-Amnésia contra Reversão de Blocos da FTL NVMe
// -----------------------------------------------------------------------------
#[test]
fn test_ftl_anti_amnesia_token_and_rollback_detection() {
    let boot_uuid = 0x0123456789abcdef_fedcba9876543210u128;
    let file_id = 999;
    let block_index = 0;
    let active_min_seq = 500;

    // Bloco legítimo persistido na geração 550 >= 500
    let token = FtlAntiAmnesiaToken::new(boot_uuid, 1, 550, block_index);
    let block = PhysicalMediaBlock {
        token,
        payload: vec![1, 2, 3, 4],
    };

    let verify_res = FtlAntiAmnesiaOracle::verify_block_provenance(file_id, &block, boot_uuid, active_min_seq);
    assert!(verify_res.is_ok(), "Bloco genuíno deve ser aceito");

    // Cenário de defeito: a FTL sofreu rollback para uma geração anterior reciclada (seq = 450 < 500)
    let stale_token = FtlAntiAmnesiaToken::new(boot_uuid, 1, 450, block_index);
    let stale_block = PhysicalMediaBlock {
        token: stale_token,
        payload: vec![1, 2, 3, 4],
    };

    let fail_res = FtlAntiAmnesiaOracle::verify_block_provenance(file_id, &stale_block, boot_uuid, active_min_seq);
    assert!(
        matches!(fail_res, Err(FtlAmnesiaViolation::StaleBlockRollbackDetected { .. })),
        "Reversão da FTL deve ser prontamente detectada e isolada"
    );
}
