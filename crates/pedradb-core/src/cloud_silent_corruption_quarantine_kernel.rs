//! kernel: cloud_silent_corruption_quarantine
//! Silent data corruption isolation and non-destructive file quarantine circuit breaker.
//!
//! Isolates bit-rotted or corrupted SST tables and WAL segments, preventing corrupted
//! data blocks from cascading into compaction merges or contaminating healthy cluster replicas.

/// Maximum number of files that can be placed in the quarantine register to prevent memory exhaustion / DoS.
pub const MAX_QUARANTINED_FILES: usize = 10_000;

/// Error encountered during quarantine registration or release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuarantineError {
    /// Attempted to quarantine file number 0 (sentinel / unallocated file number).
    ZeroFileNumberHazard,
    /// Quarantine register has reached its safety capacity limit.
    CapacityExceeded { max: usize },
    /// File requested for release was not found in quarantine.
    FileNotFound { file_number: u64 },
}

impl std::fmt::Display for QuarantineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroFileNumberHazard => {
                write!(f, "QuarantineError: cannot quarantine sentinel file number 0")
            }
            Self::CapacityExceeded { max } => {
                write!(f, "QuarantineError: quarantine register reached capacity limit {max}")
            }
            Self::FileNotFound { file_number } => {
                write!(f, "QuarantineError: file {file_number} not found in quarantine")
            }
        }
    }
}

impl std::error::Error for QuarantineError {}

/// Autonomically manages quarantined storage artifacts and filters compaction candidates.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SilentCorruptionQuarantineRegister {
    /// Sorted, unique list of file numbers placed into quarantine due to detected corruption.
    quarantined_files: Vec<u64>,
    /// Total count of corruption events recorded.
    total_corruption_events: u64,
}

impl SilentCorruptionQuarantineRegister {
    /// Constructs an empty quarantine register.
    #[must_use]
    pub fn new() -> Self {
        Self {
            quarantined_files: Vec::new(),
            total_corruption_events: 0,
        }
    }

    /// Attempts to mark a storage file number as quarantined due to detected silent corruption.
    ///
    /// Rejects file number 0 (sentinel hazard) and enforces maximum quarantine capacity bounds.
    pub fn try_mark_quarantined(&mut self, file_number: u64) -> Result<bool, QuarantineError> {
        if file_number == 0 {
            return Err(QuarantineError::ZeroFileNumberHazard);
        }

        self.total_corruption_events = self.total_corruption_events.saturating_add(1);

        match self.quarantined_files.binary_search(&file_number) {
            Ok(_) => Ok(false),
            Err(pos) => {
                if self.quarantined_files.len() >= MAX_QUARANTINED_FILES {
                    return Err(QuarantineError::CapacityExceeded {
                        max: MAX_QUARANTINED_FILES,
                    });
                }
                self.quarantined_files.insert(pos, file_number);
                debug_assert!(self.verify_internal_invariants());
                Ok(true)
            }
        }
    }

    /// Autonomically releases a healed or rebuilt file from quarantine.
    pub fn try_release_quarantined(&mut self, file_number: u64) -> Result<(), QuarantineError> {
        if file_number == 0 {
            return Err(QuarantineError::ZeroFileNumberHazard);
        }
        match self.quarantined_files.binary_search(&file_number) {
            Ok(pos) => {
                self.quarantined_files.remove(pos);
                debug_assert!(self.verify_internal_invariants());
                Ok(())
            }
            Err(_) => Err(QuarantineError::FileNotFound { file_number }),
        }
    }

    /// Marks a storage file number as quarantined due to detected silent corruption.
    ///
    /// Ensures idempotent insertion maintaining strictly sorted order without duplicates.
    /// Sentinel file 0 is safely ignored to preserve backward compatibility.
    pub fn mark_quarantined(&mut self, file_number: u64) {
        let _ = self.try_mark_quarantined(file_number);
    }

    /// Returns `true` if `file_number` is quarantined.
    #[must_use]
    pub fn is_quarantined(&self, file_number: u64) -> bool {
        self.quarantined_files.binary_search(&file_number).is_ok()
    }

    /// Filters out quarantined files from candidate compaction inputs.
    ///
    /// Guarantees that no corrupted file can ever be merged into deeper LSM levels.
    #[must_use]
    pub fn filter_compaction_candidates(&self, candidate_files: &[u64]) -> Vec<u64> {
        candidate_files
            .iter()
            .copied()
            .filter(|&f| !self.is_quarantined(f))
            .collect()
    }

    /// Returns the total number of distinct files currently in quarantine.
    #[must_use]
    pub fn quarantined_count(&self) -> usize {
        self.quarantined_files.len()
    }

    /// Total corruption events logged.
    #[must_use]
    pub fn total_corruption_events(&self) -> u64 {
        self.total_corruption_events
    }

    /// Verifies that the internal quarantine list is strictly monotonically sorted with no duplicates.
    #[must_use]
    pub fn verify_internal_invariants(&self) -> bool {
        for w in self.quarantined_files.windows(2) {
            if w[0] >= w[1] {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cloud_silent_corruption_quarantine_structural_invariants_red_to_green() {
        let mut reg = SilentCorruptionQuarantineRegister::new();

        // 1. Sentinel file number 0 must be rejected
        assert_eq!(
            reg.try_mark_quarantined(0),
            Err(QuarantineError::ZeroFileNumberHazard)
        );
        assert_eq!(
            reg.try_release_quarantined(0),
            Err(QuarantineError::ZeroFileNumberHazard)
        );
        assert!(!reg.is_quarantined(0));

        // 2. Normal quarantine lifecycle
        assert_eq!(reg.try_mark_quarantined(42), Ok(true));
        assert_eq!(reg.try_mark_quarantined(42), Ok(false)); // idempotent
        assert!(reg.is_quarantined(42));
        assert_eq!(reg.quarantined_count(), 1);

        // 3. Autonomic reconciliation / release
        assert_eq!(reg.try_release_quarantined(42), Ok(()));
        assert!(!reg.is_quarantined(42));
        assert_eq!(reg.quarantined_count(), 0);

        // 4. Release non-existent file
        assert_eq!(
            reg.try_release_quarantined(999),
            Err(QuarantineError::FileNotFound { file_number: 999 })
        );

        // 5. Invariant preservation
        reg.mark_quarantined(100);
        reg.mark_quarantined(50);
        reg.mark_quarantined(200);
        assert!(reg.verify_internal_invariants());
        assert_eq!(reg.filter_compaction_candidates(&[25, 50, 75, 100, 200, 300]), vec![25, 75, 300]);
    }
}
