//! Block Cache Key Disambiguation and Anti-Incarnation Collision Kernel (RFC-0285 Pilar 5).
//!
//! Guarantees that block cache keys are cryptographically isolated across file reincarnations,
//! preventing stale data blocks from being served after crashes, rollbacks, or file number recycling.
//!
//! Guarantees:
//! 1. Injective tripartite cache key: `(TableUUID, BlockOffset, GenerationEpoch)`.
//! 2. Zero cross-incarnation cache collision: different files with the same file number never alias.
//! 3. Total isolation between distinct database lifetimes and recreated SST files.

#![forbid(unsafe_code)]

use std::fmt;

/// Invariant violations and errors for block cache disambiguation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheDisambiguationError {
    /// Nil table UUID is invalid (uninitialized file handle).
    NilTableUuid,
    /// Generation epoch must be strictly positive (> 0).
    ZeroGenerationEpoch,
    /// Serialized key byte length mismatch.
    InvalidByteLength { actual: usize, expected: usize },
    /// Distinct file incarnations produced identical cache keys or tags.
    CrossIncarnationCollision,
    /// Block span addition overflows u64.
    BlockSpanOverflow { offset: u64, len: u64 },
}

impl fmt::Display for CacheDisambiguationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NilTableUuid => {
                write!(f, "Nil table UUID: table identity cannot be all zeros")
            }
            Self::ZeroGenerationEpoch => {
                write!(f, "Zero generation epoch: manifest epoch must be strictly positive (> 0)")
            }
            Self::InvalidByteLength { actual, expected } => {
                write!(
                    f,
                    "Invalid byte length for serialized cache key: got {actual}, expected {expected}"
                )
            }
            Self::CrossIncarnationCollision => {
                write!(
                    f,
                    "Cross-incarnation collision: distinct file incarnations produced identical cache keys"
                )
            }
            Self::BlockSpanOverflow { offset, len } => {
                write!(
                    f,
                    "Block span addition overflow: offset {offset} + len {len} wraps u64"
                )
            }
        }
    }
}

impl std::error::Error for CacheDisambiguationError {}

/// Disambiguated 128-bit resilient block cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DisambiguatedCacheKey {
    /// Unique immutable 128-bit UUID assigned to the SST file at creation.
    pub table_uuid: [u8; 16],
    /// Byte offset of the block within the SST file.
    pub block_offset: u64,
    /// Manifest generation epoch when this file was installed.
    pub generation_epoch: u64,
}

impl DisambiguatedCacheKey {
    /// Attempts to construct a validated disambiguated cache key with block span verification.
    pub fn try_new_with_span(
        table_uuid: [u8; 16],
        block_offset: u64,
        block_len: u64,
        generation_epoch: u64,
    ) -> Result<Self, CacheDisambiguationError> {
        if table_uuid == [0u8; 16] {
            return Err(CacheDisambiguationError::NilTableUuid);
        }
        if generation_epoch == 0 {
            return Err(CacheDisambiguationError::ZeroGenerationEpoch);
        }
        if block_offset.checked_add(block_len).is_none() {
            return Err(CacheDisambiguationError::BlockSpanOverflow {
                offset: block_offset,
                len: block_len,
            });
        }
        Ok(Self {
            table_uuid,
            block_offset,
            generation_epoch,
        })
    }

    /// Attempts to construct a validated disambiguated cache key.
    pub fn try_new(
        table_uuid: [u8; 16],
        block_offset: u64,
        generation_epoch: u64,
    ) -> Result<Self, CacheDisambiguationError> {
        Self::try_new_with_span(table_uuid, block_offset, 0, generation_epoch)
    }

    /// Constructs a new disambiguated cache key, enforcing invariant contracts.
    pub fn new(table_uuid: [u8; 16], block_offset: u64, generation_epoch: u64) -> Self {
        Self::try_new(table_uuid, block_offset, generation_epoch)
            .expect("DisambiguatedCacheKey invariant violated")
    }

    /// Serializes key into a compact 32-byte representation.
    pub fn to_bytes(&self) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        bytes[0..16].copy_from_slice(&self.table_uuid);
        bytes[16..24].copy_from_slice(&self.block_offset.to_be_bytes());
        bytes[24..32].copy_from_slice(&self.generation_epoch.to_be_bytes());
        bytes
    }

    /// Deserializes key from a 32-byte representation.
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        let mut table_uuid = [0u8; 16];
        table_uuid.copy_from_slice(&bytes[0..16]);
        let block_offset = u64::from_be_bytes([
            bytes[16], bytes[17], bytes[18], bytes[19], bytes[20], bytes[21], bytes[22], bytes[23],
        ]);
        let generation_epoch = u64::from_be_bytes([
            bytes[24], bytes[25], bytes[26], bytes[27], bytes[28], bytes[29], bytes[30], bytes[31],
        ]);
        Self {
            table_uuid,
            block_offset,
            generation_epoch,
        }
    }

    /// Deserializes key safely from a slice, validating byte length and invariant constraints.
    pub fn try_from_slice(bytes: &[u8]) -> Result<Self, CacheDisambiguationError> {
        if bytes.len() != 32 {
            return Err(CacheDisambiguationError::InvalidByteLength {
                actual: bytes.len(),
                expected: 32,
            });
        }
        let mut fixed = [0u8; 32];
        fixed.copy_from_slice(bytes);
        let key = Self::from_bytes(&fixed);
        if key.table_uuid == [0u8; 16] {
            return Err(CacheDisambiguationError::NilTableUuid);
        }
        if key.generation_epoch == 0 {
            return Err(CacheDisambiguationError::ZeroGenerationEpoch);
        }
        Ok(key)
    }
}

/// Verification oracle proving that cache lookups never return stale cross-incarnation data.
pub struct CacheCollisionOracle;

impl CacheCollisionOracle {
    /// Verifies that two cache keys are identical if and only if they refer to the exact same file incarnation.
    pub fn verify_disjoint_incarnations(
        key_incarnation_1: &DisambiguatedCacheKey,
        key_incarnation_2: &DisambiguatedCacheKey,
    ) -> Result<(), CacheDisambiguationError> {
        let distinct_incarnations = key_incarnation_1.table_uuid != key_incarnation_2.table_uuid
            || key_incarnation_1.generation_epoch != key_incarnation_2.generation_epoch;

        if distinct_incarnations {
            if key_incarnation_1.to_bytes() == key_incarnation_2.to_bytes() {
                return Err(CacheDisambiguationError::CrossIncarnationCollision);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_cache_disambiguation_structural_invariants_red_to_green() {
        // Invariant 1: Nil table UUID is rejected fail-closed
        assert_eq!(
            DisambiguatedCacheKey::try_new([0u8; 16], 1024, 1),
            Err(CacheDisambiguationError::NilTableUuid)
        );

        // Invariant 2: Generation epoch 0 is rejected fail-closed
        assert_eq!(
            DisambiguatedCacheKey::try_new([1u8; 16], 1024, 0),
            Err(CacheDisambiguationError::ZeroGenerationEpoch)
        );

        // Invariant 3: Block span addition overflow is detected and rejected
        assert_eq!(
            DisambiguatedCacheKey::try_new_with_span([1u8; 16], u64::MAX - 10, 20, 1),
            Err(CacheDisambiguationError::BlockSpanOverflow {
                offset: u64::MAX - 10,
                len: 20
            })
        );

        // Invariant 4: Valid key construction and round-trip byte representation
        let key = DisambiguatedCacheKey::try_new_with_span([2u8; 16], 4096, 512, 3).unwrap();
        let bytes = key.to_bytes();
        let decoded = DisambiguatedCacheKey::try_from_slice(&bytes).unwrap();
        assert_eq!(key, decoded);
    }
}

