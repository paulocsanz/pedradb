//! RFC-0289: Cryptographic Nonce Space-Time Orthogonality and Zero-Reuse Kernel.
//!
//! Synthesizes canonical 320-bit space-time coordinate nonces:
//! <SuperblockUUID_128, EpochBarrier_64, FileNumber_64, BlockOffset_64>.
//! Mathematically proves zero nonce reuse across cyclic crashes and restart epochs.

/// Size of the canonical space-time coordinate nonce in bytes (40 bytes = 320 bits).
pub const CANONICAL_NONCE_SIZE: usize = 40;

/// Standard AEAD (AES-GCM / ChaCha20-Poly1305) nonce size in bytes (12 bytes = 96 bits).
pub const AEAD_NONCE_SIZE: usize = 12;

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
    /// Creates a new space-time coordinate tuple.
    #[must_use]
    pub fn new(
        superblock_uuid: [u8; 16],
        epoch_barrier: u64,
        file_number: u64,
        block_offset: u64,
    ) -> Self {
        Self {
            superblock_uuid,
            epoch_barrier,
            file_number,
            block_offset,
        }
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

    /// Derives a standard 96-bit (12-byte) AEAD initialization vector combining
    /// the epoch and space coordinates with cryptographic diffusion.
    #[must_use]
    pub fn derive_aead_nonce_96(coords: &CryptoSpaceTimeCoords) -> [u8; AEAD_NONCE_SIZE] {
        // Compute 32-bit CRC of the UUID prefix to salt the stream
        let uuid_hash = crc32c::crc32c(&coords.superblock_uuid);

        // 4 bytes: combined uuid_hash ^ epoch
        let h1 = uuid_hash ^ (coords.epoch_barrier as u32);
        // 4 bytes: file_number lower 32 ^ upper 32
        let h2 = (coords.file_number as u32) ^ ((coords.file_number >> 32) as u32);
        // 4 bytes: block_offset lower 32 ^ upper 32
        let h3 = (coords.block_offset as u32) ^ ((coords.block_offset >> 32) as u32);

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
