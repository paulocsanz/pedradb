//! RFC-0289: Separator Order Preservation and Strict Weak Ordering Oracle Kernel.
//!
//! Enforces mathematical order preservation when generating shortest index separators
//! between adjacent SST blocks under arbitrary abstract comparators: A <= Separator(A, B) < B,
//! as well as short successors for terminating blocks: A <= Successor(A).

use std::cmp::Ordering;

/// Erro de violação de invariantes de ordenação e limites para separadores SST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeparatorOrderError {
    /// Os limites fornecidos estão invertidos ou são idênticos (requer a < b sob o comparador).
    InvertedOrEqualBounds,
    /// Chave de limite vazia onde uma chave não-vazia é mandatória.
    EmptyBoundKey,
    /// Separador gerado viola a ordenação fraca estrita (não satisfaz a <= s < b).
    OrderViolation { separator: Vec<u8> },
}

impl std::fmt::Display for SeparatorOrderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvertedOrEqualBounds => {
                write!(f, "bounds are inverted or equal: lower bound must be strictly less than upper bound")
            }
            Self::EmptyBoundKey => write!(f, "bound key cannot be empty"),
            Self::OrderViolation { separator } => {
                write!(f, "separator {:?} violates ordering invariant a <= s < b", separator)
            }
        }
    }
}

impl std::error::Error for SeparatorOrderError {}

/// Outcome of a separator computation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeparatorOutcome {
    /// The resulting separator key to be stored in the index block.
    pub separator_key: Vec<u8>,
    /// Whether the key was successfully shortened relative to `a`.
    pub was_truncated: bool,
}

impl SeparatorOutcome {
    /// Cria um resultado de separador validado garantindo que a chave não é vazia.
    pub fn try_new(separator_key: Vec<u8>, was_truncated: bool) -> Result<Self, SeparatorOrderError> {
        if separator_key.is_empty() {
            return Err(SeparatorOrderError::EmptyBoundKey);
        }
        Ok(Self {
            separator_key,
            was_truncated,
        })
    }
}

/// Oracle responsible for calculating and verifying index separators.
pub struct SeparatorOrderOracle;

impl SeparatorOrderOracle {
    /// Computes the shortest separator between `a` and `b` under comparator `cmp` with strict validation.
    pub fn try_find_shortest_separator<F>(
        a: &[u8],
        b: &[u8],
        cmp: &F,
    ) -> Result<SeparatorOutcome, SeparatorOrderError>
    where
        F: Fn(&[u8], &[u8]) -> Ordering,
    {
        if a.is_empty() || b.is_empty() {
            return Err(SeparatorOrderError::EmptyBoundKey);
        }
        if cmp(a, b) != Ordering::Less {
            return Err(SeparatorOrderError::InvertedOrEqualBounds);
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
                    return Ok(SeparatorOutcome {
                        separator_key: candidate,
                        was_truncated: true,
                    });
                }
            }
        }

        // Fallback: If truncation cannot safely satisfy comparator invariants, return A as-is
        Ok(SeparatorOutcome {
            separator_key: a.to_vec(),
            was_truncated: false,
        })
    }

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
        Self::try_find_shortest_separator(a, b, cmp).unwrap_or_else(|_| SeparatorOutcome {
            separator_key: a.to_vec(),
            was_truncated: false,
        })
    }

    /// Computes the shortest successor of `key` under comparator `cmp` with strict validation.
    pub fn try_find_short_successor<F>(
        key: &[u8],
        cmp: &F,
    ) -> Result<SeparatorOutcome, SeparatorOrderError>
    where
        F: Fn(&[u8], &[u8]) -> Ordering,
    {
        if key.is_empty() {
            return Err(SeparatorOrderError::EmptyBoundKey);
        }

        for i in 0..key.len() {
            if key[i] < 0xff {
                let mut candidate = key[..=i].to_vec();
                candidate[i] += 1;

                // Must be shorter than original key and satisfy comparator ordering
                if candidate.len() < key.len() && cmp(key, &candidate) != Ordering::Greater {
                    return Ok(SeparatorOutcome {
                        separator_key: candidate,
                        was_truncated: true,
                    });
                }
            }
        }

        Ok(SeparatorOutcome {
            separator_key: key.to_vec(),
            was_truncated: false,
        })
    }

    /// Computes the shortest successor of `key` under comparator `cmp`.
    pub fn find_short_successor<F>(
        key: &[u8],
        cmp: &F,
    ) -> SeparatorOutcome
    where
        F: Fn(&[u8], &[u8]) -> Ordering,
    {
        Self::try_find_short_successor(key, cmp).unwrap_or_else(|_| SeparatorOutcome {
            separator_key: key.to_vec(),
            was_truncated: false,
        })
    }

    /// Verifies whether a separator `s` satisfies strict ordering bounds between `a` and `b` with error propagation.
    pub fn try_verify_bounds<F>(
        a: &[u8],
        b: &[u8],
        s: &[u8],
        cmp: &F,
    ) -> Result<bool, SeparatorOrderError>
    where
        F: Fn(&[u8], &[u8]) -> Ordering,
    {
        if a.is_empty() || b.is_empty() || s.is_empty() {
            return Err(SeparatorOrderError::EmptyBoundKey);
        }
        if cmp(a, b) != Ordering::Less {
            return Err(SeparatorOrderError::InvertedOrEqualBounds);
        }
        let a_le_s = cmp(a, s) != Ordering::Greater;
        let s_lt_b = cmp(s, b) == Ordering::Less;
        Ok(a_le_s && s_lt_b)
    }

    /// Verifies whether a separator `s` satisfies strict ordering bounds between `a` and `b`.
    pub fn verify_bounds<F>(a: &[u8], b: &[u8], s: &[u8], cmp: &F) -> bool
    where
        F: Fn(&[u8], &[u8]) -> Ordering,
    {
        Self::try_verify_bounds(a, b, s, cmp).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_separator_order_structural_invariants_red_to_green() {
        let cmp = |x: &[u8], y: &[u8]| x.cmp(y);

        // 1. Valid separator truncation
        let a = b"abcdefgh";
        let b = b"abcfxxxx";
        let outcome = SeparatorOrderOracle::try_find_shortest_separator(a, b, &cmp)
            .expect("valid separator search");
        assert!(outcome.was_truncated);
        assert!(outcome.separator_key.len() < a.len());
        assert!(SeparatorOrderOracle::verify_bounds(a, b, &outcome.separator_key, &cmp));

        // 2. Reject empty keys
        let err_empty_a = SeparatorOrderOracle::try_find_shortest_separator(b"", b, &cmp);
        assert_eq!(err_empty_a, Err(SeparatorOrderError::EmptyBoundKey));

        let err_empty_b = SeparatorOrderOracle::try_find_shortest_separator(a, b"", &cmp);
        assert_eq!(err_empty_b, Err(SeparatorOrderError::EmptyBoundKey));

        // 3. Reject inverted or equal bounds
        let err_equal = SeparatorOrderOracle::try_find_shortest_separator(a, a, &cmp);
        assert_eq!(err_equal, Err(SeparatorOrderError::InvertedOrEqualBounds));

        let err_inverted = SeparatorOrderOracle::try_find_shortest_separator(b, a, &cmp);
        assert_eq!(err_inverted, Err(SeparatorOrderError::InvertedOrEqualBounds));

        // 4. Successor computation
        let succ_outcome = SeparatorOrderOracle::try_find_short_successor(b"abcd", &cmp)
            .expect("valid successor");
        assert!(cmp(b"abcd", &succ_outcome.separator_key) != Ordering::Greater);

        let err_succ_empty = SeparatorOrderOracle::try_find_short_successor(b"", &cmp);
        assert_eq!(err_succ_empty, Err(SeparatorOrderError::EmptyBoundKey));

        // 5. Try verify bounds with errors
        let err_verify_equal = SeparatorOrderOracle::try_verify_bounds(a, a, a, &cmp);
        assert_eq!(err_verify_equal, Err(SeparatorOrderError::InvertedOrEqualBounds));

        // 6. Display & Error implementations
        let d = format!("{}", SeparatorOrderError::InvertedOrEqualBounds);
        assert!(d.contains("bounds are inverted or equal"));
    }
}
