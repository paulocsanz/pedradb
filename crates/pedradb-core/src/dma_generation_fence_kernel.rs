//! RFC-0287: DMA Generation Fence and Ghost Block Invalidation Kernel.
//!
//! Enforces zero-ghost read safety at the boundary of asynchronous Direct I/O
//! and NVMe readahead buffers. Blocks are validated with explicit file UUID,
//! generation id, block index, and CRC32C before exposure to active iterators.

use std::fmt;

/// Fixed length of the DMA block header envelope (32 bytes).
pub const DMA_HEADER_SIZE: usize = 32;

/// Token identifying the active file generation associated with an LSM version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileGenerationToken {
    /// 128-bit unique file identifier.
    pub file_uuid: [u8; 16],
    /// Monotonically incremented generation counter of the SST.
    pub generation_id: u64,
    /// Physical file number in manifest.
    pub file_number: u64,
}

impl FileGenerationToken {
    /// Creates a new file generation token.
    #[must_use]
    pub fn new(file_uuid: [u8; 16], generation_id: u64, file_number: u64) -> Self {
        Self {
            file_uuid,
            generation_id,
            file_number,
        }
    }
}

/// Errors returned when an incoming DMA buffer fails validation against active generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FenceRejection {
    /// Raw buffer is shorter than the minimum header length (32 bytes).
    BufferUnderflow { actual_len: usize },
    /// File UUID does not match the requested token (recycled or cross-file buffer).
    FileUuidMismatch {
        expected: [u8; 16],
        actual: [u8; 16],
    },
    /// File generation mismatch (file was rewritten/unlinked by concurrent compaction).
    GenerationMismatch {
        expected: u64,
        actual: u64,
    },
    /// Block index mismatch (out-of-order DMA delivery).
    BlockIndexMismatch {
        expected: u32,
        actual: u32,
    },
    /// CRC32C payload corruption detected.
    ChecksumMismatch {
        expected: u32,
        calculated: u32,
    },
}

impl fmt::Display for FenceRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BufferUnderflow { actual_len } => {
                write!(f, "DMA buffer underflow: len {actual_len} < {DMA_HEADER_SIZE}")
            }
            Self::FileUuidMismatch { expected, actual } => {
                write!(f, "DMA UUID mismatch: expected {expected:?}, got {actual:?}")
            }
            Self::GenerationMismatch { expected, actual } => {
                write!(f, "DMA generation mismatch: expected {expected}, got {actual}")
            }
            Self::BlockIndexMismatch { expected, actual } => {
                write!(f, "DMA block index mismatch: expected {expected}, got {actual}")
            }
            Self::ChecksumMismatch { expected, calculated } => {
                write!(f, "DMA CRC32C mismatch: expected 0x{expected:08x}, calculated 0x{calculated:08x}")
            }
        }
    }
}

impl std::error::Error for FenceRejection {}

/// DMA generation fence engine.
pub struct DmaGenerationFence;

impl DmaGenerationFence {
    /// Encapsulates a payload slice into an authoritative DMA block buffer with header.
    #[must_use]
    pub fn seal_dma_block(
        token: &FileGenerationToken,
        block_idx: u32,
        payload: &[u8],
    ) -> Vec<u8> {
        let crc = crc32c::crc32c(payload);
        let mut buf = Vec::with_capacity(DMA_HEADER_SIZE + payload.len());

        // 16 bytes: file_uuid
        buf.extend_from_slice(&token.file_uuid);
        // 8 bytes: generation_id (LE)
        buf.extend_from_slice(&token.generation_id.to_le_bytes());
        // 4 bytes: block_index (LE)
        buf.extend_from_slice(&block_idx.to_le_bytes());
        // 4 bytes: payload_crc32c (LE)
        buf.extend_from_slice(&crc.to_le_bytes());

        // Payload
        buf.extend_from_slice(payload);
        buf
    }

    /// Verifies and unpacks an incoming raw Direct I/O DMA buffer against the active token.
    /// Returns the verified payload or an explicit rejection reason.
    pub fn verify_dma_block(
        token: &FileGenerationToken,
        expected_block_idx: u32,
        raw_block: &[u8],
    ) -> Result<Vec<u8>, FenceRejection> {
        if raw_block.len() < DMA_HEADER_SIZE {
            return Err(FenceRejection::BufferUnderflow {
                actual_len: raw_block.len(),
            });
        }

        let mut actual_uuid = [0u8; 16];
        actual_uuid.copy_from_slice(&raw_block[0..16]);
        if actual_uuid != token.file_uuid {
            return Err(FenceRejection::FileUuidMismatch {
                expected: token.file_uuid,
                actual: actual_uuid,
            });
        }

        let mut gen_bytes = [0u8; 8];
        gen_bytes.copy_from_slice(&raw_block[16..24]);
        let actual_gen = u64::from_le_bytes(gen_bytes);
        if actual_gen != token.generation_id {
            return Err(FenceRejection::GenerationMismatch {
                expected: token.generation_id,
                actual: actual_gen,
            });
        }

        let mut idx_bytes = [0u8; 4];
        idx_bytes.copy_from_slice(&raw_block[24..28]);
        let actual_idx = u32::from_le_bytes(idx_bytes);
        if actual_idx != expected_block_idx {
            return Err(FenceRejection::BlockIndexMismatch {
                expected: expected_block_idx,
                actual: actual_idx,
            });
        }

        let mut crc_bytes = [0u8; 4];
        crc_bytes.copy_from_slice(&raw_block[28..32]);
        let expected_crc = u32::from_le_bytes(crc_bytes);

        let payload = &raw_block[32..];
        let calculated_crc = crc32c::crc32c(payload);
        if calculated_crc != expected_crc {
            return Err(FenceRejection::ChecksumMismatch {
                expected: expected_crc,
                calculated: calculated_crc,
            });
        }

        Ok(payload.to_vec())
    }
}
