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

/// Verification oracle proving semilattice compliance for merge operators.
pub struct MergeSemilatticeOracle;

impl MergeSemilatticeOracle {
    /// Verifies associativity, commutativity, and idempotence on test inputs.
    pub fn verify_axioms<Op: MergeSemilatticeOperator>(
        op: &Op,
        samples: &[&[u8]],
    ) -> Result<(), &'static str> {
        for &a in samples {
            // Idempotence: merge(a, a) == a
            let idemp = op.merge(a, a);
            let a_norm = if a.is_empty() { op.bottom() } else { a.to_vec() };
            if idemp != a_norm && !a.is_empty() {
                return Err("Idempotence axiom violated: merge(a, a) != a");
            }

            for &b in samples {
                // Commutativity: merge(a, b) == merge(b, a)
                if op.merge(a, b) != op.merge(b, a) {
                    return Err("Commutativity axiom violated: merge(a, b) != merge(b, a)");
                }

                for &c in samples {
                    // Associativity: merge(merge(a, b), c) == merge(a, merge(b, c))
                    let lhs = op.merge(&op.merge(a, b), c);
                    let rhs = op.merge(a, &op.merge(b, c));
                    if lhs != rhs {
                        return Err("Associativity axiom violated: (a ⊔ b) ⊔ c != a ⊔ (b ⊔ c)");
                    }
                }
            }
        }
        Ok(())
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
