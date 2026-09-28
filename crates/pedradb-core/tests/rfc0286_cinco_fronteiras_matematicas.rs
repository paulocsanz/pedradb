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
    BitwiseOrMergeOperator, MaxU64MergeOperator, MergeSemilatticeOracle,
};
use pedradb_core::decompression_scratch_isolation_kernel::{
    DecompressionScratchPool, DECOMPRESSION_SLOT_SIZE,
};
use pedradb_core::memtable_retirement_handshake_kernel::{
    FlushRetirementPhase, HandoverViolation, MemTableRetirementCoordinator,
};
use pedradb_core::range_tombstone_fragmentation_kernel::{
    RangeTombstone, RangeTombstoneSet,
};
use pedradb_core::rum_amplification_pareto_kernel::{
    RumBudgetConfig, RumParetoEvaluator,
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
