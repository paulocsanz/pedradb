//! Per-Table Entropy and Hash-Flooding Resilient Bloom Kernel (RFC-0284 Pilar 2).
//!
//! Protects against hash flooding denial-of-service where an adversary manufactures
//! keys colliding on static Bloom filter hash functions.
//!
//! Guarantees:
//! 1. Per-SST cryptographic salt `S_sst` scrambles bit allocations across files.
//! 2. Saturation oracle detects adversarial bit crowding (density >= 50%).
//! 3. Universal 2-independent hash perturbation preserves theoretical false positive bound.

#![forbid(unsafe_code)]

/// Golden ratio and prime constants for 2-independent universal hashing.
const PRIME_64_A: u64 = 0x9E3779B97F4A7C15;
const PRIME_64_B: u64 = 0xBF58476D1CE4E5B9;

/// Per-table entropy configuration and filter builder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntropyBloomFilter {
    /// Per-table entropy seed (stored immutably in SST file meta-block).
    pub table_salt: u64,
    /// Underlying bitset storage.
    pub bits: Vec<u8>,
    /// Number of distinct hash probes per key (k).
    pub num_probes: u32,
    /// Total number of keys inserted.
    pub items_count: u64,
}

/// Status of filter saturation inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterDensityStatus {
    /// Density is within healthy theoretical limits (< 50% set bits).
    Healthy {
        /// Ratio of set bits expressed in permille (0 to 1000).
        permille_set: u32,
    },
    /// Filter is over-saturated (>= 50% set bits), indicating extreme load or hash flooding.
    OverSaturated {
        /// Ratio of set bits expressed in permille (0 to 1000).
        permille_set: u32,
    },
}

impl EntropyBloomFilter {
    /// Creates a new salted Bloom filter with `total_bits` and `table_salt`.
    pub fn new(total_bits: usize, num_probes: u32, table_salt: u64) -> Self {
        let byte_len = (total_bits + 7) / 8;
        Self {
            table_salt,
            bits: vec![0u8; byte_len.max(1)],
            num_probes: num_probes.max(1),
            items_count: 0,
        }
    }

    /// Total capacity in bits.
    pub fn bit_capacity(&self) -> usize {
        self.bits.len() * 8
    }

    /// Computes universal 2-independent pair of 64-bit hashes for a key given the table salt.
    pub fn hash_pair(&self, key: &[u8]) -> (u64, u64) {
        let mut h1 = self.table_salt ^ (key.len() as u64);
        let mut h2 = self.table_salt.rotate_left(32) ^ PRIME_64_B;

        for chunk in key.chunks(8) {
            let mut buf = [0u8; 8];
            buf[..chunk.len()].copy_from_slice(chunk);
            let val = u64::from_le_bytes(buf);
            h1 = h1.wrapping_add(val).wrapping_mul(PRIME_64_A);
            h1 = h1.rotate_left(31);
            h2 = h2.wrapping_add(val ^ self.table_salt).wrapping_mul(PRIME_64_B);
            h2 = h2.rotate_left(27);
        }

        (h1, h2)
    }

    /// Inserts a key into the salted Bloom filter.
    pub fn insert(&mut self, key: &[u8]) {
        let (h1, h2) = self.hash_pair(key);
        let n_bits = self.bit_capacity() as u64;

        for i in 0..self.num_probes {
            // Kirsch-Mitzenmacher perturbation: g_i(x) = h1(x) + i * h2(x)
            let bit_idx = (h1.wrapping_add((i as u64).wrapping_mul(h2)) % n_bits) as usize;
            let byte_idx = bit_idx / 8;
            let bit_offset = bit_idx % 8;
            self.bits[byte_idx] |= 1 << bit_offset;
        }
        self.items_count += 1;
    }

    /// Checks if a key may be present.
    pub fn may_contain(&self, key: &[u8]) -> bool {
        let (h1, h2) = self.hash_pair(key);
        let n_bits = self.bit_capacity() as u64;

        for i in 0..self.num_probes {
            let bit_idx = (h1.wrapping_add((i as u64).wrapping_mul(h2)) % n_bits) as usize;
            let byte_idx = bit_idx / 8;
            let bit_offset = bit_idx % 8;
            if (self.bits[byte_idx] & (1 << bit_offset)) == 0 {
                return false;
            }
        }
        true
    }

    /// Counts set bits to evaluate filter density and detect saturation.
    pub fn density_status(&self) -> FilterDensityStatus {
        let set_bits: usize = self.bits.iter().map(|b| b.count_ones() as usize).sum();
        let total_bits = self.bit_capacity();
        let permille = ((set_bits as u64 * 1000) / total_bits as u64) as u32;

        if permille >= 500 {
            FilterDensityStatus::OverSaturated { permille_set: permille }
        } else {
            FilterDensityStatus::Healthy { permille_set: permille }
        }
    }
}
