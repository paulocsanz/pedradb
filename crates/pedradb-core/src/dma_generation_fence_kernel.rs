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
    /// Creates a validated file generation token.
    pub fn try_new(file_uuid: [u8; 16], generation_id: u64, file_number: u64) -> Result<Self, FenceRejection> {
        if file_uuid == [0u8; 16] || generation_id == 0 {
            return Err(FenceRejection::InvalidToken);
        }
        if file_number == 0 {
            return Err(FenceRejection::ZeroFileNumber);
        }
        Ok(Self {
            file_uuid,
            generation_id,
            file_number,
        })
    }

    /// Creates a new file generation token.
    #[must_use]
    pub fn new(file_uuid: [u8; 16], generation_id: u64, file_number: u64) -> Self {
        Self {
            file_uuid,
            generation_id,
            file_number,
        }
    }

    /// Verifies that the token contains a non-nil UUID and non-zero generation.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.file_uuid != [0u8; 16] && self.generation_id > 0
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
    /// Token contains a null/uninitialized UUID or generation 0.
    InvalidToken,
    /// Header is completely zeroed out (unallocated NVMe block / disk hole).
    ZeroGhostHeader,
    /// Payload has zero bytes (empty ghost block).
    EmptyPayload,
    /// File number cannot be zero.
    ZeroFileNumber,
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
            Self::InvalidToken => {
                write!(f, "DMA rejected invalid token: nil UUID or generation zero")
            }
            Self::ZeroGhostHeader => {
                write!(f, "DMA rejected zero-ghost header: block is unallocated NVMe zero hole")
            }
            Self::EmptyPayload => {
                write!(f, "DMA rejected empty payload: zero-length block")
            }
            Self::ZeroFileNumber => {
                write!(f, "DMA file number cannot be zero")
            }
        }
    }
}

impl std::error::Error for FenceRejection {}

/// DMA generation fence engine.
pub struct DmaGenerationFence;

impl DmaGenerationFence {
    /// Encapsulates a payload slice validating token and non-empty payload.
    pub fn try_seal_dma_block(
        token: &FileGenerationToken,
        block_idx: u32,
        payload: &[u8],
    ) -> Result<Vec<u8>, FenceRejection> {
        if !token.is_valid() || token.file_number == 0 {
            return Err(FenceRejection::InvalidToken);
        }
        if payload.is_empty() {
            return Err(FenceRejection::EmptyPayload);
        }
        Ok(Self::seal_dma_block(token, block_idx, payload))
    }

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

    /// Verifies and unpacks an incoming Direct I/O DMA buffer with exact payload length.
    ///
    /// This allows zero-copy validation of blocks embedded in sector-aligned DMA buffers
    /// (e.g. 4096-byte Direct I/O sectors) without trailing zeroes corrupting CRC calculation.
    pub fn verify_dma_block_with_len<'a>(
        token: &FileGenerationToken,
        expected_block_idx: u32,
        raw_block: &'a [u8],
        exact_payload_len: usize,
    ) -> Result<&'a [u8], FenceRejection> {
        if !token.is_valid() {
            return Err(FenceRejection::InvalidToken);
        }

        if raw_block.len() < DMA_HEADER_SIZE {
            return Err(FenceRejection::BufferUnderflow {
                actual_len: raw_block.len(),
            });
        }

        // Check for NVMe unallocated zero hole in header
        if raw_block[0..DMA_HEADER_SIZE].iter().all(|&b| b == 0) {
            return Err(FenceRejection::ZeroGhostHeader);
        }

        if exact_payload_len == 0 {
            return Err(FenceRejection::EmptyPayload);
        }

        let required_len = DMA_HEADER_SIZE.saturating_add(exact_payload_len);
        if raw_block.len() < required_len {
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

        let payload = &raw_block[DMA_HEADER_SIZE..required_len];
        let calculated_crc = crc32c::crc32c(payload);
        if calculated_crc != expected_crc {
            return Err(FenceRejection::ChecksumMismatch {
                expected: expected_crc,
                calculated: calculated_crc,
            });
        }

        Ok(payload)
    }

    /// Zero-copy verification of an unpadded DMA block buffer.
    pub fn verify_dma_block_slice<'a>(
        token: &FileGenerationToken,
        expected_block_idx: u32,
        raw_block: &'a [u8],
    ) -> Result<&'a [u8], FenceRejection> {
        if !token.is_valid() {
            return Err(FenceRejection::InvalidToken);
        }

        if raw_block.len() < DMA_HEADER_SIZE {
            return Err(FenceRejection::BufferUnderflow {
                actual_len: raw_block.len(),
            });
        }

        if raw_block[0..DMA_HEADER_SIZE].iter().all(|&b| b == 0) {
            return Err(FenceRejection::ZeroGhostHeader);
        }

        if raw_block.len() == DMA_HEADER_SIZE {
            return Err(FenceRejection::EmptyPayload);
        }

        let exact_payload_len = raw_block.len() - DMA_HEADER_SIZE;
        Self::verify_dma_block_with_len(token, expected_block_idx, raw_block, exact_payload_len)
    }

    /// Verifies and unpacks an incoming raw Direct I/O DMA buffer against the active token.
    /// Returns the verified payload as an owned `Vec<u8>`.
    pub fn verify_dma_block(
        token: &FileGenerationToken,
        expected_block_idx: u32,
        raw_block: &[u8],
    ) -> Result<Vec<u8>, FenceRejection> {
        Self::verify_dma_block_slice(token, expected_block_idx, raw_block).map(|s| s.to_vec())
    }

    /// Validates an entire contiguous extent of DMA blocks.
    pub fn verify_dma_extent<'a>(
        token: &FileGenerationToken,
        start_block_idx: u32,
        blocks: &'a [&'a [u8]],
    ) -> Result<Vec<&'a [u8]>, FenceRejection> {
        let mut verified = Vec::with_capacity(blocks.len());
        for (i, block) in blocks.iter().enumerate() {
            let expected_idx = start_block_idx.saturating_add(i as u32);
            let payload = Self::verify_dma_block_slice(token, expected_idx, block)?;
            verified.push(payload);
        }
        Ok(verified)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dma_token_bounds() {
        assert_eq!(
            FileGenerationToken::try_new([0u8; 16], 1, 1),
            Err(FenceRejection::InvalidToken)
        );
        assert_eq!(
            FileGenerationToken::try_new([1u8; 16], 0, 1),
            Err(FenceRejection::InvalidToken)
        );
        assert_eq!(
            FileGenerationToken::try_new([1u8; 16], 1, 0),
            Err(FenceRejection::ZeroFileNumber)
        );

        let token = FileGenerationToken::try_new([2u8; 16], 10, 42).expect("valid token");
        assert_eq!(token.file_number, 42);

        // Try seal empty payload
        assert_eq!(
            DmaGenerationFence::try_seal_dma_block(&token, 0, &[]),
            Err(FenceRejection::EmptyPayload)
        );

        let sealed = DmaGenerationFence::try_seal_dma_block(&token, 0, b"hello dma").expect("sealed");
        let verified = DmaGenerationFence::verify_dma_block(&token, 0, &sealed).expect("verified");
        assert_eq!(verified, b"hello dma");
    }

    #[test]
    fn test_dma_fence_rejection_display() {
        let err = FenceRejection::ZeroFileNumber;
        assert_eq!(format!("{err}"), "DMA file number cannot be zero");

        let err2 = FenceRejection::InvalidToken;
        assert_eq!(
            format!("{err2}"),
            "DMA rejected invalid token: nil UUID or generation zero"
        );
    }
}
