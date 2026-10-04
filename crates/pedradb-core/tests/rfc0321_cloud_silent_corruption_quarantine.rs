//! RFC-0321: Silent Data Corruption Quarantine Circuit Breaker test suite.
//!
//! Verifies:
//! - Idempotent quarantine registration.
//! - Monotonic sorting and zero-allocation binary search lookup.
//! - Compaction candidate filtering excluding corrupted files.
//! - Internal invariant preservation across sequential corruptions.

use pedradb_core::cloud_silent_corruption_quarantine_kernel::SilentCorruptionQuarantineRegister;

#[test]
fn test_quarantine_lifecycle_and_candidate_filtering() {
    let mut reg = SilentCorruptionQuarantineRegister::new();
    assert_eq!(reg.quarantined_count(), 0);
    assert_eq!(reg.total_corruption_events(), 0);

    // Mark SST 42 and SST 15 as quarantined (out-of-order insertion)
    reg.mark_quarantined(42);
    reg.mark_quarantined(15);
    reg.mark_quarantined(100);

    assert_eq!(reg.quarantined_count(), 3);
    assert_eq!(reg.total_corruption_events(), 3);
    assert!(reg.is_quarantined(15));
    assert!(reg.is_quarantined(42));
    assert!(reg.is_quarantined(100));
    assert!(!reg.is_quarantined(16));
    assert!(reg.verify_internal_invariants());

    // Duplicate marking increments event counter but preserves unique count and sort
    reg.mark_quarantined(42);
    assert_eq!(reg.quarantined_count(), 3);
    assert_eq!(reg.total_corruption_events(), 4);
    assert!(reg.verify_internal_invariants());

    // Filter compaction candidates
    let candidates = vec![10, 15, 20, 42, 50, 100, 150];
    let filtered = reg.filter_compaction_candidates(&candidates);
    assert_eq!(filtered, vec![10, 20, 50, 150]);
}

#[test]
fn test_empty_quarantine() {
    let reg = SilentCorruptionQuarantineRegister::new();
    assert!(!reg.is_quarantined(1));
    let candidates = vec![1, 2, 3];
    assert_eq!(reg.filter_compaction_candidates(&candidates), vec![1, 2, 3]);
    assert!(reg.verify_internal_invariants());
}
