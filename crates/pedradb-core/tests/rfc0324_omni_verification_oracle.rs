//! RFC-0324: Omni Verification Oracle Verification Suite.
//!
//! Mechanically verifies runtime cross-validation of Lean 4 algebraic invariants,
//! Verus representation contracts, Kani bit-level bounds, and DST multi-fault oracles.

#![forbid(unsafe_code)]

use pedradb_core::omni_verification_oracle_kernel::{
    OmniOracleViolation, OmniVerificationOracle,
};

#[test]
fn rfc0324_omni_oracle_commutativity_and_sequence_invariants() {
    let mut oracle = OmniVerificationOracle::new();

    // 1. Disjoint keys commute algebraically
    let key_a = b"user/100/profile";
    let key_b = b"user/200/profile";
    assert!(oracle.verify_commutativity(key_a, key_b).is_ok());

    // Identical keys also handled (last write wins, not disjoint violation)
    assert!(oracle.verify_commutativity(key_a, key_a).is_ok());

    // 2. Sequence progression must be strictly monotonic
    assert!(oracle.verify_sequence_progression(10).is_ok());
    assert!(oracle.verify_sequence_progression(11).is_ok());
    assert!(oracle.verify_sequence_progression(100).is_ok());

    // Sequence regression detected
    let seq_err = oracle.verify_sequence_progression(99);
    assert_eq!(
        seq_err,
        Err(OmniOracleViolation::SequenceRegression {
            previous: 100,
            current: 99,
        })
    );
}

#[test]
fn rfc0324_omni_oracle_tombstone_checksum_and_replay_determinism() {
    let mut oracle = OmniVerificationOracle::new();

    // 1. Tombstone integrity (O3)
    let key = b"deleted_key_test";
    // Non-tombstone with value is OK
    assert!(oracle.verify_tombstone_integrity(key, false, Some(b"live_val")).is_ok());
    // Tombstone with None is OK
    assert!(oracle.verify_tombstone_integrity(key, true, None).is_ok());
    // Tombstone with Some(value) is a RESURRECTION VIOLATION
    let resurr_err = oracle.verify_tombstone_integrity(key, true, Some(b"ghost_val"));
    assert_eq!(
        resurr_err,
        Err(OmniOracleViolation::ResurrectionViolation {
            key: key.to_vec(),
        })
    );

    // 2. Checksum verification (O7 & Kani)
    assert!(oracle.verify_block_checksum(1, 0x12345678, 0x12345678).is_ok());
    let crc_err = oracle.verify_block_checksum(2, 0x12345678, 0x99999999);
    assert_eq!(
        crc_err,
        Err(OmniOracleViolation::ChecksumMismatch {
            block_id: 2,
            expected_crc: 0x12345678,
            calculated_crc: 0x99999999,
        })
    );

    // 3. Replay determinism
    assert!(oracle.verify_replay_determinism(0xABCD, 0xABCD).is_ok());
    let replay_err = oracle.verify_replay_determinism(0xABCD, 0xDCBA);
    assert_eq!(
        replay_err,
        Err(OmniOracleViolation::ReplayDeterminismViolation {
            expected_hash: 0xABCD,
            actual_hash: 0xDCBA,
        })
    );

    let m = oracle.metrics();
    assert_eq!(m.total_violations, 3);
    assert!(m.commutativity_checks == 0); // none checked on this oracle instance
    assert_eq!(m.tombstone_checks, 3);
    assert_eq!(m.checksum_checks, 2);
    assert_eq!(m.determinism_checks, 2);
}
