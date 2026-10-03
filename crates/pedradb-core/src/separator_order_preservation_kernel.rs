//! RFC-0289: Separator Order Preservation and Strict Weak Ordering Oracle Kernel.
//!
//! Enforces mathematical order preservation when generating shortest index separators
//! between adjacent SST blocks under arbitrary abstract comparators: A <= Separator(A, B) < B,
//! as well as short successors for terminating blocks: A <= Successor(A).

use std::cmp::Ordering;

/// Outcome of a separator computation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeparatorOutcome {
    /// The resulting separator key to be stored in the index block.
    pub separator_key: Vec<u8>,
    /// Whether the key was successfully shortened relative to `a`.
    pub was_truncated: bool,
}

/// Oracle responsible for calculating and verifying index separators.
pub struct SeparatorOrderOracle;

impl SeparatorOrderOracle {
    /// Computes the shortest separator between `a` and `b` under comparator `cmp`.
    ///
    /// Invariant: `cmp(a, &separator) != Ordering::Greater` and `cmp(&separator, b) == Ordering::Less`.
    /// If shortening `a` produces a key that violates the comparator's ordering or does not reduce
    /// byte size, the oracle falls back to returning `a.to_vec()` unmodified.
    pub fn find_shortest_separator<F>(
        a: &[u8],
        b: &[u8],
        cmp: &F,
    ) -> SeparatorOutcome
    where
        F: Fn(&[u8], &[u8]) -> Ordering,
    {
        // Pre-condition check: a must be strictly less than b
        if cmp(a, b) != Ordering::Less {
            return SeparatorOutcome {
                separator_key: a.to_vec(),
                was_truncated: false,
            };
        }

        // Find the first byte of divergence
        let min_len = a.len().min(b.len());
        let mut diff_idx = 0;
        while diff_idx < min_len && a[diff_idx] == b[diff_idx] {
            diff_idx += 1;
        }

        if diff_idx < min_len {
            let diff_byte = a[diff_idx];
            // Case 1: diff_byte + 1 < b[diff_idx]
            // Case 2: diff_byte + 1 == b[diff_idx] AND diff_idx + 1 < b.len() (candidate is a strict prefix of b)
            let can_increment = if diff_byte < 0xff {
                if diff_byte + 1 < b[diff_idx] {
                    true
                } else {
                    diff_byte + 1 == b[diff_idx] && diff_idx + 1 < b.len()
                }
            } else {
                false
            };

            if can_increment {
                let mut candidate = a[..=diff_idx].to_vec();
                candidate[diff_idx] += 1;

                // Validate that candidate is strictly shorter than a,
                // and satisfies the strict weak order invariant: A <= Candidate < B
                if candidate.len() < a.len()
                    && cmp(a, &candidate) != Ordering::Greater
                    && cmp(&candidate, b) == Ordering::Less
                {
                    return SeparatorOutcome {
                        separator_key: candidate,
                        was_truncated: true,
                    };
                }
            }
        }

        // Fallback: If truncation cannot safely satisfy comparator invariants, return A as-is
        SeparatorOutcome {
            separator_key: a.to_vec(),
            was_truncated: false,
        }
    }

    /// Computes the shortest successor of `key` under comparator `cmp`.
    ///
    /// Invariant: `cmp(key, &successor) != Ordering::Greater`.
    /// If shortening `key` produces a key that violates the comparator's ordering,
    /// or if no byte can be incremented to yield a shorter key, the oracle falls back
    /// to returning `key.to_vec()` unmodified.
    pub fn find_short_successor<F>(
        key: &[u8],
        cmp: &F,
    ) -> SeparatorOutcome
    where
        F: Fn(&[u8], &[u8]) -> Ordering,
    {
        for i in 0..key.len() {
            if key[i] < 0xff {
                let mut candidate = key[..=i].to_vec();
                candidate[i] += 1;

                // Must be shorter than original key and satisfy comparator ordering
                if candidate.len() < key.len() && cmp(key, &candidate) != Ordering::Greater {
                    return SeparatorOutcome {
                        separator_key: candidate,
                        was_truncated: true,
                    };
                }
            }
        }

        SeparatorOutcome {
            separator_key: key.to_vec(),
            was_truncated: false,
        }
    }

    /// Verifies whether a separator `s` satisfies strict ordering bounds between `a` and `b`.
    pub fn verify_bounds<F>(a: &[u8], b: &[u8], s: &[u8], cmp: &F) -> bool
    where
        F: Fn(&[u8], &[u8]) -> Ordering,
    {
        let a_le_s = cmp(a, s) != Ordering::Greater;
        let s_lt_b = cmp(s, b) == Ordering::Less;
        a_le_s && s_lt_b
    }
}
