//! RFC-0199 P2.3 — bloom probe count twin.
//!
//! Registered theorem: `bloom_may_contain_work_bound` (`BloomCount.lean`)
//! — a query's work twin never exceeds the policy's probe count k; the
//! short-circuit (clear bit) only pays less. The twin below replicates
//! `BloomFilter::may_contain`'s loop with the REAL production primitives
//! (`hash_pair`/`probe_bit`/`bit_index`/`test_bit` — the same defs the
//! Aeneas extract exposes at module level), keeping the probe COUNTER in
//! the test, never in the hot path. The bits view comes from the REAL
//! `encode()` trailer (bits are the private field's exact bytes).

use pedradb_core::bloom::{bit_index, hash_pair, probe_bit, test_bit, BloomFilter};

/// Twin of `BloomFilter::may_contain`: same loop shape, same real
/// primitives, plus the probe counter. Returns (probes paid, answer).
fn may_contain_counted(f: &BloomFilter, key: &[u8]) -> (usize, bool) {
    if !f.is_active() {
        return (0, true);
    }
    let (h1, h2) = hash_pair(key);
    let nbits = u64::from(f.bit_count());
    let bits = f.encode();
    let bits = &bits[12..]; // header is nbits|k|nbytes, then the bits
    let k = f.hash_count();
    let mut i = 0u32;
    let mut probes = 0usize;
    while i < k {
        probes += 1;
        if !test_bit(bits, bit_index(probe_bit(h1, h2, i, nbits))) {
            return (probes, false);
        }
        i += 1;
    }
    (probes, true)
}

/// A full pass pays exactly the policy's probe count — never more.
#[test]
fn full_pass_pays_exactly_k_probes() {
    for &(n_keys, bits_per_key) in &[(100, 10), (1_000, 8), (10, 20)] {
        let mut f = BloomFilter::with_capacity(n_keys, bits_per_key);
        // an empty filter: every probe bit is clear → first bit short-circuits
        let k = f.hash_count();
        let (probes, ans) = may_contain_counted(&f, b"absent-key");
        assert_eq!(ans, f.may_contain(b"absent-key"));
        let k_us = k as usize;
        assert!(probes <= k_us, "probes {probes} > k {k}");
        assert_eq!(probes, 1, "empty filter clears the first probed bit");
        // insert keys: each written key must pass the FULL k probes
        for n in 0..n_keys {
            f.insert(format!("key-{n}").as_bytes());
        }
        for n in 0..n_keys {
            let (probes, ans) = may_contain_counted(&f, format!("key-{n}").as_bytes());
            assert_eq!(ans, f.may_contain(format!("key-{n}").as_bytes()));
            assert!(ans, "written key false-negative (twin loop broken?)");
            assert_eq!(probes, k as usize, "written key pays exactly k probes");
        }
    }
}

/// Absent keys pay at most k (the registered bound), usually less.
#[test]
fn absent_keys_within_registered_bound() {
    let mut f = BloomFilter::with_capacity(500, 12);
    for n in 0..500 {
        f.insert(format!("present-{n}").as_bytes());
    }
    let k = f.hash_count() as usize;
    // deterministic ragged schedule (LCG) over absent keys
    let mut state = 0x853_c49e_674_8fe_a9bu64;
    let mut checked = 0;
    let mut short_circuits = 0;
    for _ in 0..500 {
        state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let key = format!("absent-{}", state % 100_000);
        let (probes, ans) = may_contain_counted(&f, key.as_bytes());
        assert_eq!(ans, f.may_contain(key.as_bytes()), "twin disagrees with real fn");
        assert!(probes <= k, "probes {probes} exceed registered bound k={k}");
        if probes < k {
            short_circuits += 1;
        }
        checked += 1;
    }
    assert_eq!(checked, 500);
    // with 500 present keys and 12 bits/key, most absent keys clear an
    // early bit: the short-circuit bridge is exercised, not just claimed
    assert!(short_circuits > 400, "expected mostly short-circuits, got {short_circuits}");
}

/// The always-true (inactive) filter pays ZERO probes — no hidden
/// per-query term outside the loop.
#[test]
fn inactive_filter_pays_zero_probes() {
    let f = BloomFilter::always_true();
    assert!(!f.is_active());
    let (probes, ans) = may_contain_counted(&f, b"any");
    assert_eq!((probes, ans), (0, true));
    assert!(f.may_contain(b"any"));
}
