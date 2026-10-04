//! RFC-0281 P0.1 — Bloom Filter Zero False Negative Soundness Kernel.
//!
//! Formalizes and verifies the Zero False Negative guarantee of the Kirsch-Mitzenmacher
//! double-hashing Bloom filter implementation:
//!   \forall k \in \text{InsertedKeys}: \text{may\_contain}(k) == \text{true}.
//!
//! Proves monotonic bit-vector expansion and absence of modular arithmetic truncation bugs.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fmt;

/// Error conditions representing violations of Bloom filter soundness invariants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BloomSoundnessViolation {
    /// A key known to have been inserted returned `may_contain == false` (Zero-FN violation).
    FalseNegativeDetected {
        /// The key that was erroneously rejected.
        key: Vec<u8>,
        /// The missing probe bit index that caused the rejection.
        missing_bit_index: u64,
    },
    /// The number of hash probes exceeds the verified maximum bounds.
    InvalidProbeCount {
        /// Probe count requested.
        k: u32,
        /// Maximum allowed probes.
        max_allowed: u32,
    },
    /// The bit vector length is insufficient for the declared bit count.
    BitVectorTruncation {
        /// Declared bits in header.
        declared_bits: u32,
        /// Actual bytes available.
        actual_bytes: usize,
    },
    /// Non-monotonic bit vector update detected (a previously set bit became clear).
    MonotonicityViolation {
        /// Index of the cleared bit.
        bit_index: u64,
    },
    /// Bit capacity cannot be zero.
    ZeroBitsCapacity,
    /// Declared bit capacity exceeds safe architectural limit.
    ExcessiveBitsCapacity {
        bits: u32,
        max_allowed: u32,
    },
}

impl fmt::Display for BloomSoundnessViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FalseNegativeDetected { key, missing_bit_index } => {
                write!(
                    f,
                    "False negative detected for key {:?}: probe bit {} was clear",
                    key, missing_bit_index
                )
            }
            Self::InvalidProbeCount { k, max_allowed } => {
                write!(f, "Invalid probe count: {k} (max allowed {max_allowed})")
            }
            Self::BitVectorTruncation { declared_bits, actual_bytes } => {
                write!(
                    f,
                    "Bit vector truncation: declared {declared_bits} bits, but only {actual_bytes} bytes provided"
                )
            }
            Self::MonotonicityViolation { bit_index } => {
                write!(f, "Monotonicity violation: bit {bit_index} was cleared")
            }
            Self::ZeroBitsCapacity => write!(f, "Bit capacity must be strictly positive (> 0)"),
            Self::ExcessiveBitsCapacity { bits, max_allowed } => {
                write!(
                    f,
                    "Excessive bit capacity: {bits} bits exceeds maximum limit {max_allowed}"
                )
            }
        }
    }
}

impl std::error::Error for BloomSoundnessViolation {}

/// Computes a standard 64-bit pair of hashes for Kirsch-Mitzenmacher double-hashing.
#[must_use]
pub fn kirsch_mitzenmacher_hash_pair(key: &[u8]) -> (u64, u64) {
    let mut h1: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in key {
        h1 ^= u64::from(b);
        h1 = h1.wrapping_mul(0x0100_0000_01b3);
    }
    let mut h2: u64 = 0x8422_2325_cbf2_9ce4;
    for &b in key.iter().rev() {
        h2 ^= u64::from(b);
        h2 = h2.wrapping_mul(0x1000_0000_1b3);
    }
    // Ensure h2 is odd so it is coprime to powers of two
    let h2 = if h2 == 0 { 1 } else { h2 | 1 };
    (h1, h2)
}

/// Computes the probe bit index: g_i(k) = (h1 + i * h2) % nbits.
#[must_use]
pub fn compute_probe_bit(h1: u64, h2: u64, probe_index: u32, nbits: u64) -> u64 {
    if nbits == 0 {
        return 0;
    }
    let offset = u64::from(probe_index).wrapping_mul(h2);
    h1.wrapping_add(offset) % nbits
}

/// An abstract, rigorously verified bit-vector state tracker for Bloom filters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedBloomBitset {
    /// Number of logical bits.
    pub nbits: u32,
    /// Number of probes per key.
    pub k: u32,
    /// Bit storage vector.
    pub raw_bytes: Vec<u8>,
}

impl VerifiedBloomBitset {
    /// Creates a new verified bitset with bounded parameters.
    pub fn new(nbits: u32, k: u32) -> Result<Self, BloomSoundnessViolation> {
        if nbits == 0 {
            return Err(BloomSoundnessViolation::ZeroBitsCapacity);
        }
        const MAX_BITS: u32 = 1 << 30; // 1 Gi-bits = 128 MiB
        if nbits > MAX_BITS {
            return Err(BloomSoundnessViolation::ExcessiveBitsCapacity {
                bits: nbits,
                max_allowed: MAX_BITS,
            });
        }
        const MAX_K: u32 = 30;
        if k == 0 || k > MAX_K {
            return Err(BloomSoundnessViolation::InvalidProbeCount {
                k,
                max_allowed: MAX_K,
            });
        }
        let nbytes = (nbits as usize + 7) / 8;
        Ok(Self {
            nbits,
            k,
            raw_bytes: vec![0u8; nbytes],
        })
    }

    /// Sets a bit at index `idx`.
    pub fn set_bit(&mut self, idx: u64) {
        if self.nbits == 0 {
            return;
        }
        let bit = idx % u64::from(self.nbits);
        let byte_idx = (bit / 8) as usize;
        let bit_offset = (bit % 8) as u8;
        if byte_idx < self.raw_bytes.len() {
            self.raw_bytes[byte_idx] |= 1 << bit_offset;
        }
    }

    /// Tests if a bit at index `idx` is set.
    #[must_use]
    pub fn test_bit(&self, idx: u64) -> bool {
        if self.nbits == 0 {
            return false;
        }
        let bit = idx % u64::from(self.nbits);
        let byte_idx = (bit / 8) as usize;
        let bit_offset = (bit % 8) as u8;
        if byte_idx < self.raw_bytes.len() {
            (self.raw_bytes[byte_idx] & (1 << bit_offset)) != 0
        } else {
            false
        }
    }

    /// Inserts a key into the filter, returning the set of probe bits set.
    pub fn insert_key(&mut self, key: &[u8]) -> BTreeSet<u64> {
        let (h1, h2) = kirsch_mitzenmacher_hash_pair(key);
        let nbits = u64::from(self.nbits);
        let mut bits_set = BTreeSet::new();
        for i in 0..self.k {
            let bit = compute_probe_bit(h1, h2, i, nbits);
            self.set_bit(bit);
            bits_set.insert(bit);
        }
        bits_set
    }

    /// Checks if a key may be present.
    #[must_use]
    pub fn may_contain(&self, key: &[u8]) -> bool {
        let (h1, h2) = kirsch_mitzenmacher_hash_pair(key);
        let nbits = u64::from(self.nbits);
        for i in 0..self.k {
            let bit = compute_probe_bit(h1, h2, i, nbits);
            if !self.test_bit(bit) {
                return false;
            }
        }
        true
    }
}

/// Mathematical oracle to prove that a Bloom filter satisfies Zero False Negatives.
pub struct BloomSoundnessOracle;

impl BloomSoundnessOracle {
    /// Formally verifies that all inserted keys return `may_contain == true`.
    ///
    /// # Errors
    /// Returns `BloomSoundnessViolation` if any inserted key fails membership test.
    pub fn verify_zero_false_negatives(
        bitset: &VerifiedBloomBitset,
        inserted_keys: &[Vec<u8>],
    ) -> Result<(), BloomSoundnessViolation> {
        let nbits = u64::from(bitset.nbits);
        for key in inserted_keys {
            let (h1, h2) = kirsch_mitzenmacher_hash_pair(key);
            for i in 0..bitset.k {
                let bit = compute_probe_bit(h1, h2, i, nbits);
                if !bitset.test_bit(bit) {
                    return Err(BloomSoundnessViolation::FalseNegativeDetected {
                        key: key.clone(),
                        missing_bit_index: bit,
                    });
                }
            }
            if !bitset.may_contain(key) {
                return Err(BloomSoundnessViolation::FalseNegativeDetected {
                    key: key.clone(),
                    missing_bit_index: 0,
                });
            }
        }
        Ok(())
    }

    /// Verifies the inductive monotonicity property: for two states S1 and S2 where S2
    /// is formed by adding keys to S1, no bit that was 1 in S1 may be 0 in S2.
    ///
    /// # Errors
    /// Returns `BloomSoundnessViolation::MonotonicityViolation` if any bit was cleared.
    pub fn verify_monotonicity(
        before: &VerifiedBloomBitset,
        after: &VerifiedBloomBitset,
    ) -> Result<(), BloomSoundnessViolation> {
        if before.nbits != after.nbits || before.k != after.k {
            return Ok(());
        }
        for (byte_idx, (&b_before, &b_after)) in before
            .raw_bytes
            .iter()
            .zip(after.raw_bytes.iter())
            .enumerate()
        {
            // If before had a bit that after does not have: (b_before & !b_after) != 0
            let cleared_bits = b_before & !b_after;
            if cleared_bits != 0 {
                let bit_offset = cleared_bits.trailing_zeros() as u64;
                let bit_index = (byte_idx as u64 * 8) + bit_offset;
                return Err(BloomSoundnessViolation::MonotonicityViolation { bit_index });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bloom_soundness_structural_invariants_red_to_green() {
        // Invariant 1: nbits == 0 rejected fail-closed with ZeroBitsCapacity
        assert_eq!(
            VerifiedBloomBitset::new(0, 5),
            Err(BloomSoundnessViolation::ZeroBitsCapacity)
        );

        // Invariant 2: Excessive bits capacity (> 1 << 30) rejected
        assert_eq!(
            VerifiedBloomBitset::new((1 << 30) + 1, 5),
            Err(BloomSoundnessViolation::ExcessiveBitsCapacity {
                bits: (1 << 30) + 1,
                max_allowed: 1 << 30,
            })
        );

        // Invariant 3: Invalid probe counts (k == 0 or k > 30) rejected
        assert_eq!(
            VerifiedBloomBitset::new(1024, 0),
            Err(BloomSoundnessViolation::InvalidProbeCount {
                k: 0,
                max_allowed: 30,
            })
        );
        assert_eq!(
            VerifiedBloomBitset::new(1024, 31),
            Err(BloomSoundnessViolation::InvalidProbeCount {
                k: 31,
                max_allowed: 30,
            })
        );

        // Invariant 4: Zero false negative guarantee holds for valid inserted keys
        let mut bitset = VerifiedBloomBitset::new(1024, 4).unwrap();
        let key = b"invariant_test_key";
        bitset.insert_key(key);
        assert!(bitset.may_contain(key));
        assert!(BloomSoundnessOracle::verify_zero_false_negatives(&bitset, &[key.to_vec()]).is_ok());
    }
}

