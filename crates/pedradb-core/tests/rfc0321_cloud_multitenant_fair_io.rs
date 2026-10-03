//! RFC-0321: Multi-Tenant Deficit Round-Robin I/O Scheduler test suite.
//!
//! Verifies:
//! - Class-prioritized bandwidth deficit tracking.
//! - Deficit exhaustion and admission rejection.
//! - Round replenishment with 4x quantum burst cap.
//! - Dynamic compaction throttling under cloud noisy neighbor load.

use pedradb_core::cloud_multitenant_fair_io_kernel::{
    CloudIoScheduler, IoScheduleError, IoTrafficClass,
};

#[test]
fn test_drr_admission_and_replenishment() {
    // Interactive: 64 KiB, WAL: 32 KiB, Compaction: 16 KiB
    let mut sched = CloudIoScheduler::new(65536, 32768, 16384).expect("valid scheduler");
    assert!(sched.verify_internal_invariants());

    // Interactive read can admit 40 KiB
    assert!(sched.can_admit(IoTrafficClass::InteractiveRead, 40960));
    assert!(sched.admit(IoTrafficClass::InteractiveRead, 40960).is_ok());
    assert_eq!(sched.current_deficit(IoTrafficClass::InteractiveRead), 65536 - 40960);
    assert_eq!(sched.total_bytes_admitted(IoTrafficClass::InteractiveRead), 40960);

    // Admitting another 30 KiB exceeds remaining deficit (24576)
    assert!(!sched.can_admit(IoTrafficClass::InteractiveRead, 30720));
    assert_eq!(
        sched.admit(IoTrafficClass::InteractiveRead, 30720).err(),
        Some(IoScheduleError::DeficitExceeded {
            requested: 30720,
            available: 24576,
        })
    );

    // Replenish round restores 64 KiB
    sched.replenish_round();
    assert_eq!(sched.current_deficit(IoTrafficClass::InteractiveRead), 24576 + 65536);
    assert!(sched.can_admit(IoTrafficClass::InteractiveRead, 30720));
    assert!(sched.admit(IoTrafficClass::InteractiveRead, 30720).is_ok());
}

#[test]
fn test_compaction_throttling_on_cloud_burst_exhaustion() {
    let mut sched = CloudIoScheduler::new(64000, 32000, 16000).expect("scheduler");

    // Throttle compaction by 50%
    assert!(sched.throttle_compaction(50).is_ok());

    // Invalid throttle > 100%
    assert_eq!(
        sched.throttle_compaction(101).err(),
        Some(IoScheduleError::InvalidThrottlePercentage(101))
    );
}

#[test]
fn test_zero_quantum_rejection() {
    assert_eq!(
        CloudIoScheduler::new(0, 1000, 1000).err(),
        Some(IoScheduleError::ZeroQuantum)
    );
    assert_eq!(
        CloudIoScheduler::new(1000, 0, 1000).err(),
        Some(IoScheduleError::ZeroQuantum)
    );
    assert_eq!(
        CloudIoScheduler::new(1000, 1000, 0).err(),
        Some(IoScheduleError::ZeroQuantum)
    );
}
