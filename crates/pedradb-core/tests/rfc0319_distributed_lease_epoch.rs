//! RFC-0319: Distributed Leader Lease Epoch test suite.
//!
//! Verifies:
//! - Drift-bounded lease expiration logic.
//! - Term exclusivity and local read admission.
//! - Parameter bounds validation (zero duration, excessive drift).
//! - Monotonic term progression upon renewal.

use pedradb_core::distributed_lease_epoch_kernel::{LeaderLeaseGuard, LeaseError};

#[test]
fn test_drift_bounded_lease_lifecycle() {
    // Leader 1, Term 5, granted at HLC 1000, duration 200, max drift 50
    // Nominal expiry = 1200, effective expiry = 1150
    let lease = LeaderLeaseGuard::try_grant(1, 5, 1000, 200, 50).expect("valid grant");
    assert_eq!(lease.effective_expiry_hlc(), 1150);
    assert!(lease.verify_internal_invariants());

    // Before effective expiry: valid and admits read
    assert!(lease.is_valid_at(1000));
    assert!(lease.is_valid_at(1149));
    assert!(lease.can_admit_local_read(5, 1100));

    // At or after effective expiry: rejected
    assert!(!lease.is_valid_at(1150));
    assert!(!lease.is_valid_at(1200));
    assert!(!lease.can_admit_local_read(5, 1150));

    // Mismatched term is rejected even if time is valid
    assert!(!lease.can_admit_local_read(4, 1100));
    assert!(!lease.can_admit_local_read(6, 1100));
}

#[test]
fn test_invalid_parameters_rejection() {
    // Zero duration
    assert_eq!(
        LeaderLeaseGuard::try_grant(1, 1, 100, 0, 0).err(),
        Some(LeaseError::ZeroDuration)
    );

    // Drift >= duration
    assert_eq!(
        LeaderLeaseGuard::try_grant(1, 1, 100, 50, 50).err(),
        Some(LeaseError::DriftExceedsDuration { drift: 50, duration: 50 })
    );
    assert_eq!(
        LeaderLeaseGuard::try_grant(1, 1, 100, 50, 60).err(),
        Some(LeaseError::DriftExceedsDuration { drift: 60, duration: 50 })
    );
}

#[test]
fn test_lease_renewal_and_term_regression() {
    let mut lease = LeaderLeaseGuard::try_grant(1, 2, 500, 100, 10).expect("grant");
    assert_eq!(lease.effective_expiry_hlc(), 590);

    // Renew with higher term
    assert!(lease.renew(3, 600).is_ok());
    assert_eq!(lease.term, 3);
    assert_eq!(lease.grant_hlc, 600);
    assert_eq!(lease.effective_expiry_hlc(), 690);

    // Renew with lower term fails
    assert_eq!(
        lease.renew(2, 700).err(),
        Some(LeaseError::TermRegressed { current: 3, attempted: 2 })
    );
}

#[test]
fn test_distributed_lease_red_invariants() {
    // 1. Error implements std::error::Error
    let err: Box<dyn std::error::Error> = Box::new(LeaseError::ZeroDuration);
    assert!(!err.to_string().is_empty());

    // 2. Reject zero leader ID and zero term
    assert_eq!(
        LeaderLeaseGuard::try_grant(0, 1, 100, 50, 10).err(),
        Some(LeaseError::ZeroLeaderId)
    );
    assert_eq!(
        LeaderLeaseGuard::try_grant(1, 0, 100, 50, 10).err(),
        Some(LeaseError::ZeroTerm)
    );

    // 3. Reject HLC overflow
    assert_eq!(
        LeaderLeaseGuard::try_grant(1, 1, u64::MAX - 10, 100, 10).err(),
        Some(LeaseError::HlcOverflow)
    );

    // 4. Critical: Renewal must reject HLC timestamp regression
    let mut lease = LeaderLeaseGuard::try_grant(1, 5, 1000, 200, 20).expect("grant");
    assert_eq!(
        lease.renew(5, 900).err(),
        Some(LeaseError::HlcRegressed { current: 1000, attempted: 900 })
    );

    // 5. remaining_ticks telemetry
    assert_eq!(lease.remaining_ticks(1000), 180);
    assert_eq!(lease.remaining_ticks(1100), 80);
    assert_eq!(lease.remaining_ticks(1180), 0);
    assert_eq!(lease.remaining_ticks(1200), 0);
}

