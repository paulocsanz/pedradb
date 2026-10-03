//! RFC-0329: Zero-Recompile Mutation Switching Kernel (Stryker Mutator Pattern).
//!
//! Provides an ultra-low-overhead ($O(1)$) in-memory atomic mutation switching
//! mechanism for continuous anti-vacuity audits and property verification without
//! incurring disk-churning recompilation cycles.
//!
//! Strictly adheres to `#![forbid(unsafe_code)]`.

use std::sync::atomic::{AtomicU32, Ordering};

/// Baseline execution: no synthetic mutant active.
pub const MUTANT_BASELINE: u32 = 0;

/// Synthetic mutant: bypass WAL `fdatasync` barrier (M1 / M6).
pub const MUTANT_BYPASS_WAL_SYNC: u32 = 1001;

/// Synthetic mutant: corrupt WAL record or SST block CRC (M2 / M4).
pub const MUTANT_CORRUPT_RECORD_CRC: u32 = 1002;

/// Synthetic mutant: leak / resurrect a deleted tombstone in scans (M5).
pub const MUTANT_RESURRECT_TOMBSTONE: u32 = 1003;

/// Synthetic mutant: invert comparator ordering in block / table scans (M6).
pub const MUTANT_INVERT_COMPARATOR: u32 = 1004;

/// Synthetic mutant: drop manifest version edit on publish.
pub const MUTANT_DROP_MANIFEST_EDIT: u32 = 1005;

/// Synthetic mutant: bypass SST decompression bomb bounds.
pub const MUTANT_DECOMPRESSION_BOMB_BYPASS: u32 = 1006;

/// Synthetic mutant: silent overflow saturation in measure monoid arithmetic.
pub const MUTANT_OVERFLOW_SATURATION_BYPASS: u32 = 1007;

/// Synthetic mutant: fabricate orphan key in scan output (M7).
pub const MUTANT_FABRICATE_ORPHAN_KEY: u32 = 1008;

/// Global atomic mutant switch (0 = clean production execution).
pub static ACTIVE_MUTANT: AtomicU32 = AtomicU32::new(MUTANT_BASELINE);

/// Sets the currently active synthetic mutant ID across the process.
#[inline]
pub fn set_active_mutant(id: u32) {
    ACTIVE_MUTANT.store(id, Ordering::SeqCst);
}

/// Returns the currently active synthetic mutant ID.
#[must_use]
#[inline]
pub fn active_mutant() -> u32 {
    ACTIVE_MUTANT.load(Ordering::Relaxed)
}

/// Checks whether a specific synthetic mutant ID is currently active.
#[must_use]
#[inline]
pub fn is_mutant_active(id: u32) -> bool {
    ACTIVE_MUTANT.load(Ordering::Relaxed) == id
}

/// Resets the synthetic mutant switch back to baseline (`MUTANT_BASELINE = 0`).
#[inline]
pub fn reset_mutant() {
    ACTIVE_MUTANT.store(MUTANT_BASELINE, Ordering::SeqCst);
}

/// Returns true if any synthetic mutant is currently active.
#[must_use]
#[inline]
pub fn is_any_mutant_active() -> bool {
    ACTIVE_MUTANT.load(Ordering::Relaxed) != MUTANT_BASELINE
}

/// Returns the descriptive name of a synthetic mutant ID.
#[must_use]
pub fn mutant_name(id: u32) -> &'static str {
    match id {
        MUTANT_BASELINE => "baseline_clean",
        MUTANT_BYPASS_WAL_SYNC => "bypass_wal_sync",
        MUTANT_CORRUPT_RECORD_CRC => "corrupt_record_crc",
        MUTANT_RESURRECT_TOMBSTONE => "resurrect_tombstone",
        MUTANT_INVERT_COMPARATOR => "invert_comparator",
        MUTANT_DROP_MANIFEST_EDIT => "drop_manifest_edit",
        MUTANT_DECOMPRESSION_BOMB_BYPASS => "decompression_bomb_bypass",
        MUTANT_OVERFLOW_SATURATION_BYPASS => "overflow_saturation_bypass",
        MUTANT_FABRICATE_ORPHAN_KEY => "fabricate_orphan_key",
        _ => "unknown_mutant",
    }
}

/// Returns a slice of all standardized synthetic mutant IDs.
#[must_use]
pub fn all_mutants() -> &'static [u32] {
    &[
        MUTANT_BYPASS_WAL_SYNC,
        MUTANT_CORRUPT_RECORD_CRC,
        MUTANT_RESURRECT_TOMBSTONE,
        MUTANT_INVERT_COMPARATOR,
        MUTANT_DROP_MANIFEST_EDIT,
        MUTANT_DECOMPRESSION_BOMB_BYPASS,
        MUTANT_OVERFLOW_SATURATION_BYPASS,
        MUTANT_FABRICATE_ORPHAN_KEY,
    ]
}

/// RAII Guard that activates a synthetic mutant ID and restores the previous ID on drop.
#[derive(Debug)]
pub struct MutantGuard {
    prev: u32,
}

impl MutantGuard {
    /// Activates mutant `id` and returns a guard that will restore the previous mutant when dropped.
    #[must_use]
    pub fn activate(id: u32) -> Self {
        let prev = ACTIVE_MUTANT.swap(id, Ordering::SeqCst);
        Self { prev }
    }

    /// Returns the previously active mutant before this guard was instantiated.
    #[must_use]
    pub fn previous_mutant(&self) -> u32 {
        self.prev
    }
}

impl Drop for MutantGuard {
    fn drop(&mut self) {
        ACTIVE_MUTANT.store(self.prev, Ordering::SeqCst);
    }
}

/// Error type for mutation switch evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationSwitchError {
    /// Inactive or unexpected mutant encountered.
    UnexpectedMutant {
        /// Expected mutant identifier.
        expected: u32,
        /// Actual active mutant identifier found.
        found: u32,
    },
    /// Mutant was expected to be killed by an oracle, but the oracle passed vacuously.
    MutantSurvivedVacuously {
        /// Identifier of the surviving mutant.
        mutant_id: u32,
        /// Name of the oracle that failed to catch the mutant.
        oracle_name: &'static str,
    },
}

impl std::fmt::Display for MutationSwitchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnexpectedMutant { expected, found } => {
                write!(f, "Unexpected active mutant: expected {expected}, found {found}")
            }
            Self::MutantSurvivedVacuously { mutant_id, oracle_name } => {
                write!(f, "Mutant {mutant_id} survived vacuously against oracle {oracle_name}")
            }
        }
    }
}

impl std::error::Error for MutationSwitchError {}

/// Macro for zero-recompile runtime mutation switching.
///
/// Under normal execution (mutant 0), evaluates `$original`.
/// When mutant `$id` is active, evaluates `$mutated` instead.
#[macro_export]
macro_rules! mutate_switch {
    ($id:expr, $original:expr, $mutated:expr) => {
        if $crate::mutation_switch_kernel::is_mutant_active($id) {
            $mutated
        } else {
            $original
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_baseline_clean_behavior() {
        reset_mutant();
        assert_eq!(active_mutant(), MUTANT_BASELINE);
        assert!(!is_mutant_active(MUTANT_BYPASS_WAL_SYNC));

        let res = mutate_switch!(MUTANT_BYPASS_WAL_SYNC, 42, 0);
        assert_eq!(res, 42);
    }

    #[test]
    fn test_mutant_guard_lifecycle() {
        reset_mutant();
        assert_eq!(active_mutant(), MUTANT_BASELINE);

        {
            let guard = MutantGuard::activate(MUTANT_CORRUPT_RECORD_CRC);
            assert_eq!(guard.previous_mutant(), MUTANT_BASELINE);
            assert_eq!(active_mutant(), MUTANT_CORRUPT_RECORD_CRC);
            assert!(is_mutant_active(MUTANT_CORRUPT_RECORD_CRC));
            assert!(!is_mutant_active(MUTANT_BYPASS_WAL_SYNC));

            let res = mutate_switch!(MUTANT_CORRUPT_RECORD_CRC, 100, 999);
            assert_eq!(res, 999);

            // Nested guard
            {
                let nested_guard = MutantGuard::activate(MUTANT_RESURRECT_TOMBSTONE);
                assert_eq!(nested_guard.previous_mutant(), MUTANT_CORRUPT_RECORD_CRC);
                assert_eq!(active_mutant(), MUTANT_RESURRECT_TOMBSTONE);
                assert!(is_mutant_active(MUTANT_RESURRECT_TOMBSTONE));
            }

            // Restored to first guard
            assert_eq!(active_mutant(), MUTANT_CORRUPT_RECORD_CRC);
        }

        // Dropped -> restored to baseline
        assert_eq!(active_mutant(), MUTANT_BASELINE);
        assert!(!is_mutant_active(MUTANT_CORRUPT_RECORD_CRC));
    }

    #[test]
    fn test_mutation_switch_error_display() {
        let err1 = MutationSwitchError::UnexpectedMutant { expected: 1001, found: 0 };
        assert!(format!("{err1}").contains("Unexpected active mutant: expected 1001, found 0"));

        let err2 = MutationSwitchError::MutantSurvivedVacuously {
            mutant_id: 1002,
            oracle_name: "O6_checksum_oracle",
        };
        assert!(format!("{err2}").contains("Mutant 1002 survived vacuously against oracle O6_checksum_oracle"));
    }
}
