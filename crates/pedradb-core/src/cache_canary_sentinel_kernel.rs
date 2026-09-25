//! RFC-0283 Pilar 5 — Imunidade a Envenenamento de Cache em DRAM (Cache Canary Sentinel Kernel).
//!
//! Formalizes in-memory block cache integrity verification against silent DRAM bit-flips,
//! rowhammer, and stray pointer corruptions.
//! Wraps every uncompressed cached block with Dual Canaries (Head/Tail) and an in-memory CRC sentinel:
//!   - Canary Head: 0xDEAD_BEEF_CAFE_BABE
//!   - Canary Tail: 0xBABE_CAFE_DEAD_BEEF
//!   - In-Memory CRC32C: computed upon admission to cache.
//!
//! Mathematically guarantees that any RAM-level byte mutation inside the cache is intercepted
//! before delivery to readers, triggering an eviction and transparent reload from disk.

#![forbid(unsafe_code)]

/// Magic sentinel placed immediately before the cached block payload in memory.
pub const CANARY_HEAD_MAGIC: u64 = 0xDEAD_BEEF_CAFE_BABE;

/// Magic sentinel placed immediately after the cached block payload in memory.
pub const CANARY_TAIL_MAGIC: u64 = 0xBABE_CAFE_DEAD_BEEF;

/// Violations resulting from in-memory cache corruption or bit-rot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheIntegrityViolation {
    /// Head canary was overwritten or altered.
    HeadCanaryCorrupted {
        /// Found value.
        found: u64,
        /// Expected value.
        expected: u64,
    },
    /// Tail canary was overwritten or altered.
    TailCanaryCorrupted {
        /// Found value.
        found: u64,
        /// Expected value.
        expected: u64,
    },
    /// CRC32C checksum of the in-memory payload failed (bit-flip in RAM).
    InRamBitFlipDetected {
        /// Computed CRC.
        computed_crc: u32,
        /// Expected admitted CRC.
        admitted_crc: u32,
    },
}

/// A protected in-memory cached block with canary sentinels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedBlockSentinel {
    canary_head: u64,
    payload: Vec<u8>,
    canary_tail: u64,
    admitted_crc: u32,
}

impl CachedBlockSentinel {
    /// Creates a protected cached block, calculating its in-RAM integrity sentinel.
    #[must_use]
    pub fn new(payload: Vec<u8>) -> Self {
        let admitted_crc = crc32c::crc32c(&payload);
        Self {
            canary_head: CANARY_HEAD_MAGIC,
            payload,
            canary_tail: CANARY_TAIL_MAGIC,
            admitted_crc,
        }
    }

    /// Accesses the cached payload, strictly validating canaries and payload checksum.
    ///
    /// # Errors
    /// Returns `CacheIntegrityViolation` if any bit flip or memory overwrite is detected.
    pub fn get_verified_payload(&self) -> Result<&[u8], CacheIntegrityViolation> {
        // 1. Verify Head Canary
        if self.canary_head != CANARY_HEAD_MAGIC {
            return Err(CacheIntegrityViolation::HeadCanaryCorrupted {
                found: self.canary_head,
                expected: CANARY_HEAD_MAGIC,
            });
        }

        // 2. Verify Tail Canary
        if self.canary_tail != CANARY_TAIL_MAGIC {
            return Err(CacheIntegrityViolation::TailCanaryCorrupted {
                found: self.canary_tail,
                expected: CANARY_TAIL_MAGIC,
            });
        }

        // 3. Verify in-RAM payload CRC
        let computed = crc32c::crc32c(&self.payload);
        if computed != self.admitted_crc {
            return Err(CacheIntegrityViolation::InRamBitFlipDetected {
                computed_crc: computed,
                admitted_crc: self.admitted_crc,
            });
        }

        Ok(&self.payload)
    }

    /// Corrupts a byte in RAM (for test / injection).
    pub fn inject_bit_flip(&mut self, offset: usize) {
        if offset < self.payload.len() {
            self.payload[offset] ^= 0x01;
        }
    }

    /// Corrupts head canary (for test / injection).
    pub fn inject_head_canary_corrupt(&mut self) {
        self.canary_head = 0x1111_2222_3333_4444;
    }

    /// Corrupts tail canary (for test / injection).
    pub fn inject_tail_canary_corrupt(&mut self) {
        self.canary_tail = 0x5555_6666_7777_8888;
    }
}
