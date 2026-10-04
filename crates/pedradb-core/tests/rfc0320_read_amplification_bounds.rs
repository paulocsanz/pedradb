//! RFC-0320: Read Amplification Bounds test suite.
//!
//! Verifies:
//! - Expected probe estimation under hierarchical false positive rates.
//! - Bounds verification against worst-case table scans.
//! - Disjoint run filtering in L1+ levels.

use pedradb_core::read_amplification_bounds_kernel::ReadAmplificationOracle;

#[test]
fn test_read_amplification_probe_estimation() {
    // 4 levels, each with 10_000 ppm (1%) false positive rate
    // 4 tables in L0 (overlapping)
    let fp_rates = vec![10_000u32, 10_000, 10_000, 10_000];
    let estimate = ReadAmplificationOracle::estimate_probes(&fp_rates, 4);

    // L0 contribution: 4 * 10_000 = 40_000 ppm
    // L1..L3 contribution: 3 * 10_000 = 30_000 ppm
    // Total FP = 70_000 ppm (0.07 miss probes)
    // Expected probes on hit = 1 + 0.07 = 1.07 probes = 1_070_000 ppm
    assert_eq!(estimate.total_false_positive_rate_ppm, 70_000);
    assert_eq!(estimate.expected_probes_ppm, 1_070_000);

    // Worst case = 4 (L0) + 3 (disjoint levels L1..L3) = 7 probes
    assert_eq!(estimate.worst_case_probes, 7);
    assert!(ReadAmplificationOracle::verify_internal_invariants(&estimate));
}

#[test]
fn test_empty_levels_estimation() {
    let estimate = ReadAmplificationOracle::estimate_probes(&[], 0);
    assert_eq!(estimate.expected_probes_ppm, 0);
    assert_eq!(estimate.worst_case_probes, 0);
    assert!(ReadAmplificationOracle::verify_internal_invariants(&estimate));
}

#[test]
fn test_read_amplification_red_invariants() {
    use pedradb_core::read_amplification_bounds_kernel::{
        ReadAmplificationError, ReadAmplificationEstimate,
    };

    // 1. Error implements std::error::Error
    let err: Box<dyn std::error::Error> = Box::new(ReadAmplificationError::InvalidFpRatePpm {
        level: 0,
        ppm: 2_000_000,
    });
    assert!(!err.to_string().is_empty());

    // 2. Reject FP rate > 1_000_000 ppm
    assert_eq!(
        ReadAmplificationOracle::try_estimate_hit_probes(&[1_500_000], 1).err(),
        Some(ReadAmplificationError::InvalidFpRatePpm {
            level: 0,
            ppm: 1_500_000,
        })
    );

    // 3. estimate_miss_probes computes expected probes without the +1 hit probe
    let fp_rates = vec![10_000u32, 10_000];
    let miss_est = ReadAmplificationOracle::estimate_miss_probes(&fp_rates, 2);
    // L0: 2 * 10_000 = 20_000; L1: 10_000; Total = 30_000 ppm
    assert_eq!(miss_est.expected_probes_ppm, 30_000);
    assert_eq!(miss_est.total_false_positive_rate_ppm, 30_000);
    assert_eq!(miss_est.worst_case_probes, 3); // 2 (L0) + 1 (L1)

    // 4. Invariant check rejects expected_probes > 0 when worst_case_probes == 0
    let invalid_est = ReadAmplificationEstimate {
        expected_probes_ppm: 500_000,
        worst_case_probes: 0,
        total_false_positive_rate_ppm: 0,
    };
    assert!(!ReadAmplificationOracle::verify_internal_invariants(&invalid_est));
}

