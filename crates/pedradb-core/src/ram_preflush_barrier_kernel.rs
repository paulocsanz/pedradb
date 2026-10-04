//! RFC-0280 P0.2 — RAM Pre-Flush Barrier & DRAM Bit-Rot Protection Kernel.
//!
//! Enforces an inductive verification barrier immediately before MemTable serialization.
//! Verifies strict key comparator monotonicity and payload checksums, intercepting
//! volatile DRAM bit-rot or concurrent corruption before it can poison physical SSTable files on disk.

#![forbid(unsafe_code)]

/// An entry staged in volatile MemTable memory prior to disk flush.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedFlushEntry {
    /// User key.
    pub key: Vec<u8>,
    /// Commit sequence number.
    pub seq: u64,
    /// Value payload.
    pub value: Vec<u8>,
    /// In-memory payload checksum.
    pub in_memory_crc: u32,
}

impl StagedFlushEntry {
    /// Creates a staged entry safely validating non-empty key and non-zero seq.
    pub fn try_new(key: Vec<u8>, seq: u64, value: Vec<u8>, crc_fn: impl Fn(&[u8]) -> u32) -> Result<Self, PreFlushBarrierResult> {
        if key.is_empty() {
            return Err(PreFlushBarrierResult::EmptyKey { index: 0 });
        }
        if seq == 0 {
            return Err(PreFlushBarrierResult::ZeroSequenceNumber { index: 0 });
        }
        let in_memory_crc = crc_fn(&value);
        Ok(Self {
            key,
            seq,
            value,
            in_memory_crc,
        })
    }

    /// Creates a staged entry with computed in-memory CRC.
    pub fn new(key: Vec<u8>, seq: u64, value: Vec<u8>, crc_fn: impl Fn(&[u8]) -> u32) -> Self {
        let in_memory_crc = crc_fn(&value);
        Self {
            key,
            seq,
            value,
            in_memory_crc,
        }
    }
}

/// Result of the pre-flush integrity barrier check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreFlushBarrierResult {
    /// All entries are strictly ordered and uncorrupted: safe to serialize SSTable.
    Pass,
    /// Bit-rot or corrupt comparator order detected: key order inverted at given index.
    KeyOrderInversion {
        /// Index where inversion was detected.
        index: usize,
    },
    /// Bit-rot detected in payload: in-memory checksum mismatch at given index.
    PayloadChecksumCorrupted {
        /// Index where checksum mismatch was detected.
        index: usize,
    },
    EmptyKey {
        index: usize,
    },
    ZeroSequenceNumber {
        index: usize,
    },
    EmptyEntries,
}

impl std::fmt::Display for PreFlushBarrierResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pass => write!(f, "Pre-flush barrier check passed"),
            Self::KeyOrderInversion { index } => write!(f, "Key order inversion at index {index}"),
            Self::PayloadChecksumCorrupted { index } => write!(f, "Payload checksum corrupted at index {index}"),
            Self::EmptyKey { index } => write!(f, "Empty key at index {index}"),
            Self::ZeroSequenceNumber { index } => write!(f, "Zero sequence number at index {index}"),
            Self::EmptyEntries => write!(f, "Entries list is empty"),
        }
    }
}

impl std::error::Error for PreFlushBarrierResult {}

/// Verifies the Pre-Flush Invariant:
/// 1. Monotonic Ordering: $\forall i: \text{Key}_i < \text{Key}_{i+1} \lor (\text{Key}_i == \text{Key}_{i+1} \land \text{Seq}_i > \text{Seq}_{i+1})$.
/// 2. Payload Integrity: $\forall i: \text{CRC}(\text{Value}_i) == \text{in\_memory\_crc}_i$.
pub fn verify_preflush_barrier(
    entries: &[StagedFlushEntry],
    crc_fn: impl Fn(&[u8]) -> u32,
) -> PreFlushBarrierResult {
    if entries.is_empty() {
        return PreFlushBarrierResult::Pass;
    }

    for i in 0..entries.len() {
        if entries[i].key.is_empty() {
            return PreFlushBarrierResult::EmptyKey { index: i };
        }
        if entries[i].seq == 0 {
            return PreFlushBarrierResult::ZeroSequenceNumber { index: i };
        }

        // 1. Verify in-memory payload integrity
        let computed_crc = crc_fn(&entries[i].value);
        if computed_crc != entries[i].in_memory_crc {
            return PreFlushBarrierResult::PayloadChecksumCorrupted { index: i };
        }

        // 2. Verify strict key comparator monotonicity against adjacent element
        if i + 1 < entries.len() {
            let curr = &entries[i];
            let next = &entries[i + 1];

            if curr.key > next.key {
                return PreFlushBarrierResult::KeyOrderInversion { index: i };
            }
            if curr.key == next.key && curr.seq <= next.seq {
                // Same key must be ordered with newest sequence number first
                return PreFlushBarrierResult::KeyOrderInversion { index: i };
            }
        }
    }

    PreFlushBarrierResult::Pass
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ram_preflush_barrier_invariants_red_to_green() {
        let crc_fn = |v: &[u8]| crc32c::crc32c(v);

        assert_eq!(
            StagedFlushEntry::try_new(vec![], 1, vec![1], crc_fn),
            Err(PreFlushBarrierResult::EmptyKey { index: 0 })
        );

        assert_eq!(
            StagedFlushEntry::try_new(vec![1], 0, vec![1], crc_fn),
            Err(PreFlushBarrierResult::ZeroSequenceNumber { index: 0 })
        );

        let entry = StagedFlushEntry::try_new(vec![1], 1, vec![1], crc_fn).unwrap();
        assert_eq!(verify_preflush_barrier(&[entry], crc_fn), PreFlushBarrierResult::Pass);

        let bad_entry = StagedFlushEntry {
            key: vec![],
            seq: 1,
            value: vec![1],
            in_memory_crc: crc_fn(&[1]),
        };
        assert_eq!(
            verify_preflush_barrier(&[bad_entry], crc_fn),
            PreFlushBarrierResult::EmptyKey { index: 0 }
        );

        let disp = format!("{}", PreFlushBarrierResult::EmptyEntries);
        assert!(!disp.is_empty());
    }
}
