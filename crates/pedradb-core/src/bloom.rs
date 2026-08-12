//! Bloom filter for SST point-lookup negative caching (RocksDB/Pebble class).
//!
//! On-disk SSTs embed a filter so [`crate::sst::SstTable::get`] can skip files
//! that cannot contain a key. False positives are allowed; false negatives are not.

/// Default bits per key (~1% false-positive rate with k≈7).
pub const DEFAULT_BITS_PER_KEY: usize = 10;

/// Double-hash Bloom filter (Kirsch–Mitzenmacher).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BloomFilter {
    bits: Vec<u8>,
    /// Number of bits in the filter.
    nbits: u32,
    /// Number of hash probes per key.
    k: u32,
}

impl BloomFilter {
    /// Empty filter that always returns [`true`] for `may_contain` (no filtering).
    #[must_use]
    pub fn always_true() -> Self {
        Self {
            bits: Vec::new(),
            nbits: 0,
            k: 0,
        }
    }

    /// Whether this filter can reject keys (non-empty).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.nbits > 0 && self.k > 0 && !self.bits.is_empty()
    }

    /// Build a filter sized for approximately `n_keys` insertions.
    #[must_use]
    pub fn with_capacity(n_keys: usize, bits_per_key: usize) -> Self {
        if n_keys == 0 || bits_per_key == 0 {
            return Self::always_true();
        }
        // Cap so nbits always fits in u32 (engine keys per SST are far below this).
        let raw = n_keys.saturating_mul(bits_per_key).max(64);
        let nbits = u32::try_from(raw.min(u32::MAX as usize)).unwrap_or(u32::MAX);
        // k ≈ bits_per_key * ln(2) ≈ bits_per_key * 0.69 — integer form.
        let k = u32::try_from((bits_per_key * 69) / 100)
            .unwrap_or(1)
            .clamp(1, 30);
        let nbytes = (nbits as usize).div_ceil(8);
        Self {
            bits: vec![0u8; nbytes],
            nbits,
            k,
        }
    }

    /// Insert a user key.
    pub fn insert(&mut self, key: &[u8]) {
        if !self.is_active() {
            return;
        }
        let (h1, h2) = hash_pair(key);
        let nbits = u64::from(self.nbits);
        for i in 0..self.k {
            let bit = h1.wrapping_add(u64::from(i).wrapping_mul(h2)) % nbits;
            set_bit(&mut self.bits, bit_index(bit));
        }
    }

    /// `false` ⇒ key is definitely absent; `true` ⇒ maybe present.
    #[must_use]
    pub fn may_contain(&self, key: &[u8]) -> bool {
        if !self.is_active() {
            return true;
        }
        let (h1, h2) = hash_pair(key);
        let nbits = u64::from(self.nbits);
        for i in 0..self.k {
            let bit = h1.wrapping_add(u64::from(i).wrapping_mul(h2)) % nbits;
            if !test_bit(&self.bits, bit_index(bit)) {
                return false;
            }
        }
        true
    }

    /// Encode for SST trailer: `nbits u32 | k u32 | nbytes u32 | bits`.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + self.bits.len());
        out.extend_from_slice(&self.nbits.to_le_bytes());
        out.extend_from_slice(&self.k.to_le_bytes());
        let nbytes = u32::try_from(self.bits.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&nbytes.to_le_bytes());
        out.extend_from_slice(&self.bits);
        out
    }

    /// Decode filter bytes. Empty / zero-sized → always-true.
    ///
    /// # Errors
    /// Truncated or inconsistent lengths.
    pub fn decode(buf: &[u8]) -> Result<Self, String> {
        if buf.is_empty() {
            return Ok(Self::always_true());
        }
        if buf.len() < 12 {
            return Err(format!("bloom too short: {} bytes", buf.len()));
        }
        let nbits = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let k = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let nbytes = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]) as usize;
        if 12 + nbytes > buf.len() {
            return Err("bloom bits truncated".into());
        }
        if nbits == 0 || k == 0 || nbytes == 0 {
            return Ok(Self::always_true());
        }
        let expected = (nbits as usize).div_ceil(8);
        if nbytes < expected {
            return Err(format!(
                "bloom nbytes {nbytes} too small for nbits {nbits}"
            ));
        }
        let bits = buf[12..12 + nbytes].to_vec();
        Ok(Self { bits, nbits, k })
    }

    /// Number of bits.
    #[must_use]
    pub fn bit_count(&self) -> u32 {
        self.nbits
    }

    /// Probe count.
    #[must_use]
    pub fn hash_count(&self) -> u32 {
        self.k
    }
}

/// `bit` is always `< nbits ≤ u32::MAX` from the modulo above.
fn bit_index(bit: u64) -> usize {
    usize::try_from(bit).unwrap_or(0)
}

fn set_bit(bits: &mut [u8], i: usize) {
    bits[i / 8] |= 1 << (i % 8);
}

fn test_bit(bits: &[u8], i: usize) -> bool {
    (bits[i / 8] & (1 << (i % 8))) != 0
}

/// FNV-1a 64 + mix for a second independent hash.
fn hash_pair(key: &[u8]) -> (u64, u64) {
    let h1 = fnv1a64(key);
    // Second hash must be non-zero for double hashing.
    let mut h2 = fnv1a64_seed(key, 0x9e37_79b9_7f4a_7c15);
    if h2 == 0 {
        h2 = 0x9e37_79b9_7f4a_7c15;
    }
    (h1, h2)
}

fn fnv1a64(data: &[u8]) -> u64 {
    fnv1a64_seed(data, 0xcbf2_9ce4_8422_2325)
}

fn fnv1a64_seed(data: &[u8], seed: u64) -> u64 {
    let mut hash = seed;
    for b in data {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_false_negatives() {
        let mut f = BloomFilter::with_capacity(100, DEFAULT_BITS_PER_KEY);
        for i in 0..100u32 {
            f.insert(format!("key-{i}").as_bytes());
        }
        for i in 0..100u32 {
            assert!(
                f.may_contain(format!("key-{i}").as_bytes()),
                "false negative for key-{i}"
            );
        }
    }

    #[test]
    fn encode_decode_round_trip() {
        let mut f = BloomFilter::with_capacity(50, DEFAULT_BITS_PER_KEY);
        f.insert(b"alpha");
        f.insert(b"beta");
        let enc = f.encode();
        let g = BloomFilter::decode(&enc).unwrap();
        assert!(g.may_contain(b"alpha"));
        assert!(g.may_contain(b"beta"));
        assert_eq!(f.bit_count(), g.bit_count());
        assert_eq!(f.hash_count(), g.hash_count());
    }

    #[test]
    fn always_true_never_rejects() {
        let f = BloomFilter::always_true();
        assert!(f.may_contain(b"anything"));
        assert!(!f.is_active());
    }

    #[test]
    fn rejects_many_absent_keys() {
        let mut f = BloomFilter::with_capacity(32, DEFAULT_BITS_PER_KEY);
        for i in 0..32u32 {
            f.insert(format!("present-{i}").as_bytes());
        }
        let mut rejects = 0usize;
        for i in 0..200u32 {
            if !f.may_contain(format!("absent-{i}").as_bytes()) {
                rejects += 1;
            }
        }
        // With ~10 bits/key, expect most of 200 random keys rejected.
        assert!(rejects > 100, "expected many rejections, got {rejects}");
    }
}
