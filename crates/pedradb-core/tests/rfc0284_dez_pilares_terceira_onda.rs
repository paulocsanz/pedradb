//! Test Suite for RFC-0284: Os Dez Pilares da Terceira Onda de Verificação Avançada.
//!
//! Validates the 10 formal pillars:
//! 1. Prefix-Free Composite Key Injective Encoding
//! 2. Bloom Hash Entropy & Per-Table Salt Protection
//! 3. Compaction Debt Pacing & Write-Stall Prevention
//! 4. Super-Atomic MultiGet Snapshot Coherence
//! 5. FTL Erase-Block Alignment & Physical WAF Minimization
//! 6. Cross-Device EXDEV Safe Migration Protocol
//! 7. Sequence Number Horizon & Overflow Prevention
//! 8. Pre-Manifest Orphan SST Recovery Scrubber
//! 9. Bitemporal Primary-Secondary Index Mutual Visibility
//! 10. Async Cancellation Safety & Group Leader Handover

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use pedradb_core::async_cancellation_kernel::{
    CommitSlot, CommitSlotState, GroupCommitReconciler,
};
use pedradb_core::bitemporal_index_kernel::{
    AtomicIndexBatch, BitemporalIndexOracle, PrimaryMutation, SecondaryIndexMutation,
};
use pedradb_core::bloom_hash_entropy_kernel::{
    EntropyBloomFilter, FilterDensityStatus,
};
use pedradb_core::compaction_pacing_kernel::{
    CompactionPacer, PacingConfig, PacingDecision,
};
use pedradb_core::cross_device_barrier_kernel::{
    CrossDevicePhase, CrossDeviceTransferOracle, CrossDeviceViolation,
};
use pedradb_core::ftl_erase_boundary_kernel::{
    FlashGeometry, FtlAlignmentViolation, FtlEraseBoundaryPlanner,
};
use pedradb_core::orphan_sst_cleanup_kernel::{
    CatalogReconciliationError, PreManifestOrphanScrubber,
};
use pedradb_core::prefix_free_key_kernel::{
    InternalValueType, KeyCodecError, PrefixFreeKeyCodec,
};
use pedradb_core::sequence_horizon_kernel::{
    SequenceAllocation, SequenceHorizonAllocator, SEQUENCE_SAFE_CEILING,
};
use pedradb_core::super_atomic_multiget_kernel::{
    SuperAtomicMultiGetCoordinator, TableEntry,
};

#[test]
fn test_pilar1_prefix_free_key_injective_and_zero_byte() {
    // 1. Injective roundtrip with null bytes embedded in user key
    let user_key_with_nulls = b"user\x00key\x00with\x00delimiters";
    let encoded = PrefixFreeKeyCodec::encode(
        user_key_with_nulls,
        42,
        InternalValueType::Value,
    );
    let decoded = PrefixFreeKeyCodec::decode(&encoded).expect("Decode should succeed");
    assert_eq!(decoded.user_key, user_key_with_nulls);
    assert_eq!(decoded.sequence_number, 42);
    assert_eq!(decoded.value_type, InternalValueType::Value);

    // 2. MVCC Ordering: higher seq sorts before lower seq for identical key
    let k_seq100 = PrefixFreeKeyCodec::encode(b"alpha", 100, InternalValueType::Value);
    let k_seq50 = PrefixFreeKeyCodec::encode(b"alpha", 50, InternalValueType::Value);
    assert_eq!(
        PrefixFreeKeyCodec::compare(&k_seq100, &k_seq50).unwrap(),
        std::cmp::Ordering::Less // k_seq100 is earlier/fresher in iteration
    );

    // 3. User key order takes precedence
    let k_beta = PrefixFreeKeyCodec::encode(b"beta", 10, InternalValueType::Value);
    assert_eq!(
        PrefixFreeKeyCodec::compare(&k_seq100, &k_beta).unwrap(),
        std::cmp::Ordering::Less
    );

    // 4. Injective distinctness: "a" vs "a\0"
    let enc_a = PrefixFreeKeyCodec::encode(b"a", 1, InternalValueType::Value);
    let enc_a0 = PrefixFreeKeyCodec::encode(b"a\x00", 1, InternalValueType::Value);
    assert_ne!(enc_a, enc_a0);

    // 5. Error handling
    assert_eq!(
        PrefixFreeKeyCodec::decode(&[0, 1, 2]),
        Err(KeyCodecError::BufferTooShort)
    );
}

#[test]
fn test_pilar2_bloom_hash_entropy_salt_and_saturation() {
    let mut filter_a = EntropyBloomFilter::new(1024, 4, 0x1111_2222_3333_4444);
    let mut filter_b = EntropyBloomFilter::new(1024, 4, 0xAAAA_BBBB_CCCC_DDDD);

    let key = b"adversarial_target_key";

    // Different salts must produce distinct probe hash pairs
    let (h1_a, h2_a) = filter_a.hash_pair(key);
    let (h1_b, h2_b) = filter_b.hash_pair(key);
    assert_ne!((h1_a, h2_a), (h1_b, h2_b));

    // Insert into A and verify membership
    filter_a.insert(key);
    assert!(filter_a.may_contain(key));

    // Filter A should be healthy (< 50% density)
    match filter_a.density_status() {
        FilterDensityStatus::Healthy { permille_set } => {
            assert!(permille_set < 500);
        }
        FilterDensityStatus::OverSaturated { .. } => panic!("Filter A should be healthy"),
    }

    // Force saturation on filter B
    for i in 0..500u32 {
        let k = i.to_le_bytes();
        filter_b.insert(&k);
    }
    match filter_b.density_status() {
        FilterDensityStatus::OverSaturated { permille_set } => {
            assert!(permille_set >= 500);
        }
        FilterDensityStatus::Healthy { .. } => panic!("Filter B should be saturated"),
    }
}

#[test]
fn test_pilar3_compaction_pacing_write_stall_prevention() {
    let config = PacingConfig {
        soft_debt_bytes: 64 * 1024 * 1024,
        hard_debt_bytes: 256 * 1024 * 1024,
        max_delay_micros: 5_000,
        base_delay_micros: 200,
    };
    let pacer = CompactionPacer::new(config);

    // 1. Debt below soft limit: no delay
    assert_eq!(pacer.evaluate_debt(32 * 1024 * 1024), PacingDecision::NoDelay);

    // 2. Debt in soft range: smooth backpressure
    let mid_debt = 160 * 1024 * 1024; // halfway between 64 and 256
    match pacer.evaluate_debt(mid_debt) {
        PacingDecision::SoftPacing { delay_micros, debt_bytes } => {
            assert_eq!(debt_bytes, mid_debt);
            assert!(delay_micros > 200 && delay_micros < 5_000);
        }
        _ => panic!("Expected soft pacing"),
    }

    // 3. Debt at or above hard limit: capped at max_delay_micros
    match pacer.evaluate_debt(300 * 1024 * 1024) {
        PacingDecision::HardPacing { delay_micros, .. } => {
            assert_eq!(delay_micros, 5_000);
        }
        _ => panic!("Expected hard pacing"),
    }
}

#[test]
fn test_pilar4_super_atomic_multiget_snapshot_coherence() {
    let mut coordinator = SuperAtomicMultiGetCoordinator::new();

    // Initial version 2
    let mut v1_data = BTreeMap::new();
    v1_data.insert(
        b"key_a".to_vec(),
        TableEntry {
            key: b"key_a".to_vec(),
            value: Some(b"val_a_v1".to_vec()),
            sequence_number: 10,
        },
    );
    v1_data.insert(
        b"key_b".to_vec(),
        TableEntry {
            key: b"key_b".to_vec(),
            value: Some(b"val_b_v1".to_vec()),
            sequence_number: 10,
        },
    );
    let v1_id = coordinator.install_version(v1_data);

    // MultiGet pins version 1
    let pinned_h1 = coordinator.pin_version();

    // Concurrent compaction installs version 3 with mutated keys and additions
    let mut v2_data = BTreeMap::new();
    v2_data.insert(
        b"key_a".to_vec(),
        TableEntry {
            key: b"key_a".to_vec(),
            value: Some(b"val_a_v2_compacted".to_vec()),
            sequence_number: 25,
        },
    );
    v2_data.insert(
        b"key_b".to_vec(),
        TableEntry {
            key: b"key_b".to_vec(),
            value: None, // Tombstone
            sequence_number: 26,
        },
    );
    v2_data.insert(
        b"key_c".to_vec(),
        TableEntry {
            key: b"key_c".to_vec(),
            value: Some(b"val_c_new".to_vec()),
            sequence_number: 27,
        },
    );
    let _v2_id = coordinator.install_version(v2_data);

    // MultiGet on pinned_h1 MUST observe v1 across all keys
    let keys: Vec<&[u8]> = vec![b"key_a", b"key_b", b"key_c"];
    let results_v1 = SuperAtomicMultiGetCoordinator::execute_multiget(&pinned_h1, &keys);

    assert_eq!(results_v1[0].value, Some(b"val_a_v1".to_vec()));
    assert_eq!(results_v1[0].version_id, v1_id);
    assert_eq!(results_v1[1].value, Some(b"val_b_v1".to_vec()));
    assert_eq!(results_v1[1].version_id, v1_id);
    assert_eq!(results_v1[2].value, None); // key_c didn't exist in v1
    assert_eq!(results_v1[2].version_id, v1_id);

    // All results have strictly identical version_id
    assert!(results_v1.iter().all(|r| r.version_id == v1_id));

    // Release pinned handle
    drop(pinned_h1);

    // New MultiGet observes new version
    let pinned_h2 = coordinator.pin_version();
    let results_v2 = SuperAtomicMultiGetCoordinator::execute_multiget(&pinned_h2, &keys);
    assert_eq!(results_v2[0].value, Some(b"val_a_v2_compacted".to_vec()));
    assert_eq!(results_v2[1].value, None); // tombstone
    assert_eq!(results_v2[2].value, Some(b"val_c_new".to_vec()));
}

#[test]
fn test_pilar5_ftl_erase_boundary_alignment_and_discard() {
    let planner = FtlEraseBoundaryPlanner::new(FlashGeometry::EraseBlock4MiB);
    let block_size = 4 * 1024 * 1024;

    // 1. Misaligned offset rejected
    assert!(matches!(
        planner.plan_aligned_extent(1024, 1_000_000),
        Err(FtlAlignmentViolation::OffsetMisaligned { .. })
    ));

    // 2. Aligned extent planned: 5 MiB payload requires 8 MiB (2 erase blocks)
    let extent = planner.plan_aligned_extent(0, 5 * 1024 * 1024).expect("Extent plan");
    assert_eq!(extent.start_offset, 0);
    assert_eq!(extent.aligned_size, 2 * block_size);
    assert_eq!(extent.payload_size, 5 * 1024 * 1024);
    assert_eq!(extent.padding_bytes, 3 * 1024 * 1024);

    // 3. Clean discard validation: 2 full blocks reclaimed cleanly
    let reclaimed = planner.verify_clean_discard(extent).expect("Discard verification");
    assert_eq!(reclaimed, 2);
}

#[test]
fn test_pilar6_cross_device_exdev_migration_barrier() {
    let file_id = 999;
    let mut oracle = CrossDeviceTransferOracle::new(file_id);

    // 1. Advance through valid lifecycle
    assert!(oracle.advance_to(CrossDevicePhase::TargetTempSynced).is_ok());
    assert!(oracle.advance_to(CrossDevicePhase::TargetFinalized).is_ok());

    // 2. Pre-manifest crash recovery: rolls back to source
    let (recovery_action, safe) = oracle.recover_from_crash();
    assert!(safe);
    assert!(recovery_action.starts_with("RollbackToSource"));

    // 3. Premature source deletion attempt fails
    assert_eq!(
        oracle.advance_to(CrossDevicePhase::SourceUnlinkedCompleted),
        Err(CrossDeviceViolation::PrematureSourceDeletion)
    );

    // 4. Commit to manifest
    assert!(oracle.advance_to(CrossDevicePhase::ManifestCommitted).is_ok());

    // 5. Post-manifest crash recovery: rolls forward to target
    let (recovery_action_post, safe_post) = oracle.recover_from_crash();
    assert!(safe_post);
    assert!(recovery_action_post.starts_with("RollforwardToTarget"));

    // 6. Complete source unlink
    assert!(oracle.advance_to(CrossDevicePhase::SourceUnlinkedCompleted).is_ok());
}

#[test]
fn test_pilar7_sequence_horizon_monotonicity_and_rollover_prevention() {
    let allocator = SequenceHorizonAllocator::new(100);

    // Monotonic sequential allocations
    let alloc1 = allocator.allocate_batch(5);
    assert_eq!(alloc1, SequenceAllocation::Allocated(100));

    let alloc2 = allocator.allocate_batch(10);
    assert_eq!(alloc2, SequenceAllocation::Allocated(105));

    assert_eq!(allocator.current_seq(), 115);

    // Verify monotonicity helper
    assert!(SequenceHorizonAllocator::verify_monotonicity(&[10, 20, 30, 40]));
    assert!(!SequenceHorizonAllocator::verify_monotonicity(&[10, 20, 15, 40]));

    // Horizon exhaustion protection near 2^64 - 2^32
    let near_ceiling_allocator = SequenceHorizonAllocator::new(SEQUENCE_SAFE_CEILING - 10);
    let ok_batch = near_ceiling_allocator.allocate_batch(5);
    assert_eq!(ok_batch, SequenceAllocation::Allocated(SEQUENCE_SAFE_CEILING - 10));

    // Next batch breaches ceiling: rejected, preventing overflow
    let breach_batch = near_ceiling_allocator.allocate_batch(10);
    assert!(matches!(
        breach_batch,
        SequenceAllocation::HorizonReached { .. }
    ));
}

#[test]
fn test_pilar8_pre_manifest_orphan_sst_cleanup() {
    let mut manifest_active = BTreeSet::new();
    manifest_active.insert(1);
    manifest_active.insert(2);
    manifest_active.insert(3);
    let manifest_next_fn = 10;

    // Normal scenario: disk has active files {1, 2, 3} + orphan {4, 5}
    let mut disk_files = BTreeSet::new();
    disk_files.insert(1);
    disk_files.insert(2);
    disk_files.insert(3);
    disk_files.insert(4);
    disk_files.insert(5);

    let plan = PreManifestOrphanScrubber::reconcile(
        &manifest_active,
        manifest_next_fn,
        &disk_files,
    )
    .expect("Reconciliation succeeds");

    assert_eq!(plan.active_files, manifest_active);
    let expected_orphans: BTreeSet<u64> = [4, 5].into_iter().collect();
    assert_eq!(plan.orphan_files_to_delete, expected_orphans);

    // Error case 1: Active file missing from disk (data loss detected fail-closed)
    let incomplete_disk: BTreeSet<u64> = [1, 2].into_iter().collect();
    assert_eq!(
        PreManifestOrphanScrubber::reconcile(&manifest_active, manifest_next_fn, &incomplete_disk),
        Err(CatalogReconciliationError::MissingActiveFile { file_number: 3 })
    );

    // Error case 2: Orphan file exceeds next_file_number watermark
    let mut corrupted_disk = disk_files.clone();
    corrupted_disk.insert(15); // >= manifest_next_fn (10)
    assert_eq!(
        PreManifestOrphanScrubber::reconcile(&manifest_active, manifest_next_fn, &corrupted_disk),
        Err(CatalogReconciliationError::OrphanViolatesWatermark {
            file_number: 15,
            next_file_number: 10,
        })
    );
}

#[test]
fn test_pilar9_bitemporal_index_mutual_visibility() {
    let batch = AtomicIndexBatch {
        commit_seq: 100,
        primary_mutations: vec![PrimaryMutation {
            pk: b"user:123".to_vec(),
            old_val: None,
            new_val: Some(b"Alice".to_vec()),
        }],
        index_mutations: vec![SecondaryIndexMutation {
            index_key: b"name:Alice:user:123".to_vec(),
            target_pk: b"user:123".to_vec(),
            is_delete: false,
        }],
    };

    let query_snapshots = vec![50, 99, 100, 101, 200];
    assert!(BitemporalIndexOracle::verify_mutual_entailment(&batch, &query_snapshots).is_ok());

    // Before commit (t < 100): neither is visible
    assert!(!BitemporalIndexOracle::is_visible(batch.commit_seq, 99));
    // At commit and after (t >= 100): both are visible
    assert!(BitemporalIndexOracle::is_visible(batch.commit_seq, 100));
    assert!(BitemporalIndexOracle::is_visible(batch.commit_seq, 150));
}

#[test]
fn test_pilar10_async_cancellation_leader_handover() {
    let slot1 = CommitSlot::new(1, b"op1".to_vec());
    let slot2 = CommitSlot::new(2, b"op2".to_vec());
    let slot3 = CommitSlot::new(3, b"op3".to_vec());

    // Task 2 cancels asynchronously prior to commit
    assert!(slot2.try_cancel());
    assert_eq!(slot2.current_state(), CommitSlotState::Cancelled);

    // Group leader commits active batch
    let slots = vec![slot1, slot2, slot3];
    let (active_payloads, committed_ids) = GroupCommitReconciler::assemble_active_batch(&slots);

    // Slot 2 was skipped; Slots 1 and 3 were committed
    assert_eq!(committed_ids, vec![1, 3]);
    assert_eq!(active_payloads, vec![b"op1".as_slice(), b"op3".as_slice()]);
    assert_eq!(slots[0].current_state(), CommitSlotState::Committed);
    assert_eq!(slots[1].current_state(), CommitSlotState::Cancelled);
    assert_eq!(slots[2].current_state(), CommitSlotState::Committed);

    // Leader handover test: when leader aborts, hand over to first waiting follower
    let f1 = CommitSlot::new(10, b"follower1".to_vec());
    let f2 = CommitSlot::new(11, b"follower2".to_vec());
    let followers = vec![f1, f2];

    let new_leader = GroupCommitReconciler::handover_leadership(&followers).expect("Handover succeeds");
    assert_eq!(new_leader.slot_id, 10);
    assert_eq!(new_leader.current_state(), CommitSlotState::ElectedLeader);
}
