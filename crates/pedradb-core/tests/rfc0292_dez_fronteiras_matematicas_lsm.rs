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
    GenerationQuiescenceOracle, GenerationState, MemTableGeneration, QuiescenceViolation,
};
use pedradb_core::extent_dispersal_vfs_kernel::{
    ExtentDispersalViolation, VfsExtentManager,
};
use pedradb_core::foster_lyapunov_stochastic_stall_kernel::{
    FosterLyapunovConfig, FosterLyapunovController, LsmDynamicState,
};
use pedradb_core::ftl_anti_amnesia_token_kernel::{
    FtlAmnesiaViolation, FtlAntiAmnesiaOracle, FtlAntiAmnesiaToken, PhysicalMediaBlock,
};
use pedradb_core::key_space_quotient_kernel::{
    AsciiCaseInsensitiveNormalizer, CanonicalNormalizer, QuotientBlockIndex, QuotientBlockMeta,
    QuotientBloomFilter, QuotientViolation,
};
use pedradb_core::petri_net_background_liveness_kernel::{
    BackgroundWorkerPetriNet, PetriMarking, PetriNetLivenessViolation, PetriTransition,
};
use pedradb_core::range_delete_merge_semiring_kernel::{
    PointMutation, RangeMergeSemiringEvaluator, RangeTombstone,
};
use pedradb_core::release_acquire_cache_coherence_kernel::{
    BlockCacheSlot, DecompressedBlockEntry, ReleaseAcquireCoherenceOracle,
    WeakMemoryCoherenceViolation,
};
use pedradb_core::sst_metadata_measure_monoid_kernel::{
    MeasureMonoidViolation, SstMeasureMonoidOracle, SstMetadataMeasure,
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

#[test]
fn test_key_space_quotient_edge_cases_red() {
    let normalizer = AsciiCaseInsensitiveNormalizer;

    // 1. Normalização multi-espaço e caracteres em branco (\t, \r, \n)
    let raw_multi = b"  \t  User:JohnDoe@Example.com \r\n  ";
    let raw_canonical = b"user:johndoe@example.com";
    assert_eq!(
        normalizer.project_canonical(raw_multi),
        raw_canonical.to_vec(),
        "Espaços múltiplos e caracteres em branco devem ser removidos na projeção canônica"
    );

    // 2. Não deve dar panic quando amostras de equivalência divergirem, mas retornar erro controlado
    let index = QuotientBlockIndex::build(vec![QuotientBlockMeta {
        block_index: 0,
        min_canonical_key: b"user:a".to_vec(),
        max_canonical_key: b"user:z".to_vec(),
        record_count: 10,
    }]);

    let divergent_samples: &[(&[u8], &[u8])] = &[
        (b"user:alice", b"user:bob"),
    ];
    let div_res = index.verify_quotient_invariants(&normalizer, divergent_samples);
    assert!(
        matches!(div_res, Err(QuotientViolation::NonCanonicalProjectionMismatch { .. })),
        "Amostras com projeções canônicas distintas devem falhar com NonCanonicalProjectionMismatch: {:?}",
        div_res
    );

    // 3. Convexidade interna de bloco único invertido (min > max) deve falhar
    let inverted_index = QuotientBlockIndex::build(vec![QuotientBlockMeta {
        block_index: 0,
        min_canonical_key: b"user:z".to_vec(),
        max_canonical_key: b"user:a".to_vec(),
        record_count: 10,
    }]);
    let inv_res = inverted_index.verify_quotient_invariants(&normalizer, &[]);
    assert!(
        matches!(inv_res, Err(QuotientViolation::QuotientConvexityBroken { .. })),
        "Bloco com min > max deve quebrar convexidade quociente: {:?}",
        inv_res
    );
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
        .verify_step_drift(overloaded_state, &arrival_bursts)
        .expect("A deriva de Foster-Lyapunov deve ser estritamente negativa");
    assert!(drift < 0.0, "Deriva estocástica deve atrair o sistema de volta para o conjunto compacto C");
}

#[test]
fn test_foster_lyapunov_edge_cases_red() {
    // 1. Configuração com compact limits = 0 e gamma degenerado
    let config_zero = FosterLyapunovConfig {
        compact_memtable_bytes: 0,
        compact_l0_files: 0,
        gamma: 0.0,
        drift_epsilon: 0.0,
        drain_capacity_bytes: 0,
        drain_capacity_l0: 0,
    };
    let controller_zero = FosterLyapunovController::new(config_zero);
    let state = LsmDynamicState { memtable_bytes: 100, l0_files: 5 };
    let v = controller_zero.compute_v(state);
    assert!(!v.is_nan() && v.is_finite());

    // 2. verify_step_drift com arrival_samples vazio não deve causar division by zero (NaN)
    let res = controller_zero.verify_step_drift(state, &[]);
    assert!(res.is_ok());

    // 3. evaluate_admission com v NaN ou infinito deve manter admission_fraction bounded
    let adm = controller_zero.evaluate_admission(state);
    assert!(adm.admission_fraction >= 0.0 && adm.admission_fraction <= 1.0);
    assert!(!adm.admission_fraction.is_nan());
}

#[test]
fn test_foster_lyapunov_permanent_starvation_and_multimem_red() {
    let mut config = FosterLyapunovConfig::default();
    config.drain_capacity_bytes = 0; // Sem drenagem
    config.drain_capacity_l0 = 0;
    let controller = FosterLyapunovController::new(config);

    // 1. Estado sobrecarregado persistente deve detectar inanição permanente
    let state = LsmDynamicState {
        memtable_bytes: 10 * 1024 * 1024 * 1024, // 10 GB
        l0_files: 50,
    };
    let arrivals = vec![100 * 1024 * 1024; 20]; // 20 rajadas de 100 MB
    let sim_res = controller.simulate_trajectory(state, &arrivals, 5);
    assert!(
        matches!(sim_res, Err(pedradb_core::foster_lyapunov_stochastic_stall_kernel::StochasticStallViolation::PermanentStarvationDetected { zero_admission_steps }) if zero_admission_steps > 5),
        "Inanição prolongada sem drenagem deve disparar PermanentStarvationDetected: {:?}",
        sim_res
    );
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

#[test]
fn test_trans_crash_bisimulation_edge_cases_red() {
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
        keys: vec![b"keyB".to_vec()],
        is_fsynced_and_acknowledged: false,
    });

    // 1. Transação fantasma (resurrected torn write não catalogado no pré-crash)
    let mut phantom_post = CausalHistoryPoset::default();
    phantom_post.record_transaction(TransactionRecord {
        tx_id: 1,
        seq: 100,
        keys: vec![b"keyA".to_vec()],
        is_fsynced_and_acknowledged: true,
    });
    phantom_post.record_transaction(TransactionRecord {
        tx_id: 99, // Fantasma
        seq: 102,
        keys: vec![b"ghost".to_vec()],
        is_fsynced_and_acknowledged: false,
    });
    let phantom_res = TransCrashBisimulationOracle::verify_trans_crash_bisimulation(&pre_crash, &phantom_post);
    assert!(
        matches!(phantom_res, Err(TransCrashViolation::UnpersistedTornWriteResurrected { tx_id: 99 })),
        "Transação fantasma não catalogada no pré-crash deve ser rejeitada como torn write: {:?}",
        phantom_res
    );

    // 2. Transação un-synced ressuscitada com sequence divergente
    let mut mutated_post = CausalHistoryPoset::default();
    mutated_post.record_transaction(TransactionRecord {
        tx_id: 1,
        seq: 100,
        keys: vec![b"keyA".to_vec()],
        is_fsynced_and_acknowledged: true,
    });
    mutated_post.record_transaction(TransactionRecord {
        tx_id: 2,
        seq: 999, // Sequence corrompido pós-crash
        keys: vec![b"keyB".to_vec()],
        is_fsynced_and_acknowledged: false,
    });
    let mut_res = TransCrashBisimulationOracle::verify_trans_crash_bisimulation(&pre_crash, &mutated_post);
    assert!(
        matches!(mut_res, Err(TransCrashViolation::UnpersistedTornWriteResurrected { tx_id: 2 })),
        "Transação un-synced com sequence adulterado pós-crash deve ser rejeitada: {:?}",
        mut_res
    );
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

#[test]
fn test_range_delete_merge_edge_cases_red() {
    let key = b"counter:user_42";

    // 1. Operando de merge vazio não deve causar pânico por slice::windows(0)
    let l1_empty_operand = vec![PointMutation::Merge { seq: 40, operand: vec![] }];
    let l0_rt = vec![RangeTombstone {
        start_key: b"counter:".to_vec(),
        end_key: b"counter;".to_vec(),
        seq: 100,
    }];
    let l0_mut = vec![PointMutation::Put { seq: 120, value: b"active".to_vec() }];
    let empty_op_res = RangeMergeSemiringEvaluator::verify_range_merge_confluence(
        key,
        200,
        &l0_mut,
        &l0_rt,
        &l1_empty_operand,
    );
    assert!(empty_op_res.is_ok(), "Operando de merge vazio não deve causar pânico: {:?}", empty_op_res);

    // 2. Mutações passadas fora de ordem não podem perder mutação mais recente acima do range tombstone
    let unsorted_mutations = vec![
        PointMutation::Put { seq: 40, value: b"old".to_vec() },
        PointMutation::Put { seq: 120, value: b"new".to_vec() },
    ];
    let val = RangeMergeSemiringEvaluator::evaluate_read_time(
        key,
        200,
        &unsorted_mutations,
        &l0_rt, // cobre seq <= 100
    );
    assert_eq!(
        val,
        Some(b"new".to_vec()),
        "Mutações não ordenadas devem ser avaliadas por ordem causal estrita de sequence"
    );

    // 3. Range tombstone degenerado com start_key >= end_key nunca deve cobrir chaves
    let degen_rt = RangeTombstone {
        start_key: b"z".to_vec(),
        end_key: b"a".to_vec(),
        seq: 100,
    };
    assert!(!degen_rt.covers_key(b"m"), "Range tombstone invertido não deve cobrir chaves");
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

#[test]
fn test_extent_dispersal_vfs_edge_cases_red() {
    let total_bytes = 16 * 1024 * 1024; // 16 MiB
    let min_chunk = 2 * 1024 * 1024;   // 2 MiB
    let mut vfs = VfsExtentManager::new(total_bytes, min_chunk);

    // 1. Punch hole com arithmetic overflow não deve dar panic
    vfs.punch_hole(u64::MAX - 10, 100);

    // 2. Punch hole com tamanho microscópico (< min_chunk) deve falhar na validação de invariantes
    vfs.punch_hole(4 * 1024 * 1024, 512); // Apenas 512 bytes quando min_chunk = 2 MiB
    let micro_res = vfs.verify_extent_dispersal_invariants();
    assert!(
        matches!(micro_res, Err(ExtentDispersalViolation::MicroFragmentDetected { length: 512, .. })),
        "Buraco microscópico deve ser detectado como violação: {:?}",
        micro_res
    );
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

#[test]
fn test_atomic_generation_quiescence_cross_generation_rejection() {
    let gen1 = MemTableGeneration::new(1);
    let mut gen2 = MemTableGeneration::new(2);

    let ticket1 = gen1.acquire_write_ticket().unwrap();
    assert_eq!(ticket1.generation(), 1);

    // Tentativa de usar ticket da geração 1 na geração 2 é rejeitada
    let res = gen2.record_insertion_with_ticket(&ticket1, b"rogue_key".to_vec(), 42);
    assert_eq!(
        res,
        Err(QuiescenceViolation::GenerationMismatch {
            ticket_generation: 1,
            memtable_generation: 2,
        })
    );

    // Oráculo também rejeita descompasso de ticket
    let oracle_res = GenerationQuiescenceOracle::verify_ticket_belongs_to_generation(&ticket1, &gen2);
    assert_eq!(
        oracle_res,
        Err(QuiescenceViolation::GenerationMismatch {
            ticket_generation: 1,
            memtable_generation: 2,
        })
    );

    drop(ticket1);
}

#[test]
fn test_atomic_generation_quiescence_lifecycle_monotonicity_red() {
    let mut gen = MemTableGeneration::new(10);
    let ticket = gen.acquire_write_ticket().unwrap();
    gen.record_insertion_with_ticket(&ticket, b"k1".to_vec(), 1).unwrap();
    drop(ticket);

    gen.begin_quiescence();
    assert!(gen.try_quiesce());
    assert_eq!(gen.state, GenerationState::Quiesced);

    // 1. Marcar como Flushed
    assert!(gen.mark_flushed(), "Transição Quiesced -> Flushed deve ter sucesso");
    assert_eq!(gen.state, GenerationState::Flushed);

    // 2. verify_flush_safety deve aceitar geração Flushed se todas as chaves estiverem presentes
    let sst = vec![(b"k1".to_vec(), 1)];
    let flush_res = GenerationQuiescenceOracle::verify_flush_safety(&gen, &sst);
    assert!(flush_res.is_ok(), "Geração Flushed com 0 escritores pendentes deve passar em verify_flush_safety: {:?}", flush_res);

    // 3. begin_quiescence NÃO PODE regredir de Flushed para Quiescing
    gen.begin_quiescence();
    assert_eq!(gen.state, GenerationState::Flushed, "Estado do ciclo de vida deve ser monotônico (sem reversão de Flushed)");

    // 4. Inserção em geração Flushed deve ser rejeitada
    let dummy_ticket = MemTableGeneration::new(10).acquire_write_ticket().unwrap();
    let insert_res = gen.record_insertion_with_ticket(&dummy_ticket, b"late_key".to_vec(), 99);
    assert!(insert_res.is_err(), "Inserção em geração já persistida (Flushed) deve ser rejeitada");
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

#[test]
fn test_release_acquire_cache_coherence_edge_cases_red() {
    // 1. Block ID mismatch entre slot e entry publicada
    let mut slot = BlockCacheSlot::empty(42);
    let entry_mismatched = DecompressedBlockEntry {
        block_id: 99, // Mismatched!
        payload: vec![1, 2, 3],
        crc32c: 0,
    };
    slot.publish_entry(entry_mismatched, 1);
    let mis_res = ReleaseAcquireCoherenceOracle::verify_coherence_contract(&slot, 3);
    assert!(
        matches!(mis_res, Err(WeakMemoryCoherenceViolation::BlockIdMismatch { slot_block_id: 42, entry_block_id: 99 })),
        "Divergência de block_id deve ser rejeitada: {:?}",
        mis_res
    );

    // 2. Verificação de integridade CRC32C do payload
    let entry_valid = DecompressedBlockEntry::new(100, b"valid_block_payload".to_vec());
    assert!(ReleaseAcquireCoherenceOracle::verify_entry_integrity(&entry_valid).is_ok());

    let entry_corrupted = DecompressedBlockEntry {
        block_id: 100,
        payload: b"valid_block_payload".to_vec(),
        crc32c: 0xDEADBEEF, // Checksum forjado/corrompido
    };
    let corrupt_res = ReleaseAcquireCoherenceOracle::verify_entry_integrity(&entry_corrupted);
    assert!(
        matches!(corrupt_res, Err(WeakMemoryCoherenceViolation::ChecksumMismatch { block_id: 100, .. })),
        "Checksum CRC32C corrompido deve ser detectado: {:?}",
        corrupt_res
    );
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

#[test]
fn test_sst_metadata_measure_bounds_and_axioms_red() {
    // 1. Obsolete bytes > raw data bytes é impossível e deve ser rejeitado
    let m_corrupt_bytes = SstMetadataMeasure {
        raw_data_bytes: 100,
        obsolete_bytes: 200, // 200 > 100
        record_count: 10,
        tombstone_count: 2,
    };
    assert!(!m_corrupt_bytes.is_valid());
    let res_bytes = SstMeasureMonoidOracle::verify_slice_conservation(m_corrupt_bytes, &[m_corrupt_bytes]);
    assert!(
        matches!(res_bytes, Err(MeasureMonoidViolation::InvalidMeasureBounds { .. })),
        "Metadados com obsolete_bytes > raw_data_bytes devem ser rejeitados: {:?}",
        res_bytes
    );

    // 2. Tombstone count > record count é impossível e deve ser rejeitado
    let m_corrupt_tombstones = SstMetadataMeasure {
        raw_data_bytes: 1000,
        obsolete_bytes: 100,
        record_count: 5,
        tombstone_count: 20, // 20 > 5
    };
    assert!(!m_corrupt_tombstones.is_valid());
    let res_tomb = SstMeasureMonoidOracle::verify_slice_conservation(m_corrupt_tombstones, &[m_corrupt_tombstones]);
    assert!(
        matches!(res_tomb, Err(MeasureMonoidViolation::InvalidMeasureBounds { .. })),
        "Metadados com tombstone_count > record_count devem ser rejeitados: {:?}",
        res_tomb
    );
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

#[test]
fn test_petri_net_background_capacity_bounds_and_underflow_red() {
    // 1. Marcação inicial que excede limite de capacidade configurado
    let net_bounded = BackgroundWorkerPetriNet::default().with_capacity_limit(5);
    let over_capacity_marking = PetriMarking {
        p_fd_available: 10, // 10 > 5
        p_quota_available: 2,
        p_manifest_lock: 1,
        p_flush_tasks: 0,
        p_compact_tasks: 0,
        p_blob_gc_tasks: 0,
    };
    let res = net_bounded.verify_liveness_and_deadlock_freedom(over_capacity_marking, 100);
    assert_eq!(
        res,
        Err(PetriNetLivenessViolation::CapacityLimitExceeded {
            resource: "p_fd_available",
            tokens: 10,
        })
    );

    // 2. Transição customizada com vazamento gerando tokens além do limite
    let leaking_trans = PetriTransition {
        name: "T_leaking",
        required_fd: 1,
        required_quota: 1,
        required_manifest: 0,
        required_task: "blob_gc",
        produced_fd: 6, // 1 - 1 + 6 = 6 > 5
        produced_quota: 1,
        produced_manifest: 0,
    };
    let custom_net = BackgroundWorkerPetriNet::new(vec![leaking_trans]).with_capacity_limit(5);
    let valid_start = PetriMarking {
        p_fd_available: 1,
        p_quota_available: 1,
        p_manifest_lock: 0,
        p_flush_tasks: 0,
        p_compact_tasks: 0,
        p_blob_gc_tasks: 1,
    };
    let res_leak = custom_net.verify_liveness_and_deadlock_freedom(valid_start, 100);
    assert!(
        matches!(res_leak, Err(PetriNetLivenessViolation::CapacityLimitExceeded { resource: "p_fd_available", .. })),
        "Transição com vazamento de tokens deve estourar capacidade: {:?}",
        res_leak
    );

    // 3. try_fire não deve entrar em pânico nem dar underflow quando transição desabilitada
    let disabled_trans = PetriTransition {
        name: "T_needs_much",
        required_fd: 10,
        required_quota: 10,
        required_manifest: 5,
        required_task: "flush",
        produced_fd: 10,
        produced_quota: 10,
        produced_manifest: 5,
    };
    let sparse_m = PetriMarking {
        p_fd_available: 1,
        p_quota_available: 0,
        p_manifest_lock: 0,
        p_flush_tasks: 0,
        p_compact_tasks: 0,
        p_blob_gc_tasks: 0,
    };
    let fire_opt = custom_net.try_fire(&disabled_trans, &sparse_m);
    assert_eq!(fire_opt, None, "try_fire em transição desabilitada deve retornar None sem panic");

    // 4. fire não deve causar panic por underflow
    let fire_res = custom_net.fire(&disabled_trans, &sparse_m);
    assert_eq!(fire_res, sparse_m, "fire em transição desabilitada não deve alterar marcação nem causar panic");
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

#[test]
fn test_ftl_anti_amnesia_edge_cases_and_misdirection_red() {
    let boot_uuid = 0x0123456789abcdef_fedcba9876543210u128;
    let file_id = 999;
    let block_index = 0;
    let active_min_seq = 500;

    // 1. Checksum do token corrompido
    let mut bad_token = FtlAntiAmnesiaToken::new(boot_uuid, 1, 550, block_index);
    bad_token.token_crc ^= 0xdeadbeef;
    let bad_crc_block = PhysicalMediaBlock {
        token: bad_token,
        payload: vec![1, 2, 3, 4],
    };
    let crc_res = FtlAntiAmnesiaOracle::verify_block_provenance(file_id, &bad_crc_block, boot_uuid, active_min_seq);
    assert!(
        matches!(crc_res, Err(FtlAmnesiaViolation::TokenChecksumMismatch { .. })),
        "Token com CRC corrompido deve ser rejeitado: {:?}",
        crc_res
    );

    // 2. Boot UUID de outra encarnação (Foreign Incarnation)
    let foreign_uuid = 0x9999999999999999_8888888888888888u128;
    let foreign_token = FtlAntiAmnesiaToken::new(foreign_uuid, 1, 550, block_index);
    let foreign_block = PhysicalMediaBlock {
        token: foreign_token,
        payload: vec![1, 2, 3, 4],
    };
    let foreign_res = FtlAntiAmnesiaOracle::verify_block_provenance(file_id, &foreign_block, boot_uuid, active_min_seq);
    assert!(
        matches!(foreign_res, Err(FtlAmnesiaViolation::ForeignIncarnationDetected { .. })),
        "Bloco de encarnação alienígena deve ser rejeitado: {:?}",
        foreign_res
    );

    // 3. Payload vazio (bloco corrompido ou trim não inicializado)
    let empty_payload_token = FtlAntiAmnesiaToken::new(boot_uuid, 1, 550, block_index);
    let empty_block = PhysicalMediaBlock {
        token: empty_payload_token,
        payload: vec![],
    };
    let empty_res = FtlAntiAmnesiaOracle::verify_block_provenance(file_id, &empty_block, boot_uuid, active_min_seq);
    assert_eq!(
        empty_res,
        Err(FtlAmnesiaViolation::EmptyPayloadNotAllowed),
        "Bloco físico com payload vazio deve ser rejeitado"
    );

    // 4. Misdirection de índice de bloco da FTL (LBA misdirection)
    let genuine_token = FtlAntiAmnesiaToken::new(boot_uuid, 1, 550, block_index); // index = 0
    let block = PhysicalMediaBlock {
        token: genuine_token,
        payload: vec![1, 2, 3, 4],
    };
    let misdirected_res = FtlAntiAmnesiaOracle::verify_block_at_index(file_id, &block, boot_uuid, active_min_seq, 5); // esperava bloco 5!
    assert_eq!(
        misdirected_res,
        Err(FtlAmnesiaViolation::BlockIndexMisdirection {
            expected_block_index: 5,
            actual_block_index: 0,
        })
    );

    // 5. Stale Global Epoch divergence
    let epoch_res = FtlAntiAmnesiaOracle::verify_block_with_epoch(file_id, &block, boot_uuid, active_min_seq, 2); // gravado com epoch 1, esperado 2
    assert_eq!(
        epoch_res,
        Err(FtlAmnesiaViolation::StaleGlobalEpochDetected {
            recorded_epoch: 1,
            expected_epoch: 2,
        })
    );
}

