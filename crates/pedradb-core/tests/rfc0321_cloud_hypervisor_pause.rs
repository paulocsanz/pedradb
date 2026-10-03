//! RFC-0321: Cloud VM Hypervisor Pause and Preemption Detector test suite.
//!
//! Verifies:
//! - Normal scheduler loop progression.
//! - Autonomic lease quarantine on wall-clock jumps (vCPU steal / live migration).
//! - Consensus re-validation clearing quarantine.
//! - Physical monotonic regression rejection.

use pedradb_core::cloud_hypervisor_pause_kernel::{
    HypervisorPauseDetector, HypervisorPauseError, PauseObservation,
};

#[test]
fn test_normal_scheduler_progress() {
    let mut detector = HypervisorPauseDetector::new(1_000_000, 50_000_000).expect("valid detector");
    assert!(!detector.is_quarantined());
    assert_eq!(detector.total_pauses_detected(), 0);

    // 10ms passed with 10 cooperative ticks: normal
    let res = detector.observe_step(11_000_000, 10).unwrap();
    assert!(matches!(res, PauseObservation::NormalProgress { elapsed_ns: 10_000_000 }));
    assert!(!detector.is_quarantined());
    assert!(detector.verify_internal_invariants());
}

#[test]
fn test_hypervisor_pause_and_autonomic_quarantine() {
    // 50ms tolerance threshold
    let mut detector = HypervisorPauseDetector::new(100_000_000, 50_000_000).expect("valid detector");

    // Hypervisor pause of 200ms occurs, scheduler starved (0 ticks)
    let res = detector.observe_step(300_000_000, 0).unwrap();
    assert!(matches!(
        res,
        PauseObservation::HypervisorPauseDetected {
            pause_ns: 200_000_000,
            tolerated_threshold_ns: 50_000_000
        }
    ));
    assert!(detector.is_quarantined());
    assert_eq!(detector.total_pauses_detected(), 1);

    // Node re-synchronizes with Raft heartbeat and clears quarantine
    detector.clear_quarantine_after_sync(310_000_000);
    assert!(!detector.is_quarantined());
    assert_eq!(detector.total_pauses_detected(), 1);

    // Subsequent normal step proceeds cleanly
    let res2 = detector.observe_step(315_000_000, 5).unwrap();
    assert!(matches!(res2, PauseObservation::NormalProgress { elapsed_ns: 5_000_000 }));
    assert!(!detector.is_quarantined());
}

#[test]
fn test_monotonic_regression_rejection() {
    let mut detector = HypervisorPauseDetector::new(500_000_000, 100_000_000).expect("detector");

    // Clock steps backwards
    assert_eq!(
        detector.observe_step(400_000_000, 1).err(),
        Some(HypervisorPauseError::MonotonicClockRegression {
            last_ts_ns: 500_000_000,
            attempted_ts_ns: 400_000_000,
        })
    );

    // Zero tolerance threshold is rejected
    assert_eq!(
        HypervisorPauseDetector::new(0, 0).err(),
        Some(HypervisorPauseError::ZeroTolerance)
    );
}
