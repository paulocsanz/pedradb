//! RFC-0320: Adaptive Bloom Filter Bit-Budget Allocator test suite.
//!
//! Verifies:
//! - Monkey / Proteus hierarchical bit allocation across LSM levels.
//! - Shallow level bit prioritization over deep levels.
//! - Budget boundary enforcement and error handling.
//! - Mathematical invariant consistency.

use pedradb_core::adaptive_bloom_budget_kernel::{
    AdaptiveBloomBudgetPlanner, BloomBudgetError,
};

#[test]
fn test_hierarchical_monkey_bit_allocation() {
    // 5 levels with exponential growth: L0: 100, L1: 1000, L2: 10_000, L3: 100_000, L4: 1_000_000
    let key_counts = vec![100u64, 1_000, 10_000, 100_000, 1_000_000];
    let total_keys: u64 = key_counts.iter().sum();
    let budget = total_keys * 10; // 10 bits per key on average

    let allocations = AdaptiveBloomBudgetPlanner::compute_allocations(&key_counts, budget)
        .expect("valid allocation");

    assert_eq!(allocations.len(), 5);
    // L0 must have >= bits than L1, L1 >= L2, etc.
    for i in 0..4 {
        assert!(
            allocations[i].bits_per_key >= allocations[i + 1].bits_per_key,
            "Level {} ({} bits) must be >= Level {} ({} bits)",
            i, allocations[i].bits_per_key, i + 1, allocations[i + 1].bits_per_key
        );
    }

    // Invariant check passes
    assert!(AdaptiveBloomBudgetPlanner::verify_internal_invariants(&allocations, &key_counts, budget));
}

#[test]
fn test_budget_error_rejection() {
    // Empty levels
    assert_eq!(
        AdaptiveBloomBudgetPlanner::compute_allocations(&[], 1000).err(),
        Some(BloomBudgetError::EmptyLevels)
    );

    // Zero budget
    assert_eq!(
        AdaptiveBloomBudgetPlanner::compute_allocations(&[100], 0).err(),
        Some(BloomBudgetError::ZeroBudget)
    );

    // Insufficient budget (< 2 bits per key)
    assert_eq!(
        AdaptiveBloomBudgetPlanner::compute_allocations(&[100], 150).err(),
        Some(BloomBudgetError::InsufficientBudget { required: 200, available: 150 })
    );
}

#[test]
fn test_adaptive_bloom_red_invariants() {
    // 1. Error implements std::error::Error
    let err: Box<dyn std::error::Error> = Box::new(BloomBudgetError::ZeroBudget);
    assert!(!err.to_string().is_empty());

    // 2. Reject zero total keys
    assert_eq!(
        AdaptiveBloomBudgetPlanner::compute_allocations(&[0, 0, 0], 1000).err(),
        Some(BloomBudgetError::ZeroTotalKeys)
    );

    // 3. Critical: Total allocated bits must NOT exceed total_bit_budget (preventing OOM)
    let key_counts = vec![50_000u64, 200_000, 1_000_000];
    let budget = 10_000_000u64; // strictly 10M bits
    let allocs = AdaptiveBloomBudgetPlanner::compute_allocations(&key_counts, budget)
        .expect("valid allocation");

    let total_used: u64 = allocs.iter().zip(&key_counts)
        .map(|(a, &k)| a.bits_per_key as u64 * k)
        .sum();
    assert!(
        total_used <= budget,
        "Total allocated bits {} exceeded strict memory budget {}",
        total_used, budget
    );

    // 4. Helper test: estimated_fpp_percent
    assert!(allocs[0].estimated_fpp_percent() > 0.0);
    assert!(allocs[0].estimated_fpp_percent() <= 100.0);
}

