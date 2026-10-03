//! RFC-0326: Hardware Liar Resilience Verification Suite.
//!
//! Mechanically verifies detection of storage controllers and hypervisors
//! that report false `fdatasync` completions, using persistent barrier canaries.

#![forbid(unsafe_code)]

use pedradb_core::hardware_liar_resilience_kernel::{
    HardwareCanaryEntry, HardwareLiarResilienceJudge, HardwareLiarViolation,
};

#[test]
fn rfc0326_canary_integrity_and_checksum() {
    let nonce = [0x42u8; 16];
    let canary = HardwareCanaryEntry::new(100, nonce);
    assert_eq!(canary.epoch, 100);
    assert_eq!(canary.nonce, nonce);
    assert!(canary.verify_checksum());

    // Tampered canary fails checksum
    let mut tampered = canary;
    tampered.epoch = 101; // changed without recomputing checksum
    assert!(!tampered.verify_checksum());
}

#[test]
fn rfc0326_hardware_liar_detection_lifecycle() {
    let mut judge = HardwareLiarResilienceJudge::new();
    assert_eq!(judge.last_fsynced_epoch(), 0);

    let nonce_epoch_10 = [0x10u8; 16];
    let canary_10 = HardwareCanaryEntry::new(10, nonce_epoch_10);
    judge.register_fsynced_barrier(canary_10);
    assert_eq!(judge.last_fsynced_epoch(), 10);

    // 1. Clean post-crash verification: recovered record from epoch 10 matches canary 10
    assert!(judge.verify_post_crash_canary(&canary_10, 10).is_ok());

    // 2. Hardware Fsync Lie: recovered record claims epoch 12 was acked, but physical disk canary stopped at 10!
    let lie_err = judge.verify_post_crash_canary(&canary_10, 12);
    assert_eq!(
        lie_err,
        Err(HardwareLiarViolation::FsyncLieDetected {
            claimed_epoch: 12,
            physical_canary_epoch: 10,
        })
    );

    // 3. Stale snapshot / rollback: canary epoch is 10, but nonce is from an older run
    let stale_nonce = [0x99u8; 16];
    let stale_canary = HardwareCanaryEntry::new(10, stale_nonce);
    let nonce_err = judge.verify_post_crash_canary(&stale_canary, 10);
    assert_eq!(
        nonce_err,
        Err(HardwareLiarViolation::CanaryNonceMismatch {
            expected_nonce: nonce_epoch_10,
            found_nonce: stale_nonce,
        })
    );
}
