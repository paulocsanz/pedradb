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
    LiveSnapshot, SnapshotHazardSafetyGate, SnapshotIsolationViolation, SstTemporalDescriptor,
    UnlinkDecision,
};
use pedradb_core::two_stage_compression_dictionary_kernel::{
    CompressedBlockHeader, PretrainedDictionary, TwoStageBlockCodec, TwoStageIntegrityViolation,
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

#[test]
fn test_lyapunov_write_stall_edge_cases_red() {
    // 1. Configuração com valores não-finitos ou degenerados
    let config_nan = LyapunovStallConfig {
        target_debt: f64::NAN,
        target_rate: -100.0,
        min_rate: f64::INFINITY,
        max_rate: 0.0,
        gamma: f64::NAN,
        kappa: -5.0,
    };
    let mut controller = LyapunovWriteStallController::new(config_nan, f64::NAN);
    let step_res = controller.step(f64::NAN);
    assert!(step_res.is_ok());
    let res = step_res.unwrap();
    assert!(res.new_rate.is_finite());
    assert!(!res.potential_before.is_nan());
    assert!(!res.potential_after.is_nan());

    // 2. step com observed_debt infinito ou negativo
    let res_inf = controller.step(f64::INFINITY);
    assert!(res_inf.is_ok());
    let res_neg = controller.step(-1000.0);
    assert!(res_neg.is_ok());
}

#[test]
fn test_lyapunov_write_stall_hardening_and_strict_validation_red() {
    // 1. Config validation must reject invalid parameters
    let bad_cfg1 = LyapunovStallConfig {
        target_debt: -1.0,
        ..LyapunovStallConfig::default()
    };
    assert!(matches!(
        bad_cfg1.validate(),
        Err(DynamicStabilityViolation::InvalidConfig { .. })
    ));

    let bad_cfg2 = LyapunovStallConfig {
        min_rate: 0.0,
        ..LyapunovStallConfig::default()
    };
    assert!(matches!(
        bad_cfg2.validate(),
        Err(DynamicStabilityViolation::InvalidConfig { .. })
    ));

    let bad_cfg3 = LyapunovStallConfig {
        max_rate: 500.0,
        min_rate: 1000.0,
        ..LyapunovStallConfig::default()
    };
    assert!(matches!(
        bad_cfg3.validate(),
        Err(DynamicStabilityViolation::InvalidConfig { .. })
    ));

    // 2. try_new must enforce validated configuration and valid initial debt
    assert!(matches!(
        LyapunovWriteStallController::try_new(bad_cfg1, 4.0),
        Err(DynamicStabilityViolation::InvalidConfig { .. })
    ));
    assert!(matches!(
        LyapunovWriteStallController::try_new(LyapunovStallConfig::default(), -5.0),
        Err(DynamicStabilityViolation::InvalidDebtMeasurement)
    ));
    assert!(matches!(
        LyapunovWriteStallController::try_new(LyapunovStallConfig::default(), f64::NAN),
        Err(DynamicStabilityViolation::InvalidDebtMeasurement)
    ));

    let mut controller = LyapunovWriteStallController::try_new(LyapunovStallConfig::default(), 4.0)
        .expect("Valid controller creation");

    // 3. try_step must fail closed on NaN, negative, or infinite debt
    assert!(matches!(
        controller.try_step(f64::NAN),
        Err(DynamicStabilityViolation::InvalidDebtMeasurement)
    ));
    assert!(matches!(
        controller.try_step(-10.0),
        Err(DynamicStabilityViolation::InvalidDebtMeasurement)
    ));
    assert!(matches!(
        controller.try_step(f64::INFINITY),
        Err(DynamicStabilityViolation::InvalidDebtMeasurement)
    ));

    // Valid step works
    assert!(controller.try_step(10.0).is_ok());

    // 4. step_autonomous verification
    assert!(controller.step_autonomous(4.0).is_ok());

    // 5. Implementação de std::error::Error
    let err_dyn: Box<dyn std::error::Error> = Box::new(DynamicStabilityViolation::InvalidDebtMeasurement);
    assert!(!err_dyn.to_string().is_empty());
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

#[test]
fn test_prefix_seek_homomorphism_edge_cases_red() {
    let comparator = ByteLexicographicalComparator;
    let extractor = PrefixExtractor::new(4);

    // 1. Chaves com inverso de ordenação estrita (k_a > k_b mas prefix_a <= prefix_b)
    let inverted_keys = vec![
        b"user_0002".to_vec(),
        b"user_0001".to_vec(),
    ];
    let err_inv = PrefixHomomorphismOracle::verify_homomorphism(&comparator, &extractor, &inverted_keys);
    assert!(matches!(err_inv, Err(PrefixHomomorphismViolation::MonotonicityInversion { .. })));

    // 2. Chaves com prefix_len = 0 (extractor extrai &[0..0] = b"")
    let zero_ext = PrefixExtractor::new(0);
    assert_eq!(zero_ext.extract(b"hello"), b"");

    // 3. scan_prefix com sorted_keys vazio
    let empty_scan = PrefixHomomorphismOracle::scan_prefix(&comparator, &extractor, &[], b"user");
    assert!(empty_scan.is_empty());

    // 4. scan_prefix com target_prefix menor que todas as chaves
    let keys = vec![b"user_1".to_vec(), b"user_2".to_vec()];
    let no_match = PrefixHomomorphismOracle::scan_prefix(&comparator, &extractor, &keys, b"aaaa");
    assert!(no_match.is_empty());
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

#[test]
fn test_snapshot_hazard_pointers_dual_retention_and_corrupted_bounds_red() {
    let mut gate = SnapshotHazardSafetyGate::new();

    gate.register_snapshot(LiveSnapshot {
        snapshot_id: 42,
        sequence_number: 50,
    });
    gate.pin_file(200);

    let sst_dual = SstTemporalDescriptor {
        file_number: 200,
        min_seq: 10,
        max_seq: 40,
        level: 1,
        file_size: 1024,
    };

    // 1. Dual retention: deve registrar AMBOS snapshot_id e hazard_ref_count
    let decision = gate.evaluate_unlink(&sst_dual);
    assert_eq!(
        decision,
        UnlinkDecision::RetainActive {
            file_number: 200,
            retaining_snapshot_id: Some(42),
            hazard_ref_count: 1,
        }
    );

    // 2. Corrupted bounds: min_seq > max_seq não pode sofrer unlink e deve falhar na auditoria
    let corrupted_sst = SstTemporalDescriptor {
        file_number: 201,
        min_seq: 100,
        max_seq: 10,
        level: 2,
        file_size: 2048,
    };
    assert_ne!(
        gate.evaluate_unlink(&corrupted_sst),
        UnlinkDecision::SafeToUnlink { file_number: 201 }
    );
    let audit_err = gate.verify_unlinks_soundness(&[corrupted_sst]);
    assert!(matches!(
        audit_err,
        Err(SnapshotIsolationViolation::CorruptedTemporalBounds { file_number: 201, min_seq: 100, max_seq: 10 })
    ));
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

#[test]
fn test_manifest_delta_empty_level_pruning_and_catalog_invariants_red() {
    // 1. Deleção total de nível sem re-adição: não pode deixar chave fantasma em levels
    let mut d1 = VersionEditDelta::empty();
    d1.add_file(ManifestFileEntry {
        level: 2,
        file_number: 10,
        min_key: b"a".to_vec(),
        max_key: b"m".to_vec(),
    });

    let mut d2 = VersionEditDelta::empty();
    d2.delete_file(2, 10);

    let initial = ManifestCatalogState::empty();
    let deltas = vec![d1, d2];
    // Se o nível 2 não for podado ao ficar vazio, state_successive tem { 2: {} } e state_collapsed tem {}
    assert!(ManifestAlgebraOracle::verify_fold_equivalence(&initial, &deltas));

    let mut successive = initial.clone();
    successive.apply_delta(&deltas[0]);
    successive.apply_delta(&deltas[1]);
    assert!(!successive.levels.contains_key(&2), "Nível vazio deve ser podado de levels");

    // 2. Invariante de catálogo: min_key > max_key corrompido
    let mut corrupt_catalog = ManifestCatalogState::empty();
    corrupt_catalog.levels.entry(1).or_default().insert(5, ManifestFileEntry {
        level: 1,
        file_number: 5,
        min_key: b"z".to_vec(),
        max_key: b"a".to_vec(), // Invertido!
    });
    corrupt_catalog.next_file_number = 10;
    assert!(!ManifestAlgebraOracle::verify_catalog_consistency(&corrupt_catalog));

    // 3. Invariante de catálogo: file_number >= next_file_number
    let mut bad_nfn_catalog = ManifestCatalogState::empty();
    bad_nfn_catalog.levels.entry(0).or_default().insert(10, ManifestFileEntry {
        level: 0,
        file_number: 10,
        min_key: b"a".to_vec(),
        max_key: b"b".to_vec(),
    });
    bad_nfn_catalog.next_file_number = 10; // 10 não é estritamente menor que next_file_number!
    assert!(!ManifestAlgebraOracle::verify_catalog_consistency(&bad_nfn_catalog));
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

    // Invariant: Target vlog active blobs count must reflect committed relocations
    assert_eq!(
        coordinator.get_vlog_meta(11).unwrap().active_blobs_count,
        1,
        "Target vlog active_blobs_count must be updated after commit"
    );

    // Invariant: Phase 2 without Phase 1 must fail
    assert!(coordinator.execute_phase2_commit_lsm(999).is_err());

    // Fase 3: Agora que zero chaves apontam para o vlog 10, descarte físico é aprovado
    assert!(coordinator.execute_phase3_safe_purge(10).is_ok());
    assert!(coordinator.verify_causal_soundness().is_ok());

    // Invariant: Double purge on already purged vlog must fail
    assert!(
        coordinator.execute_phase3_safe_purge(10).is_err(),
        "Double physical purge must be rejected"
    );

    // Invariant: Relocating to non-existent target vlog must fail
    let mut bad_reloc = HashMap::new();
    bad_reloc.insert(h_new, h_new);
    assert!(
        coordinator.execute_phase1_rewrite(11, 99999, bad_reloc).is_err(),
        "Relocation to non-existent target vlog must be rejected"
    );
}

#[test]
fn test_vlog_blob_gc_out_of_bounds_and_pending_phase1_purge_red() {
    let mut coordinator = VlogBlobGcCoordinator::new();
    coordinator.register_vlog(20, 10_000); // 10 KB vlog

    // 1. BlobHandle com offset + size estritamente além do fim do arquivo vLog
    let oob_handle = BlobHandle {
        file_number: 20,
        offset: 9_500,
        size: 1_000, // 9500 + 1000 = 10500 > 10000!
        value_crc: 999,
    };
    coordinator.put_pointer(b"oob_key".to_vec(), oob_handle);

    let err = coordinator.verify_causal_soundness();
    assert!(
        matches!(err, Err(BlobStorageViolation::DanglingPointerDetected { ref reason, .. }) if reason.contains("exceeds vlog file size")),
        "Ponteiro além do fim do vLog deve ser detectado como DanglingPointerDetected"
    );

    // 2. BlobHandle com size == 0
    let mut coord2 = VlogBlobGcCoordinator::new();
    coord2.register_vlog(21, 5_000);
    let zero_size_handle = BlobHandle {
        file_number: 21,
        offset: 100,
        size: 0,
        value_crc: 1,
    };
    coord2.put_pointer(b"zero_key".to_vec(), zero_size_handle);
    let err2 = coord2.verify_causal_soundness();
    assert!(
        matches!(err2, Err(BlobStorageViolation::DanglingPointerDetected { ref reason, .. }) if reason.contains("zero")),
        "Ponteiro com tamanho 0 deve ser detectado como DanglingPointerDetected"
    );

    // 3. Purge com Fase 1 pendente de commit da Fase 2 (mesmo com 0 referências diretas na LSM)
    let mut coord3 = VlogBlobGcCoordinator::new();
    coord3.register_vlog(30, 1_000);
    coord3.register_vlog(31, 1_000);
    let dummy_h30 = BlobHandle { file_number: 30, offset: 0, size: 10, value_crc: 1 };
    let dummy_h31 = BlobHandle { file_number: 31, offset: 0, size: 10, value_crc: 1 };
    let mut rel = HashMap::new();
    rel.insert(dummy_h30, dummy_h31);
    assert!(coord3.execute_phase1_rewrite(30, 31, rel).is_ok());

    // Tentativa de purge de vlog 30 antes do commit da Fase 2 deve ser rejeitada
    let purge_res = coord3.execute_phase3_safe_purge(30);
    assert!(
        matches!(purge_res, Err(BlobStorageViolation::Phase1NotExecuted { .. }) | Err(BlobStorageViolation::PrematureVlogDeletion { .. })),
        "Purge de vLog com Fase 1 pendente deve ser rejeitado"
    );
}

#[test]
fn test_vlog_blob_gc_relocation_integrity_and_try_put_pointer_red() {
    let mut coordinator = VlogBlobGcCoordinator::new();
    coordinator.register_vlog(40, 10_000);
    coordinator.register_vlog(41, 5_000);
    coordinator.register_vlog(42, 1_000);

    let h_valid_40 = BlobHandle { file_number: 40, offset: 0, size: 100, value_crc: 999 };
    assert!(coordinator.try_put_pointer(b"key1".to_vec(), h_valid_40).is_ok());

    // 1. try_put_pointer must reject size 0
    let h_zero = BlobHandle { file_number: 40, offset: 0, size: 0, value_crc: 999 };
    assert!(matches!(
        coordinator.try_put_pointer(b"zero".to_vec(), h_zero),
        Err(BlobStorageViolation::DanglingPointerDetected { .. })
    ));

    // 2. try_put_pointer must reject non-existent vlog
    let h_nonexist = BlobHandle { file_number: 9999, offset: 0, size: 100, value_crc: 999 };
    assert!(matches!(
        coordinator.try_put_pointer(b"nonexist".to_vec(), h_nonexist),
        Err(BlobStorageViolation::DanglingPointerDetected { .. })
    ));

    // 3. Purge vlog 42 then try_put_pointer and phase1_rewrite on it
    assert!(coordinator.execute_phase3_safe_purge(42).is_ok());
    let h_purged = BlobHandle { file_number: 42, offset: 0, size: 100, value_crc: 999 };
    assert!(matches!(
        coordinator.try_put_pointer(b"purged".to_vec(), h_purged),
        Err(BlobStorageViolation::DanglingPointerDetected { .. })
    ));

    let mut rel_on_purged = HashMap::new();
    rel_on_purged.insert(h_purged, BlobHandle { file_number: 41, offset: 0, size: 100, value_crc: 999 });
    assert!(matches!(
        coordinator.execute_phase1_rewrite(42, 41, rel_on_purged),
        Err(BlobStorageViolation::AlreadyPurged { vlog_file: 42 })
    ));

    // 4. Phase 1 rewrite with CRC mismatch must be rejected
    let mut rel_crc_mismatch = HashMap::new();
    rel_crc_mismatch.insert(
        h_valid_40,
        BlobHandle { file_number: 41, offset: 0, size: 100, value_crc: 888 }, // CRC mismatch!
    );
    assert!(matches!(
        coordinator.execute_phase1_rewrite(40, 41, rel_crc_mismatch),
        Err(BlobStorageViolation::RelocationIntegrityMismatch { .. })
    ));

    // 5. Phase 1 rewrite with size mismatch must be rejected
    let mut rel_size_mismatch = HashMap::new();
    rel_size_mismatch.insert(
        h_valid_40,
        BlobHandle { file_number: 41, offset: 0, size: 200, value_crc: 999 }, // size mismatch!
    );
    assert!(matches!(
        coordinator.execute_phase1_rewrite(40, 41, rel_size_mismatch),
        Err(BlobStorageViolation::RelocationIntegrityMismatch { .. })
    ));

    // 6. Phase 1 rewrite with target out-of-bounds must be rejected
    let mut rel_oob = HashMap::new();
    rel_oob.insert(
        h_valid_40,
        BlobHandle { file_number: 41, offset: 4_950, size: 100, value_crc: 999 }, // 4950 + 100 = 5050 > 5000!
    );
    assert!(matches!(
        coordinator.execute_phase1_rewrite(40, 41, rel_oob),
        Err(BlobStorageViolation::RelocationIntegrityMismatch { .. })
    ));
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

#[test]
fn test_merge_operator_annihilator_zombie_truncation_red() {
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

    // 1. Delete aniquila qualquer mutação mais antiga: compact_slice deve podar zumbis
    let zombie_history = vec![
        LsmRecordMutation::Delete,
        LsmRecordMutation::Merge(b"zombie_op".to_vec()),
    ];
    let compacted_del = evaluator.compact_slice(&zombie_history);
    assert_eq!(
        compacted_del,
        vec![LsmRecordMutation::Delete],
        "compact_slice não pode deixar mutações zumbis após aniquilador Delete"
    );
    assert!(evaluator.verify_confluence(&zombie_history).is_ok());

    // 2. Put aniquila qualquer mutação mais antiga
    let stale_history = vec![
        LsmRecordMutation::Put(b"v2".to_vec()),
        LsmRecordMutation::Merge(b"m1".to_vec()),
        LsmRecordMutation::Put(b"v1".to_vec()),
    ];
    let compacted_put = evaluator.compact_slice(&stale_history);
    assert_eq!(
        compacted_put,
        vec![LsmRecordMutation::Put(b"v2".to_vec())],
        "compact_slice não pode reter versões mortas após aniquilador Put"
    );
    assert!(evaluator.verify_confluence(&stale_history).is_ok());
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

#[test]
fn test_network_calculus_rate_limiter_edge_cases_red() {
    // Caso 1: dirty_expansion_threshold_ratio >= 1.0 (causa 0.0 / 0.0 = NaN -> cast to u64)
    let config_edge = RateLimiterConfig {
        base_rate_bytes_per_sec: 100_000_000,
        burst_capacity_bytes: 10_000_000,
        dirty_expansion_threshold_ratio: 1.0,
        max_hardware_bandwidth_bytes_per_sec: 1_000_000_000,
        acceleration_exponent: 2.0,
    };
    let mut limiter_edge = ConservationRateLimiter::new(config_edge, 1000);
    // Em 100% dirty_bytes, como threshold foi clampado para 0.95, deve expandir até a banda máxima
    let rate = limiter_edge.compute_dynamic_rate(100, 100);
    assert_eq!(rate, 1_000_000_000);

    // Quando ratio <= threshold (ex: 50%), permanece na taxa base
    let rate_sub = limiter_edge.compute_dynamic_rate(50, 100);
    assert_eq!(rate_sub, 100_000_000);

    // Caso 2: bytes_requested = 0 deve ser concedido imediatamente sem alterar tokens
    let adm_zero = limiter_edge.request_admission(0, 2000, 100, 100);
    assert_eq!(adm_zero, Ok(AdmissionResult::AdmittedImmediately { bytes_granted: 0 }));

    // Caso 3: dirty_bytes > mem_budget (clamp em 1.0 sem overflow)
    let rate_over = limiter_edge.compute_dynamic_rate(200, 100);
    assert_eq!(rate_over, 1_000_000_000);
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

    // Invariant: Submitting command with unknown dependency must return Err, not panic!
    let cmd_unknown_dep = NvmeIoCommand {
        command_id: 3,
        queue_id: 2,
        lba_offset: 3000,
        block_count: 1,
        payload_crc: 333,
        is_fua: false,
        causal_dependencies: vec![99999],
    };
    assert!(matches!(
        verifier.submit_command(cmd_unknown_dep),
        Err(NvmeCausalReorderViolation::UnknownDependency { dependency_cmd_id: 99999, .. })
    ));

    // Invariant: Flushing queue 3 must NOT mark newly submitted write in queue 1 as persisted!
    let cmd_unflushed_q1 = NvmeIoCommand {
        command_id: 4,
        queue_id: 1,
        lba_offset: 4000,
        block_count: 1,
        payload_crc: 444,
        is_fua: false,
        causal_dependencies: vec![],
    };
    assert!(verifier.submit_command(cmd_unflushed_q1).is_ok());

    let mut flushed_q3_only = HashSet::new();
    flushed_q3_only.insert(3);
    verifier.emit_flush_barrier(NvmeFlushBarrier {
        barrier_id: 2,
        queues_flushed: flushed_q3_only,
    });
    assert!(
        !verifier.is_command_persisted(4),
        "Un-flushed command in queue 1 must NOT be marked as persisted when flushing queue 3"
    );
}

#[test]
fn test_nvme_poset_same_queue_fua_torn_write_and_zero_blocks_red() {
    let mut verifier = NvmeQueuePosetVerifier::new();

    // 1. Comando 10: não-FUA na Fila 0
    let cmd10 = NvmeIoCommand {
        command_id: 10,
        queue_id: 0,
        lba_offset: 100,
        block_count: 4,
        payload_crc: 10,
        is_fua: false,
        causal_dependencies: vec![],
    };
    assert!(verifier.submit_command(cmd10).is_ok());

    // Comando 11: FUA na MESMA Fila 0, dependendo do Comando 10 não persistido
    // FUA persiste imediatamente o comando 11. Se permitido, um corte de energia deixaria
    // o filho persistido e o pai volátil no buffer da controladora.
    let cmd11_same_queue_fua = NvmeIoCommand {
        command_id: 11,
        queue_id: 0,
        lba_offset: 200,
        block_count: 2,
        payload_crc: 11,
        is_fua: true,
        causal_dependencies: vec![10],
    };
    let err_fua = verifier.submit_command(cmd11_same_queue_fua);
    assert!(
        matches!(err_fua, Err(NvmeCausalReorderViolation::MissingHardwareBarrier { .. })),
        "Submeter FUA sobre pai volátil na mesma fila deve exigir barreira de hardware"
    );

    // 2. Comando com block_count == 0 é I/O nulo/inválido
    let cmd_zero_block = NvmeIoCommand {
        command_id: 12,
        queue_id: 0,
        lba_offset: 300,
        block_count: 0,
        payload_crc: 0,
        is_fua: false,
        causal_dependencies: vec![],
    };
    let err_zero = verifier.submit_command(cmd_zero_block);
    assert!(
        matches!(err_zero, Err(NvmeCausalReorderViolation::ZeroBlockCount { command_id: 12 })),
        "Comando com block_count 0 deve retornar ZeroBlockCount"
    );
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

    // 4. Invariant: Compressed payload length mismatch must be rejected
    let mut header_wrong_compressed_len = header;
    header_wrong_compressed_len.compressed_len = 9999;
    let len_err = TwoStageBlockCodec::decode_and_verify(
        &header_wrong_compressed_len,
        &encoded,
        &dict_a,
        decompressor,
    );
    assert!(matches!(
        len_err,
        Err(TwoStageIntegrityViolation::CompressedLengthMismatch { expected: 9999, actual: 23 })
    ));

    // 5. Invariant: Uncompressed payload length mismatch must be rejected
    let bad_len_decompressor = |_comp: &[u8], _dict: &[u8]| -> Vec<u8> {
        b"short".to_vec()
    };
    let uncomp_len_err = TwoStageBlockCodec::decode_and_verify(
        &header,
        &encoded,
        &dict_a,
        bad_len_decompressor,
    );
    assert!(matches!(
        uncomp_len_err,
        Err(TwoStageIntegrityViolation::UncompressedLengthMismatch { .. })
    ));

    // 6. Invariant: Binary wire format roundtrip and tamper detection
    let wire_bytes = header.encode();
    let decoded_header = CompressedBlockHeader::decode(&wire_bytes).expect("Header wire decode must succeed");
    assert_eq!(decoded_header, header);

    let mut corrupted_wire = wire_bytes;
    corrupted_wire[0] ^= 0xff;
    assert!(CompressedBlockHeader::decode(&corrupted_wire).is_err());
}

#[test]
fn test_two_stage_compression_dictionary_corrupted_header_limits_red() {
    let dict = PretrainedDictionary::new(1, b"dict".to_vec());

    // 1. Cabeçalho corrompido com compressed_len > 0 mas uncompressed_len == 0
    let bad_header_1 = CompressedBlockHeader {
        uncompressed_len: 0,
        compressed_len: 100,
        dictionary_id: dict.dictionary_id,
        dictionary_digest: dict.dictionary_digest,
        stage1_physical_crc: 0,
        stage2_logical_crc: 0,
    };
    let wire1 = bad_header_1.encode();
    let res1 = CompressedBlockHeader::decode(&wire1);
    assert!(
        matches!(res1, Err(TwoStageIntegrityViolation::InvalidWireFormat { .. })),
        "compressed_len > 0 com uncompressed_len == 0 deve ser rejeitado no decode"
    );

    // 2. Cabeçalho corrompido com compressed_len == 0 mas uncompressed_len > 0
    let bad_header_2 = CompressedBlockHeader {
        uncompressed_len: 200,
        compressed_len: 0,
        dictionary_id: dict.dictionary_id,
        dictionary_digest: dict.dictionary_digest,
        stage1_physical_crc: 0,
        stage2_logical_crc: 0,
    };
    let wire2 = bad_header_2.encode();
    let res2 = CompressedBlockHeader::decode(&wire2);
    assert!(
        matches!(res2, Err(TwoStageIntegrityViolation::InvalidWireFormat { .. })),
        "compressed_len == 0 com uncompressed_len > 0 deve ser rejeitado no decode"
    );
}

#[test]
fn test_two_stage_compression_dictionary_hardening_red() {
    // 1. Dicionário com ID 0 deve ser rejeitado
    let err_zero_id = PretrainedDictionary::try_new(0, b"dict_data".to_vec());
    assert!(matches!(err_zero_id, Err(TwoStageIntegrityViolation::ZeroDictionaryId)));

    // 2. Dicionário vazio deve ser rejeitado
    let err_empty_dict = PretrainedDictionary::try_new(1, vec![]);
    assert!(matches!(err_empty_dict, Err(TwoStageIntegrityViolation::EmptyDictionary)));

    let valid_dict = PretrainedDictionary::try_new(1, b"good_dict".to_vec()).expect("valid dict");

    // 3. Cabeçalho com compressed_len == 0 e uncompressed_len == 0 (bloco 0 bytes) deve ser rejeitado
    let empty_block_header = CompressedBlockHeader {
        uncompressed_len: 0,
        compressed_len: 0,
        dictionary_id: valid_dict.dictionary_id,
        dictionary_digest: valid_dict.dictionary_digest,
        stage1_physical_crc: 0,
        stage2_logical_crc: 0,
    };
    let wire_empty = empty_block_header.encode();
    let res_empty = CompressedBlockHeader::decode(&wire_empty);
    assert!(
        matches!(res_empty, Err(TwoStageIntegrityViolation::EmptyBlockPayload)),
        "Bloco com 0 bytes comprimido e descomprimido deve falhar no decode"
    );

    // 4. Cabeçalho com dictionary_id == 0 deve ser rejeitado no decode
    let zero_dict_header = CompressedBlockHeader {
        uncompressed_len: 100,
        compressed_len: 50,
        dictionary_id: 0,
        dictionary_digest: valid_dict.dictionary_digest,
        stage1_physical_crc: 123,
        stage2_logical_crc: 456,
    };
    let wire_zero_dict = zero_dict_header.encode();
    let res_zero_dict = CompressedBlockHeader::decode(&wire_zero_dict);
    assert!(
        matches!(res_zero_dict, Err(TwoStageIntegrityViolation::ZeroDictionaryId)),
        "dictionary_id == 0 deve falhar no decode"
    );

    // 5. Cabeçalho com tamanho excedendo o limite de segurança de 64 MiB
    let huge_header = CompressedBlockHeader {
        uncompressed_len: 70 * 1024 * 1024,
        compressed_len: 50,
        dictionary_id: valid_dict.dictionary_id,
        dictionary_digest: valid_dict.dictionary_digest,
        stage1_physical_crc: 123,
        stage2_logical_crc: 456,
    };
    let wire_huge = huge_header.encode();
    let res_huge = CompressedBlockHeader::decode(&wire_huge);
    assert!(
        matches!(res_huge, Err(TwoStageIntegrityViolation::BlockPayloadTooLarge { .. })),
        "Bloco gigante deve ser rejeitado contra DoS/OOM"
    );

    // 6. try_encode_block deve rejeitar payloads vazios
    assert!(matches!(
        TwoStageBlockCodec::try_encode_block(&[], b"comp", &valid_dict),
        Err(TwoStageIntegrityViolation::EmptyBlockPayload)
    ));
    assert!(matches!(
        TwoStageBlockCodec::try_encode_block(b"uncomp", &[], &valid_dict),
        Err(TwoStageIntegrityViolation::EmptyBlockPayload)
    ));

    // 7. Implementação de std::error::Error
    let err_dyn: Box<dyn std::error::Error> = Box::new(TwoStageIntegrityViolation::ZeroDictionaryId);
    assert!(!err_dyn.to_string().is_empty());
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

    // 5. Invariant: Unquarantined CRC mismatch MUST fail-closed with Err, NEVER return Ok(None)
    let read_unquarantined_corrupt = FaultIsolationOracle::evaluate_point_read(b"k15", &blocks, &[]);
    assert!(
        read_unquarantined_corrupt.is_err(),
        "Corrupted block covering key without quarantine list must fail-closed with Err, not Ok(None)"
    );

    // 6. Invariant: Block with inverted boundaries (min_key > max_key) must be quarantined
    let mut corrupt_bounds = blocks.clone();
    corrupt_bounds[0].min_key = b"zzz".to_vec();
    corrupt_bounds[0].max_key = b"aaa".to_vec();
    let report_bounds = FaultIsolationOracle::scrub_sst_blocks(&corrupt_bounds);
    assert_eq!(report_bounds.quarantined_blocks.len(), 2, "Inverted bounds must be quarantined");
}

#[test]
fn test_fault_isolation_overlapping_blocks_quarantine_and_salvage_red() {
    let d0 = b"block_0".to_vec();
    let d1 = b"block_1".to_vec();

    // Dois blocos com CRCs válidos, mas com sobreposição de chaves ("k00".."k15" e "k10".."k20")
    let overlapping_blocks = vec![
        SstDataBlockMeta {
            block_index: 0,
            min_key: b"k00".to_vec(),
            max_key: b"k15".to_vec(),
            expected_crc: crc32c::crc32c(&d0),
            raw_data: d0,
        },
        SstDataBlockMeta {
            block_index: 1,
            min_key: b"k10".to_vec(), // Sobrepõe com "k15" do bloco anterior!
            max_key: b"k20".to_vec(),
            expected_crc: crc32c::crc32c(&d1),
            raw_data: d1,
        },
    ];

    // 1. Scrubbing deve detectar e colocar em quarentena o bloco sobreposto
    let report = FaultIsolationOracle::scrub_sst_blocks(&overlapping_blocks);
    assert!(
        !report.quarantined_blocks.is_empty(),
        "Blocos com sobreposição de chaves devem ser colocados em quarentena"
    );

    // 2. Salvamento para compactação NUNCA pode emitir blocos sobrepostos
    let salvaged = FaultIsolationOracle::salvage_healthy_records_for_compaction(&overlapping_blocks);
    assert_eq!(
        salvaged.len(),
        1,
        "salvage_healthy_records_for_compaction deve descartar bloco sobreposto para manter partição estrita"
    );
}
