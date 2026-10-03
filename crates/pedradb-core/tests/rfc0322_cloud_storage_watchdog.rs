//! RFC-0322: Cloud Storage Watchdog Verification Suite.

use pedradb_core::cloud_storage_watchdog_kernel::{
    CloudStorageWatchdogKernel, StorageOpClass, StorageWatchdogState,
};

#[test]
fn test_storage_watchdog_normal_lifecycle() {
    // Default deadline = 100 ticks, max tolerable hung ops = 2.
    let mut watchdog = CloudStorageWatchdogKernel::new(100, 2).unwrap();
    assert!(watchdog.is_write_admitted());
    assert_eq!(watchdog.evaluate_health(), StorageWatchdogState::StorageHealthy);

    // Register WAL sync at tick 10.
    watchdog.register_io(1, StorageOpClass::WalSync, 10).unwrap();
    assert_eq!(watchdog.inflight_count(), 1);

    // Advance to tick 50 and complete.
    watchdog.advance_tick(50);
    let latency = watchdog.complete_io(1).unwrap();
    assert_eq!(latency, 40);
    assert_eq!(watchdog.inflight_count(), 0);
    assert_eq!(watchdog.total_completed(), 1);
    assert_eq!(watchdog.evaluate_health(), StorageWatchdogState::StorageHealthy);
}

#[test]
fn test_storage_watchdog_degradation_and_fencing() {
    let mut watchdog = CloudStorageWatchdogKernel::new(100, 2).unwrap();

    // Register 3 I/O operations at tick 0.
    watchdog.register_io(101, StorageOpClass::WalSync, 0).unwrap();
    watchdog.register_io(102, StorageOpClass::SstWrite, 0).unwrap();
    watchdog.register_io(103, StorageOpClass::ManifestSync, 0).unwrap();

    // Advance tick to 150 (elapsed = 150 > deadline 100).
    // All 3 operations are hung. Since 3 > max_tolerable (2), watchdog must fence!
    watchdog.advance_tick(150);
    assert_eq!(watchdog.evaluate_health(), StorageWatchdogState::StorageHungFenced);
    assert!(!watchdog.is_write_admitted());

    // Complete 1 operation (leaves 2 hung <= max_tolerable 2) -> Degraded.
    watchdog.complete_io(101).unwrap();
    match watchdog.evaluate_health() {
        StorageWatchdogState::StorageDegraded { hung_ops, max_stall_ticks } => {
            assert_eq!(hung_ops, 2);
            assert_eq!(max_stall_ticks, 50);
        }
        other => panic!("expected degraded, got {:?}", other),
    }
    assert!(watchdog.is_write_admitted());
}
