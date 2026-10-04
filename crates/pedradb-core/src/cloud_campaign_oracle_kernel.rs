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
use std::fmt;

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

impl fmt::Display for OracleOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvariantSatisfied => write!(f, "Oracle invariant satisfied"),
            Self::ViolationAckedWriteLost => {
                write!(f, "Oracle violation: acknowledged write lost after crash recovery")
            }
            Self::ViolationSilentCorruptionAccepted => {
                write!(f, "Oracle violation: corrupted storage block accepted checksum")
            }
            Self::ViolationTombstoneResurrection => {
                write!(f, "Oracle violation: deleted tombstone key resurrected")
            }
            Self::ViolationNondeterministicReplay => {
                write!(f, "Oracle violation: nondeterministic replay across identical seeds")
            }
        }
    }
}

impl std::error::Error for OracleOutcome {}

/// A sequenced mutation operation in the workload history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SequencedOp {
    /// Put operation updating a key-value pair.
    Put(Vec<u8>, Vec<u8>),
    /// Delete operation removing a key.
    Delete(Vec<u8>),
}

/// Mechanical oracle engine enforcing cloud storage invariants.
pub struct CloudCampaignOracleKernel;

impl CloudCampaignOracleKernel {
    /// Verify Oracle O1: Every acknowledged write must survive post-crash reopen.
    ///
    /// Evaluates latest sequenced writes to prevent false-positive failures on key overwrites.
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

        // Resolve latest value per key in acked_writes to handle sequential overwrites cleanly
        let mut latest_acked: BTreeMap<&[u8], &[u8]> = BTreeMap::new();
        for (k, v) in acked_writes {
            latest_acked.insert(k.as_slice(), v.as_slice());
        }

        for (k, expected_v) in latest_acked {
            match rec_map.get(k) {
                Some(&actual_v) if actual_v == expected_v => {}
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

    /// Verify sequenced lifecycle including both puts and deletes.
    pub fn verify_sequenced_lifecycle(
        ops: &[SequencedOp],
        recovered_state: &[(Vec<u8>, Vec<u8>)],
    ) -> Result<OracleOutcome, OracleOutcome> {
        let mut expected: BTreeMap<&[u8], Option<&[u8]>> = BTreeMap::new();
        for op in ops {
            match op {
                SequencedOp::Put(k, v) => {
                    expected.insert(k.as_slice(), Some(v.as_slice()));
                }
                SequencedOp::Delete(k) => {
                    expected.insert(k.as_slice(), None);
                }
            }
        }

        let actual: BTreeMap<&[u8], &[u8]> = recovered_state
            .iter()
            .map(|(k, v)| (k.as_slice(), v.as_slice()))
            .collect();

        for (k, exp_val) in expected {
            match exp_val {
                Some(v) => match actual.get(k) {
                    Some(&act_v) if act_v == v => {}
                    _ => return Err(OracleOutcome::ViolationAckedWriteLost),
                },
                None => {
                    if actual.contains_key(k) {
                        return Err(OracleOutcome::ViolationTombstoneResurrection);
                    }
                }
            }
        }

        Ok(OracleOutcome::InvariantSatisfied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cloud_campaign_oracle_structural_invariants_red_to_green() {
        // Invariant 1: Overwrite sequences do not cause false-positive ViolationAckedWriteLost
        let acked_with_overwrites = vec![
            (b"key1".to_vec(), b"val1".to_vec()),
            (b"key1".to_vec(), b"val2".to_vec()), // overwrite
        ];
        let recovered = vec![(b"key1".to_vec(), b"val2".to_vec())];
        assert_eq!(
            CloudCampaignOracleKernel::verify_acked_durability(&acked_with_overwrites, &recovered),
            Ok(OracleOutcome::InvariantSatisfied)
        );

        // Invariant 2: Missing acked key correctly detected
        let recovered_empty = vec![];
        assert_eq!(
            CloudCampaignOracleKernel::verify_acked_durability(&acked_with_overwrites, &recovered_empty),
            Err(OracleOutcome::ViolationAckedWriteLost)
        );

        // Invariant 3: Sequenced put and delete correctly verified
        let ops = vec![
            SequencedOp::Put(b"k1".to_vec(), b"v1".to_vec()),
            SequencedOp::Put(b"k2".to_vec(), b"v2".to_vec()),
            SequencedOp::Delete(b"k1".to_vec()),
        ];
        let state_valid = vec![(b"k2".to_vec(), b"v2".to_vec())];
        assert_eq!(
            CloudCampaignOracleKernel::verify_sequenced_lifecycle(&ops, &state_valid),
            Ok(OracleOutcome::InvariantSatisfied)
        );

        // Invariant 4: Deleted key resurrection correctly detected
        let state_resurrected = vec![
            (b"k1".to_vec(), b"v1".to_vec()),
            (b"k2".to_vec(), b"v2".to_vec()),
        ];
        assert_eq!(
            CloudCampaignOracleKernel::verify_sequenced_lifecycle(&ops, &state_resurrected),
            Err(OracleOutcome::ViolationTombstoneResurrection)
        );
    }
}

