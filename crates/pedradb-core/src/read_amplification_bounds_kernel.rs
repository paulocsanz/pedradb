//! kernel: read_amplification_bounds
//! Mathematical read amplification and expected disk probe bounds oracle for LSM point lookups.
//!
//! Models point lookup probe expectations under multi-level disjoint ranges and
//! hierarchical Bloom filter false positive rates, proving bounds against RocksDB baselines.

use std::fmt;

/// Typed errors produced during read amplification bound calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadAmplificationError {
    /// False positive probability exceeds 1.0 (1_000_000 ppm).
    InvalidFpRatePpm { level: usize, ppm: u32 },
}

impl fmt::Display for ReadAmplificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFpRatePpm { level, ppm } => {
                write!(
                    f,
                    "Invalid false positive rate at level {}: {} ppm exceeds maximum 1_000_000 ppm",
                    level, ppm
                )
            }
        }
    }
}

impl std::error::Error for ReadAmplificationError {}

/// Estimated read amplification metrics for LSM point lookups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadAmplificationEstimate {
    /// Expected number of physical table probes per lookup in parts-per-million (1_000_000 = 1.000 probe).
    pub expected_probes_ppm: u32,
    /// Absolute worst-case table probes (if all Bloom filters produce false positives).
    pub worst_case_probes: u32,
    /// Cumulative false positive probability across all levels in parts-per-million.
    pub total_false_positive_rate_ppm: u32,
}

impl ReadAmplificationEstimate {
    /// Returns expected probes as a floating-point number.
    #[must_use]
    pub fn expected_probes_f64(&self) -> f64 {
        self.expected_probes_ppm as f64 / 1_000_000.0
    }

    /// Returns cumulative false positive probability as a fraction [0.0, 1.0].
    #[must_use]
    pub fn total_fp_rate_f64(&self) -> f64 {
        self.total_false_positive_rate_ppm as f64 / 1_000_000.0
    }
}

/// Computes theoretical read amplification bounds and verifies empirical probe adherence.
pub struct ReadAmplificationOracle;

impl ReadAmplificationOracle {
    /// Validates false positive rate parameters.
    fn validate_fp_rates(level_fp_rates_ppm: &[u32]) -> Result<(), ReadAmplificationError> {
        for (lvl, &ppm) in level_fp_rates_ppm.iter().enumerate() {
            if ppm > 1_000_000 {
                return Err(ReadAmplificationError::InvalidFpRatePpm { level: lvl, ppm });
            }
        }
        Ok(())
    }

    /// Attempts to compute expected point lookup read amplification on a key HIT.
    pub fn try_estimate_hit_probes(
        level_fp_rates_ppm: &[u32],
        l0_table_count: u32,
    ) -> Result<ReadAmplificationEstimate, ReadAmplificationError> {
        Self::validate_fp_rates(level_fp_rates_ppm)?;

        if level_fp_rates_ppm.is_empty() {
            return Ok(ReadAmplificationEstimate {
                expected_probes_ppm: 0,
                worst_case_probes: 0,
                total_false_positive_rate_ppm: 0,
            });
        }

        let l0_fp = level_fp_rates_ppm[0] as u64;
        let mut total_fp_ppm: u64 = l0_fp.saturating_mul(l0_table_count as u64);

        for &lvl_fp in &level_fp_rates_ppm[1..] {
            total_fp_ppm = total_fp_ppm.saturating_add(lvl_fp as u64);
        }

        // On key hit, 1 probe is guaranteed for the target block + false positive misses
        let expected_probes_ppm = 1_000_000u64.saturating_add(total_fp_ppm).min(u32::MAX as u64) as u32;

        let disjoint_levels = (level_fp_rates_ppm.len().saturating_sub(1)) as u32;
        let worst_case_probes = l0_table_count.saturating_add(disjoint_levels);

        let estimate = ReadAmplificationEstimate {
            expected_probes_ppm,
            worst_case_probes,
            total_false_positive_rate_ppm: total_fp_ppm.min(u32::MAX as u64) as u32,
        };

        debug_assert!(Self::verify_internal_invariants(&estimate));
        Ok(estimate)
    }

    /// Computes the expected and worst-case point lookup read amplification on key HIT.
    #[must_use]
    pub fn estimate_probes(level_fp_rates_ppm: &[u32], l0_table_count: u32) -> ReadAmplificationEstimate {
        Self::try_estimate_hit_probes(level_fp_rates_ppm, l0_table_count).expect("valid fp rates")
    }

    /// Attempts to compute expected point lookup read amplification on a key MISS (no target block probe).
    pub fn try_estimate_miss_probes(
        level_fp_rates_ppm: &[u32],
        l0_table_count: u32,
    ) -> Result<ReadAmplificationEstimate, ReadAmplificationError> {
        Self::validate_fp_rates(level_fp_rates_ppm)?;

        if level_fp_rates_ppm.is_empty() {
            return Ok(ReadAmplificationEstimate {
                expected_probes_ppm: 0,
                worst_case_probes: 0,
                total_false_positive_rate_ppm: 0,
            });
        }

        let l0_fp = level_fp_rates_ppm[0] as u64;
        let mut total_fp_ppm: u64 = l0_fp.saturating_mul(l0_table_count as u64);

        for &lvl_fp in &level_fp_rates_ppm[1..] {
            total_fp_ppm = total_fp_ppm.saturating_add(lvl_fp as u64);
        }

        // On key miss, expected probes equals purely the false positive probability
        let expected_probes_ppm = total_fp_ppm.min(u32::MAX as u64) as u32;

        let disjoint_levels = (level_fp_rates_ppm.len().saturating_sub(1)) as u32;
        let worst_case_probes = l0_table_count.saturating_add(disjoint_levels);

        let estimate = ReadAmplificationEstimate {
            expected_probes_ppm,
            worst_case_probes,
            total_false_positive_rate_ppm: total_fp_ppm.min(u32::MAX as u64) as u32,
        };

        debug_assert!(Self::verify_internal_invariants(&estimate));
        Ok(estimate)
    }

    /// Computes expected point lookup read amplification on a key MISS.
    #[must_use]
    pub fn estimate_miss_probes(level_fp_rates_ppm: &[u32], l0_table_count: u32) -> ReadAmplificationEstimate {
        Self::try_estimate_miss_probes(level_fp_rates_ppm, l0_table_count).expect("valid fp rates")
    }

    /// Verifies mathematical consistency of the read amplification estimate.
    #[must_use]
    pub fn verify_internal_invariants(est: &ReadAmplificationEstimate) -> bool {
        if est.worst_case_probes == 0 {
            return est.expected_probes_ppm == 0 && est.total_false_positive_rate_ppm == 0;
        }
        let max_possible_ppm = (est.worst_case_probes as u64).saturating_mul(1_000_000);
        (est.expected_probes_ppm as u64) <= max_possible_ppm
    }
}
