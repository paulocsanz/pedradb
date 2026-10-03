//! kernel: cloud_silent_corruption_quarantine
//! Silent data corruption isolation and non-destructive file quarantine circuit breaker.
//!
//! Isolates bit-rotted or corrupted SST tables and WAL segments, preventing corrupted
//! data blocks from cascading into compaction merges or contaminating healthy cluster replicas.

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

    /// Marks a storage file number as quarantined due to detected silent corruption.
    ///
    /// Ensures idempotent insertion maintaining strictly sorted order without duplicates.
    pub fn mark_quarantined(&mut self, file_number: u64) {
        self.total_corruption_events = self.total_corruption_events.saturating_add(1);

        match self.quarantined_files.binary_search(&file_number) {
            Ok(_) => {
                // Already present in quarantine register
            }
            Err(pos) => {
                self.quarantined_files.insert(pos, file_number);
            }
        }

        debug_assert!(self.verify_internal_invariants());
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
