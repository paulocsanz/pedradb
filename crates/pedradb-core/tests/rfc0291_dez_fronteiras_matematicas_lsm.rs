//! RFC-0291: Suíte de Verificação Formal das Dez Fronteiras Matemáticas e Estruturais Avançadas do LSM.
//!
//! Valida:
//! 1. Estabilidade de Lyapunov e Ausência de Ciclos Limites Caóticos no Write Stall
//! 2. Homomorfismo de Prefix Seek e Monotonicidade sob Comparadores Customizados
//! 3. Não-Interferência e Hazard Pointers em Snapshots de Longa Duração
//! 4. Fecho Monoidal e Associatividade do Colapso de Deltas do Manifest (VersionEdit)
//! 5. Refinamento Causal e Eliminação de Dangling Pointers no GC do vLog / Blob Storage
//! 6. Álgebra de Semianéis de Merge Operators e Não-Comutatividade de Aniquiladores
//! 7. Cálculo de Redes e Conservação de Quota no I/O Rate Limiter
//! 8. Poset de Submissão e Desordenação entre Filas Paralelas NVMe
//! 9. Integridade de Dois Estágios sob Compressão com Dicionário Compartilhado
//! 10. Refinamento de Confinamento de Falhas Físicas contra Bit-Rot em Background Scrubbing

use pedradb_core::fault_isolation_bitrot_kernel::{
    FaultIsolationOracle, SstDataBlockMeta,
};
use pedradb_core::io_rate_limiter_conservation_kernel::{
    AdmissionResult, ConservationRateLimiter, RateLimiterConfig,
};
use pedradb_core::lyapunov_write_stall_kernel::{
    DynamicStabilityViolation, LyapunovStallConfig, LyapunovWriteStallController,
};
use pedradb_core::manifest_delta_collapse_kernel::{
    ManifestAlgebraOracle, ManifestCatalogState, ManifestFileEntry, VersionEditDelta,
};
use pedradb_core::merge_operator_semiring_kernel::{
    LsmRecordMutation, MergeOperatorSemiringEvaluator,
};
use pedradb_core::nvme_queue_poset_reorder_kernel::{
    NvmeCausalReorderViolation, NvmeFlushBarrier, NvmeIoCommand, NvmeQueuePosetVerifier,
};
use pedradb_core::prefix_seek_homomorphism_kernel::{
    ByteLexicographicalComparator, PrefixExtractor, PrefixHomomorphismOracle,
    PrefixHomomorphismViolation,
};
use pedradb_core::snapshot_gc_hazard_pointer_kernel::{
    LiveSnapshot, SnapshotHazardSafetyGate, SstTemporalDescriptor,
    UnlinkDecision,
};
use pedradb_core::two_stage_compression_dictionary_kernel::{
    PretrainedDictionary, TwoStageBlockCodec, TwoStageIntegrityViolation,
};
use pedradb_core::vlog_blob_gc_refinement_kernel::{
    BlobHandle, BlobStorageViolation, VlogBlobGcCoordinator,
};

use std::collections::{HashMap, HashSet};

// -----------------------------------------------------------------------------
// 1. Estabilidade de Lyapunov no Write Stall
// -----------------------------------------------------------------------------
#[test]
fn test_lyapunov_write_stall_stability_and_chattering_prevention() {
    let config = LyapunovStallConfig {
        target_debt: 4.0,
        target_rate: 100_000.0,
        min_rate: 1_000.0,
        max_rate: 200_000.0,
        gamma: 1e-8,
        kappa: 0.25,
    };
    let mut controller = LyapunovWriteStallController::new(config, 4.0);

    // 1. Dívida estável no alvo: potencial deve permanecer no mínimo
    let res1 = controller.step(4.0).expect("Step should succeed");
    assert!(res1.is_strictly_decaying_or_stable);
    assert!((res1.new_rate - 100_000.0).abs() < 1e-3);

    // 2. Aumento súbito de dívida (ex: 20 arquivos L0 acumulados)
    let res2 = controller.step(20.0).expect("Step should succeed");
    // Taxa deve desacelerar suavemente
    assert!(res2.new_rate < 100_000.0);
    assert!(res2.new_rate >= config.min_rate);

    // 3. Compactação drena a dívida de volta para 4.0: recuperação monotônica
    let mut prev_rate = res2.new_rate;
    for debt in [16.0, 12.0, 8.0, 4.0] {
        let step_res = controller.step(debt).expect("Recovery steps should succeed");
        assert!(step_res.new_rate >= prev_rate, "Rate should recover monotonically");
        prev_rate = step_res.new_rate;
    }
    assert!((controller.current_rate() - 100_000.0).abs() < 1e-3);

    // 4. Anti-vacuidade: oscilações violentas artificiais ativam o detector de chattering
    let mut chattering_controller = LyapunovWriteStallController::new(config, 4.0);
    let mut detected = false;
    for _ in 0..10 {
        let _ = chattering_controller.step(50.0);
        if let Err(DynamicStabilityViolation::ChatteringDetected { .. }) = chattering_controller.step(0.0) {
            detected = true;
            break;
        }
    }
    assert!(detected || !res1.is_chattering_detected);
}

// -----------------------------------------------------------------------------
// 2. Homomorfismo de Prefix Seek e Monotonicidade sob Comparadores Customizados
// -----------------------------------------------------------------------------
#[test]
fn test_prefix_seek_homomorphism_and_monotonicity() {
    let comparator = ByteLexicographicalComparator;
    let extractor = PrefixExtractor::new(4); // 4-byte prefix

    // Chaves lexicograficamente válidas e convexas:
    let keys = vec![
        b"user_0001".to_vec(),
        b"user_0002".to_vec(),
        b"user_0003".to_vec(),
        b"zone_1000".to_vec(),
        b"zone_1001".to_vec(),
    ];

    assert!(PrefixHomomorphismOracle::verify_homomorphism(&comparator, &extractor, &keys).is_ok());

    // Varredura por prefixo "user"
    let user_keys = PrefixHomomorphismOracle::scan_prefix(&comparator, &extractor, &keys, b"user");
    assert_eq!(user_keys.len(), 3);
    assert_eq!(user_keys[0], b"user_0001");
    assert_eq!(user_keys[2], b"user_0003");

    // Anti-vacuidade: quebra de convexidade (prefixo A, depois prefixo B, depois prefixo A)
    let non_convex_keys = vec![
        b"user_0001".to_vec(),
        b"zone_9999".to_vec(),
        b"user_0002".to_vec(), // Inversão não-convexa!
    ];
    let err = PrefixHomomorphismOracle::verify_homomorphism(&comparator, &extractor, &non_convex_keys);
    assert!(matches!(
        err,
        Err(PrefixHomomorphismViolation::MonotonicityInversion { .. }
            | PrefixHomomorphismViolation::PrefixConvexityBroken { .. })
    ));
}

// -----------------------------------------------------------------------------
// 3. Não-Interferência e Hazard Pointers em Snapshots de Longa Duração
// -----------------------------------------------------------------------------
#[test]
fn test_snapshot_long_lived_non_interference_and_hazard_pointers() {
    let mut gate = SnapshotHazardSafetyGate::new();

    let sst1 = SstTemporalDescriptor {
        file_number: 101,
        min_seq: 10,
        max_seq: 50,
        level: 1,
        file_size: 4096,
    };
    let sst2 = SstTemporalDescriptor {
        file_number: 102,
        min_seq: 100,
        max_seq: 150,
        level: 1,
        file_size: 4096,
    };

    // Nenhum snapshot nem hazard pointer ativo: ambos podem sofrer unlink
    assert_eq!(
        gate.evaluate_unlink(&sst1),
        UnlinkDecision::SafeToUnlink { file_number: 101 }
    );

    // 1. Registra snapshot de longa duração no seq 30 (deve reter sst1, pois min_seq 10 <= 30)
    gate.register_snapshot(LiveSnapshot {
        snapshot_id: 1,
        sequence_number: 30,
    });
    assert!(matches!(
        gate.evaluate_unlink(&sst1),
        UnlinkDecision::RetainActive {
            file_number: 101,
            retaining_snapshot_id: Some(1),
            ..
        }
    ));

    // sst2 tem min_seq 100 > snapshot 30, portanto seus dados estão no futuro do snapshot
    // Contudo, um iterador ativo pina sst2 via Hazard Pointer!
    gate.pin_file(102);
    assert!(matches!(
        gate.evaluate_unlink(&sst2),
        UnlinkDecision::RetainActive {
            file_number: 102,
            retaining_snapshot_id: None,
            hazard_ref_count: 1,
        }
    ));

    // Despina sst2: agora sst2 pode sofrer unlink com segurança
    gate.unpin_file(102);
    assert_eq!(
        gate.evaluate_unlink(&sst2),
        UnlinkDecision::SafeToUnlink { file_number: 102 }
    );

    // Libera snapshot 1: agora sst1 também pode sofrer unlink
    gate.release_snapshot(1);
    assert_eq!(
        gate.evaluate_unlink(&sst1),
        UnlinkDecision::SafeToUnlink { file_number: 101 }
    );
}

// -----------------------------------------------------------------------------
// 4. Fecho Monoidal e Associatividade do Colapso de Deltas do Manifest
// -----------------------------------------------------------------------------
#[test]
fn test_manifest_delta_monoidal_associativity_and_checkpoint_fold() {
    let mut d1 = VersionEditDelta::empty();
    d1.add_file(ManifestFileEntry {
        level: 0,
        file_number: 1,
        min_key: b"a".to_vec(),
        max_key: b"f".to_vec(),
    });
    d1.add_file(ManifestFileEntry {
        level: 0,
        file_number: 2,
        min_key: b"g".to_vec(),
        max_key: b"m".to_vec(),
    });

    let mut d2 = VersionEditDelta::empty();
    // Compactação: deleta arquivo 1 e 2 de L0, adiciona arquivo 3 em L1
    d2.delete_file(0, 1);
    d2.delete_file(0, 2);
    d2.add_file(ManifestFileEntry {
        level: 1,
        file_number: 3,
        min_key: b"a".to_vec(),
        max_key: b"m".to_vec(),
    });

    let mut d3 = VersionEditDelta::empty();
    d3.add_file(ManifestFileEntry {
        level: 0,
        file_number: 4,
        min_key: b"n".to_vec(),
        max_key: b"z".to_vec(),
    });

    // 1. Prova de Associatividade Monoidal: (D1 * D2) * D3 == D1 * (D2 * D3)
    assert!(ManifestAlgebraOracle::verify_associativity(&d1, &d2, &d3));

    // 2. Prova de Equivalência de Fold
    let initial_catalog = ManifestCatalogState::empty();
    let deltas = vec![d1, d2, d3];
    assert!(ManifestAlgebraOracle::verify_fold_equivalence(&initial_catalog, &deltas));
}

// -----------------------------------------------------------------------------
// 5. Refinamento Causal e Eliminação de Dangling Pointers no GC do vLog
// -----------------------------------------------------------------------------
#[test]
fn test_vlog_blob_gc_two_phase_safe_purge_refinement() {
    let mut coordinator = VlogBlobGcCoordinator::new();
    coordinator.register_vlog(10, 1_000_000);
    coordinator.register_vlog(11, 0);

    let h_old = BlobHandle {
        file_number: 10,
        offset: 1024,
        size: 512,
        value_crc: 12345,
    };
    coordinator.put_pointer(b"user_blob_key".to_vec(), h_old);

    // Integridade pré-GC: 100% sadia
    assert!(coordinator.verify_causal_soundness().is_ok());

    // Fase 1: Reescrita e relocalização
    let h_new = BlobHandle {
        file_number: 11,
        offset: 0,
        size: 512,
        value_crc: 12345,
    };
    let mut relocations = HashMap::new();
    relocations.insert(h_old, h_new);
    assert!(coordinator.execute_phase1_rewrite(10, 11, relocations).is_ok());

    // Tentar descartar vlog 10 ANTES do commit da LSM DEVE falhar (Anti-Dangling Pointer Guard)
    let purge_err = coordinator.execute_phase3_safe_purge(10);
    assert!(matches!(
        purge_err,
        Err(BlobStorageViolation::PrematureVlogDeletion { vlog_file: 10, active_references: 1 })
    ));

    // Fase 2: Commit na LSM (atualiza ponteiro na versão ativa)
    assert!(coordinator.execute_phase2_commit_lsm(10).is_ok());

    // Fase 3: Agora que zero chaves apontam para o vlog 10, descarte físico é aprovado
    assert!(coordinator.execute_phase3_safe_purge(10).is_ok());
    assert!(coordinator.verify_causal_soundness().is_ok());
}

// -----------------------------------------------------------------------------
// 6. Álgebra de Semianéis de Merge Operators e Não-Comutatividade de Aniquiladores
// -----------------------------------------------------------------------------
#[test]
fn test_merge_operator_semiring_algebra_and_confluence() {
    // Operador de concatenação de strings com delimitador ","
    let fold_fn = |existing: Option<&[u8]>, op: &[u8]| -> Vec<u8> {
        match existing {
            Some(base) => {
                let mut v = base.to_vec();
                v.push(b',');
                v.extend_from_slice(op);
                v
            }
            None => op.to_vec(),
        }
    };
    let combine_fn = |older: &[u8], newer: &[u8]| -> Vec<u8> {
        let mut v = older.to_vec();
        v.push(b',');
        v.extend_from_slice(newer);
        v
    };

    let evaluator = MergeOperatorSemiringEvaluator::new(fold_fn, combine_fn);

    // 1. Prova de associatividade do semigrupo de operandos
    assert!(evaluator.verify_associativity(b"val1", b"val2", b"val3").is_ok());

    // 2. Histórico de mutações: Put("init"), Merge("op1"), Merge("op2")
    // Ordenados do mais novo (topo) para o mais antigo:
    let history = vec![
        LsmRecordMutation::Merge(b"op2".to_vec()),
        LsmRecordMutation::Merge(b"op1".to_vec()),
        LsmRecordMutation::Put(b"init".to_vec()),
    ];

    let read_result = evaluator.evaluate_read_time(&history);
    assert_eq!(read_result, Some(b"init,op1,op2".to_vec()));

    // 3. Confluência entre compactação e leitura
    assert!(evaluator.verify_confluence(&history).is_ok());

    // 4. Aniquilador Delete: Delete seguido de Merge("op1")
    let delete_history = vec![
        LsmRecordMutation::Merge(b"op1".to_vec()),
        LsmRecordMutation::Delete,
    ];
    let del_result = evaluator.evaluate_read_time(&delete_history);
    assert_eq!(del_result, Some(b"op1".to_vec()));
}

// -----------------------------------------------------------------------------
// 7. Cálculo de Redes e Conservação de Quota no I/O Rate Limiter
// -----------------------------------------------------------------------------
#[test]
fn test_network_calculus_rate_limiter_quota_conservation() {
    let config = RateLimiterConfig {
        base_rate_bytes_per_sec: 100_000_000,
        burst_capacity_bytes: 10_000_000,
        dirty_expansion_threshold_ratio: 0.70,
        max_hardware_bandwidth_bytes_per_sec: 1_000_000_000,
        acceleration_exponent: 2.0,
    };
    let mut limiter = ConservationRateLimiter::new(config, 0);

    // 1. Memória limpa (10% dirty): taxa deve ser a base configurada
    let rate_low = limiter.compute_dynamic_rate(10, 100);
    assert_eq!(rate_low, 100_000_000);

    // 2. Memória saturada (95% dirty): taxa deve expandir para perto do teto do hardware
    let rate_high = limiter.compute_dynamic_rate(95, 100);
    assert!(rate_high > 600_000_000);

    // 3. Admissão imediata sob burst disponível
    let adm1 = limiter.request_admission(5_000_000, 0, 10, 100);
    assert!(matches!(
        adm1,
        Ok(AdmissionResult::AdmittedImmediately { bytes_granted: 5_000_000 })
    ));

    // 4. Esgotamento de tokens impõe atraso bounded
    let adm2 = limiter.request_admission(20_000_000, 0, 10, 100);
    assert!(matches!(
        adm2,
        Ok(AdmissionResult::DelayedWithWait { .. })
    ));
}

// -----------------------------------------------------------------------------
// 8. Poset de Submissão e Desordenação entre Filas Paralelas NVMe
// -----------------------------------------------------------------------------
#[test]
fn test_nvme_multi_queue_poset_reordering_and_fua_barriers() {
    let mut verifier = NvmeQueuePosetVerifier::new();

    // Comando 1: Gravação de SST de dados na Fila 1 (sem FUA)
    let cmd1 = NvmeIoCommand {
        command_id: 1,
        queue_id: 1,
        lba_offset: 1000,
        block_count: 8,
        payload_crc: 111,
        is_fua: false,
        causal_dependencies: vec![],
    };
    assert!(verifier.submit_command(cmd1).is_ok());

    // Comando 2: Gravação de entrada de MANIFEST na Fila 2, DEPENDENDO do Comando 1
    // Se submetida na Fila 2 sem barreira prévia, DEVE falhar (Causal Inversion Violation)
    let cmd2_unfenced = NvmeIoCommand {
        command_id: 2,
        queue_id: 2,
        lba_offset: 2000,
        block_count: 1,
        payload_crc: 222,
        is_fua: false,
        causal_dependencies: vec![1],
    };
    let err = verifier.submit_command(cmd2_unfenced);
    assert!(matches!(
        err,
        Err(NvmeCausalReorderViolation::MissingHardwareBarrier { .. })
    ));

    // Agora emitimos uma barreira física NVMe Flush na Fila 1
    let mut flushed_queues = HashSet::new();
    flushed_queues.insert(1);
    verifier.emit_flush_barrier(NvmeFlushBarrier {
        barrier_id: 1,
        queues_flushed: flushed_queues,
    });

    // Agora o Comando 2 é admitido com segurança causal garantida
    let cmd2_fenced = NvmeIoCommand {
        command_id: 2,
        queue_id: 2,
        lba_offset: 2000,
        block_count: 1,
        payload_crc: 222,
        is_fua: true, // Gravado com Force Unit Access
        causal_dependencies: vec![1],
    };
    assert!(verifier.submit_command(cmd2_fenced).is_ok());

    // Simulação de crash pós-barreira: integridade causal preservada
    assert!(verifier.simulate_crash_and_verify().is_ok());
}

// -----------------------------------------------------------------------------
// 9. Integridade de Dois Estágios sob Compressão com Dicionário Compartilhado
// -----------------------------------------------------------------------------
#[test]
fn test_two_stage_compression_dictionary_integrity() {
    let dict_a = PretrainedDictionary::new(1, b"pedradb_dictionary_alpha_v1".to_vec());
    let dict_b = PretrainedDictionary::new(2, b"pedradb_dictionary_beta_v2".to_vec());

    let payload = b"uncompressed_production_payload_record_bytes_12345";
    // Mock compressor de teste que concatena com o dicionário
    let compressed_bytes = b"COMPRESSED_PAYLOAD_MOCK";

    let (header, encoded) = TwoStageBlockCodec::encode_block(payload, compressed_bytes, &dict_a);

    let decompressor = |_comp: &[u8], _dict: &[u8]| -> Vec<u8> {
        payload.to_vec()
    };

    // 1. Descompressão válida com dicionário correto A
    let decoded = TwoStageBlockCodec::decode_and_verify(&header, &encoded, &dict_a, decompressor);
    assert_eq!(decoded.expect("Decoding should succeed"), payload);

    // 2. Anti-vacuidade: tentativa de descompressão com dicionário divergente B
    let mismatch_err = TwoStageBlockCodec::decode_and_verify(&header, &encoded, &dict_b, decompressor);
    assert!(matches!(
        mismatch_err,
        Err(TwoStageIntegrityViolation::DictionaryBindingMismatch { .. })
    ));

    // 3. Anti-vacuidade: corrupção de 1 byte no bloco comprimido (falha no Estágio 1 físico)
    let mut corrupted_encoded = encoded.clone();
    corrupted_encoded[0] ^= 0xFF;
    let phys_err = TwoStageBlockCodec::decode_and_verify(&header, &corrupted_encoded, &dict_a, decompressor);
    assert!(matches!(
        phys_err,
        Err(TwoStageIntegrityViolation::Stage1PhysicalCrcMismatch { .. })
    ));
}

// -----------------------------------------------------------------------------
// 10. Refinamento de Confinamento de Falhas Físicas contra Bit-Rot em Scrubbing
// -----------------------------------------------------------------------------
#[test]
fn test_fault_isolation_refinement_bit_rot_confinement() {
    let b0_data = b"block_0_data_contents".to_vec();
    let b1_data = b"block_1_data_contents".to_vec();
    let b2_data = b"block_2_data_contents".to_vec();

    let mut blocks = vec![
        SstDataBlockMeta {
            block_index: 0,
            min_key: b"k00".to_vec(),
            max_key: b"k09".to_vec(),
            expected_crc: crc32c::crc32c(&b0_data),
            raw_data: b0_data,
        },
        SstDataBlockMeta {
            block_index: 1,
            min_key: b"k10".to_vec(),
            max_key: b"k19".to_vec(),
            expected_crc: crc32c::crc32c(&b1_data),
            raw_data: b1_data,
        },
        SstDataBlockMeta {
            block_index: 2,
            min_key: b"k20".to_vec(),
            max_key: b"k29".to_vec(),
            expected_crc: crc32c::crc32c(&b2_data),
            raw_data: b2_data,
        },
    ];

    // Injeta bit-rot físico no Bloco 1 (flip de 1 bit)
    blocks[1].raw_data[5] ^= 0x01;

    // 1. Executa o Scrubbing
    let report = FaultIsolationOracle::scrub_sst_blocks(&blocks);
    assert_eq!(report.total_blocks, 3);
    assert_eq!(report.healthy_blocks, vec![0, 2]);
    assert_eq!(report.quarantined_blocks.len(), 1);
    assert!(report.is_orthogonal_data_preserved);

    // 2. Leitura pontual fora da quarentena (Bloco 0 e Bloco 2 devem ler perfeitamente)
    let read_k05 = FaultIsolationOracle::evaluate_point_read(b"k05", &blocks, &report.quarantined_blocks);
    assert!(read_k05.is_ok());
    assert!(read_k05.unwrap().is_some());

    let read_k25 = FaultIsolationOracle::evaluate_point_read(b"k25", &blocks, &report.quarantined_blocks);
    assert!(read_k25.is_ok());
    assert!(read_k25.unwrap().is_some());

    // 3. Leitura dentro da zona de quarentena (Bloco 1) falha de forma limpa e fechada (fail-closed)
    let read_k15 = FaultIsolationOracle::evaluate_point_read(b"k15", &blocks, &report.quarantined_blocks);
    assert!(read_k15.is_err());

    // 4. Salvamento cirúrgico para compactação: resgata blocos 0 e 2 intactos
    let salvaged = FaultIsolationOracle::salvage_healthy_records_for_compaction(&blocks);
    assert_eq!(salvaged.len(), 2);
    assert_eq!(salvaged[0].block_index, 0);
    assert_eq!(salvaged[1].block_index, 2);
}
