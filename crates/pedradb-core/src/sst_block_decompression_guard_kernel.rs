//! SST Block Decompression Bomb and Varint Safe Parser Guard Kernel (RFC-0311).
//!
//! Provides mathematically bounded, allocation-capped parsing and decompression
//! of SSTable blocks to prevent Denial of Service (DoS), Out-Of-Memory (OOM) crashes,
//! buffer over-reads, and restart array index corruption from hostile or corrupted SST inputs.
//!
//! # Invariants and Formal Guarantees
//! 1. **Zero Allocation on Size Rejection:** If declared uncompressed size exceeds
//!    `max_uncompressed_bytes`, parsing fails immediately before allocating any heap buffer.
//! 2. **Expansion Ratio Ceiling:** Rejects blocks whose expansion ratio exceeds `max_expansion_ratio`
//!    to neutralize compressed zip-bomb payloads.
//! 3. **Integrity Before Decode:** CRC32C integrity validation is performed prior to decoding.
//! 4. **Bounded Restart Parsing:** Validates `num_restarts * 4 + 4 <= block_len` with overflow protection
//!    and enforces monotonic restart offsets.
//! 5. **Constant-Time Bounded Varint:** Enforces strict 5-byte (u32) and 10-byte (u64) varint limits,
//!    preventing infinite loops or shift overflows on corrupted streams.

#![forbid(unsafe_code)]

/// Default maximum uncompressed block size: 64 MiB.
pub const DEFAULT_MAX_UNCOMPRESSED_BLOCK_SIZE: usize = 64 * 1024 * 1024;

/// Default maximum allowed expansion ratio for compressed blocks (256x).
pub const DEFAULT_MAX_EXPANSION_RATIO: usize = 256;

/// Minimum compressed byte length before applying expansion ratio checks.
pub const MIN_BYTES_FOR_EXPANSION_CHECK: usize = 64;

/// Errors returned by the SST block decompression guard kernel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecompressionGuardError {
    /// Raw block is truncated and cannot contain the 4-byte CRC trailer.
    TruncatedCrc {
        /// Actual length of raw block slice.
        actual_len: usize,
    },
    /// Stored CRC32C does not match computed CRC32C.
    CrcMismatch {
        /// CRC stored in block trailer.
        stored: u32,
        /// Computed CRC of the payload.
        computed: u32,
    },
    /// Declared uncompressed size exceeds strict security ceiling.
    ExceedsMaxUncompressedSize {
        /// Size declared in block header.
        declared: usize,
        /// Maximum allowed limit.
        limit: usize,
    },
    /// Declared expansion ratio is physically anomalous (decompression bomb).
    ExpansionRatioExplosion {
        /// Declared uncompressed size.
        declared: usize,
        /// Compressed payload length.
        compressed: usize,
        /// Computed expansion ratio.
        ratio: usize,
        /// Allowed expansion ceiling.
        max_ratio: usize,
    },
    /// Restart array is malformed, truncated, or contains out-of-bounds offsets.
    MalformedRestartArray {
        /// Reason for failure.
        reason: &'static str,
    },
    /// Varint decoding exceeded standard byte boundaries or stream truncated.
    VarintOverflow,
    /// Underlying decompression engine error.
    DecompressionFailed(String),
}

impl std::fmt::Display for DecompressionGuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TruncatedCrc { actual_len } => {
                write!(f, "SST block truncated for CRC trailer: {actual_len} bytes")
            }
            Self::CrcMismatch { stored, computed } => {
                write!(f, "SST block CRC mismatch: stored={stored:#010x}, computed={computed:#010x}")
            }
            Self::ExceedsMaxUncompressedSize { declared, limit } => {
                write!(f, "SST block uncompressed size {declared} exceeds limit {limit}")
            }
            Self::ExpansionRatioExplosion { declared, compressed, ratio, max_ratio } => {
                write!(
                    f,
                    "SST block expansion ratio {ratio}x (declared {declared}, compressed {compressed}) exceeds ceiling {max_ratio}x"
                )
            }
            Self::MalformedRestartArray { reason } => {
                write!(f, "SST block malformed restart array: {reason}")
            }
            Self::VarintOverflow => {
                write!(f, "SST block varint overflow or invalid continuation")
            }
            Self::DecompressionFailed(msg) => {
                write!(f, "SST block decompression failed: {msg}")
            }
        }
    }
}

impl std::error::Error for DecompressionGuardError {}

/// Configuration for SST block decompression guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockDecompressionGuardConfig {
    /// Maximum allowed uncompressed block size in bytes.
    pub max_uncompressed_bytes: usize,
    /// Maximum allowed expansion ratio (uncompressed / compressed).
    pub max_expansion_ratio: usize,
    /// Whether to enforce CRC32C trailer validation.
    pub verify_crc: bool,
}

impl Default for BlockDecompressionGuardConfig {
    fn default() -> Self {
        Self {
            max_uncompressed_bytes: DEFAULT_MAX_UNCOMPRESSED_BLOCK_SIZE,
            max_expansion_ratio: DEFAULT_MAX_EXPANSION_RATIO,
            verify_crc: true,
        }
    }
}

/// Core safe block decoder and validation routines.
pub struct SafeBlockDecoder;

impl SafeBlockDecoder {
    /// Validates and strips the 4-byte CRC32C trailer from a raw on-disk block.
    ///
    /// # Safety and Guarantees
    /// Returns a slice to the payload body only if CRC validation succeeds.
    /// Operates with zero memory allocation.
    pub fn split_and_verify_crc<'a>(
        raw: &'a [u8],
        verify_crc: bool,
    ) -> Result<&'a [u8], DecompressionGuardError> {
        if raw.len() < 4 {
            return Err(DecompressionGuardError::TruncatedCrc {
                actual_len: raw.len(),
            });
        }

        let (body, crc_bytes) = raw.split_at(raw.len() - 4);
        if verify_crc {
            let stored = u32::from_le_bytes([crc_bytes[0], crc_bytes[1], crc_bytes[2], crc_bytes[3]]);
            let computed = crc32c::crc32c(body);
            if stored != computed {
                return Err(DecompressionGuardError::CrcMismatch { stored, computed });
            }
        }

        Ok(body)
    }

    /// Inspects the declared uncompressed size of an LZ4-compressed block body,
    /// validating that it does not violate maximum allocation caps or expansion ratios.
    ///
    /// # Safety and Guarantees
    /// Returns the declared uncompressed size and input slice *without* allocating memory.
    pub fn inspect_lz4_size<'a>(
        body: &'a [u8],
        config: &BlockDecompressionGuardConfig,
    ) -> Result<(usize, &'a [u8]), DecompressionGuardError> {
        let (size, input) = lz4_flex::block::uncompressed_size(body).map_err(|e| {
            DecompressionGuardError::DecompressionFailed(format!("Invalid LZ4 size header: {e}"))
        })?;

        let bypass_bomb_guard = crate::mutate_switch!(
            crate::mutation_switch_kernel::MUTANT_DECOMPRESSION_BOMB_BYPASS,
            false,
            true
        );

        if !bypass_bomb_guard {
            // Invariant 1: Size ceiling check
            if size > config.max_uncompressed_bytes {
                return Err(DecompressionGuardError::ExceedsMaxUncompressedSize {
                    declared: size,
                    limit: config.max_uncompressed_bytes,
                });
            }

            // Invariant 2: Expansion ratio check
            if body.len() >= MIN_BYTES_FOR_EXPANSION_CHECK {
                let ratio = size.checked_div(body.len()).unwrap_or(0);
                if ratio > config.max_expansion_ratio {
                    return Err(DecompressionGuardError::ExpansionRatioExplosion {
                        declared: size,
                        compressed: body.len(),
                        ratio,
                        max_ratio: config.max_expansion_ratio,
                    });
                }
            }
        }

        Ok((size, input))
    }

    /// Bounded decompression of an LZ4-compressed block into a caller-supplied scratch vector.
    ///
    /// Ensures pre-allocation size does not exceed `max_uncompressed_bytes` and verifies
    /// that exact declared bytes were written.
    pub fn decompress_lz4_into(
        body: &[u8],
        config: &BlockDecompressionGuardConfig,
        scratch: &mut Vec<u8>,
    ) -> Result<usize, DecompressionGuardError> {
        let (size, input) = Self::inspect_lz4_size(body, config)?;

        scratch.clear();
        scratch.resize(size, 0);

        let written = lz4_flex::block::decompress_into(input, scratch.as_mut_slice()).map_err(|e| {
            DecompressionGuardError::DecompressionFailed(format!("LZ4 decompression error: {e}"))
        })?;

        if written != size {
            return Err(DecompressionGuardError::DecompressionFailed(format!(
                "Decompressed size mismatch: wrote {written}, expected {size}"
            )));
        }

        Ok(written)
    }

    /// Verifies and extracts the restart offsets array from the tail of a decompressed block.
    ///
    /// Block Tail Format:
    /// `[restart_0: u32_le] [restart_1: u32_le] ... [restart_{k-1}: u32_le] [num_restarts: u32_le]`
    pub fn verify_and_extract_restarts(
        block: &[u8],
    ) -> Result<Vec<u32>, DecompressionGuardError> {
        if block.len() < 4 {
            return Err(DecompressionGuardError::MalformedRestartArray {
                reason: "Block too short for restart count trailer",
            });
        }

        let num_restarts = u32::from_le_bytes([
            block[block.len() - 4],
            block[block.len() - 3],
            block[block.len() - 2],
            block[block.len() - 1],
        ]);

        if num_restarts == 0 {
            if block.len() > 4 {
                return Err(DecompressionGuardError::MalformedRestartArray {
                    reason: "Non-empty block cannot have 0 restarts",
                });
            }
            return Ok(Vec::new());
        }

        let restart_bytes = (num_restarts as usize).checked_mul(4).ok_or(
            DecompressionGuardError::MalformedRestartArray {
                reason: "num_restarts multiplication overflow",
            },
        )?;

        let total_tail_len = restart_bytes.checked_add(4).ok_or(
            DecompressionGuardError::MalformedRestartArray {
                reason: "total restart tail length overflow",
            },
        )?;

        if total_tail_len > block.len() {
            return Err(DecompressionGuardError::MalformedRestartArray {
                reason: "Restart array exceeds block size",
            });
        }

        let payload_len = block.len() - total_tail_len;
        let restarts_slice = &block[payload_len..block.len() - 4];

        let mut offsets = Vec::with_capacity(num_restarts as usize);
        let mut prev_offset: Option<u32> = None;

        for chunk in restarts_slice.chunks_exact(4) {
            let offset = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            if (offset as usize) > payload_len {
                return Err(DecompressionGuardError::MalformedRestartArray {
                    reason: "Restart offset points into or past restart array itself",
                });
            }

            if offsets.is_empty() {
                if offset != 0 {
                    return Err(DecompressionGuardError::MalformedRestartArray {
                        reason: "First restart offset must be 0",
                    });
                }
            } else if let Some(prev) = prev_offset {
                if offset <= prev {
                    return Err(DecompressionGuardError::MalformedRestartArray {
                        reason: "Restart offsets must be strictly monotonically increasing",
                    });
                }
            }

            prev_offset = Some(offset);
            offsets.push(offset);
        }

        Ok(offsets)
    }

    /// Decodes a variable-length 32-bit unsigned integer (varint32) safely.
    ///
    /// Consumes at most 5 bytes. Returns `(value, bytes_consumed)`.
    pub fn decode_varint32(slice: &[u8]) -> Result<(u32, usize), DecompressionGuardError> {
        let mut result = 0u32;
        let mut shift = 0u32;
        let mut consumed = 0usize;

        for &byte in slice.iter().take(5) {
            consumed += 1;
            let val = (byte & 0x7F) as u32;
            if shift >= 32 || (shift == 28 && val > 0x0F) {
                return Err(DecompressionGuardError::VarintOverflow);
            }
            result |= val << shift;
            if byte & 0x80 == 0 {
                return Ok((result, consumed));
            }
            shift += 7;
        }

        Err(DecompressionGuardError::VarintOverflow)
    }

    /// Decodes a variable-length 64-bit unsigned integer (varint64) safely.
    ///
    /// Consumes at most 10 bytes. Returns `(value, bytes_consumed)`.
    pub fn decode_varint64(slice: &[u8]) -> Result<(u64, usize), DecompressionGuardError> {
        let mut result = 0u64;
        let mut shift = 0u32;
        let mut consumed = 0usize;

        for &byte in slice.iter().take(10) {
            consumed += 1;
            let val = (byte & 0x7F) as u64;
            if shift >= 64 || (shift == 63 && val > 0x01) {
                return Err(DecompressionGuardError::VarintOverflow);
            }
            result |= val << shift;
            if byte & 0x80 == 0 {
                return Ok((result, consumed));
            }
            shift += 7;
        }

        Err(DecompressionGuardError::VarintOverflow)
    }
}
