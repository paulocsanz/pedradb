//! RFC-0326: Hardware Liar Resilience & Fsync Integrity Judge Kernel.
//!
//! Protects against storage controllers and hypervisors that report successful
//! `fdatasync` completions while retaining uncommitted blocks in volatile cache
//! (RFC-0041, RFC-0229, RFC-0326).
//! Enforces persistent canary nonces across barriers to detect and isolate hardware lies.

#![forbid(unsafe_code)]

use std::fmt;

/// Errors emitted when storage hardware or controller lies about durability barriers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HardwareLiarViolation {
    /// Controller reported fsync completion, but physical disk lacks the barrier canary.
    FsyncLieDetected {
        claimed_epoch: u64,
        physical_canary_epoch: u64,
    },
    /// Barrier canary nonce mismatch: disk reverted to stale snapshot block.
    CanaryNonceMismatch {
        expected_nonce: [u8; 16],
        found_nonce: [u8; 16],
    },
    /// Canary block header or CRC corrupted.
    CorruptedCanaryBlock {
        epoch: u64,
        expected_crc: u32,
        found_crc: u32,
    },
    /// Epoch 0 is reserved and invalid for durability barrier.
    ZeroEpochHazard,
    /// Nil nonce (all zeroes) provides zero entropy.
    NilNonceHazard,
}

impl fmt::Display for HardwareLiarViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FsyncLieDetected { claimed_epoch, physical_canary_epoch } => {
                write!(
                    f,
                    "Hardware fsync lie detected: claimed epoch {claimed_epoch} > physical canary {physical_canary_epoch}"
                )
            }
            Self::CanaryNonceMismatch { expected_nonce, found_nonce } => {
                write!(
                    f,
                    "Hardware canary nonce mismatch: expected {:?}, found {:?}",
                    expected_nonce, found_nonce
                )
            }
            Self::CorruptedCanaryBlock { epoch, expected_crc, found_crc } => {
                write!(
                    f,
                    "Canary block corrupted at epoch {epoch}: expected CRC 0x{expected_crc:08x}, found 0x{found_crc:08x}"
                )
            }
            Self::ZeroEpochHazard => write!(f, "Hardware canary epoch cannot be 0"),
            Self::NilNonceHazard => write!(f, "Hardware canary nonce cannot be nil (all zeros)"),
        }
    }
}

impl std::error::Error for HardwareLiarViolation {}

/// Persistent barrier canary record written alongside epoch commits.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HardwareCanaryEntry {
    pub epoch: u64,
    pub nonce: [u8; 16],
    pub checksum: u32,
}

impl HardwareCanaryEntry {
    /// Safely creates a new canary entry, rejecting epoch 0 and nil nonce.
    pub fn try_new(epoch: u64, nonce: [u8; 16]) -> Result<Self, HardwareLiarViolation> {
        if epoch == 0 {
            return Err(HardwareLiarViolation::ZeroEpochHazard);
        }
        if nonce == [0u8; 16] {
            return Err(HardwareLiarViolation::NilNonceHazard);
        }
        Ok(Self::new(epoch, nonce))
    }

    /// Creates a new canary entry and computes its CRC.
    #[must_use]
    pub fn new(epoch: u64, nonce: [u8; 16]) -> Self {
        let mut crc = 0x82f6_3b78u32;
        for b in epoch.to_le_bytes().iter().chain(nonce.iter()) {
            crc = crc.wrapping_add(u32::from(*b)).rotate_left(3);
        }
        Self {
            epoch,
            nonce,
            checksum: crc,
        }
    }

    /// Verifies the checksum of the canary entry.
    #[must_use]
    pub fn verify_checksum(&self) -> bool {
        let expected = Self::new(self.epoch, self.nonce).checksum;
        self.checksum == expected
    }
}

/// Evaluator checking for storage controller fsync lies post-crash.
#[derive(Debug, Clone, Default)]
pub struct HardwareLiarResilienceJudge {
    last_fsynced_epoch: u64,
    last_fsynced_nonce: [u8; 16],
}

impl HardwareLiarResilienceJudge {
    /// Creates a new judge starting at epoch 0.
    #[must_use]
    pub fn new() -> Self {
        Self {
            last_fsynced_epoch: 0,
            last_fsynced_nonce: [0u8; 16],
        }
    }

    /// Validates and records that an epoch barrier with the given nonce was successfully fsynced.
    pub fn try_register_fsynced_barrier(&mut self, canary: HardwareCanaryEntry) -> Result<(), HardwareLiarViolation> {
        if canary.epoch == 0 {
            return Err(HardwareLiarViolation::ZeroEpochHazard);
        }
        if canary.nonce == [0u8; 16] {
            return Err(HardwareLiarViolation::NilNonceHazard);
        }
        if !canary.verify_checksum() {
            return Err(HardwareLiarViolation::CorruptedCanaryBlock {
                epoch: canary.epoch,
                expected_crc: HardwareCanaryEntry::new(canary.epoch, canary.nonce).checksum,
                found_crc: canary.checksum,
            });
        }
        if canary.epoch >= self.last_fsynced_epoch {
            self.last_fsynced_epoch = canary.epoch;
            self.last_fsynced_nonce = canary.nonce;
        }
        Ok(())
    }

    /// Records that an epoch barrier with the given nonce was successfully fsynced.
    pub fn register_fsynced_barrier(&mut self, canary: HardwareCanaryEntry) {
        let _ = self.try_register_fsynced_barrier(canary);
    }

    /// Verifies that a recovered record's dependency matches the physical canary on disk.
    ///
    /// If the recovered record claims epoch $E$, but the physically recovered canary on disk
    /// has epoch $< E$, the hardware controller lied about an fsync completion prior to power loss.
    pub fn verify_post_crash_canary(
        &self,
        physical_canary: &HardwareCanaryEntry,
        recovered_record_epoch: u64,
    ) -> Result<(), HardwareLiarViolation> {
        if !physical_canary.verify_checksum() {
            return Err(HardwareLiarViolation::CorruptedCanaryBlock {
                epoch: physical_canary.epoch,
                expected_crc: HardwareCanaryEntry::new(physical_canary.epoch, physical_canary.nonce).checksum,
                found_crc: physical_canary.checksum,
            });
        }

        if physical_canary.epoch < recovered_record_epoch {
            return Err(HardwareLiarViolation::FsyncLieDetected {
                claimed_epoch: recovered_record_epoch,
                physical_canary_epoch: physical_canary.epoch,
            });
        }

        if self.last_fsynced_epoch > 0
            && physical_canary.epoch == self.last_fsynced_epoch
            && physical_canary.nonce != self.last_fsynced_nonce
        {
            return Err(HardwareLiarViolation::CanaryNonceMismatch {
                expected_nonce: self.last_fsynced_nonce,
                found_nonce: physical_canary.nonce,
            });
        }

        Ok(())
    }

    /// Last verified physical epoch.
    #[must_use]
    pub fn last_fsynced_epoch(&self) -> u64 {
        self.last_fsynced_epoch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hardware_liar_resilience_structural_invariants_red_to_green() {
        // 1. Zero epoch rejected
        assert_eq!(
            HardwareCanaryEntry::try_new(0, [1u8; 16]),
            Err(HardwareLiarViolation::ZeroEpochHazard)
        );

        // 2. Nil nonce rejected
        assert_eq!(
            HardwareCanaryEntry::try_new(1, [0u8; 16]),
            Err(HardwareLiarViolation::NilNonceHazard)
        );

        // 3. Valid canary entry
        let canary = HardwareCanaryEntry::try_new(5, [9u8; 16]).unwrap();
        assert!(canary.verify_checksum());

        // 4. Judge registers barrier and detects fsync lie
        let mut judge = HardwareLiarResilienceJudge::new();
        assert_eq!(judge.try_register_fsynced_barrier(canary), Ok(()));
        assert_eq!(judge.last_fsynced_epoch(), 5);

        // Physical canary has epoch 4, but recovered record claims epoch 5 -> Fsync lie!
        let stale_canary = HardwareCanaryEntry::try_new(4, [8u8; 16]).unwrap();
        assert_eq!(
            judge.verify_post_crash_canary(&stale_canary, 5),
            Err(HardwareLiarViolation::FsyncLieDetected {
                claimed_epoch: 5,
                physical_canary_epoch: 4,
            })
        );
    }
}

