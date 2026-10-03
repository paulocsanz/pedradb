//! RFC-0294: Suíte de Verificação das Dez Fronteiras Fundacionais do LSM PedraDB.
//!
//! Cobre exaustivamente:
//! 1. Invertibilidade Estrita e Monotonicidade em Restart Points sob Compressão Delta
//! 2. O Paradoxo do Vácuo de Tombstones e Invariante de Drenagem Ativa
//! 3. Cobertura Temporal Fechada e Visibilidade Estável de Snapshot sob Compactação
//! 4. Álgebra de Codificação Injetiva de Chaves Compostas (Prefix-Free Framing)
//! 5. Tagging Generacional de Arenas e Imunidade ao Problema ABA
//! 6. Desacoplamento Espectral e Amortecimento de Ressonância Harmônica
//! 7. Fecho de Coerência de Inodes e Consistência Topológica em Hot-Backup
//! 8. Autômato Histerético de Partição de Cache de Blocos
//! 9. Isolamento Epocal de Column Families no WAL Compartilhado
//! 10. Leases Temporais Atômicos de I/O com Revogação Pré-Syscall

use pedradb_core::arena_generational_aba_kernel::{
    AbaMemoryHazardViolation, GenerationalArena, GenerationalPtr,
};
use pedradb_core::block_cache_hysteretic_partition_kernel::HystereticCacheGovernor;
use pedradb_core::column_family_epoch_wal_kernel::{
    ColumnFamilyCatalog, ColumnFamilyEpochViolation, SharedWalRecord, SharedWalRecoveryOracle,
};
use pedradb_core::compaction_spectral_decoupling_kernel::{
    CompactionSpectralOracle, InterLevelCouplingMatrix, SpectralResonanceViolation,
};
use pedradb_core::composite_key_framing_kernel::{FramingViolation, OrderPreservingTupleCodec};
use pedradb_core::delta_restart_monotonicity_kernel::DeltaBlock;
use pedradb_core::hot_backup_inode_closure_kernel::{
    BackupDirectoryState, BackupIncompletenessViolation, HotBackupClosureOracle,
    ManifestSnapshotView,
};
use pedradb_core::preemptible_io_lease_kernel::{
    IoLeaseToken, PreSyscallGuardDecision, PreSyscallIoGuard, PreemptedIoLeaseViolation,
};
use pedradb_core::snapshot_compaction_stability_kernel::{
    ActiveSnapshotDescriptor, SnapshotCompactionStabilityOracle, SstRecordEntry,
};
use pedradb_core::tombstone_vacuum_cascade_kernel::{
    CompactionTriggerReason, LevelTombstoneState, TombstoneDrainConfig, TombstoneVacuumController,
    TombstoneVacuumViolation,
};

use std::collections::HashSet;

// -----------------------------------------------------------------------------
// 1. Invertibilidade Estrita e Monotonicidade em Restart Points sob Compressão Delta
// -----------------------------------------------------------------------------
#[test]
fn test_delta_restart_monotonicity_and_seek_invertibility() {
    let kvs = vec![
        (b"customer:001:profile".to_vec(), b"alice".to_vec()),
        (b"customer:001:settings".to_vec(), b"dark_mode".to_vec()),
        (b"customer:002:profile".to_vec(), b"bob".to_vec()),
        (b"customer:003:profile".to_vec(), b"carol".to_vec()),
        (b"order:1001:item".to_vec(), b"laptop".to_vec()),
        (b"order:1002:item".to_vec(), b"phone".to_vec()),
    ];

    // Cria bloco com restart points a cada 2 chaves
    let block = DeltaBlock::encode(&kvs, 2);

    // Decodificação completa com bijeção estrita
    let decoded = block.decode_all().expect("Decodificação sequencial deve ser válida");
    assert_eq!(decoded, kvs, "D(E(K)) deve ser identicamente igual a K");

    // Seek via busca binária nos restart points
    let found = block.seek(b"customer:002:profile").unwrap();
    assert_eq!(found, Some((b"customer:002:profile".to_vec(), b"bob".to_vec())));

    let found_order = block.seek(b"order:1002:item").unwrap();
    assert_eq!(found_order, Some((b"order:1002:item".to_vec(), b"phone".to_vec())));

    // Seek por chave inexistente
    let missing = block.seek(b"customer:001:missing").unwrap();
    assert!(missing.is_none());
}

#[test]
fn test_delta_restart_monotonicity_edge_cases_red() {
    let kvs = vec![
        (b"key:20".to_vec(), b"val20".to_vec()),
        (b"key:30".to_vec(), b"val30".to_vec()),
        (b"key:40".to_vec(), b"val40".to_vec()),
        (b"key:50".to_vec(), b"val50".to_vec()),
    ];

    let block = DeltaBlock::encode(&kvs, 2);

    // 1. target_key menor que o primeiro restart point
    // Não deve escanear desnecessariamente nem dar falso positivo
    let before_first = block.seek(b"key:10").unwrap();
    assert!(before_first.is_none());

    // 2. target_key maior que o último restart point mas menor que a última chave
    let mid_last = block.seek(b"key:45").unwrap();
    assert!(mid_last.is_none());

    // 3. target_key exatamente igual à primeira chave
    let first = block.seek(b"key:20").unwrap();
    assert_eq!(first, Some((b"key:20".to_vec(), b"val20".to_vec())));

    // 4. seek em bloco vazio
    let empty_block = DeltaBlock::encode(&[], 2);
    assert_eq!(empty_block.seek(b"any").unwrap(), None);
}

#[test]
fn test_delta_restart_corrupted_indices_and_unsorted_keys_red() {
    use pedradb_core::delta_restart_monotonicity_kernel::{DeltaDecodingViolation, DeltaEncodedEntry};

    // 1. Chaves não-ordenadas devem ser rejeitadas em decode_all com UnsortedKeySequence
    let unsorted_kvs = vec![
        (b"beta".to_vec(), b"1".to_vec()),
        (b"alpha".to_vec(), b"2".to_vec()),
    ];
    let bad_block = DeltaBlock::encode(&unsorted_kvs, 1);
    let decode_err = bad_block.decode_all();
    assert!(
        matches!(decode_err, Err(DeltaDecodingViolation::UnsortedKeySequence { .. })),
        "decode_all deve detectar violação de ordem lexicográfica entre chaves consecutivas"
    );

    // 2. Restart point com índice fora dos limites (buffer overrun / corrupted metadata) não pode entrar em pânico
    let corrupt_block = DeltaBlock::from_raw_parts(
        vec![DeltaEncodedEntry {
            shared_len: 0,
            unshared_suffix: b"hello".to_vec(),
            value: b"world".to_vec(),
        }],
        vec![999], // 999 está fora dos limites de entries (len = 1)
    );
    let seek_err = corrupt_block.seek(b"hello");
    assert!(
        matches!(seek_err, Err(DeltaDecodingViolation::CorruptedRestartPointIndex { .. })),
        "seek em bloco com restart_points corrompidos deve retornar erro, nunca entrar em pânico"
    );
}

// -----------------------------------------------------------------------------
// 2. O Paradoxo do Vácuo de Tombstones e Invariante de Drenagem Ativa
// -----------------------------------------------------------------------------
#[test]
fn test_tombstone_vacuum_cascade_active_draining() {
    let config = TombstoneDrainConfig {
        size_threshold_bytes: 64 * 1024 * 1024, // 64 MiB
        min_tombstone_permille: 200,            // 20%
        tombstone_ttl_ticks: 500,
        max_space_amplification_permille: 2000,
    };
    let controller = TombstoneVacuumController::new(config);

    // Nível pequeno (1 MiB), mas com 50% de tombstones antigos (idade 600 > 500)
    let starved_level = LevelTombstoneState {
        level: 2,
        total_bytes: 1 * 1024 * 1024,
        total_records: 10_000,
        tombstone_records: 5_000, // 50%
        oldest_tombstone_age_ticks: 600,
    };

    let decision = controller.evaluate_level(&starved_level);
    assert_eq!(
        decision,
        Some(CompactionTriggerReason::ActiveTombstoneDrain),
        "Drenagem ativa de tombstones deve quebrar o paradoxo do vácuo"
    );

    let verify_res = controller.verify_draining_invariants(&[starved_level]);
    assert!(verify_res.is_ok());

    // 3. Invariant: Tombstone density must be strictly clamped to [0, 1000]
    let corrupt_level = LevelTombstoneState {
        level: 1,
        total_bytes: 1000,
        total_records: 100,
        tombstone_records: 500,
        oldest_tombstone_age_ticks: 100,
    };
    assert_eq!(corrupt_level.tombstone_permille(), 1000);

    // 4. Invariant: Space amplification limit evaluation
    let sa_err = controller.evaluate_space_amplification(10_000_000, 1_000_000);
    assert!(matches!(
        sa_err,
        Err(TombstoneVacuumViolation::SpaceAmplificationExceeded { .. })
    ));
}

#[test]
fn test_tombstone_vacuum_pure_dead_level_and_corrupted_space_accounting_red() {
    let config = TombstoneDrainConfig {
        size_threshold_bytes: 64 * 1024 * 1024,
        min_tombstone_permille: 200,
        tombstone_ttl_ticks: 500,
        max_space_amplification_permille: 2000,
    };
    let controller = TombstoneVacuumController::new(config);

    // 1. Contabilidade impossível: allocated_bytes (0) < live_bytes (5000)
    let impossible_sa = controller.evaluate_space_amplification(0, 5000);
    assert!(
        matches!(impossible_sa, Err(TombstoneVacuumViolation::CorruptedSpaceAccounting { allocated_bytes: 0, live_bytes: 5000 })),
        "allocated_bytes < live_bytes deve ser detectado como CorruptedSpaceAccounting"
    );

    // 2. Nível 100% morto (pure dead level): 1000 registros, 1000 tombstones, mas idade recente (idade 10 < TTL 500)
    // Não pode ficar esperando TTL de 500 ticks quando 100% do nível é lixo não-ergódico
    let pure_dead_level = LevelTombstoneState {
        level: 3,
        total_bytes: 100_000,
        total_records: 1_000,
        tombstone_records: 1_000,
        oldest_tombstone_age_ticks: 10,
    };
    let decision = controller.evaluate_level(&pure_dead_level);
    assert_eq!(
        decision,
        Some(CompactionTriggerReason::ActiveTombstoneDrain),
        "Nível 100% tombstones deve drenar imediatamente sem esperar expiração de TTL"
    );

    // 3. evaluate_level_with_space_amp deve disparar SpaceAmplificationExceeded
    let bloated_level = LevelTombstoneState {
        level: 1,
        total_bytes: 5_000_000, // 5 MB alocados
        total_records: 10_000,
        tombstone_records: 100, // baixa densidade
        oldest_tombstone_age_ticks: 10,
    };
    // 5MB alocados para 1MB vivo = 5.0x (> 2.0x max)
    let sa_decision = controller.evaluate_level_with_space_amp(&bloated_level, 1_000_000);
    assert_eq!(
        sa_decision,
        Some(CompactionTriggerReason::SpaceAmplificationExceeded),
        "Estouro de amplificação de espaço em evaluate_level_with_space_amp deve retornar SpaceAmplificationExceeded"
    );
}

#[test]
fn test_tombstone_vacuum_cascade_hardening_red() {
    // 1. Configurações inválidas de drenagem devem falhar na validação
    let bad_cfg1 = TombstoneDrainConfig {
        size_threshold_bytes: 0,
        ..TombstoneDrainConfig::default()
    };
    assert!(matches!(
        bad_cfg1.validate(),
        Err(TombstoneVacuumViolation::InvalidConfig { .. })
    ));

    let bad_cfg2 = TombstoneDrainConfig {
        min_tombstone_permille: 1500, // > 1000 permille
        ..TombstoneDrainConfig::default()
    };
    assert!(matches!(
        bad_cfg2.validate(),
        Err(TombstoneVacuumViolation::InvalidConfig { .. })
    ));

    let bad_cfg3 = TombstoneDrainConfig {
        max_space_amplification_permille: 500, // < 1000 (menor que 1.0x)
        ..TombstoneDrainConfig::default()
    };
    assert!(matches!(
        bad_cfg3.validate(),
        Err(TombstoneVacuumViolation::InvalidConfig { .. })
    ));

    // 2. try_new deve rejeitar configuração inválida
    assert!(matches!(
        TombstoneVacuumController::try_new(bad_cfg1),
        Err(TombstoneVacuumViolation::InvalidConfig { .. })
    ));

    let valid_controller = TombstoneVacuumController::try_new(TombstoneDrainConfig::default())
        .expect("Valid controller creation");

    // 3. LevelTombstoneState::validate deve detectar contabilidade impossível de registros
    let impossible_records_state = LevelTombstoneState {
        level: 1,
        total_bytes: 10_000,
        total_records: 100,
        tombstone_records: 200, // 200 > 100!
        oldest_tombstone_age_ticks: 10,
    };
    assert!(matches!(
        impossible_records_state.validate(),
        Err(TombstoneVacuumViolation::CorruptedRecordAccounting { total_records: 100, tombstone_records: 200 })
    ));

    // 4. try_evaluate_level_with_space_amp deve falhar fechado com CorruptedSpaceAccounting se total_bytes < live_bytes
    let undercounted_level = LevelTombstoneState {
        level: 1,
        total_bytes: 1_000,
        total_records: 50,
        tombstone_records: 10,
        oldest_tombstone_age_ticks: 10,
    };
    let err_undercount = valid_controller.try_evaluate_level_with_space_amp(&undercounted_level, 5_000);
    assert!(matches!(
        err_undercount,
        Err(TombstoneVacuumViolation::CorruptedSpaceAccounting { allocated_bytes: 1000, live_bytes: 5000 })
    ));

    // 5. verify_draining_invariants deve rejeitar níveis com contabilidade impossível de registros
    assert!(matches!(
        valid_controller.verify_draining_invariants(&[impossible_records_state]),
        Err(TombstoneVacuumViolation::CorruptedRecordAccounting { .. })
    ));

    // 6. Implementação de std::error::Error
    let err_dyn: Box<dyn std::error::Error> = Box::new(TombstoneVacuumViolation::InvalidConfig { reason: "test" });
    assert!(!err_dyn.to_string().is_empty());
}

// -----------------------------------------------------------------------------
// 3. Cobertura Temporal Fechada e Visibilidade Estável de Snapshot sob Compactação
// -----------------------------------------------------------------------------
#[test]
fn test_snapshot_compaction_stability_and_tombstone_retention() {
    let key = b"session_token:user_99";

    // Snapshot ativo aberto no seq 100
    let snapshot = ActiveSnapshotDescriptor {
        snapshot_id: 1,
        snapshot_seq: 100,
    };

    // Registros antes da compactação:
    // seq 10: Put("val1")
    // seq 90: Delete (tombstone)
    // seq 110: Put("val2") (fora do snapshot)
    let pre_compaction = vec![
        SstRecordEntry {
            key: key.to_vec(),
            seq: 110,
            is_tombstone: false,
            value: Some(b"val2".to_vec()),
        },
        SstRecordEntry {
            key: key.to_vec(),
            seq: 90,
            is_tombstone: true,
            value: None,
        },
        SstRecordEntry {
            key: key.to_vec(),
            seq: 10,
            is_tombstone: false,
            value: Some(b"val1".to_vec()),
        },
    ];

    // Como há um snapshot vivo em seq 100 >= 90, o tombstone NÃO PODE ser purgado!
    assert!(
        !SnapshotCompactionStabilityOracle::can_purge_tombstone(90, &[snapshot]),
        "Tombstone cobrindo chave em snapshot ativo não pode ser purgado"
    );

    let post_compaction = SnapshotCompactionStabilityOracle::compact_entries(
        &pre_compaction,
        &[snapshot],
        true, // bottommost
    );

    // O leitor de snapshot deve enxergar exatamente None (deletado) antes e depois
    let inv_res = SnapshotCompactionStabilityOracle::verify_snapshot_invariance(
        key,
        snapshot,
        &pre_compaction,
        &post_compaction,
    );
    assert!(inv_res.is_ok(), "Visibilidade de snapshot deve ser invariante sob compactação");
}

#[test]
fn test_snapshot_compaction_no_shadowed_resurrection_red() {
    let key = b"user:123";

    // 1. Catástrofe de Ressurreição: Tombstone no topo purgado sem purgar o Put antigo mascarado por ele
    let records = vec![
        SstRecordEntry {
            key: key.to_vec(),
            seq: 90,
            is_tombstone: true,
            value: None,
        },
        SstRecordEntry {
            key: key.to_vec(),
            seq: 10,
            is_tombstone: false,
            value: Some(b"deleted_payload".to_vec()),
        },
    ];

    // Sem snapshots no nível mais baixo: o tombstone é purgado, MAS o Put antigo DEVE ser purgado junto!
    let compacted = SnapshotCompactionStabilityOracle::compact_entries(
        &records,
        &[],
        true, // bottommost
    );
    assert!(
        compacted.is_empty(),
        "Tombstone purgado no nível mais baixo não pode deixar o Put antigo vivo (Ressurreição Fantasma): {:?}",
        compacted
    );

    // 2. Múltiplos Puts da mesma chave sem snapshots intermediários: versões antigas sombreadas devem ser descartadas
    let multi_puts = vec![
        SstRecordEntry {
            key: key.to_vec(),
            seq: 200,
            is_tombstone: false,
            value: Some(b"newest_value".to_vec()),
        },
        SstRecordEntry {
            key: key.to_vec(),
            seq: 150,
            is_tombstone: false,
            value: Some(b"stale_value_1".to_vec()),
        },
        SstRecordEntry {
            key: key.to_vec(),
            seq: 100,
            is_tombstone: false,
            value: Some(b"stale_value_2".to_vec()),
        },
    ];

    let compacted_puts = SnapshotCompactionStabilityOracle::compact_entries(
        &multi_puts,
        &[],
        false,
    );
    assert_eq!(
        compacted_puts.len(),
        1,
        "Versões sombreadas sem snapshots ativos devem ser podadas na compactação: {:?}",
        compacted_puts
    );
    assert_eq!(compacted_puts[0].seq, 200);
}

// -----------------------------------------------------------------------------
// 4. Álgebra de Codificação Injetiva de Chaves Compostas (Prefix-Free Framing)
// -----------------------------------------------------------------------------
#[test]
fn test_composite_key_order_preserving_prefix_free_framing() {
    // Caso de ataque de colapso de separador:
    // Tupla 1: tenant="tenantA/sub", user="alice"
    // Tupla 2: tenant="tenantA", user="sub/alice"
    let t1: &[&[u8]] = &[b"tenantA/sub", b"alice"];
    let t2: &[&[u8]] = &[b"tenantA", b"sub/alice"];

    let enc1 = OrderPreservingTupleCodec::encode_tuple(t1);
    let enc2 = OrderPreservingTupleCodec::encode_tuple(t2);

    assert_ne!(enc1, enc2, "Chaves compostas distintas JAMAIS devem colapsar nos mesmos bytes");

    // Prova de bijeção e preservação de ordem
    let res = OrderPreservingTupleCodec::verify_tuple_invariants(t1, t2);
    assert!(res.is_ok(), "Homomorfismo de ordem e bijeção devem ser estritamente preservados");

    // Teste com bytes nulos (0x00) internos
    let t_null1: &[&[u8]] = &[b"user\x00name", b"id1"];
    let t_null2: &[&[u8]] = &[b"user\x00name", b"id2"];
    let res_null = OrderPreservingTupleCodec::verify_tuple_invariants(t_null1, t_null2);
    assert!(res_null.is_ok(), "Bytes nulos devem ser escapados sem alterar a ordem lexicográfica");
}

#[test]
fn test_composite_key_framing_edge_cases_red() {
    // 1. decode_tuple em slice vazio deve retornar Ok(vec![])
    let empty_dec = OrderPreservingTupleCodec::decode_tuple(&[]);
    assert_eq!(empty_dec, Ok(vec![]));

    // 2. decode_tuple em trailing escape 0x00 isolado ou truncado
    let malformed1 = OrderPreservingTupleCodec::decode_tuple(&[0x00]);
    assert_eq!(malformed1, Err(FramingViolation::MalformedFramingSequence));

    let malformed2 = OrderPreservingTupleCodec::decode_tuple(&[0x01, 0x02, 0x00]);
    assert_eq!(malformed2, Err(FramingViolation::MalformedFramingSequence));

    // 3. Delimitador desconhecido após 0x00 (ex: 0x00 0x42)
    let malformed3 = OrderPreservingTupleCodec::decode_tuple(&[0x00, 0x42]);
    assert_eq!(malformed3, Err(FramingViolation::MalformedFramingSequence));

    // 4. Tupla com componente vazio: &[]
    let t_empty: &[&[u8]] = &[b""];
    let enc_empty = OrderPreservingTupleCodec::encode_tuple(t_empty);
    let dec_empty = OrderPreservingTupleCodec::decode_tuple(&enc_empty).unwrap();
    assert_eq!(dec_empty, vec![b"".to_vec()]);

    // 5. Tupla vazia de componentes: &[]
    let t_none: &[&[u8]] = &[];
    let enc_none = OrderPreservingTupleCodec::encode_tuple(t_none);
    let dec_none = OrderPreservingTupleCodec::decode_tuple(&enc_none).unwrap();
    assert_eq!(dec_none, Vec::<Vec<u8>>::new());
}

#[test]
fn test_composite_key_prefix_range_upper_bound_red() {
    // 1. encode_tuple_owned com componentes pertencidos
    let owned = vec![b"tenantA".to_vec(), b"user1".to_vec()];
    let enc_owned = OrderPreservingTupleCodec::encode_tuple_owned(&owned);
    let enc_borrowed = OrderPreservingTupleCodec::encode_tuple(&[&b"tenantA"[..], &b"user1"[..]]);
    assert_eq!(enc_owned, enc_borrowed);

    // 2. prefix_upper_bound para isolamento estrito de prefixos multi-tenant
    let prefix = OrderPreservingTupleCodec::encode_tuple(&[&b"tenantA"[..]]);
    let upper = OrderPreservingTupleCodec::prefix_upper_bound(&prefix).expect("Upper bound deve existir");

    // Qualquer tupla pertencente a tenantA deve ser >= prefix e < upper
    let child1 = OrderPreservingTupleCodec::encode_tuple(&[&b"tenantA"[..], &b"alice"[..]]);
    let child2 = OrderPreservingTupleCodec::encode_tuple(&[&b"tenantA"[..], &b"bob"[..], &b"doc99"[..]]);
    assert!(child1.as_slice() >= prefix.as_slice() && child1.as_slice() < upper.as_slice());
    assert!(child2.as_slice() >= prefix.as_slice() && child2.as_slice() < upper.as_slice());

    // Qualquer tupla de outro tenant (ex: tenantB) deve ser >= upper
    let foreign = OrderPreservingTupleCodec::encode_tuple(&[&b"tenantB"[..], &b"alice"[..]]);
    assert!(foreign.as_slice() >= upper.as_slice());

    // 3. Vetor só com 0xFF não tem limitador superior válido
    assert_eq!(OrderPreservingTupleCodec::prefix_upper_bound(&[0xFF, 0xFF]), None);
}

// -----------------------------------------------------------------------------
// 5. Tagging Generacional de Arenas e Imunidade ao Problema ABA
// -----------------------------------------------------------------------------
#[test]
fn test_arena_generational_aba_immunity() {
    let mut arena = GenerationalArena::new(2, 4096);

    // Alocação na página 0, geração 1
    let ptr1 = arena.allocate_at(0, 0, b"data_generation_1").unwrap();
    assert_eq!(ptr1.generation_id, 1);

    // Leitura válida
    let read_val = arena.read_ptr(ptr1, 17).unwrap();
    assert_eq!(read_val, b"data_generation_1");

    // Reciclagem da página 0 (avança geração para 2 e zera memória)
    let new_gen = arena.recycle_page(0);
    assert_eq!(new_gen, 2);

    // Leitor retardado tentando desreferenciar o ponteiro antigo (geração 1)
    let stale_access = arena.read_ptr(ptr1, 17);
    assert!(
        matches!(stale_access, Err(AbaMemoryHazardViolation::StaleGenerationalReference { .. })),
        "Acesso via ponteiro de geração obsoleta deve ser imediatamente bloqueado contra perigo ABA"
    );

    // Nova alocação legítima na geração 2
    let ptr2 = arena.allocate_at(0, 0, b"data_generation_2").unwrap();
    assert_eq!(ptr2.generation_id, 2);
    let read_new = arena.read_ptr(ptr2, 17).unwrap();
    assert_eq!(read_new, b"data_generation_2");
}

#[test]
fn test_arena_generational_aba_edge_cases_red() {
    let mut arena = GenerationalArena::new(2, 4096);

    // 1. page_index inválido (out of bounds) em allocate_at
    let bad_page_alloc = arena.allocate_at(5, 0, b"data");
    assert!(bad_page_alloc.is_err());
    assert!(matches!(bad_page_alloc, Err(AbaMemoryHazardViolation::InvalidPageIndex { page_index: 5, total_pages: 2 })));

    // 2. page_index inválido em read_ptr
    let bad_ptr = GenerationalPtr {
        generation_id: 1,
        page_index: 99,
        offset: 0,
    };
    let bad_page_read = arena.read_ptr(bad_ptr, 10);
    assert!(bad_page_read.is_err());
    assert!(matches!(bad_page_read, Err(AbaMemoryHazardViolation::InvalidPageIndex { page_index: 99, total_pages: 2 })));

    // 3. Overflow aritmético em offset + len
    let ptr_overflow = arena.allocate_at(0, 0, b"test").unwrap();
    let overflow_read = arena.read_ptr(ptr_overflow, usize::MAX);
    assert!(overflow_read.is_err());
    assert!(matches!(overflow_read, Err(AbaMemoryHazardViolation::PageOffsetOutOfBounds { .. })));

    // 4. recycle_page com page_index inválido deve retornar 0 ou falhar com segurança sem panic
    let rec_res = arena.recycle_page(42);
    assert_eq!(rec_res, 0);
}

// -----------------------------------------------------------------------------
// 6. Desacoplamento Espectral e Amortecimento de Ressonância Harmônica
// -----------------------------------------------------------------------------
#[test]
fn test_compaction_spectral_decoupling_and_resonance_damping() {
    // Matriz de acoplamento 4x4 (L0, L1, L2, L3)
    // Retenção própria diagonal = 0.4 (< 1.0)
    // Ganho em cascata para nível inferior = 0.2
    let matrix = InterLevelCouplingMatrix::build_damped_cascade(4, 0.4, 0.2);

    // Validação formal de raio espectral rho(A) <= 0.8 < 1.0
    let radius = CompactionSpectralOracle::verify_spectral_damping(&matrix, 0.8)
        .expect("O raio espectral deve estar estritamente dentro do limiar de amortecimento");
    assert!(radius < 0.8, "Raio espectral deve garantir convergência exponencial");

    // Teste de decaimento sob choque impulsivo (rajada maciça em L0)
    let impulse = vec![1000.0, 0.0, 0.0, 0.0];
    let decay_res = CompactionSpectralOracle::verify_impulse_decay(&matrix, &impulse, 20);
    assert!(decay_res.is_ok(), "Perturbação impulsiva deve se dissipar sem ressonância harmônica");
}

#[test]
fn test_compaction_spectral_decoupling_edge_cases_red() {
    // 1. Matriz de tamanho 0: compute_spectral_radius deve retornar 0.0 sem panic
    let m_zero = InterLevelCouplingMatrix::build_damped_cascade(0, 0.5, 0.5);
    let r_zero = m_zero.compute_spectral_radius(10);
    assert_eq!(r_zero, 0.0);

    // 2. multiply_vector com vetor de tamanho incompatível não deve dar panic em produção
    let m4 = InterLevelCouplingMatrix::build_damped_cascade(4, 0.4, 0.2);
    let out = m4.multiply_vector(&[1.0, 2.0]);
    assert_eq!(out.len(), 4);

    // 3. Matriz com diagonal retention >= 1.0 deve falhar no verify_spectral_damping
    let m_unstable = InterLevelCouplingMatrix::build_damped_cascade(4, 1.2, 0.5);
    let damp_res = CompactionSpectralOracle::verify_spectral_damping(&m_unstable, 0.9);
    assert!(damp_res.is_err());
}

#[test]
fn test_compaction_spectral_nan_and_dimension_mismatch_red() {
    let m4 = InterLevelCouplingMatrix::build_damped_cascade(4, 0.4, 0.2);

    // 1. Vetor de impulso com NaN deve ser rejeitado imediatamente
    let nan_res = CompactionSpectralOracle::verify_impulse_decay(&m4, &[f64::NAN, 0.0, 0.0, 0.0], 10);
    assert!(
        matches!(nan_res, Err(SpectralResonanceViolation::NonFiniteOrCorruptedEnergy { .. })),
        "Impulso com NaN deve ser rejeitado: {:?}",
        nan_res
    );

    // 2. Dimensão incompatível entre matriz e vetor de impulso
    let dim_res = CompactionSpectralOracle::verify_impulse_decay(&m4, &[100.0, 50.0], 10);
    assert!(
        matches!(dim_res, Err(SpectralResonanceViolation::DimensionMismatch { matrix_size: 4, vector_size: 2 })),
        "Incompatibilidade de dimensões deve ser rejeitada: {:?}",
        dim_res
    );

    // 3. Limiar de estabilidade inválido (>= 1.0 ou <= 0.0)
    let invalid_thresh = CompactionSpectralOracle::verify_spectral_damping(&m4, 1.5);
    assert!(
        matches!(invalid_thresh, Err(SpectralResonanceViolation::InvalidStabilityThreshold { .. })),
        "Limiar de estabilidade >= 1.0 deve ser rejeitado: {:?}",
        invalid_thresh
    );

    // 4. Matriz explosiva em horizonte curto (steps = 3) não pode passar despercebida
    let m_explosive = InterLevelCouplingMatrix::build_damped_cascade(4, 5.0, 2.0);
    let explosive_res = CompactionSpectralOracle::verify_impulse_decay(&m_explosive, &[1000.0, 0.0, 0.0, 0.0], 3);
    assert!(
        matches!(explosive_res, Err(SpectralResonanceViolation::HarmonicStandingWaveDetected { .. })),
        "Explosão harmônica em horizonte curto deve ser detectada: {:?}",
        explosive_res
    );
}

// -----------------------------------------------------------------------------
// 7. Fecho de Coerência de Inodes e Consistência Topológica em Hot-Backup
// -----------------------------------------------------------------------------
#[test]
fn test_hot_backup_inode_closure_and_topological_consistency() {
    let mut referenced_ssts = HashSet::new();
    referenced_ssts.insert(1);
    referenced_ssts.insert(2);
    referenced_ssts.insert(3);

    let manifest_view = ManifestSnapshotView {
        manifest_seq: 42,
        referenced_sst_ids: referenced_ssts,
    };

    let mut backup = BackupDirectoryState::default();
    backup.add_file("MANIFEST-000042");
    backup.add_file("000001.sst");
    backup.add_file("000002.sst");
    backup.add_file("000003.sst");

    let verify_res = HotBackupClosureOracle::verify_backup_topological_closure(&manifest_view, &backup);
    assert!(verify_res.is_ok(), "Backup completo com todos os SSTs deve ser aprovado");

    // Cenário de defeito: compactação concorrente causou a falta do SST #2 no backup
    let mut broken_backup = backup.clone();
    broken_backup.files.remove("000002.sst");

    let fail_res = HotBackupClosureOracle::verify_backup_topological_closure(&manifest_view, &broken_backup);
    assert!(
        matches!(fail_res, Err(BackupIncompletenessViolation::MissingReferencedSstFile { file_id: 2, .. })),
        "Buraco no backup por ausência de SST referenciado no MANIFEST deve ser detectado"
    );
}

#[test]
fn test_hot_backup_inode_closure_edge_cases_red() {
    let mut referenced_ssts = HashSet::new();
    referenced_ssts.insert(1);
    referenced_ssts.insert(2);

    let manifest_view = ManifestSnapshotView {
        manifest_seq: 42,
        referenced_sst_ids: referenced_ssts,
    };

    // 1. Ghost SST file: backup contém SST não catalogado 000099.sst
    let mut ghost_backup = BackupDirectoryState::default();
    ghost_backup.add_file("MANIFEST-000042");
    ghost_backup.add_file("000001.sst");
    ghost_backup.add_file("000002.sst");
    ghost_backup.add_file("000099.sst");
    let ghost_res = HotBackupClosureOracle::verify_backup_topological_closure(&manifest_view, &ghost_backup);
    assert!(
        matches!(ghost_res, Err(BackupIncompletenessViolation::GhostFileInBackup { ref file_name }) if file_name == "000099.sst"),
        "SST fantasma não catalogado deve ser rejeitado: {:?}",
        ghost_res
    );

    // 2. Mismatched Manifest version: manifest_seq = 42, mas backup tem MANIFEST-000099
    let mut wrong_manifest_backup = BackupDirectoryState::default();
    wrong_manifest_backup.add_file("MANIFEST-000099");
    wrong_manifest_backup.add_file("000001.sst");
    wrong_manifest_backup.add_file("000002.sst");
    let wrong_man_res = HotBackupClosureOracle::verify_backup_topological_closure(&manifest_view, &wrong_manifest_backup);
    assert!(
        matches!(wrong_man_res, Err(BackupIncompletenessViolation::MissingOrCorruptedManifest)),
        "MANIFEST com versão incompatível deve ser rejeitado"
    );

    // 3. Alien / corrupt SST file name
    let mut corrupt_sst_backup = BackupDirectoryState::default();
    corrupt_sst_backup.add_file("MANIFEST-000042");
    corrupt_sst_backup.add_file("000001.sst");
    corrupt_sst_backup.add_file("000002.sst");
    corrupt_sst_backup.add_file("alien_table.sst");
    let corrupt_res = HotBackupClosureOracle::verify_backup_topological_closure(&manifest_view, &corrupt_sst_backup);
    assert!(
        matches!(corrupt_res, Err(BackupIncompletenessViolation::GhostFileInBackup { ref file_name }) if file_name == "alien_table.sst"),
        "Arquivo .sst com formato alienígena deve ser rejeitado como fantasma"
    );

    // 4. Backup vazio
    let empty_backup = BackupDirectoryState::default();
    let empty_res = HotBackupClosureOracle::verify_backup_topological_closure(&manifest_view, &empty_backup);
    assert!(matches!(empty_res, Err(BackupIncompletenessViolation::MissingOrCorruptedManifest)));
}

#[test]
fn test_hot_backup_conflicting_manifest_and_zero_sst_id_red() {
    let mut referenced_ssts = HashSet::new();
    referenced_ssts.insert(1);
    referenced_ssts.insert(2);

    let manifest_view = ManifestSnapshotView {
        manifest_seq: 42,
        referenced_sst_ids: referenced_ssts,
    };

    // 1. Conflicting multiple MANIFEST files in backup (e.g. MANIFEST-000042 AND MANIFEST-000099)
    let mut multi_manifest_backup = BackupDirectoryState::default();
    multi_manifest_backup.add_file("MANIFEST-000042");
    multi_manifest_backup.add_file("MANIFEST-000099"); // Conflicting manifest!
    multi_manifest_backup.add_file("000001.sst");
    multi_manifest_backup.add_file("000002.sst");

    let err_multi = HotBackupClosureOracle::verify_backup_topological_closure(&manifest_view, &multi_manifest_backup);
    assert!(
        matches!(err_multi, Err(BackupIncompletenessViolation::ConflictingManifestFiles { .. })),
        "Multiple conflicting MANIFEST files in backup must be rejected: {:?}", err_multi
    );

    // 2. Invalid referenced SST ID 0 in manifest_view
    let mut bad_sst_ids = HashSet::new();
    bad_sst_ids.insert(0); // ID 0 is invalid
    let bad_manifest_view = ManifestSnapshotView {
        manifest_seq: 42,
        referenced_sst_ids: bad_sst_ids,
    };
    let mut backup = BackupDirectoryState::default();
    backup.add_file("MANIFEST-000042");
    backup.add_file("000000.sst");

    let err_zero_id = HotBackupClosureOracle::verify_backup_topological_closure(&bad_manifest_view, &backup);
    assert!(
        matches!(err_zero_id, Err(BackupIncompletenessViolation::InvalidReferencedSstId { file_id: 0 })),
        "Referenced SST ID 0 must be rejected: {:?}", err_zero_id
    );
}


// -----------------------------------------------------------------------------
// 8. Autômato Histerético de Partição de Cache de Blocos
// -----------------------------------------------------------------------------
#[test]
fn test_block_cache_hysteretic_partition_anti_thrashing() {
    let capacity = 100 * 1024 * 1024; // 100 MiB
    let floor_fraction = 0.3;          // 30% piso de metadados = 30 MiB
    let mut cache = HystereticCacheGovernor::new(capacity, floor_fraction);

    // Carrega 30 MiB de índices e filtros
    cache.admit_metadata_block(30 * 1024 * 1024);
    assert_eq!(cache.state().metadata_bytes, 30 * 1024 * 1024);

    // Simula um scan sequencial massivo de 500 MiB de dados de usuário
    for _ in 0..50 {
        cache.admit_data_block(10 * 1024 * 1024, true);
    }

    // Validação formal: a cota de metadados JAMAIS pode cair abaixo do piso garantido
    assert!(
        cache.state().metadata_bytes >= cache.state().metadata_floor_bytes,
        "Piso de metadados deve ser inviolável sob varreduras analíticas"
    );
    let verify_res = cache.verify_cache_invariants();
    assert!(verify_res.is_ok());
}

#[test]
fn test_block_cache_hysteretic_edge_cases_red() {
    let capacity = 100 * 1024 * 1024; // 100 MiB
    let floor_fraction = 0.3;          // 30 MiB
    let mut cache = HystereticCacheGovernor::new(capacity, floor_fraction);

    // 1. Carrega metadados e dados simultaneamente até o teto
    cache.admit_metadata_block(50 * 1024 * 1024);
    cache.admit_data_block(50 * 1024 * 1024, false);
    assert_eq!(cache.state().metadata_bytes + cache.state().data_bytes, capacity);

    // 2. Tenta admitir mais 20 MiB de metadados quando cache já está cheio:
    // Deve desalojar dados (data evictions) para dar espaço aos metadados,
    // mantendo total <= capacity!
    cache.admit_metadata_block(20 * 1024 * 1024);
    assert!(cache.state().metadata_bytes <= capacity);
    assert!(cache.state().metadata_bytes + cache.state().data_bytes <= capacity);
    assert!(cache.verify_cache_invariants().is_ok());

    // 3. Overflow numérico em admit_metadata_block ou admit_data_block com u64::MAX
    cache.admit_metadata_block(u64::MAX);
    assert!(cache.state().metadata_bytes <= capacity);
    assert!(cache.state().metadata_bytes + cache.state().data_bytes <= capacity);

    cache.admit_data_block(u64::MAX, false);
    assert!(cache.state().metadata_bytes + cache.state().data_bytes <= capacity);

    // 4. floor_fraction com NaN ou negativo
    let safe_cache = HystereticCacheGovernor::new(1000, f64::NAN);
    assert!(safe_cache.state().metadata_floor_bytes <= 1000);
}

// -----------------------------------------------------------------------------
// 9. Isolamento Epocal de Column Families no WAL Compartilhado
// -----------------------------------------------------------------------------
#[test]
fn test_column_family_epochal_isolation_in_shared_wal() {
    let mut catalog = ColumnFamilyCatalog::default();
    let cf_id = 7;

    // Encarnação 1 da CF 7
    let inc1 = catalog.register_or_recreate(cf_id);
    assert_eq!(inc1, 1);

    let wal_records = vec![
        // Mutação na encarnação 1
        SharedWalRecord {
            cf_id,
            cf_incarnation: 1,
            seq: 50,
            key: b"user:alice".to_vec(),
            value: Some(b"profile_v1".to_vec()),
        },
    ];

    // O usuário executa DROP e recriação da CF 7 (encarnação avança para 2)
    let inc2 = catalog.register_or_recreate(cf_id);
    assert_eq!(inc2, 2);

    // Replay do WAL pós-crash
    let recovered = SharedWalRecoveryOracle::replay_wal(&wal_records, &catalog)
        .expect("Replay do WAL deve executar sem erros");

    // A chave "user:alice" da encarnação 1 NÃO DEVE estar presente no estado recuperado!
    assert!(
        !recovered.contains_key(&(cf_id, b"user:alice".to_vec())),
        "Dados da encarnação morta anterior ao DROP não devem ressuscitar na nova encarnação"
    );

    let dead_records = vec![SharedWalRecord {
        cf_id,
        cf_incarnation: 1,
        seq: 50,
        key: b"user:alice".to_vec(),
        value: Some(b"profile_v1".to_vec()),
    }];
    let iso_res = SharedWalRecoveryOracle::verify_drop_isolation(&recovered, &dead_records);
    assert!(iso_res.is_ok());
}

#[test]
fn test_column_family_epochal_isolation_edge_cases_red() {
    let mut catalog = ColumnFamilyCatalog::default();
    catalog.register_or_recreate(1);

    // 1. set_incarnation com regressão deve falhar com IncarnationRegressed
    let reg_res = catalog.set_incarnation(1, 0);
    assert_eq!(reg_res, Err(ColumnFamilyEpochViolation::IncarnationRegressed {
        cf_id: 1,
        prev_incarnation: 1,
        new_incarnation: 0,
    }));

    // 2. Wal records com sequências regressivas para a mesma chave dentro da mesma encarnação
    // devem respeitar a última escrita por seq (LWW - Last Write Wins por sequence number)
    let records = vec![
        SharedWalRecord {
            cf_id: 1,
            cf_incarnation: 1,
            seq: 100,
            key: b"key".to_vec(),
            value: Some(b"val_100".to_vec()),
        },
        SharedWalRecord {
            cf_id: 1,
            cf_incarnation: 1,
            seq: 50, // Out of order seq
            key: b"key".to_vec(),
            value: Some(b"val_50_stale".to_vec()),
        },
    ];
    let recovered = SharedWalRecoveryOracle::replay_wal(&records, &catalog).unwrap();
    assert_eq!(recovered.get(&(1, b"key".to_vec())), Some(&Some(b"val_100".to_vec())));

    // 3. Replay de registro para CF nunca registrada no catálogo (active_inc = 0) com cf_incarnation > 0
    let unregistered_records = vec![SharedWalRecord {
        cf_id: 999,
        cf_incarnation: 1,
        seq: 1,
        key: b"k".to_vec(),
        value: Some(b"v".to_vec()),
    }];
    let res = SharedWalRecoveryOracle::replay_wal(&unregistered_records, &catalog);
    assert!(matches!(res, Err(ColumnFamilyEpochViolation::DeadIncarnationResurrected { cf_id: 999, .. })));
}

// -----------------------------------------------------------------------------
// 10. Leases Temporais Atômicos de I/O com Revogação Pré-Syscall
// -----------------------------------------------------------------------------
#[test]
fn test_preemptible_io_lease_guard_and_phantom_burst_prevention() {
    let token = IoLeaseToken {
        token_id: 888,
        bytes_allowed: 10 * 1024 * 1024, // 10 MiB
        issued_at_tick: 1000,
        max_duration_ticks: 50, // Válido até tick 1050
    };

    // Caso 1: I/O executado imediatamente (tick 1020 <= 1050)
    let normal_decision = PreSyscallIoGuard::evaluate_guard(&token, 1020);
    assert_eq!(normal_decision, PreSyscallGuardDecision::ProceedWithIo);
    let normal_verify = PreSyscallIoGuard::verify_execution_safety(&token, 1020);
    assert!(normal_verify.is_ok());

    // Caso 2: A thread sofreu preempção pelo kernel ou vCPU steal de 200 ticks (tick 1200 > 1050)
    let preempted_decision = PreSyscallIoGuard::evaluate_guard(&token, 1200);
    assert!(
        matches!(preempted_decision, PreSyscallGuardDecision::AbortAndRenegotiate { overrun_ticks: 150 }),
        "Lease expirado por preempção de kernel deve ser abortado antes da syscall"
    );

    let fail_verify = PreSyscallIoGuard::verify_execution_safety(&token, 1200);
    assert!(
        matches!(fail_verify, Err(PreemptedIoLeaseViolation::ExpiredIoLeaseExecuted { .. })),
        "Submissão de I/O sob lease expirado deve ser detectada e bloqueada"
    );
}

#[test]
fn test_preemptible_io_lease_edge_cases_red() {
    let token = IoLeaseToken {
        token_id: 1,
        bytes_allowed: 1000,
        issued_at_tick: 500,
        max_duration_ticks: 100,
    };

    // 1. Clock skew / regression: current_tick < issued_at_tick (retrocesso no relógio)
    // Não deve mascarar o tempo decorrido como 0 silenciosamente; deve abortar com segurança
    let skew_decision = PreSyscallIoGuard::evaluate_guard(&token, 400);
    assert_eq!(skew_decision, PreSyscallGuardDecision::AbortAndRenegotiate { overrun_ticks: 0 });

    let skew_verify = PreSyscallIoGuard::verify_execution_safety(&token, 400);
    assert!(matches!(skew_verify, Err(PreemptedIoLeaseViolation::ClockSkewDetected { .. })));

    // 2. verify_quota_safety com bytes requisitados acima do autorizado
    let quota_check = PreSyscallIoGuard::verify_quota_safety(&token, 2000);
    assert_eq!(quota_check, Err(PreemptedIoLeaseViolation::QuotaExceededPerIo {
        requested_bytes: 2000,
        max_allowed_bytes: 1000,
    }));
}
