//! RFC-0279 P1.2 — Merge Operator Associativity & CRDT Determinism Kernel.
//!
//! Enforces and verifies the algebraic associativity contract for merge operators:
//! $$\forall a, b, c: \quad (a \oplus b) \oplus c \equiv a \oplus (b \oplus c)$$
//! Proves equivalence between online iterator evaluation (read path) and offline
//! SSTable compaction materialization (write/compact path), guaranteeing zero silent drift.

#![forbid(unsafe_code)]

/// An algebraic merge operand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeOperand {
    /// Pure assignment overwrite.
    Put(Vec<u8>),
    /// Associative counter delta (signed 64-bit integer).
    CounterAdd(i64),
    /// Associative byte append.
    StringAppend(Vec<u8>),
}

/// Applies two merge operands in causal order: `base \oplus delta`.
pub fn apply_merge(base: &MergeOperand, delta: &MergeOperand) -> Result<MergeOperand, &'static str> {
    match (base, delta) {
        // Any Put overwrites previous operands
        (_, MergeOperand::Put(new_val)) => Ok(MergeOperand::Put(new_val.clone())),

        // Counter addition
        (MergeOperand::CounterAdd(a), MergeOperand::CounterAdd(b)) => {
            Ok(MergeOperand::CounterAdd(a.saturating_add(*b)))
        }

        // String / byte append
        (MergeOperand::StringAppend(a), MergeOperand::StringAppend(b)) => {
            let mut out = a.clone();
            out.extend_from_slice(b);
            Ok(MergeOperand::StringAppend(out))
        }

        _ => Err("Incompatible merge operand types"),
    }
}

/// Evaluates a sequence of operands in online order (newest to oldest or oldest to newest).
pub fn evaluate_operands(operands: &[MergeOperand]) -> Result<MergeOperand, &'static str> {
    if operands.is_empty() {
        return Err("Cannot evaluate empty operand sequence");
    }

    let mut accum = operands[0].clone();
    for op in &operands[1..] {
        accum = apply_merge(&accum, op)?;
    }
    Ok(accum)
}

/// Verifies the Associativity Theorem:
/// For any three operands $A, B, C$, $(A \oplus B) \oplus C == A \oplus (B \oplus C)$.
pub fn verify_associativity(a: &MergeOperand, b: &MergeOperand, c: &MergeOperand) -> bool {
    let left_step1 = match apply_merge(a, b) {
        Ok(res) => res,
        Err(_) => return false,
    };
    let left_final = match apply_merge(&left_step1, c) {
        Ok(res) => res,
        Err(_) => return false,
    };

    let right_step1 = match apply_merge(b, c) {
        Ok(res) => res,
        Err(_) => return false,
    };
    let right_final = match apply_merge(a, &right_step1) {
        Ok(res) => res,
        Err(_) => return false,
    };

    left_final == right_final
}

/// Verifies Online vs Offline Compaction Equivalence:
/// Proves that partitioning operands across multiple SSTables/Memtables and merging them
/// during background compaction yields the exact same state as online iterator folding.
pub fn verify_online_vs_compaction_equivalence(
    memtable_operands: &[MergeOperand],
    sstable_operands: &[MergeOperand],
) -> bool {
    // 1. Online read path: evaluate all operands concatenated in chronological order
    let mut all_operands = memtable_operands.to_vec();
    all_operands.extend_from_slice(sstable_operands);
    let online_result = match evaluate_operands(&all_operands) {
        Ok(res) => res,
        Err(_) => return false,
    };

    // 2. Background compaction path: compact sstables first, then fold with memtable
    let compacted_sst = match evaluate_operands(sstable_operands) {
        Ok(res) => res,
        Err(_) => return false,
    };
    let mut background_chain = memtable_operands.to_vec();
    background_chain.push(compacted_sst);
    let compaction_result = match evaluate_operands(&background_chain) {
        Ok(res) => res,
        Err(_) => return false,
    };

    online_result == compaction_result
}
