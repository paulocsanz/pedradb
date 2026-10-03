//! RFC-0319: Distributed Snapshot Read Linearizability test suite.
//!
//! Verifies:
//! - Monotonic client watermark tracking.
//! - Future snapshot rejection exceeding committed high water mark.
//! - Causal stale read rejection below established client watermark.
//! - High water mark progression.

use pedradb_core::snapshot_read_linearizability_kernel::{
    LinearizabilityError, SnapshotWatermarkTracker,
};

#[test]
fn test_linearizable_snapshot_acquisition() {
    let mut tracker = SnapshotWatermarkTracker::new(100);
    assert_eq!(tracker.committed_high_water, 100);
    assert_eq!(tracker.client_read_watermark, 0);

    // Read at seq 50 (valid historical snapshot)
    let s1 = tracker.acquire_snapshot_for_read(Some(50)).expect("read at 50");
    assert_eq!(s1, 50);
    assert_eq!(tracker.client_read_watermark, 50);
    assert!(tracker.verify_internal_invariants());

    // Advance to latest snapshot (None picks 100)
    let s2 = tracker.acquire_snapshot_for_read(None).expect("read latest");
    assert_eq!(s2, 100);
    assert_eq!(tracker.client_read_watermark, 100);
    assert!(tracker.verify_internal_invariants());
}

#[test]
fn test_future_snapshot_rejection() {
    let mut tracker = SnapshotWatermarkTracker::new(200);

    // Requesting seq 201 exceeds committed high water mark (200)
    assert_eq!(
        tracker.acquire_snapshot_for_read(Some(201)).err(),
        Some(LinearizabilityError::FutureSnapshot {
            snapshot_seq: 201,
            committed_high_water: 200,
        })
    );

    // Advance high water mark to 250, then 201 is accepted
    tracker.advance_committed_high_water(250);
    assert_eq!(tracker.acquire_snapshot_for_read(Some(201)).unwrap(), 201);
}

#[test]
fn test_stale_read_rejection() {
    let mut tracker = SnapshotWatermarkTracker::new(300);

    // Client establishes read watermark at 150
    tracker.acquire_snapshot_for_read(Some(150)).unwrap();

    // Client attempts to read at seq 100 (< 150), which violates causality
    assert_eq!(
        tracker.acquire_snapshot_for_read(Some(100)).err(),
        Some(LinearizabilityError::StaleRead {
            snapshot_seq: 100,
            required_watermark: 150,
        })
    );
}

#[test]
fn test_snapshot_linearizability_red_invariants() {
    // 1. Error implements std::error::Error
    let err: Box<dyn std::error::Error> = Box::new(LinearizabilityError::ZeroSnapshotSeq);
    assert!(!err.to_string().is_empty());

    let mut tracker = SnapshotWatermarkTracker::new(200);

    // 2. Reject snapshot 0
    assert_eq!(
        tracker.acquire_snapshot_for_read(Some(0)).err(),
        Some(LinearizabilityError::ZeroSnapshotSeq)
    );

    // 3. Reject high water mark regression
    assert_eq!(
        tracker.try_advance_committed_high_water(150).err(),
        Some(LinearizabilityError::HighWaterRegressed { current: 200, attempted: 150 })
    );

    // 4. Client watermark update: causal token from peer
    tracker.update_client_watermark(120).expect("token update");
    assert_eq!(tracker.client_read_watermark, 120);

    // Client watermark cannot regress
    assert_eq!(
        tracker.update_client_watermark(110).err(),
        Some(LinearizabilityError::WatermarkRegressed { previous: 120, attempted: 110 })
    );

    // Client watermark cannot jump to future uncommitted sequence
    assert_eq!(
        tracker.update_client_watermark(300).err(),
        Some(LinearizabilityError::FutureSnapshot { snapshot_seq: 300, committed_high_water: 200 })
    );
}

