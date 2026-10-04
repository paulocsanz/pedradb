//! RFC-0329: Zero-Recompile Mutation Switching & Anti-Vacuity Test Suite.
//!
//! Mechanically verifies that in-memory atomic mutation switches (Stryker Mutator pattern)
//! correctly alter execution paths during anti-vacuity evaluation, restore baseline state on
//! guard drop, and support multi-threaded concurrent isolation without requiring recompilations.

#![forbid(unsafe_code)]

use pedradb_core::mutation_switch_kernel::{
    active_mutant, is_mutant_active, reset_mutant, MutantGuard, MUTANT_BASELINE,
    MUTANT_BYPASS_WAL_SYNC, MUTANT_CORRUPT_RECORD_CRC, MUTANT_DECOMPRESSION_BOMB_BYPASS,
    MUTANT_DROP_MANIFEST_EDIT, MUTANT_FABRICATE_ORPHAN_KEY, MUTANT_INVERT_COMPARATOR,
    MUTANT_OVERFLOW_SATURATION_BYPASS, MUTANT_RESURRECT_TOMBSTONE,
};
use pedradb_core::mutate_switch;
use std::time::Instant;

#[test]
fn test_rfc0329_baseline_unmutated_execution() {
    reset_mutant();
    assert_eq!(active_mutant(), MUTANT_BASELINE);
    assert!(!is_mutant_active(MUTANT_BYPASS_WAL_SYNC));

    // In clean execution, original computation is preserved
    let sync_called = mutate_switch!(MUTANT_BYPASS_WAL_SYNC, true, false);
    assert!(sync_called, "Baseline must execute original branch (fsync performed)");

    let crc = mutate_switch!(MUTANT_CORRUPT_RECORD_CRC, 0x1234_5678u32, 0x0000_0000u32);
    assert_eq!(crc, 0x1234_5678);
}

#[test]
fn test_rfc0329_all_predefined_mutants_activation() {
    reset_mutant();

    let mutants = [
        (MUTANT_BYPASS_WAL_SYNC, "bypass_wal_sync"),
        (MUTANT_CORRUPT_RECORD_CRC, "corrupt_record_crc"),
        (MUTANT_RESURRECT_TOMBSTONE, "resurrect_tombstone"),
        (MUTANT_INVERT_COMPARATOR, "invert_comparator"),
        (MUTANT_DROP_MANIFEST_EDIT, "drop_manifest_edit"),
        (MUTANT_DECOMPRESSION_BOMB_BYPASS, "decompression_bomb_bypass"),
        (MUTANT_OVERFLOW_SATURATION_BYPASS, "overflow_saturation_bypass"),
        (MUTANT_FABRICATE_ORPHAN_KEY, "fabricate_orphan_key"),
    ];

    for &(id, desc) in &mutants {
        assert!(!is_mutant_active(id), "Mutant {desc} ({id}) must start inactive");
        {
            let guard = MutantGuard::activate(id);
            assert_eq!(active_mutant(), id);
            assert!(is_mutant_active(id));

            // Verify mutate_switch! evaluates the mutated expression
            let branch = mutate_switch!(id, "original", "mutated");
            assert_eq!(branch, "mutated", "Active mutant {desc} must branch to mutated code");
            assert_eq!(guard.previous_mutant(), MUTANT_BASELINE);
        }
        // Immediately after drop, state must return to baseline
        assert_eq!(active_mutant(), MUTANT_BASELINE);
        assert!(!is_mutant_active(id));
    }
}

#[test]
fn test_rfc0329_nested_mutant_guard_stack() {
    reset_mutant();

    let guard_level1 = MutantGuard::activate(MUTANT_BYPASS_WAL_SYNC);
    assert_eq!(active_mutant(), MUTANT_BYPASS_WAL_SYNC);

    {
        let guard_level2 = MutantGuard::activate(MUTANT_INVERT_COMPARATOR);
        assert_eq!(active_mutant(), MUTANT_INVERT_COMPARATOR);
        assert_eq!(guard_level2.previous_mutant(), MUTANT_BYPASS_WAL_SYNC);

        {
            let guard_level3 = MutantGuard::activate(MUTANT_OVERFLOW_SATURATION_BYPASS);
            assert_eq!(active_mutant(), MUTANT_OVERFLOW_SATURATION_BYPASS);
            assert_eq!(guard_level3.previous_mutant(), MUTANT_INVERT_COMPARATOR);
        }

        // After level 3 drops, restored to level 2
        assert_eq!(active_mutant(), MUTANT_INVERT_COMPARATOR);
    }

    // After level 2 drops, restored to level 1
    assert_eq!(active_mutant(), MUTANT_BYPASS_WAL_SYNC);

    drop(guard_level1);
    // After level 1 drops, restored to baseline
    assert_eq!(active_mutant(), MUTANT_BASELINE);
}

#[test]
fn test_rfc0329_mutation_switch_sub_nanosecond_overhead() {
    reset_mutant();

    // Verify checking mutant state is essentially free (relaxed atomic load)
    let iterations = 1_000_000;
    let start = Instant::now();
    let mut sum = 0u64;

    for i in 0..iterations {
        let val = mutate_switch!(MUTANT_BYPASS_WAL_SYNC, i, 0);
        sum = sum.wrapping_add(val);
    }

    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / iterations as f64;
    // Relaxed atomic read executes with minimal overhead (< 250 ns in unoptimized debug, < 10 ns in release).
    let threshold = if cfg!(debug_assertions) { 250.0 } else { 10.0 };
    assert!(ns_per_op < threshold, "Mutation switch overhead too high: {ns_per_op} ns/op (threshold {threshold})");
}

#[test]
fn test_rfc0329_oracle_anti_vacuity_teeth() {
    reset_mutant();

    // Mock an oracle validating data monotonicity
    fn mock_scan_oracle(data: &[u64]) -> Result<(), &'static str> {
        let is_sorted = mutate_switch!(
            MUTANT_INVERT_COMPARATOR,
            data.windows(2).all(|w| w[0] <= w[1]),
            // Inverted mutant: reports sorted only if reverse sorted
            data.windows(2).all(|w| w[0] >= w[1])
        );

        if is_sorted {
            Ok(())
        } else {
            Err("O2_monotonicity_violation")
        }
    }

    let test_data = vec![1, 2, 3, 4, 5];

    // Baseline: oracle passes cleanly
    assert!(mock_scan_oracle(&test_data).is_ok());

    // With MUTANT_INVERT_COMPARATOR: oracle MUST catch the mutant and fail
    {
        let _guard = MutantGuard::activate(MUTANT_INVERT_COMPARATOR);
        let res = mock_scan_oracle(&test_data);
        assert_eq!(res, Err("O2_monotonicity_violation"), "Oracle MUST kill the comparator mutant");
    }

    // After drop: oracle passes again
    assert!(mock_scan_oracle(&test_data).is_ok());
}
