//! RFC-0312: Deterministic Verification Oracle for Multi-CF Safe WAL Pruning Watermarks.
//!
//! Enforces zero-twin production verification of:
//! 1. Prevention of premature WAL segment deletion under asymmetric multi-CF flush rates.
//! 2. Preservation of un-flushed memtable records across all active Column Families.
//! 3. Long-lived snapshot watermark pinning (preventing deletion of history required by readers).
//! 4. Full crash recovery coverage invariant (zero data loss across power cuts).

use pedradb_core::wal_prune_watermark_kernel::{
    WalPruneWatermarkOracle, WalSegmentDescriptor,
};

#[test]
fn test_asymmetric_multi_cf_pruning_safety() {
    let mut oracle = WalPruneWatermarkOracle::new();

    // CF 1: default, fast writer, already flushed to seq 1000
    oracle.register_cf(0, "default", 0);
    oracle.record_flush_complete(0, 1000);

    // CF 2: audit_log, slow writer, only flushed to seq 200, active memtable at 201
    oracle.register_cf(1, "audit_log", 0);
    oracle.record_flush_complete(1, 200);
    oracle.update_memtable_min_seq(1, 201);

    // Physical WAL log segments on disk
    let seg1 = WalSegmentDescriptor { segment_id: 1, min_seq: 1, max_seq: 150, file_size_bytes: 64 * 1024 };
    let seg2 = WalSegmentDescriptor { segment_id: 2, min_seq: 151, max_seq: 200, file_size_bytes: 64 * 1024 };
    let seg3 = WalSegmentDescriptor { segment_id: 3, min_seq: 201, max_seq: 600, file_size_bytes: 64 * 1024 };
    let seg4 = WalSegmentDescriptor { segment_id: 4, min_seq: 601, max_seq: 1000, file_size_bytes: 64 * 1024 };

    let segments = [seg1, seg2, seg3, seg4];

    // Safe prune watermark must be throttled by the slowest CF (CF 2: 201)
    let watermark = oracle.compute_prune_watermark().expect("Watermark must exist");
    assert_eq!(watermark, 201, "Watermark must be held by CF 2 at 201 despite CF 1 at 1000");

    // Segments 1 and 2 are fully flushed by all CFs -> Safe to prune
    assert!(oracle.is_segment_prunable(&seg1), "Seg 1 (max 150 < 201) must be prunable");
    assert!(oracle.is_segment_prunable(&seg2), "Seg 2 (max 200 < 201) must be prunable");

    // Segments 3 and 4 contain un-flushed data for CF 2 -> CANNOT BE PRUNED
    assert!(!oracle.is_segment_prunable(&seg3), "Seg 3 contains unflushed CF2 data: must NOT be pruned");
    assert!(!oracle.is_segment_prunable(&seg4), "Seg 4 must NOT be pruned");

    let prunable = oracle.select_prunable_segments(&segments);
    assert_eq!(prunable.len(), 2);
    assert_eq!(prunable[0].segment_id, 1);
    assert_eq!(prunable[1].segment_id, 2);

    // Verify recovery coverage of surviving segments (seg3, seg4)
    let surviving = [seg3, seg4];
    assert!(
        oracle.verify_crash_recovery_coverage(&surviving).is_ok(),
        "Surviving segments must cover all un-flushed sequences"
    );
}

#[test]
fn test_active_snapshot_watermark_retention() {
    let mut oracle = WalPruneWatermarkOracle::new();
    oracle.register_cf(0, "default", 0);

    // CF flushes up to seq 1000
    oracle.record_flush_complete(0, 1000);

    let seg1 = WalSegmentDescriptor { segment_id: 1, min_seq: 1, max_seq: 200, file_size_bytes: 64 * 1024 };
    let seg2 = WalSegmentDescriptor { segment_id: 2, min_seq: 201, max_seq: 1000, file_size_bytes: 64 * 1024 };

    // Before snapshot: watermark is 1001 -> Segments 1 and 2 are prunable
    assert_eq!(oracle.compute_prune_watermark(), Some(1001));
    assert!(oracle.is_segment_prunable(&seg1));
    assert!(oracle.is_segment_prunable(&seg2));

    // A long-lived read transaction acquires a snapshot at seq 150
    oracle.retain_snapshot(150);

    // Watermark must immediately clamp down to snapshot sequence 150
    assert_eq!(oracle.compute_prune_watermark(), Some(150));
    assert!(!oracle.is_segment_prunable(&seg1), "Seg 1 (max 200 > 150) must be pinned by active snapshot");
    assert!(!oracle.is_segment_prunable(&seg2));

    // Transaction commits/aborts and snapshot is released
    oracle.release_snapshot(150);

    // Watermark rebounds to 1001
    assert_eq!(oracle.compute_prune_watermark(), Some(1001));
    assert!(oracle.is_segment_prunable(&seg1));
    assert!(oracle.is_segment_prunable(&seg2));
}

#[test]
fn test_uninitialized_oracle_fails_closed() {
    let empty_oracle = WalPruneWatermarkOracle::new();
    let seg = WalSegmentDescriptor { segment_id: 1, min_seq: 1, max_seq: 10, file_size_bytes: 1024 };

    // Invariant: Uninitialized oracle never prunes (fail-closed)
    assert_eq!(empty_oracle.compute_prune_watermark(), None);
    assert!(!empty_oracle.is_segment_prunable(&seg));
    assert!(empty_oracle.select_prunable_segments(&[seg]).is_empty());
}
