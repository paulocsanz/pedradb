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

use pedradb_core::arena_generational_aba_kernel::{AbaMemoryHazardViolation, GenerationalArena};
use pedradb_core::block_cache_hysteretic_partition_kernel::HystereticCacheGovernor;
use pedradb_core::column_family_epoch_wal_kernel::{
    ColumnFamilyCatalog, SharedWalRecord, SharedWalRecoveryOracle,
};
use pedradb_core::compaction_spectral_decoupling_kernel::{
    CompactionSpectralOracle, InterLevelCouplingMatrix,
};
use pedradb_core::composite_key_framing_kernel::OrderPreservingTupleCodec;
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

// -----------------------------------------------------------------------------
// 2. O Paradoxo do Vácuo de Tombstones e Invariante de Drenagem Ativa
// -----------------------------------------------------------------------------
#[test]
fn test_tombstone_vacuum_cascade_active_draining() {
    let config = TombstoneDrainConfig {
        size_threshold_bytes: 64 * 1024 * 1024, // 64 MiB
        min_tombstone_permille: 200,            // 20%
        tombstone_ttl_ticks: 500,
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
