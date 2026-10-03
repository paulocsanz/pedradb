//! Test Suite for RFC-0286: Cinco Fronteiras Matemáticas e Estruturais do Motor Puro PedraDB.
//!
//! Validates the 5 formal frontiers:
//! 1. Range Tombstone Fragmentation & Point Coverage Invariant
//! 2. Compaction Merge Operator Join-Semilattice Confluence
//! 3. RUM Amplification Pareto Boundary & IOPS Headroom Budget
//! 4. Decompression Scratch Arena Strict Isolation & Purge
//! 5. Linearizable MemTable Retirement & VersionSet Handshake

#![forbid(unsafe_code)]

use pedradb_core::compaction_merge_semilattice_kernel::{
    BitwiseOrMergeOperator, MaxU64MergeOperator, MergeSemilatticeOperator, MergeSemilatticeOracle,
    MergeSemilatticeViolation, MinU64MergeOperator,
};
use pedradb_core::decompression_scratch_isolation_kernel::{
    DecompressionScratchPool, DECOMPRESSION_SLOT_SIZE,
};
use pedradb_core::memtable_retirement_handshake_kernel::{
    AuthoritativeSource, FlushRetirementPhase, HandoverViolation, MemTableRetirementCoordinator,
};
use pedradb_core::range_tombstone_fragmentation_kernel::{
    RangeTombstone, RangeTombstoneSet,
};
use pedradb_core::rum_amplification_pareto_kernel::{
    RumBudgetConfig, RumBudgetViolation, RumParetoEvaluator,
};

#[test]
fn test_fronteira1_range_tombstone_fragmentation_coverage() {
    let mut set = RangeTombstoneSet::new();

    // 1. Tombstone [a, m) @ seq 100
    let t1 = RangeTombstone::new(b"a".to_vec(), b"m".to_vec(), 100).expect("Valid t1");
    // 2. Tombstone [d, z) @ seq 200
    let t2 = RangeTombstone::new(b"d".to_vec(), b"z".to_vec(), 200).expect("Valid t2");

    set.add(t1);
    set.add(t2);
    assert_eq!(set.count(), 2);

    // Key "c" (only in [a, m)):
    // Point seq 50: shadowed (100 > 50)
    assert!(set.is_key_shadowed(b"c", 50));
    // Point seq 150: NOT shadowed (100 < 150)
    assert!(!set.is_key_shadowed(b"c", 150));

    // Key "f" (in both [a, m) @ 100 and [d, z) @ 200):
    // Max covering seq is 200
    assert_eq!(set.max_covering_seq(b"f"), Some(200));
    // Point seq 150: shadowed by t2 (200 > 150)
    assert!(set.is_key_shadowed(b"f", 150));
    // Point seq 250: NOT shadowed by either
    assert!(!set.is_key_shadowed(b"f", 250));

    // Key "z" (boundary of [d, z)):
    // Strictly exclusive: NOT shadowed
    assert!(!set.is_key_shadowed(b"z", 50));

    // Validation: invalid range where start >= end returns Err
    assert!(RangeTombstone::new(b"z".to_vec(), b"a".to_vec(), 50).is_err());
    assert!(RangeTombstone::new(b"k".to_vec(), b"k".to_vec(), 50).is_err());
}

#[test]
fn test_range_tombstone_range_overlap_and_full_shadow_red() {
    // 1. Inverted or degenerate tombstone validation
    let bad_tomb = RangeTombstone {
        start: b"z".to_vec(),
        end: b"a".to_vec(),
        seq_num: 50,
    };
    assert!(!bad_tomb.is_valid());

    let mut set = RangeTombstoneSet::new();
    assert!(set.add_checked(bad_tomb.clone()).is_err());

    // 2. Direct injection of invalid tombstone does not corrupt fragmentation
    set.add(bad_tomb);
    let valid_t1 = RangeTombstone::new(b"a".to_vec(), b"m".to_vec(), 100).unwrap();
    let valid_t2 = RangeTombstone::new(b"m".to_vec(), b"z".to_vec(), 100).unwrap();
    set.add(valid_t1);
    set.add(valid_t2);

    let frags = set.fragment();
    // [a, m) and [m, z) coalesce into [a, z) @ 100, bad_tomb is safely ignored
    assert_eq!(frags.len(), 1);
    assert_eq!(frags[0].start, b"a");
    assert_eq!(frags[0].end, b"z");

    // 3. Range-range overlap checks
    let t = RangeTombstone::new(b"c".to_vec(), b"k".to_vec(), 100).unwrap();
    assert!(t.overlaps(b"a", b"d")); // overlaps [c, d)
    assert!(t.overlaps(b"e", b"g")); // interior overlap
    assert!(t.overlaps(b"j", b"z")); // overlaps [j, k)
    assert!(!t.overlaps(b"a", b"c")); // boundary touch [a, c) does not overlap [c, k)
    assert!(!t.overlaps(b"k", b"z")); // boundary touch [k, z) does not overlap [c, k)
    assert!(!t.overlaps(b"z", b"a")); // inverted query range

    // 4. Overlap query on set
    assert!(set.overlaps_range(b"b", b"y"));
    assert!(!set.overlaps_range(b"0", b"a"));

    // 5. Full range shadowing (contiguous coverage check)
    // set covers [a, z) @ 100
    assert!(set.is_range_fully_shadowed(b"b", b"h", 50));
    assert!(set.is_range_fully_shadowed(b"a", b"z", 50));
    assert!(!set.is_range_fully_shadowed(b"a", b"z", 100)); // seq not strictly greater
    assert!(!set.is_range_fully_shadowed(b"0", b"m", 50)); // [0, a) is uncovered

    // 6. Binary search lookup on fragmented set
    let frag_set = set.into_fragmented();
    let found = frag_set.seek_fragmented(b"m");
    assert!(found.is_some());
    assert_eq!(found.unwrap().seq_num, 100);
    assert!(frag_set.seek_fragmented(b"z").is_none());
}


#[test]
fn test_fronteira2_compaction_merge_semilattice_confluence() {
    // 1. Test BitwiseOrMergeOperator against semilattice axioms
    let bit_op = BitwiseOrMergeOperator;
    let bit_samples: [&[u8]; 3] = [b"\x0F", b"\xF0", b"\x55"];
    assert!(MergeSemilatticeOracle::verify_axioms(&bit_op, &bit_samples).is_ok());

    // 2. Test MaxU64MergeOperator against semilattice axioms
    let max_op = MaxU64MergeOperator;
    let b10 = 10u64.to_be_bytes();
    let b20 = 20u64.to_be_bytes();
    let b50 = 50u64.to_be_bytes();
    let max_samples: [&[u8]; 3] = [&b10, &b20, &b50];
    assert!(MergeSemilatticeOracle::verify_axioms(&max_op, &max_samples).is_ok());

    // 3. Confluence under crash duplicate replay and reordering
    // Sequence 1: 10, 50, 20
    let seq1: [&[u8]; 3] = [&b10, &b50, &b20];
    let res1 = MergeSemilatticeOracle::fold_confluent(&max_op, &seq1);

    // Sequence 2 (with crash duplicates and permutation): 20, 10, 50, 50, 10
    let seq2: [&[u8]; 5] = [&b20, &b10, &b50, &b50, &b10];
    let res2 = MergeSemilatticeOracle::fold_confluent(&max_op, &seq2);

    assert_eq!(res1, res2);
    assert_eq!(u64::from_be_bytes(res1.try_into().unwrap()), 50);
}

#[test]
fn test_compaction_merge_semilattice_strict_violations_and_min_operator_red() {
    // 1. Rejeição de amostras vazias
    let bit_op = BitwiseOrMergeOperator;
    let empty_res = MergeSemilatticeOracle::verify_axioms_strict(&bit_op, &[]);
    assert_eq!(empty_res, Err(MergeSemilatticeViolation::EmptySampleSet));

    // 2. Operador não-comutativo e não-idempotente deve falhar
    struct NonCommutativeAppendOp;
    impl MergeSemilatticeOperator for NonCommutativeAppendOp {
        fn merge(&self, a: &[u8], b: &[u8]) -> Vec<u8> {
            let mut res = a.to_vec();
            res.extend_from_slice(b);
            res
        }
        fn bottom(&self) -> Vec<u8> {
            Vec::new()
        }
    }
    let non_op = NonCommutativeAppendOp;
    let fail_res = MergeSemilatticeOracle::verify_axioms_strict(&non_op, &[b"a", b"b"]);
    assert!(
        matches!(
            fail_res,
            Err(MergeSemilatticeViolation::IdempotenceViolated { .. })
                | Err(MergeSemilatticeViolation::CommutativityViolated { .. })
        ),
        "Operador de concatenação não deve ser aceito como semirrede: {:?}",
        fail_res
    );

    // 3. Bottom/Identidade incorreto deve ser rejeitado
    struct BrokenBottomOp;
    impl MergeSemilatticeOperator for BrokenBottomOp {
        fn merge(&self, a: &[u8], b: &[u8]) -> Vec<u8> {
            MaxU64MergeOperator.merge(a, b)
        }
        fn bottom(&self) -> Vec<u8> {
            9999u64.to_be_bytes().to_vec()
        }
    }
    let broken_bottom = BrokenBottomOp;
    let b10 = 10u64.to_be_bytes();
    let b_res = MergeSemilatticeOracle::verify_axioms_strict(&broken_bottom, &[&b10]);
    assert!(
        matches!(b_res, Err(MergeSemilatticeViolation::BottomIdentityViolated { .. })),
        "Bottom element que viola identidade deve ser rejeitado: {:?}",
        b_res
    );

    // 4. MinU64MergeOperator satisfaz todos os axiomas
    let min_op = MinU64MergeOperator;
    let b20 = 20u64.to_be_bytes();
    let b50 = 50u64.to_be_bytes();
    let min_samples: [&[u8]; 3] = [&b10, &b20, &b50];
    assert!(MergeSemilatticeOracle::verify_axioms_strict(&min_op, &min_samples).is_ok());

    // 5. Confluência de MinU64MergeOperator com duplicatas e reordenação
    let min_seq: [&[u8]; 5] = [&b50, &b10, &b20, &b10, &b50];
    let min_folded = MergeSemilatticeOracle::fold_confluent(&min_op, &min_seq);
    assert_eq!(u64::from_be_bytes(min_folded.try_into().unwrap()), 10);
}


#[test]
fn test_fronteira3_rum_amplification_pareto_budget() {
    let config = RumBudgetConfig::default();
    let evaluator = RumParetoEvaluator::new(config);
    let bounds = evaluator.compute_bounds();

    // 1. Write Amplification bound: T * L = 10 * 7 = 70
    assert_eq!(bounds.max_write_amp, 70);

    // 2. Read Amplification bound: 1.0 + 7 * 0.01 = 1.07 (1070 permille)
    assert_eq!(bounds.max_read_amp_permille, 1070);

    // 3. IOPS split: 30% of 100,000 = 30,000 reserved for reads
    assert_eq!(bounds.reserved_read_iops, 30_000);
    assert_eq!(bounds.max_compaction_iops, 70_000);

    // 4. Budget check
    assert!(evaluator.is_compaction_within_budget(50_000));
    assert!(evaluator.is_compaction_within_budget(70_000));
    assert!(!evaluator.is_compaction_within_budget(70_001));
}

#[test]
fn test_rum_pareto_budget_validation_space_amp_and_overflow_red() {
    // 1. Validação de configurações inválidas
    let bad_reserved = RumBudgetConfig {
        reserved_read_iops_percent: 105, // > 100%
        ..Default::default()
    };
    assert_eq!(
        bad_reserved.validate(),
        Err(RumBudgetViolation::ReservedPercentageExceeds100 { percent: 105 })
    );

    let zero_levels = RumBudgetConfig {
        num_levels: 0,
        ..Default::default()
    };
    assert_eq!(zero_levels.validate(), Err(RumBudgetViolation::InvalidNumLevels));

    let bad_ratio = RumBudgetConfig {
        level_ratio: 1, // ratio < 2
        ..Default::default()
    };
    assert_eq!(bad_ratio.validate(), Err(RumBudgetViolation::InvalidLevelRatio));

    let zero_iops = RumBudgetConfig {
        total_device_iops: 0,
        ..Default::default()
    };
    assert_eq!(zero_iops.validate(), Err(RumBudgetViolation::ZeroTotalDeviceIops));

    // 2. Proteção contra overflow aritmético em total_device_iops astronômico
    let huge_iops = RumBudgetConfig {
        total_device_iops: u64::MAX,
        reserved_read_iops_percent: 50,
        ..Default::default()
    };
    let huge_eval = RumParetoEvaluator::new(huge_iops);
    let bounds_res = huge_eval.checked_compute_bounds();
    assert!(bounds_res.is_ok(), "Cálculo com u64::MAX não deve entrar em pânico: {:?}", bounds_res);

    // 3. Avaliação de Space Amplification (RUM completeness)
    let def_eval = RumParetoEvaluator::new(RumBudgetConfig::default());
    let bounds = def_eval.compute_bounds();
    // Para T = 10, SA = 1 + 1/(T-1) ≈ 1.111 (1111 permille) ou 1 + 1/T = 1.100 (1100 permille)
    assert!(bounds.max_space_amp_permille >= 1100 && bounds.max_space_amp_permille <= 1112);

    // 4. Admissão de IOPS com proteção do piso de leitura
    let admission = def_eval.admit_compaction_iops(50_000);
    assert_eq!(admission, Ok(50_000));
    let capped_admission = def_eval.admit_compaction_iops(90_000);
    assert_eq!(capped_admission, Ok(70_000), "Burst de compactação deve ser limitado ao teto seguro");
}


#[test]
fn test_fronteira4_decompression_scratch_isolation() {
    let pool = DecompressionScratchPool::with_slots(2);
    assert_eq!(pool.capacity(), 2);
    assert_eq!(pool.active_leases(), 0);

    // 1. Concurrent checkouts get distinct slot indices
    let mut lease1 = pool.acquire_lease().expect("Lease 1");
    let mut lease2 = pool.acquire_lease().expect("Lease 2");
    assert_ne!(lease1.slot_index(), lease2.slot_index());
    assert_eq!(pool.active_leases(), 2);

    // 2. Thread 1 decompresses data into slot 1
    let secret_payload = b"sensitive_tenant_data_block";
    let n1 = lease1
        .execute_decompression(|buf| {
            buf[..secret_payload.len()].copy_from_slice(secret_payload);
            Ok(secret_payload.len())
        })
        .expect("Decompress succeeds");
    assert_eq!(n1, secret_payload.len());

    let mut out_buf = [0u8; 64];
    let copied = lease1.copy_out(&mut out_buf).expect("Copy out");
    assert_eq!(&out_buf[..copied], secret_payload);

    // 3. Thread 2 buffer is completely independent and unpolluted
    let n2 = lease2
        .execute_decompression(|buf| {
            buf[0] = 0xAA;
            Ok(1)
        })
        .expect("Decompress 2");
    assert_eq!(n2, 1);

    // 4. Dropping lease 1 triggers zeroization purge
    let slot1_idx = lease1.slot_index();
    let slot1_gen = lease1.generation();
    drop(lease1);
    assert_eq!(pool.active_leases(), 1);

    // 5. Re-acquiring slot 1 proves zero residual plaintext
    let lease1_reacquired = pool.acquire_lease().expect("Reacquire");
    assert_eq!(lease1_reacquired.slot_index(), slot1_idx);
    assert!(lease1_reacquired.generation() > slot1_gen);

    let mut dest_check = [0xFFu8; DECOMPRESSION_SLOT_SIZE];
    let zero_len = lease1_reacquired.copy_out(&mut dest_check).expect("Copy out empty");
    assert_eq!(zero_len, 0); // written_len was reset to 0
}

#[test]
fn test_decompression_scratch_aborted_purge_and_zero_copy_slice_red() {
    let pool = DecompressionScratchPool::with_slots(1);

    // 1. Falha durante a descompressão (abort/corrupção) não deve deixar resíduo de plaintext
    {
        let mut lease = pool.acquire_lease().expect("Lease");
        let fail_res = lease.execute_decompression(|buf| {
            buf[..15].copy_from_slice(b"TOP_SECRET_LEAK");
            Err("Stream CRC mismatch")
        });
        assert!(fail_res.is_err());
        // Lease é dropado aqui
    }

    // 2. Reaquisição do slot deve provar que os 15 bytes secretos foram zerados (anti-forense completo)
    {
        let lease2 = pool.acquire_lease().expect("Lease 2");
        let is_purged = lease2
            .with_decompressed_slice(|slice| {
                assert_eq!(slice.len(), 0);
            })
            .is_ok();
        assert!(is_purged, "with_decompressed_slice deve funcionar");

        // Checagem direta de memória no slot via lease:
        // Executamos uma descompressão de 1 byte para poder fazer copy_out de 16 bytes
        let mut lease2 = lease2;
        lease2
            .execute_decompression(|buf| {
                buf[0] = 0x42;
                Ok(16) // inspeciona os primeiros 16 bytes do buffer
            })
            .unwrap();

        let mut check_buf = [0xFFu8; 16];
        lease2.copy_out(&mut check_buf).unwrap();
        // Byte 0 é 0x42, mas bytes 1..15 NUNCA devem conter "OP_SECRET_LEAK"
        assert_eq!(&check_buf[1..15], &[0u8; 14], "Bytes vazados do erro anterior não foram zerados!");
    }

    // 3. Execuções repetidas no mesmo lease: se a 2a falhar, não deve reter dados da 1a
    {
        let mut lease = pool.acquire_lease().expect("Lease");
        lease
            .execute_decompression(|buf| {
                buf[..5].copy_from_slice(b"FIRST");
                Ok(5)
            })
            .unwrap();

        // 2a chamada falha
        let second_res = lease.execute_decompression(|_buf| Err("Second failed"));
        assert!(second_res.is_err());

        // copy_out agora deve falhar ou ter len 0, nunca retornar "FIRST"
        let mut out = [0u8; 5];
        let copy_res = lease.copy_out(&mut out);
        assert!(
            copy_res.is_err() || copy_res.unwrap() == 0,
            "Chamada falha não deve manter dados da chamada anterior: {:?}",
            copy_res
        );
    }
}


#[test]
fn test_fronteira5_memtable_retirement_handshake_linearizability() {
    let mut coordinator = MemTableRetirementCoordinator::new();
    assert_eq!(coordinator.current_phase(), FlushRetirementPhase::FlushingToDisk);

    // 1. Prior to version installation, reads resolve to ImmutableMemTable
    assert_eq!(coordinator.resolve_authoritative_source(50), "ImmutableMemTable");
    assert_eq!(coordinator.resolve_authoritative_source(999), "ImmutableMemTable");

    // 2. Premature reclamation fails
    assert_eq!(
        coordinator.mark_memtable_reclaimed(),
        Err(HandoverViolation::PrematureReclamation)
    );

    // 3. MANIFEST commits version at epoch 100
    assert!(coordinator.mark_version_installed(100).is_ok());
    assert_eq!(
        coordinator.current_phase(),
        FlushRetirementPhase::VersionInstalledOnDisk
    );

    // 4. Dual-source resolution:
    // Snapshot initiated at epoch 50 (< 100) resolves to ImmutableMemTable
    assert_eq!(coordinator.resolve_authoritative_source(50), "ImmutableMemTable");
    // Snapshot initiated at epoch 100 (== 100) resolves to VersionSetL0
    assert_eq!(coordinator.resolve_authoritative_source(100), "VersionSetL0");
    // Snapshot initiated at epoch 150 (> 100) resolves to VersionSetL0
    assert_eq!(coordinator.resolve_authoritative_source(150), "VersionSetL0");

    // 5. Safely reclaim MemTable
    assert!(coordinator.mark_memtable_reclaimed().is_ok());
    assert_eq!(
        coordinator.current_phase(),
        FlushRetirementPhase::MemTableReclaimed
    );
}

#[test]
fn test_memtable_retirement_active_readers_and_post_reclaim_isolation_red() {
    let mut coordinator = MemTableRetirementCoordinator::new();

    // 1. Pinar leitor na MemTable imutável
    assert!(coordinator.pin_memtable_reader().is_ok());
    assert_eq!(coordinator.active_readers(), 1);

    // 2. Instalar versão no disco
    assert!(coordinator.mark_version_installed(100).is_ok());

    // 3. Tentativa de reclamar com leitor ativo deve ser bloqueada fail-closed
    let reclaim_err = coordinator.mark_memtable_reclaimed();
    assert_eq!(reclaim_err, Err(HandoverViolation::ActiveReadersPending { count: 1 }));

    // 4. Liberar leitor e reclamar com sucesso
    coordinator.unpin_memtable_reader();
    assert_eq!(coordinator.active_readers(), 0);
    assert!(coordinator.mark_memtable_reclaimed().is_ok());

    // 5. Dupla reclamação não é PrematureReclamation e sim IllegalPhaseTransition
    let double_reclaim = coordinator.mark_memtable_reclaimed();
    assert_eq!(
        double_reclaim,
        Err(HandoverViolation::IllegalPhaseTransition {
            from: FlushRetirementPhase::MemTableReclaimed,
            to: FlushRetirementPhase::MemTableReclaimed,
        })
    );

    // 6. Resolução segura pós-reclamacão:
    // Snapshot anterior ao commit (50 < 100) aponta para fonte já destruída -> Erro explícito
    let stale_read = coordinator.resolve_source(50);
    assert_eq!(
        stale_read,
        Err(HandoverViolation::ReclaimedSourceUnavailable {
            read_epoch: 50,
            commit_epoch: 100,
        })
    );

    // Snapshot contemporâneo ou posterior resolve perfeitamente para VersionSetL0
    assert_eq!(coordinator.resolve_source(100), Ok(AuthoritativeSource::VersionSetL0));
    assert_eq!(coordinator.resolve_source(120), Ok(AuthoritativeSource::VersionSetL0));
}

#[test]
fn test_memtable_retirement_underflow_and_epoch_guards_red() {
    let mut coordinator = MemTableRetirementCoordinator::new();

    // 1. try_unpin when no readers are active must fail with NoActiveReadersToUnpin
    let err_unpin = coordinator.try_unpin_memtable_reader();
    assert_eq!(err_unpin, Err(HandoverViolation::NoActiveReadersToUnpin));

    // unpin_memtable_reader must NOT underflow active_readers
    coordinator.unpin_memtable_reader();
    assert_eq!(coordinator.active_readers(), 0, "active_readers must not underflow usize");

    // 2. mark_version_installed must reject commit_epoch 0
    let err_epoch0 = coordinator.mark_version_installed(0);
    assert_eq!(err_epoch0, Err(HandoverViolation::InvalidCommitEpoch { epoch: 0 }));

    // 3. mark_version_installed must reject commit_epoch u64::MAX
    let err_epoch_max = coordinator.mark_version_installed(u64::MAX);
    assert_eq!(err_epoch_max, Err(HandoverViolation::InvalidCommitEpoch { epoch: u64::MAX }));

    // 4. RAII guard auto-unpins on drop
    {
        let guard = coordinator.acquire_reader_guard().expect("acquire guard ok");
        assert_eq!(coordinator.active_readers(), 1);
        drop(guard);
    }
    assert_eq!(coordinator.active_readers(), 0, "RAII guard must unpin reader on drop");
}


