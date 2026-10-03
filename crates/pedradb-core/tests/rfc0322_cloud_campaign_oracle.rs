//! RFC-0322: Cloud Campaign Oracle Verification Suite.

use pedradb_core::cloud_campaign_oracle_kernel::{
    CloudCampaignOracleKernel, OracleOutcome,
};

#[test]
fn test_oracle_o1_acked_durability() {
    let acked = vec![
        (b"user:101".to_vec(), b"alice".to_vec()),
        (b"user:102".to_vec(), b"bob".to_vec()),
    ];

    // Perfect recovery matches acked.
    let recovered_ok = vec![
        (b"user:101".to_vec(), b"alice".to_vec()),
        (b"user:102".to_vec(), b"bob".to_vec()),
        (b"user:103".to_vec(), b"charlie".to_vec()),
    ];
    assert_eq!(
        CloudCampaignOracleKernel::verify_acked_durability(&acked, &recovered_ok),
        Ok(OracleOutcome::InvariantSatisfied)
    );

    // Missing key fails O1.
    let recovered_missing = vec![(b"user:101".to_vec(), b"alice".to_vec())];
    assert_eq!(
        CloudCampaignOracleKernel::verify_acked_durability(&acked, &recovered_missing),
        Err(OracleOutcome::ViolationAckedWriteLost)
    );

    // Value mismatch fails O1.
    let recovered_diverged = vec![
        (b"user:101".to_vec(), b"alice".to_vec()),
        (b"user:102".to_vec(), b"wrong_value".to_vec()),
    ];
    assert_eq!(
        CloudCampaignOracleKernel::verify_acked_durability(&acked, &recovered_diverged),
        Err(OracleOutcome::ViolationAckedWriteLost)
    );
}

#[test]
fn test_oracle_o3_zero_tombstone_resurrection() {
    let deleted = vec![b"session:999".to_vec(), b"cart:888".to_vec()];

    // Recovered state has no deleted keys.
    let recovered_clean = vec![
        (b"session:1000".to_vec(), b"active".to_vec()),
    ];
    assert_eq!(
        CloudCampaignOracleKernel::verify_zero_resurrection(&deleted, &recovered_clean),
        Ok(OracleOutcome::InvariantSatisfied)
    );

    // Resurrected tombstone fails O3.
    let recovered_ghost = vec![
        (b"session:999".to_vec(), b"stale_session".to_vec()),
    ];
    assert_eq!(
        CloudCampaignOracleKernel::verify_zero_resurrection(&deleted, &recovered_ghost),
        Err(OracleOutcome::ViolationTombstoneResurrection)
    );
}

#[test]
fn test_oracle_o2_checksum_alert() {
    // Normal healthy block passing checksum -> OK.
    assert_eq!(
        CloudCampaignOracleKernel::verify_checksum_alert(false, true),
        Ok(OracleOutcome::InvariantSatisfied)
    );

    // Corrupted block detected by checksum failure -> OK.
    assert_eq!(
        CloudCampaignOracleKernel::verify_checksum_alert(true, false),
        Ok(OracleOutcome::InvariantSatisfied)
    );

    // Corrupted block silently accepted -> VIOLATION.
    assert_eq!(
        CloudCampaignOracleKernel::verify_checksum_alert(true, true),
        Err(OracleOutcome::ViolationSilentCorruptionAccepted)
    );
}

#[test]
fn test_oracle_o8_deterministic_replay() {
    let run_a = vec![(b"k1".to_vec(), b"v1".to_vec()), (b"k2".to_vec(), b"v2".to_vec())];
    let run_b = vec![(b"k1".to_vec(), b"v1".to_vec()), (b"k2".to_vec(), b"v2".to_vec())];
    let run_c = vec![(b"k1".to_vec(), b"v1".to_vec()), (b"k2".to_vec(), b"v_diverged".to_vec())];

    assert_eq!(
        CloudCampaignOracleKernel::verify_deterministic_replay(&run_a, &run_b),
        Ok(OracleOutcome::InvariantSatisfied)
    );
    assert_eq!(
        CloudCampaignOracleKernel::verify_deterministic_replay(&run_a, &run_c),
        Err(OracleOutcome::ViolationNondeterministicReplay)
    );
}
