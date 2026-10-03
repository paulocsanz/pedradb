//! RFC-0329: Zero-Recompile Mutation Switching (Stryker Mutator Pattern).
//!
//! Re-exports and wraps the foundational atomic mutation switching primitives
//! from [`pedradb_core::mutation_switch_kernel`].

pub use pedradb_core::mutation_switch_kernel::{
    active_mutant, all_mutants, is_any_mutant_active, is_mutant_active, mutant_name, reset_mutant,
    set_active_mutant, MutantGuard, MutationSwitchError, ACTIVE_MUTANT, MUTANT_BASELINE,
    MUTANT_BYPASS_WAL_SYNC, MUTANT_CORRUPT_RECORD_CRC, MUTANT_DECOMPRESSION_BOMB_BYPASS,
    MUTANT_DROP_MANIFEST_EDIT, MUTANT_FABRICATE_ORPHAN_KEY, MUTANT_INVERT_COMPARATOR,
    MUTANT_OVERFLOW_SATURATION_BYPASS, MUTANT_RESURRECT_TOMBSTONE,
};

/// Macro for zero-recompile runtime mutation switching.
///
/// In production/clean execution, it evaluates `$original`. If mutant `$id` is
/// activated via `set_active_mutant($id)`, it evaluates `$mutant` instead.
#[macro_export]
macro_rules! mutate_switch {
    ($id:expr, $original:expr, $mutant:expr) => {
        if $crate::mutation_switch::is_mutant_active($id) {
            $mutant
        } else {
            $original
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mutation_switching() {
        reset_mutant();
        assert!(!is_mutant_active(1));

        let val = mutate_switch!(1, 10 + 2, 10 - 2);
        assert_eq!(val, 12);

        {
            let _guard = MutantGuard::activate(1);
            assert!(is_mutant_active(1));
            let mutated_val = mutate_switch!(1, 10 + 2, 10 - 2);
            assert_eq!(mutated_val, 8);
        }

        // Guard dropped -> back to baseline
        assert!(!is_mutant_active(1));
        let restored_val = mutate_switch!(1, 10 + 2, 10 - 2);
        assert_eq!(restored_val, 12);
    }
}
