//! RFC-0322: Cloud Campaign Multi-Fault Oracle Kernel.
//!
//! Formal mechanical oracles validating LSM storage invariants across continuous
//! multi-fault simulation runs (RFC-0270 Zero-Twin / RFC-0273 Absolute Rigor):
//! - Oracle O1: Acked write durability post-crash.
//! - Oracle O2: Zero silent bit-rot acceptance.
//! - Oracle O3: Zero resurrection of deleted keys.
//! - Oracle O8: Bit-exact deterministic replay reproducibility.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

/// Formal verification status returned by continuous campaign oracles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleOutcome {
    /// Invariant mathematically holds.
    InvariantSatisfied,
    /// Acked write lost after crash recovery.
    ViolationAckedWriteLost,
    /// Corrupted storage block passed checksum verification.
    ViolationSilentCorruptionAccepted,
    /// Deleted key resurrected after crash recovery.
    ViolationTombstoneResurrection,
    /// Replay divergence across identical fault seeds.
    ViolationNondeterministicReplay,
}

/// Mechanical oracle engine enforcing cloud storage invariants.
pub struct CloudCampaignOracleKernel;

impl CloudCampaignOracleKernel {
    /// Verify Oracle O1: Every acknowledged write must survive post-crash reopen.
    ///
    /// # Errors
    /// Returns `OracleOutcome::ViolationAckedWriteLost` if any acked key is missing or diverged.
    pub fn verify_acked_durability(
        acked_writes: &[(Vec<u8>, Vec<u8>)],
        recovered_state: &[(Vec<u8>, Vec<u8>)],
    ) -> Result<OracleOutcome, OracleOutcome> {
        let rec_map: BTreeMap<&[u8], &[u8]> = recovered_state
            .iter()
            .map(|(k, v)| (k.as_slice(), v.as_slice()))
            .collect();

        for (k, expected_v) in acked_writes {
            match rec_map.get(k.as_slice()) {
                Some(&actual_v) if actual_v == expected_v.as_slice() => {}
                _ => return Err(OracleOutcome::ViolationAckedWriteLost),
            }
        }

        Ok(OracleOutcome::InvariantSatisfied)
    }

    /// Verify Oracle O3: No key acknowledged as deleted may appear in recovered state.
    ///
    /// # Errors
    /// Returns `OracleOutcome::ViolationTombstoneResurrection` if a deleted key is found.
    pub fn verify_zero_resurrection(
        deleted_keys: &[Vec<u8>],
        recovered_state: &[(Vec<u8>, Vec<u8>)],
    ) -> Result<OracleOutcome, OracleOutcome> {
        let del_set: BTreeSet<&[u8]> = deleted_keys.iter().map(|k| k.as_slice()).collect();

        for (k, _) in recovered_state {
            if del_set.contains(k.as_slice()) {
                return Err(OracleOutcome::ViolationTombstoneResurrection);
            }
        }

        Ok(OracleOutcome::InvariantSatisfied)
    }

    /// Verify Oracle O2: Corrupted blocks must never return `checksum_ok == true`.
    ///
    /// # Errors
    /// Returns `OracleOutcome::ViolationSilentCorruptionAccepted` if corruption is silently ignored.
    pub fn verify_checksum_alert(
        has_bit_rot: bool,
        checksum_passed: bool,
    ) -> Result<OracleOutcome, OracleOutcome> {
        if has_bit_rot && checksum_passed {
            Err(OracleOutcome::ViolationSilentCorruptionAccepted)
        } else {
            Ok(OracleOutcome::InvariantSatisfied)
        }
    }

    /// Verify Oracle O8: Identical seeds must produce bit-exact identical recovered states.
    ///
    /// # Errors
    /// Returns `OracleOutcome::ViolationNondeterministicReplay` if states diverge.
    pub fn verify_deterministic_replay(
        run_a: &[(Vec<u8>, Vec<u8>)],
        run_b: &[(Vec<u8>, Vec<u8>)],
    ) -> Result<OracleOutcome, OracleOutcome> {
        if run_a == run_b {
            Ok(OracleOutcome::InvariantSatisfied)
        } else {
            Err(OracleOutcome::ViolationNondeterministicReplay)
        }
    }
}
