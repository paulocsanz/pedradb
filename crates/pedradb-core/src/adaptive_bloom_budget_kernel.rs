//! kernel: adaptive_bloom_budget
//! Adaptive multi-level Bloom filter bit-budget allocator (Monkey / Proteus model).
//!
//! Allocates varying bits-per-key per LSM level to minimize overall false positive rate
//! across hierarchical tree depths while strictly adhering to total memory budgets.

/// Typed errors produced during Bloom filter bit-budget allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BloomBudgetError {
    /// Total bit budget must be greater than zero.
    ZeroBudget,
    /// No levels specified for allocation.
    EmptyLevels,
    /// Total keys across all levels is zero.
    ZeroTotalKeys,
    /// Memory budget is insufficient to satisfy minimum bits-per-key constraint.
    InsufficientBudget { required: u64, available: u64 },
}

impl core::fmt::Display for BloomBudgetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZeroBudget => write!(f, "Total Bloom bit budget must be greater than zero"),
            Self::EmptyLevels => write!(f, "Level key counts slice is empty"),
            Self::ZeroTotalKeys => write!(f, "Total key count across all levels is zero"),
            Self::InsufficientBudget { required, available } => {
                write!(f, "Insufficient budget: required {} bits, available {}", required, available)
            }
        }
    }
}

impl std::error::Error for BloomBudgetError {}

/// Bit and hash allocation parameters computed for a specific LSM level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelBitAllocation {
    /// LSM level index (0 = L0, 1 = L1, etc.).
    pub level: usize,
    /// Allocated bits per key in this level.
    pub bits_per_key: u32,
    /// Optimal count of hash probes k = floor(b * ln(2)).
    pub hash_functions_k: u32,
    /// Estimated false positive probability in parts-per-million (e.g. 10_000 = 1.0%).
    pub false_positive_rate_ppm: u32,
}

impl LevelBitAllocation {
    /// Returns the estimated false positive probability as a percentage (0.0% to 100.0%).
    #[must_use]
    pub fn estimated_fpp_percent(&self) -> f64 {
        self.false_positive_rate_ppm as f64 / 10_000.0
    }
}

/// Computes mathematically optimal bit allocations across LSM tree levels.
pub struct AdaptiveBloomBudgetPlanner;

impl AdaptiveBloomBudgetPlanner {
    /// Computes Monkey-style bit allocations for each level given estimated key counts.
    ///
    /// Allocates higher bits-per-key to shallow levels (L0, L1) and lower to deep levels (L5+),
    /// minimizing total query false positive rates under `total_bit_budget` without exceeding RAM bounds.
    pub fn compute_allocations(
        level_key_counts: &[u64],
        total_bit_budget: u64,
    ) -> Result<Vec<LevelBitAllocation>, BloomBudgetError> {
        if level_key_counts.is_empty() {
            return Err(BloomBudgetError::EmptyLevels);
        }
        if total_bit_budget == 0 {
            return Err(BloomBudgetError::ZeroBudget);
        }

        let num_levels = level_key_counts.len();
        let total_keys: u64 = level_key_counts.iter().sum();
        if total_keys == 0 {
            return Err(BloomBudgetError::ZeroTotalKeys);
        }

        let min_required = total_keys.saturating_mul(2);
        if total_bit_budget < min_required {
            return Err(BloomBudgetError::InsufficientBudget {
                required: min_required,
                available: total_bit_budget,
            });
        }

        // Initialize every level with minimum 2 bits
        let mut bits: Vec<u32> = vec![2; num_levels];
        let remaining_budget = total_bit_budget - min_required;

        // Weight each level by its depth (L0 has weight num_levels, last level has weight 1)
        let weights: Vec<u64> = (0..num_levels)
            .map(|lvl| (num_levels - lvl) as u64)
            .collect();

        let weighted_key_sum: u128 = level_key_counts
            .iter()
            .zip(&weights)
            .map(|(&k, &w)| (k as u128) * (w as u128))
            .sum();

        if weighted_key_sum > 0 && remaining_budget > 0 {
            for (lvl, &w) in weights.iter().enumerate() {
                let add_bits = ((remaining_budget as u128 * w as u128) / weighted_key_sum) as u64;
                bits[lvl] = (2 + add_bits).clamp(2, 24) as u32;
            }
        }

        // Enforce monotonicity: bits[0] >= bits[1] >= ... >= bits[N-1]
        for i in 1..num_levels {
            if bits[i] > bits[i - 1] {
                bits[i] = bits[i - 1];
            }
        }

        // Adjust downwards if total exceeds budget
        let mut current_total: u64 = bits
            .iter()
            .zip(level_key_counts)
            .map(|(&b, &k)| (b as u64) * k)
            .sum();

        while current_total > total_bit_budget {
            let mut decreased = false;
            for i in (0..num_levels).rev() {
                if bits[i] > 2 {
                    bits[i] -= 1;
                    // re-enforce monotonicity forwards if needed
                    for j in i + 1..num_levels {
                        if bits[j] > bits[j - 1] {
                            bits[j] = bits[j - 1];
                        }
                    }
                    current_total = bits
                        .iter()
                        .zip(level_key_counts)
                        .map(|(&b, &k)| (b as u64) * k)
                        .sum();
                    decreased = true;
                    if current_total <= total_bit_budget {
                        break;
                    }
                }
            }
            if !decreased {
                break;
            }
        }

        // Top up shallowest levels with leftover budget while preserving budget and monotonicity
        for i in 0..num_levels {
            while bits[i] < 24 {
                let keys = level_key_counts[i];
                if current_total + keys <= total_bit_budget {
                    let next_val = bits[i] + 1;
                    if i == 0 || next_val <= bits[i - 1] {
                        bits[i] = next_val;
                        current_total += keys;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
        }

        let mut allocations = Vec::with_capacity(num_levels);
        for (lvl, &b_key) in bits.iter().enumerate() {
            // k = floor(b * ln(2)) = floor(b * 0.6931)
            let hash_functions_k = ((b_key as u64 * 693) / 1000).clamp(1, 16) as u32;

            // fp_ppm approx 1_000_000 * (1/2)^k
            let denom = 1u64 << hash_functions_k.min(20);
            let false_positive_rate_ppm = (1_000_000u64 / denom) as u32;

            allocations.push(LevelBitAllocation {
                level: lvl,
                bits_per_key: b_key,
                hash_functions_k,
                false_positive_rate_ppm,
            });
        }

        debug_assert!(Self::verify_internal_invariants(&allocations, level_key_counts, total_bit_budget));
        Ok(allocations)
    }

    /// Verifies that the allocations satisfy budget and monotonicity invariants.
    #[must_use]
    pub fn verify_internal_invariants(
        allocations: &[LevelBitAllocation],
        level_key_counts: &[u64],
        budget: u64,
    ) -> bool {
        if allocations.len() != level_key_counts.len() {
            return false;
        }

        let mut total_allocated_bits: u64 = 0;
        for (alloc, &keys) in allocations.iter().zip(level_key_counts) {
            if alloc.bits_per_key < 2 || alloc.bits_per_key > 24 {
                return false;
            }
            if alloc.hash_functions_k == 0 {
                return false;
            }
            total_allocated_bits = total_allocated_bits.saturating_add(keys.saturating_mul(alloc.bits_per_key as u64));
        }

        // Strict budget invariant: allocated bits must not exceed total budget
        if total_allocated_bits > budget {
            return false;
        }

        // Monotonicity: earlier levels must have >= bits_per_key than later levels
        for w in allocations.windows(2) {
            if w[0].bits_per_key < w[1].bits_per_key {
                return false;
            }
        }

        true
    }
}
