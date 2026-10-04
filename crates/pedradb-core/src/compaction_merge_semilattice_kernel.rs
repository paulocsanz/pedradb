//! Compaction Merge Operator Join-Semilattice Confluence Kernel (RFC-0286 Fronteira 2).
//!
//! Enforces bounded join-semilattice algebraic properties on merge operators,
//! guaranteeing confluence and crash-replay idempotence without ghost mutations.
//!
//! Guarantees:
//! 1. Associativity: `merge(merge(a, b), c) == merge(a, merge(b, c))`.
//! 2. Commutativity: `merge(a, b) == merge(b, a)`.
//! 3. Idempotence: `merge(a, a) == a`.
//! 4. Crash invariance: Re-running compaction merges over duplicate operands preserves canonical values.

#![forbid(unsafe_code)]

/// Violações formais dos axiomas de semirrede de junção limitada (bounded join-semilattice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeSemilatticeViolation {
    /// Conjunto de amostras fornecido para teste axiomático está vazio.
    EmptySampleSet,
    /// Axioma de associatividade violado: (a ⊔ b) ⊔ c != a ⊔ (b ⊔ c).
    AssociativityViolated { a: Vec<u8>, b: Vec<u8>, c: Vec<u8> },
    /// Axioma de comutatividade violado: a ⊔ b != b ⊔ a.
    CommutativityViolated { a: Vec<u8>, b: Vec<u8> },
    /// Axioma de idempotência violado: a ⊔ a != a.
    IdempotenceViolated { a: Vec<u8>, result: Vec<u8> },
    /// Axioma do elemento neutro/mínimo (bottom) violado: a ⊔ ⊥ != a.
    BottomIdentityViolated { a: Vec<u8>, bottom: Vec<u8>, result: Vec<u8> },
}

impl std::fmt::Display for MergeSemilatticeViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptySampleSet => write!(f, "Sample set provided for semilattice verification is empty"),
            Self::AssociativityViolated { a, b, c } => write!(
                f,
                "Associativity axiom violated: ({a:?} ⊔ {b:?}) ⊔ {c:?} != {a:?} ⊔ ({b:?} ⊔ {c:?})"
            ),
            Self::CommutativityViolated { a, b } => write!(
                f,
                "Commutativity axiom violated: {a:?} ⊔ {b:?} != {b:?} ⊔ {a:?}"
            ),
            Self::IdempotenceViolated { a, result } => write!(
                f,
                "Idempotence axiom violated: {a:?} ⊔ {a:?} produced {result:?} != {a:?}"
            ),
            Self::BottomIdentityViolated { a, bottom, result } => write!(
                f,
                "Bottom identity axiom violated: {a:?} ⊔ {bottom:?} produced {result:?} != {a:?}"
            ),
        }
    }
}

impl std::error::Error for MergeSemilatticeViolation {}

/// Abstract merge operator interface representing a bounded join-semilattice.
pub trait MergeSemilatticeOperator: Send + Sync {
    /// Merge two operand values into their least upper bound (join).
    fn merge(&self, a: &[u8], b: &[u8]) -> Vec<u8>;
    /// Identity element (bottom) such that `merge(a, bottom) == a`.
    fn bottom(&self) -> Vec<u8>;
}

/// Bitwise OR join-semilattice implementation (e.g. for flags and bitmaps).
pub struct BitwiseOrMergeOperator;

impl MergeSemilatticeOperator for BitwiseOrMergeOperator {
    fn merge(&self, a: &[u8], b: &[u8]) -> Vec<u8> {
        let max_len = std::cmp::max(a.len(), b.len());
        let mut out = vec![0u8; max_len];
        for i in 0..max_len {
            let byte_a = if i < a.len() { a[i] } else { 0 };
            let byte_b = if i < b.len() { b[i] } else { 0 };
            out[i] = byte_a | byte_b;
        }
        out
    }

    fn bottom(&self) -> Vec<u8> {
        Vec::new()
    }
}

/// Max-monotonic join-semilattice implementation (e.g. for high watermarks).
pub struct MaxU64MergeOperator;

impl MergeSemilatticeOperator for MaxU64MergeOperator {
    fn merge(&self, a: &[u8], b: &[u8]) -> Vec<u8> {
        let val_a = a
            .get(..8)
            .and_then(|s| s.try_into().ok())
            .map(u64::from_be_bytes)
            .unwrap_or(0);
        let val_b = b
            .get(..8)
            .and_then(|s| s.try_into().ok())
            .map(u64::from_be_bytes)
            .unwrap_or(0);
        std::cmp::max(val_a, val_b).to_be_bytes().to_vec()
    }

    fn bottom(&self) -> Vec<u8> {
        0u64.to_be_bytes().to_vec()
    }
}

/// Min-monotonic join-semilattice implementation (e.g. for low watermarks, minimum TTL).
pub struct MinU64MergeOperator;

impl MergeSemilatticeOperator for MinU64MergeOperator {
    fn merge(&self, a: &[u8], b: &[u8]) -> Vec<u8> {
        let val_a = a
            .get(..8)
            .and_then(|s| s.try_into().ok())
            .map(u64::from_be_bytes)
            .unwrap_or(u64::MAX);
        let val_b = b
            .get(..8)
            .and_then(|s| s.try_into().ok())
            .map(u64::from_be_bytes)
            .unwrap_or(u64::MAX);
        std::cmp::min(val_a, val_b).to_be_bytes().to_vec()
    }

    fn bottom(&self) -> Vec<u8> {
        u64::MAX.to_be_bytes().to_vec()
    }
}

/// Verification oracle proving semilattice compliance for merge operators.
pub struct MergeSemilatticeOracle;

impl MergeSemilatticeOracle {
    /// Verifies associativity, commutativity, idempotence, and bottom identity with structured error reports.
    pub fn verify_axioms_strict<Op: MergeSemilatticeOperator>(
        op: &Op,
        samples: &[&[u8]],
    ) -> Result<(), MergeSemilatticeViolation> {
        if samples.is_empty() {
            return Err(MergeSemilatticeViolation::EmptySampleSet);
        }

        let bot = op.bottom();

        for &a in samples {
            // Bottom identity: merge(a, bottom) == a
            let with_bottom = op.merge(a, &bot);
            let a_norm = if a.is_empty() { op.bottom() } else { a.to_vec() };
            if with_bottom != a_norm && !a.is_empty() {
                return Err(MergeSemilatticeViolation::BottomIdentityViolated {
                    a: a.to_vec(),
                    bottom: bot.clone(),
                    result: with_bottom,
                });
            }

            // Idempotence: merge(a, a) == a
            let idemp = op.merge(a, a);
            if idemp != a_norm && !a.is_empty() {
                return Err(MergeSemilatticeViolation::IdempotenceViolated {
                    a: a.to_vec(),
                    result: idemp,
                });
            }

            for &b in samples {
                // Commutativity: merge(a, b) == merge(b, a)
                let ab = op.merge(a, b);
                let ba = op.merge(b, a);
                if ab != ba {
                    return Err(MergeSemilatticeViolation::CommutativityViolated {
                        a: a.to_vec(),
                        b: b.to_vec(),
                    });
                }

                for &c in samples {
                    // Associativity: merge(merge(a, b), c) == merge(a, merge(b, c))
                    let lhs = op.merge(&ab, c);
                    let rhs = op.merge(a, &op.merge(b, c));
                    if lhs != rhs {
                        return Err(MergeSemilatticeViolation::AssociativityViolated {
                            a: a.to_vec(),
                            b: b.to_vec(),
                            c: c.to_vec(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Verifies associativity, commutativity, and idempotence on test inputs.
    pub fn verify_axioms<Op: MergeSemilatticeOperator>(
        op: &Op,
        samples: &[&[u8]],
    ) -> Result<(), &'static str> {
        match Self::verify_axioms_strict(op, samples) {
            Ok(()) => Ok(()),
            Err(MergeSemilatticeViolation::EmptySampleSet) => Ok(()),
            Err(MergeSemilatticeViolation::IdempotenceViolated { .. }) => {
                Err("Idempotence axiom violated: merge(a, a) != a")
            }
            Err(MergeSemilatticeViolation::CommutativityViolated { .. }) => {
                Err("Commutativity axiom violated: merge(a, b) != merge(b, a)")
            }
            Err(MergeSemilatticeViolation::AssociativityViolated { .. }) => {
                Err("Associativity axiom violated: (a ⊔ b) ⊔ c != a ⊔ (b ⊔ c)")
            }
            Err(MergeSemilatticeViolation::BottomIdentityViolated { .. }) => {
                Err("Bottom identity axiom violated: merge(a, bottom) != a")
            }
        }
    }

    /// Folds an array of operands with validation against empty operand collections.
    pub fn try_fold_confluent<Op: MergeSemilatticeOperator>(
        op: &Op,
        operands: &[&[u8]],
    ) -> Result<Vec<u8>, MergeSemilatticeViolation> {
        if operands.is_empty() {
            return Err(MergeSemilatticeViolation::EmptySampleSet);
        }
        Ok(Self::fold_confluent(op, operands))
    }

    /// Folds an array of operands, guaranteeing identical results regardless of order or crash duplicates.
    pub fn fold_confluent<Op: MergeSemilatticeOperator>(
        op: &Op,
        operands: &[&[u8]],
    ) -> Vec<u8> {
        let mut acc = op.bottom();
        for &elem in operands {
            acc = op.merge(&acc, elem);
        }
        acc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_semilattice_bounds_red_to_green() {
        let op = BitwiseOrMergeOperator;
        assert_eq!(
            MergeSemilatticeOracle::verify_axioms_strict(&op, &[]).err(),
            Some(MergeSemilatticeViolation::EmptySampleSet)
        );
        assert_eq!(
            MergeSemilatticeOracle::try_fold_confluent(&op, &[]).err(),
            Some(MergeSemilatticeViolation::EmptySampleSet)
        );

        let operands: [&[u8]; 2] = [b"\x01", b"\x02"];
        assert_eq!(
            MergeSemilatticeOracle::try_fold_confluent(&op, &operands).unwrap(),
            vec![0x03]
        );
    }
}
