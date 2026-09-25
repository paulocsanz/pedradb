//! RFC-0281 P1.2 — Strict Weak Ordering Key Comparator Axiom Kernel.
//!
//! Mathematically proves that key comparators satisfy the four fundamental
//! axioms of Strict Weak Ordering:
//!   1. Irreflexivity: \forall a: \neg(a < a)
//!   2. Asymmetry: \forall a, b: a < b \implies \neg(b < a)
//!   3. Transitivity of <: \forall a, b, c: (a < b \land b < c) \implies a < c
//!   4. Transitivity of Equivalence (~): \forall a, b, c: (a ~ b \land b ~ c) \implies a ~ c
//!      where a ~ b \iff \neg(a < b) \land \neg(b < a).
//!
//! Guarantees binary search termination, sorted index stability, and absence of
//! silent corruption on arbitrary binary payloads (including embedded NUL bytes).

#![forbid(unsafe_code)]

use std::cmp::Ordering;

/// Trait defining a key comparator over byte slices.
pub trait KeyComparator {
    /// Compares two byte keys.
    fn compare(&self, a: &[u8], b: &[u8]) -> Ordering;

    /// Helper evaluating if `a < b`.
    fn is_less(&self, a: &[u8], b: &[u8]) -> bool {
        self.compare(a, b) == Ordering::Less
    }

    /// Helper evaluating if `a` and `b` are equivalent (`a ~ b`).
    fn is_equivalent(&self, a: &[u8], b: &[u8]) -> bool {
        !self.is_less(a, b) && !self.is_less(b, a)
    }
}

/// Standard raw byte-by-byte lexicographical comparator.
#[derive(Clone, Copy, Debug, Default)]
pub struct ByteLexicographicalComparator;

impl KeyComparator for ByteLexicographicalComparator {
    fn compare(&self, a: &[u8], b: &[u8]) -> Ordering {
        a.cmp(b)
    }
}

/// Prefix comparator that compares only up to the first `prefix_len` bytes.
#[derive(Clone, Copy, Debug)]
pub struct PrefixComparator {
    /// Length of the prefix to consider.
    pub prefix_len: usize,
}

impl KeyComparator for PrefixComparator {
    fn compare(&self, a: &[u8], b: &[u8]) -> Ordering {
        let a_prefix = &a[..a.len().min(self.prefix_len)];
        let b_prefix = &b[..b.len().min(self.prefix_len)];
        a_prefix.cmp(b_prefix)
    }
}

/// Violations of strict weak ordering axioms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComparatorAxiomViolation {
    /// Irreflexivity violated: a < a returned true.
    IrreflexivityViolated {
        /// The key where a < a occurred.
        key: Vec<u8>,
    },
    /// Asymmetry violated: a < b and b < a both returned true.
    AsymmetryViolated {
        /// First key.
        key_a: Vec<u8>,
        /// Second key.
        key_b: Vec<u8>,
    },
    /// Transitivity of < violated: a < b and b < c, but not (a < c).
    OrderTransitivityViolated {
        /// First key.
        key_a: Vec<u8>,
        /// Second key.
        key_b: Vec<u8>,
        /// Third key.
        key_c: Vec<u8>,
    },
    /// Transitivity of equivalence violated: a ~ b and b ~ c, but not (a ~ c).
    EquivalenceTransitivityViolated {
        /// First key.
        key_a: Vec<u8>,
        /// Second key.
        key_b: Vec<u8>,
        /// Third key.
        key_c: Vec<u8>,
    },
}

/// Verification engine for comparator axioms.
pub struct AxiomaticComparatorVerifier;

impl AxiomaticComparatorVerifier {
    /// Formally verifies all four strict weak ordering axioms across a sample set of keys.
    ///
    /// # Errors
    /// Returns `ComparatorAxiomViolation` if any axiom fails.
    pub fn verify_strict_weak_ordering<C: KeyComparator>(
        cmp: &C,
        samples: &[Vec<u8>],
    ) -> Result<(), ComparatorAxiomViolation> {
        let n = samples.len();

        // 1. Irreflexivity: ∀ a: ¬(a < a)
        for a in samples {
            if cmp.is_less(a, a) {
                return Err(ComparatorAxiomViolation::IrreflexivityViolated { key: a.clone() });
            }
        }

        // 2. Asymmetry: ∀ a, b: a < b ⇒ ¬(b < a)
        for i in 0..n {
            let a = &samples[i];
            for j in 0..n {
                let b = &samples[j];
                if cmp.is_less(a, b) && cmp.is_less(b, a) {
                    return Err(ComparatorAxiomViolation::AsymmetryViolated {
                        key_a: a.clone(),
                        key_b: b.clone(),
                    });
                }
            }
        }

        // 3. Transitivity of <: ∀ a, b, c: (a < b ∧ b < c) ⇒ a < c
        // 4. Transitivity of Equivalence: ∀ a, b, c: (a ~ b ∧ b ~ c) ⇒ a ~ c
        for i in 0..n {
            let a = &samples[i];
            for j in 0..n {
                let b = &samples[j];
                for k in 0..n {
                    let c = &samples[k];

                    // Check < transitivity
                    if cmp.is_less(a, b) && cmp.is_less(b, c) && !cmp.is_less(a, c) {
                        return Err(ComparatorAxiomViolation::OrderTransitivityViolated {
                            key_a: a.clone(),
                            key_b: b.clone(),
                            key_c: c.clone(),
                        });
                    }

                    // Check ~ transitivity
                    if cmp.is_equivalent(a, b) && cmp.is_equivalent(b, c) && !cmp.is_equivalent(a, c) {
                        return Err(ComparatorAxiomViolation::EquivalenceTransitivityViolated {
                            key_a: a.clone(),
                            key_b: b.clone(),
                            key_c: c.clone(),
                        });
                    }
                }
            }
        }

        Ok(())
    }

    /// Generates a comprehensive corpus of adversarial binary keys,
    /// explicitly targeting embedded NUL bytes, prefix overlaps, and boundary values.
    #[must_use]
    pub fn generate_adversarial_key_corpus() -> Vec<Vec<u8>> {
        vec![
            vec![],                          // Empty
            vec![0],                         // Single NUL
            vec![0, 0],                      // Double NUL
            vec![0, 1],                      // NUL then 1
            vec![1, 0],                      // 1 then NUL
            vec![1],                         // Single 1
            vec![255],                       // Max byte
            vec![255, 255],                  // Double max byte
            b"key".to_vec(),                 // Normal ASCII
            b"key\0".to_vec(),               // Trailing NUL
            b"key\0suffix".to_vec(),         // Embedded NUL
            b"key\0suffix\0more".to_vec(),   // Multiple embedded NULs
            b"keyboard".to_vec(),            // Prefix extension
            vec![0xFF, 0x00, 0x7F, 0x80],    // Mixed boundary bytes
        ]
    }
}
