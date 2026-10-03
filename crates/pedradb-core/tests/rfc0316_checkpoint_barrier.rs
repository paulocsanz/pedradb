//! RFC-0316: Point-In-Time Continuous Checkpoint Barrier Test Suite
//!
//! Verifies:
//! - Reference count tracking preventing obsolete deletion of active snapshot SSTs.
//! - Clean unpin lifecycle releasing files for compaction GC.
//! - Closure verification over complete physical checkpoint manifests.

use pedradb_core::checkpoint_barrier_kernel::{
    CheckpointManifest, CheckpointPinTracker,
};

#[test]
fn test_checkpoint_pin_and_safe_obsolete_filtering() {
    let mut tracker = CheckpointPinTracker::new();

    let manifest = CheckpointManifest::new(
        1000,
        vec![10, 20, 30],
        vec![1, 2],
        1700000000,
    );

    assert_eq!(manifest.total_files(), 5);
    assert!(manifest.contains_sst(20));
    assert!(!manifest.contains_sst(99));

    // Pin the checkpoint
    tracker.pin_checkpoint(&manifest);
    assert_eq!(tracker.total_pinned_files(), 5);
    assert!(tracker.is_sst_pinned(10));
    assert!(tracker.is_sst_pinned(20));
    assert!(tracker.is_sst_pinned(30));
    assert!(tracker.is_wal_pinned(1));
    assert!(!tracker.is_sst_pinned(40));

    // Compactor wants to delete [10, 20, 40, 50]
    let candidate_obsolete = vec![10, 20, 40, 50];
    let safe_to_delete = tracker.filter_safe_obsolete_ssts(&candidate_obsolete);

    // Only 40 and 50 are allowed to be deleted
    assert_eq!(safe_to_delete, vec![40, 50]);
}

#[test]
fn test_checkpoint_unpin_lifecycle() {
    let mut tracker = CheckpointPinTracker::new();

    let cp1 = CheckpointManifest::new(100, vec![5, 6], vec![1], 1000);
    let cp2 = CheckpointManifest::new(200, vec![6, 7], vec![2], 2000);

    // Both share SST 6
    tracker.pin_checkpoint(&cp1);
    tracker.pin_checkpoint(&cp2);

    assert!(tracker.is_sst_pinned(5));
    assert!(tracker.is_sst_pinned(6));
    assert!(tracker.is_sst_pinned(7));

    // Unpin cp1: SST 6 must STILL be pinned by cp2!
    tracker.unpin_checkpoint(&cp1);
    assert!(!tracker.is_sst_pinned(5));
    assert!(tracker.is_sst_pinned(6));
    assert!(tracker.is_sst_pinned(7));

    // Unpin cp2: SST 6 and 7 are now released
    tracker.unpin_checkpoint(&cp2);
    assert!(!tracker.is_sst_pinned(6));
    assert!(!tracker.is_sst_pinned(7));
    assert_eq!(tracker.total_pinned_files(), 0);
}

#[test]
fn test_checkpoint_manifest_closure_validation() {
    let manifest = CheckpointManifest::new(500, vec![1, 2, 3], vec![10], 5000);

    // 1. All files present
    let available_ssts = vec![1, 2, 3, 4, 5];
    let available_wals = vec![10, 11];
    assert!(CheckpointPinTracker::verify_manifest_closure(
        &manifest,
        &available_ssts,
        &available_wals,
    ));

    // 2. Missing an SST (file 2 deleted or not yet synced)
    let incomplete_ssts = vec![1, 3, 4, 5];
    assert!(!CheckpointPinTracker::verify_manifest_closure(
        &manifest,
        &incomplete_ssts,
        &available_wals,
    ));

    // 3. Missing a WAL segment
    let incomplete_wals = vec![11];
    assert!(!CheckpointPinTracker::verify_manifest_closure(
        &manifest,
        &available_ssts,
        &incomplete_wals,
    ));
}

#[test]
fn test_checkpoint_barrier_red_safety() {
    use pedradb_core::checkpoint_barrier_kernel::CheckpointError;

    // 1. Validation: snapshot_seq == 0 rejected
    let inv1 = CheckpointManifest::try_new(0, vec![1], vec![], 100);
    assert_eq!(inv1.err(), Some(CheckpointError::ZeroSnapshotSeq));

    // 2. Validation: empty manifest rejected
    let inv2 = CheckpointManifest::try_new(10, vec![], vec![], 100);
    assert_eq!(inv2.err(), Some(CheckpointError::EmptyManifest));

    // 3. Validation: file number 0 rejected
    let inv3 = CheckpointManifest::try_new(10, vec![0], vec![], 100);
    assert_eq!(inv3.err(), Some(CheckpointError::ZeroFileNumber));

    // 4. Critical data-loss prevention: unpinning an unpinned checkpoint must NOT decrement other checkpoints' pins
    let mut tracker = CheckpointPinTracker::new();
    let cp_active = CheckpointManifest::try_new(100, vec![42], vec![], 1000).unwrap();
    let cp_phantom = CheckpointManifest::try_new(200, vec![42], vec![], 2000).unwrap();

    tracker.try_pin_checkpoint(&cp_active).expect("pinned");
    assert!(tracker.is_sst_pinned(42));

    // Attempting to unpin cp_phantom (which was never pinned) must error out!
    let res = tracker.try_unpin_checkpoint(&cp_phantom);
    assert_eq!(res.err(), Some(CheckpointError::CheckpointNotPinned(200)));

    // Crucial: SST 42 MUST STILL BE PINNED by cp_active!
    assert!(
        tracker.is_sst_pinned(42),
        "SST 42 was corrupted/unpinned by phantom unpin! Data loss vulnerability!"
    );
}

