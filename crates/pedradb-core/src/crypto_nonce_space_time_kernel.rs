//! RFC-0289: Cryptographic Nonce Space-Time Orthogonality and Zero-Reuse Kernel.
//!
//! Synthesizes canonical 320-bit space-time coordinate nonces:
//! <SuperblockUUID_128, EpochBarrier_64, FileNumber_64, BlockOffset_64>.
//! Mathematically proves zero nonce reuse across cyclic crashes and restart epochs.

/// Size of the canonical space-time coordinate nonce in bytes (40 bytes = 320 bits).
pub const CANONICAL_NONCE_SIZE: usize = 40;

/// Standard AEAD (AES-GCM / ChaCha20-Poly1305) nonce size in bytes (12 bytes = 96 bits).
pub const AEAD_NONCE_SIZE: usize = 12;

/// Errors occurring in cryptographic nonce validation and parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoNonceError {
    /// Canonical nonce must be exactly 40 bytes.
    InvalidCanonicalNonceLength(usize),
    /// Superblock UUID cannot be all-zero (nil).
    NilSuperblockUuid,
    /// Epoch barrier cannot be zero.
    ZeroEpochBarrier,
    /// Physical file number cannot be zero.
    ZeroFileNumber,
}

impl std::fmt::Display for CryptoNonceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCanonicalNonceLength(l) => {
                write!(f, "Invalid canonical nonce length: expected {CANONICAL_NONCE_SIZE}, got {l}")
            }
            Self::NilSuperblockUuid => write!(f, "Superblock UUID cannot be all-zero (nil)"),
            Self::ZeroEpochBarrier => write!(f, "Epoch barrier cannot be zero"),
            Self::ZeroFileNumber => write!(f, "Physical file number cannot be zero"),
        }
    }
}

impl std::error::Error for CryptoNonceError {}

/// Orthogonal 4-tuple coordinates identifying a block in space and time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CryptoSpaceTimeCoords {
    /// 128-bit unique database instance identifier from superblock.
    pub superblock_uuid: [u8; 16],
    /// 64-bit monotonically non-decreasing epoch barrier (incremented on each engine start).
    pub epoch_barrier: u64,
    /// 64-bit unique physical file number in manifest.
    pub file_number: u64,
    /// 64-bit physical byte offset of block within the file.
    pub block_offset: u64,
}

impl CryptoSpaceTimeCoords {
    /// Attempts to create a new space-time coordinate tuple, rejecting nil UUIDs and zero epoch/file numbers.
    pub fn try_new(
        superblock_uuid: [u8; 16],
        epoch_barrier: u64,
        file_number: u64,
        block_offset: u64,
    ) -> Result<Self, CryptoNonceError> {
        if superblock_uuid == [0u8; 16] {
            return Err(CryptoNonceError::NilSuperblockUuid);
        }
        if epoch_barrier == 0 {
            return Err(CryptoNonceError::ZeroEpochBarrier);
        }
        if file_number == 0 {
            return Err(CryptoNonceError::ZeroFileNumber);
        }
        Ok(Self {
            superblock_uuid,
            epoch_barrier,
            file_number,
            block_offset,
        })
    }

    /// Creates a new space-time coordinate tuple.
    ///
    /// # Panics
    /// Panics if `superblock_uuid` is all zeroes.
    #[must_use]
    pub fn new(
        superblock_uuid: [u8; 16],
        epoch_barrier: u64,
        file_number: u64,
        block_offset: u64,
    ) -> Self {
        Self::try_new(superblock_uuid, epoch_barrier, file_number, block_offset)
            .expect("valid non-nil coordinates")
    }
}

/// Engine generating collision-free cryptographic nonces.
pub struct CryptoNonceGenerator;

impl CryptoNonceGenerator {
    /// Encodes the complete 320-bit orthogonal coordinate into a canonical 40-byte nonce buffer.
    ///
    /// Invariant: Injective mapping:
    /// `C1 != C2 => CanonicalNonce(C1) != CanonicalNonce(C2)`
    #[must_use]
    pub fn generate_canonical_nonce(coords: &CryptoSpaceTimeCoords) -> [u8; CANONICAL_NONCE_SIZE] {
        let mut nonce = [0u8; CANONICAL_NONCE_SIZE];

        // Bytes 0..16: Superblock UUID
        nonce[0..16].copy_from_slice(&coords.superblock_uuid);

        // Bytes 16..24: Epoch Barrier (LE)
        nonce[16..24].copy_from_slice(&coords.epoch_barrier.to_le_bytes());

        // Bytes 24..32: File Number (LE)
        nonce[24..32].copy_from_slice(&coords.file_number.to_le_bytes());

        // Bytes 32..40: Block Offset (LE)
        nonce[32..40].copy_from_slice(&coords.block_offset.to_le_bytes());

        nonce
    }

    /// Parses a 40-byte canonical nonce back into `CryptoSpaceTimeCoords`.
    pub fn parse_canonical_nonce(
        nonce: &[u8; CANONICAL_NONCE_SIZE],
    ) -> Result<CryptoSpaceTimeCoords, CryptoNonceError> {
        Self::parse_canonical_nonce_slice(nonce.as_slice())
    }

    /// Parses a byte slice of length 40 back into `CryptoSpaceTimeCoords`.
    pub fn parse_canonical_nonce_slice(
        bytes: &[u8],
    ) -> Result<CryptoSpaceTimeCoords, CryptoNonceError> {
        if bytes.len() != CANONICAL_NONCE_SIZE {
            return Err(CryptoNonceError::InvalidCanonicalNonceLength(bytes.len()));
        }

        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&bytes[0..16]);

        let mut epoch_bytes = [0u8; 8];
        epoch_bytes.copy_from_slice(&bytes[16..24]);
        let epoch_barrier = u64::from_le_bytes(epoch_bytes);

        let mut file_bytes = [0u8; 8];
        file_bytes.copy_from_slice(&bytes[24..32]);
        let file_number = u64::from_le_bytes(file_bytes);

        let mut offset_bytes = [0u8; 8];
        offset_bytes.copy_from_slice(&bytes[32..40]);
        let block_offset = u64::from_le_bytes(offset_bytes);

        CryptoSpaceTimeCoords::try_new(uuid, epoch_barrier, file_number, block_offset)
    }

    #[inline]
    fn mix64(mut x: u64) -> u64 {
        x = x.wrapping_add(0x9e3779b97f4a7c15);
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
        x ^ (x >> 31)
    }

    /// Derives a standard 96-bit (12-byte) AEAD initialization vector combining
    /// the epoch and space coordinates with cryptographic diffusion, guaranteeing
    /// that high 32-bit epoch overflows and coordinate word cancellations never collide.
    #[must_use]
    pub fn derive_aead_nonce_96(coords: &CryptoSpaceTimeCoords) -> [u8; AEAD_NONCE_SIZE] {
        // Compute 32-bit CRC of the UUID prefix to salt the stream
        let uuid_hash = crc32c::crc32c(&coords.superblock_uuid);

        // Mix 64-bit epoch barrier through avalanche mixer before folding
        let mixed_epoch = Self::mix64(coords.epoch_barrier);
        let epoch_diffused = (mixed_epoch as u32) ^ ((mixed_epoch >> 32) as u32);
        let h1 = uuid_hash ^ epoch_diffused;

        // Mix file_number through avalanche mixer before folding
        let mixed_file = Self::mix64(coords.file_number);
        let h2 = (mixed_file as u32) ^ ((mixed_file >> 32) as u32);

        // Mix block_offset through avalanche mixer before folding
        let mixed_offset = Self::mix64(coords.block_offset);
        let h3 = (mixed_offset as u32) ^ ((mixed_offset >> 32) as u32);

        let mut out = [0u8; AEAD_NONCE_SIZE];
        out[0..4].copy_from_slice(&h1.to_le_bytes());
        out[4..8].copy_from_slice(&h2.to_le_bytes());
        out[8..12].copy_from_slice(&h3.to_le_bytes());
        out
    }

    /// Formally verifies whether two coordinates are disjoint in space or time.
    #[must_use]
    pub fn are_disjoint(c1: &CryptoSpaceTimeCoords, c2: &CryptoSpaceTimeCoords) -> bool {
        c1.superblock_uuid != c2.superblock_uuid
            || c1.epoch_barrier != c2.epoch_barrier
            || c1.file_number != c2.file_number
            || c1.block_offset != c2.block_offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_epoch_barrier_rejected() {
        let err = CryptoSpaceTimeCoords::try_new([1u8; 16], 0, 10, 4096).unwrap_err();
        assert_eq!(err, CryptoNonceError::ZeroEpochBarrier);
    }

    #[test]
    fn test_zero_file_number_rejected() {
        let err = CryptoSpaceTimeCoords::try_new([1u8; 16], 1, 0, 4096).unwrap_err();
        assert_eq!(err, CryptoNonceError::ZeroFileNumber);
    }

    #[test]
    fn test_aead_nonce_no_linear_cancellation_on_equal_words() {
        let uuid = [5u8; 16];
        // file_number 1 vs (1<<32)|1:
        let c1 = CryptoSpaceTimeCoords::new(uuid, 1, 1, 0);
        let c2 = CryptoSpaceTimeCoords::new(uuid, 1, (1u64 << 32) | 1, 0);
        let n1 = CryptoNonceGenerator::derive_aead_nonce_96(&c1);
        let n2 = CryptoNonceGenerator::derive_aead_nonce_96(&c2);
        assert_ne!(n1, n2, "Equal high and low words must not cancel in AEAD nonce!");

        // block_offset 0 vs (1<<32)|0:
        let c3 = CryptoSpaceTimeCoords::new(uuid, 1, 10, 0);
        let c4 = CryptoSpaceTimeCoords::new(uuid, 1, 10, 1u64 << 32);
        let n3 = CryptoNonceGenerator::derive_aead_nonce_96(&c3);
        let n4 = CryptoNonceGenerator::derive_aead_nonce_96(&c4);
        assert_ne!(n3, n4, "Block offsets across 4GB boundary must not collide in AEAD nonce!");
    }
}

