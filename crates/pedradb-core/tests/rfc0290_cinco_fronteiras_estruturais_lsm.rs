//! RFC-0290: Testes Unitários das Cinco Fronteiras Matemáticas e Estruturais Finais do LSM.
//!
//! Valida:
//! 1. Dualidade de Filtro de Intervalo e Blindspot de Range Tombstones sob Bloom Filters
//! 2. Álgebra de Confluência de Sub-Compactação Paralela
//! 3. Teto Bounded de Retardo e Escalonamento Fair-Share no Group Commit
//! 4. Oráculo de Feedback de Utilização e Estabilidade de Readahead em Scans
//! 5. Bisimulação de Camada Transacional Local e Teorema de Transparência Causal (RYOW)

use pedradb_core::group_commit_fair_share_kernel::{
    FairSharePolicy, GroupAdmissionPlan, GroupCommitFairShareScheduler, WriteAdmissionRequest,
};
use pedradb_core::parallel_subcompaction_slice_kernel::{
    ParallelSubCompactionVerifier, SubCompactedSstOutput, SubCompactionGeometryError,
    SubCompactionSlice,
};
use pedradb_core::range_tombstone_bloom_dual_kernel::{
    PruneDecision, RangeTombstoneBloomDualFilter, RangeTombstoneSpan,
};
use pedradb_core::readahead_consumption_feedback_kernel::{
    AdaptiveReadaheadController, ReadaheadFeedbackPolicy,
};
use pedradb_core::transactional_overlay_read_view_kernel::TransactionalOverlay;
use std::collections::HashMap;

#[test]
fn test_range_tombstone_bloom_dual_pruning() {
    let tombstones = vec![
        RangeTombstoneSpan::new(b"key_10".to_vec(), b"key_50".to_vec(), 100),
        RangeTombstoneSpan::new(b"key_80".to_vec(), b"key_90".to_vec(), 200),
    ];

    // 1. Point hit in Bloom with no range tombstone
    let dec1 = RangeTombstoneBloomDualFilter::evaluate(true, &tombstones, b"key_05", 150);
    assert_eq!(dec1, PruneDecision::InspectPointHit);

    // 2. Safe to prune: Bloom missed and no range tombstone covers key_05
    let dec2 = RangeTombstoneBloomDualFilter::evaluate(false, &tombstones, b"key_05", 150);
    assert_eq!(dec2, PruneDecision::SafeToPrune);

    // 3. THE CRITICAL BLINDSPOT: Bloom missed key_25, but range tombstone [10, 50) @ seq 100 covers it!
    // Pruning must be FORBIDDEN to prevent resurrecting stale keys from lower levels.
    let dec3 = RangeTombstoneBloomDualFilter::evaluate(false, &tombstones, b"key_25", 150);
    assert_eq!(
        dec3,
        PruneDecision::InspectRangeTombstoneHit { tombstone_seq: 100 }
    );

    // 4. Tombstone exists but is in the future relative to snapshot (seq 200 > snapshot 150)
    // Key key_85 is not covered by any visible tombstone, Bloom missed -> SafeToPrune
    let dec4 = RangeTombstoneBloomDualFilter::evaluate(false, &tombstones, b"key_85", 150);
    assert_eq!(dec4, PruneDecision::SafeToPrune);

    // 5. Same key_85 with snapshot 250 becomes visible -> InspectRangeTombstoneHit
    let dec5 = RangeTombstoneBloomDualFilter::evaluate(false, &tombstones, b"key_85", 250);
    assert_eq!(
        dec5,
        PruneDecision::InspectRangeTombstoneHit { tombstone_seq: 200 }
    );
}

#[test]
fn test_parallel_subcompaction_slice_equivalence() {
    // 1. Valid disjoint contiguous partitions
    let slices = vec![
        SubCompactionSlice {
            partition_id: 0,
            start_bound: None,
            end_bound: Some(b"key_30".to_vec()),
        },
        SubCompactionSlice {
            partition_id: 1,
            start_bound: Some(b"key_30".to_vec()),
            end_bound: Some(b"key_70".to_vec()),
        },
        SubCompactionSlice {
            partition_id: 2,
            start_bound: Some(b"key_70".to_vec()),
            end_bound: None,
        },
    ];
    assert!(ParallelSubCompactionVerifier::verify_slice_geometry(&slices).is_ok());

    // 2. Overlapping slice bounds detection
    let overlapping = vec![
        SubCompactionSlice {
            partition_id: 0,
            start_bound: None,
            end_bound: Some(b"key_40".to_vec()),
        },
        SubCompactionSlice {
            partition_id: 1,
            start_bound: Some(b"key_30".to_vec()), // Collision! 30 < 40
            end_bound: None,
        },
    ];
    let err_overlap = ParallelSubCompactionVerifier::verify_slice_geometry(&overlapping);
    assert!(matches!(
        err_overlap,
        Err(SubCompactionGeometryError::OverlappingSlices { index: 0 })
    ));

    // 3. Valid outputs produced by workers
    let outputs = vec![
        SubCompactedSstOutput {
            partition_id: 0,
            min_key: b"key_05".to_vec(),
            max_key: b"key_25".to_vec(),
            record_count: 100,
        },
        SubCompactedSstOutput {
            partition_id: 1,
            min_key: b"key_35".to_vec(),
            max_key: b"key_65".to_vec(),
            record_count: 150,
        },
        SubCompactedSstOutput {
            partition_id: 2,
            min_key: b"key_75".to_vec(),
            max_key: b"key_95".to_vec(),
            record_count: 200,
        },
    ];
    assert!(ParallelSubCompactionVerifier::verify_outputs(&slices, &outputs).is_ok());

    // 4. Output violating its partition boundary
    let out_bounds_violation = vec![SubCompactedSstOutput {
        partition_id: 0,
        min_key: b"key_05".to_vec(),
        max_key: b"key_35".to_vec(), // >= slice 0 end_bound (key_30)!
        record_count: 50,
    }];
    assert!(matches!(
        ParallelSubCompactionVerifier::verify_outputs(&slices, &out_bounds_violation),
        Err(SubCompactionGeometryError::KeyOutsideSliceBounds { partition_id: 0 })
    ));

    // 5. Inter-file key order inversion
    let inverted_outputs = vec![
        SubCompactedSstOutput {
            partition_id: 0,
            min_key: b"key_15".to_vec(),
            max_key: b"key_25".to_vec(),
            record_count: 100,
        },
        SubCompactedSstOutput {
            partition_id: 0,
            min_key: b"key_10".to_vec(), // Inversion! 25 >= 10
            max_key: b"key_20".to_vec(),
            record_count: 100,
        },
    ];
    assert!(matches!(
        ParallelSubCompactionVerifier::verify_outputs(&slices, &inverted_outputs),
        Err(SubCompactionGeometryError::InterPartitionKeyInversion { .. })
    ));
}

#[test]
fn test_group_commit_fair_share_scheduler() {
    let policy = FairSharePolicy {
        max_group_bytes: 1000,
        huge_writer_threshold: 400,
    };
    let scheduler = GroupCommitFairShareScheduler::new(policy);

    let requests = vec![
        WriteAdmissionRequest {
            writer_id: 1,
            payload_bytes: 100,
            is_sync: true,
        },
        WriteAdmissionRequest {
            writer_id: 2,
            payload_bytes: 200,
            is_sync: true,
        },
        // Writer 3 is huge (500 >= 400 threshold)
        WriteAdmissionRequest {
            writer_id: 3,
            payload_bytes: 500,
            is_sync: true,
        },
        WriteAdmissionRequest {
            writer_id: 4,
            payload_bytes: 50,
            is_sync: true,
        },
        WriteAdmissionRequest {
            writer_id: 5,
            payload_bytes: 350,
            is_sync: true,
        },
    ];

    let plans = scheduler.schedule_admissions(&requests);
    assert_eq!(plans.len(), 3);

    // Plan 1: Small writers 1 and 2 coalesced (100 + 200 = 300 bytes)
    assert_eq!(
        plans[0],
        GroupAdmissionPlan::CoalescedGroup {
            writers: vec![1, 2],
            total_bytes: 300,
        }
    );

    // Plan 2: Writer 3 isolated to protect p99 latency
    assert_eq!(
        plans[1],
        GroupAdmissionPlan::IsolatedHugeWriter {
            writer_id: 3,
            payload_bytes: 500,
        }
    );

    // Plan 3: Small writers 4 and 5 coalesced (50 + 350 = 400 bytes)
    assert_eq!(
        plans[2],
        GroupAdmissionPlan::CoalescedGroup {
            writers: vec![4, 5],
            total_bytes: 400,
        }
    );
}

#[test]
fn test_readahead_consumption_feedback_controller() {
    let policy = ReadaheadFeedbackPolicy {
        min_window_bytes: 4096,
        max_window_bytes: 65536,
        expand_threshold: 0.80,
        contract_threshold: 0.30,
    };
    let mut controller = AdaptiveReadaheadController::new(policy);
    assert_eq!(controller.current_window(), 4096);

    // 1. High consumption epoch (100% consumed): expands window 4096 -> 8192
    controller.record_prefetch(4096);
    controller.record_consumption(4096);
    assert_eq!(controller.evaluate_and_adapt(), 8192);

    // 2. Another high consumption epoch (90% consumed): expands window 8192 -> 16384
    controller.record_prefetch(8192);
    controller.record_consumption(7500); // 7500 / 8192 = 91.5% >= 80%
    assert_eq!(controller.evaluate_and_adapt(), 16384);

    // 3. Low consumption epoch (client aborted scan early: 10% consumed): contracts window 16384 -> 8192
    controller.record_prefetch(16384);
    controller.record_consumption(1600); // 1600 / 16384 = 9.7% < 30%
    assert_eq!(controller.evaluate_and_adapt(), 8192);

    // 4. Repeated low consumption: contracts window 8192 -> 4096
    controller.record_prefetch(8192);
    controller.record_consumption(500);
    assert_eq!(controller.evaluate_and_adapt(), 4096);

    // 5. Floor ceiling: does not contract below min_window_bytes (4096)
    controller.record_prefetch(4096);
    controller.record_consumption(0);
    assert_eq!(controller.evaluate_and_adapt(), 4096);
}

#[test]
fn test_transactional_overlay_read_your_own_writes() {
    // Immutable snapshot state
    let mut snapshot = HashMap::new();
    snapshot.insert(b"k1".to_vec(), b"v1_snap".to_vec());
    snapshot.insert(b"k2".to_vec(), b"v2_snap".to_vec());
    snapshot.insert(b"k3".to_vec(), b"v3_snap".to_vec());

    let snapshot_lookup = |k: &[u8]| snapshot.get(k).cloned();

    let mut overlay = TransactionalOverlay::new();

    // 1. Initial state reads identical to snapshot
    assert_eq!(overlay.get(b"k1", snapshot_lookup), Some(b"v1_snap".to_vec()));
    assert_eq!(overlay.get(b"k2", snapshot_lookup), Some(b"v2_snap".to_vec()));
    assert_eq!(overlay.get(b"k3", snapshot_lookup), Some(b"v3_snap".to_vec()));
    assert_eq!(overlay.get(b"k4", snapshot_lookup), None);

    // 2. Perform local transactional mutations
    overlay.put(b"k1".to_vec(), b"v1_local_update".to_vec());
    overlay.delete(b"k2".to_vec());
    overlay.put(b"k4".to_vec(), b"v4_local_new".to_vec());

    assert_eq!(overlay.local_mutation_count(), 3);
    assert!(overlay.is_locally_dirty(b"k1"));
    assert!(overlay.is_locally_deleted(b"k2"));
    assert!(!overlay.is_locally_dirty(b"k3"));

    // 3. Read-Your-Own-Writes Verification:
    // k1 returns local update
    assert_eq!(overlay.get(b"k1", snapshot_lookup), Some(b"v1_local_update".to_vec()));
    // k2 local delete masks snapshot value
    assert_eq!(overlay.get(b"k2", snapshot_lookup), None);
    // k3 untouched delegates to snapshot
    assert_eq!(overlay.get(b"k3", snapshot_lookup), Some(b"v3_snap".to_vec()));
    // k4 local new insert returned
    assert_eq!(overlay.get(b"k4", snapshot_lookup), Some(b"v4_local_new".to_vec()));
    // k5 absent everywhere
    assert_eq!(overlay.get(b"k5", snapshot_lookup), None);

    // 4. Rollback clears local mutations and restores pure snapshot view
    overlay.rollback();
    assert_eq!(overlay.local_mutation_count(), 0);
    assert_eq!(overlay.get(b"k1", snapshot_lookup), Some(b"v1_snap".to_vec()));
    assert_eq!(overlay.get(b"k2", snapshot_lookup), Some(b"v2_snap".to_vec()));
    assert_eq!(overlay.get(b"k4", snapshot_lookup), None);
}
