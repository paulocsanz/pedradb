//! RFC-0288: Decompression Expansion Cap and Anti-Zip-Bomb Streaming Kernel.
//!
//! Enforces strict mathematical limits on decompression amplification ratios (R_max <= 1024)
//! and incremental chunk processing, preventing OOM exhaustion from adversarial low-entropy payloads.

use std::fmt;

/// Maximum allowed decompression amplification factor (1024x).
pub const DEFAULT_MAX_EXPANSION_RATIO: u32 = 1024;

/// Default maximum allowed uncompressed block size in bytes (64 MiB).
pub const DEFAULT_MAX_ABSOLUTE_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

/// Errors returned when decompression exceeds expansion safety invariants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecompressionCapError {
    /// Declared uncompressed length in block header exceeds allowed amplification factor.
    ExcessiveDeclaredExpansion {
        /// Declared uncompressed length.
        declared_len: usize,
        /// Compressed payload length.
        compressed_len: usize,
        /// Computed ratio.
        ratio: u32,
        /// Maximum allowed ratio.
        max_ratio: u32,
    },
    /// Declared uncompressed length exceeds absolute sanity ceiling.
    AbsoluteCeilingExceeded {
        /// Declared length.
        declared_len: usize,
        /// Absolute ceiling.
        ceiling: usize,
    },
    /// Decompression produced more bytes than declared or exceeded runtime budget.
    RuntimeBudgetExceeded {
        /// Bytes produced so far.
        bytes_produced: usize,
        /// Maximum allowed bytes.
        max_allowed: usize,
    },
    /// Compressed input is corrupted or malformed.
    CorruptedInput {
        /// Diagnostic reason.
        reason: String,
    },
}

impl fmt::Display for DecompressionCapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExcessiveDeclaredExpansion { declared_len, compressed_len, ratio, max_ratio } => {
                write!(
                    f,
                    "decompression ratio {ratio}x (declared {declared_len}B / compressed {compressed_len}B) exceeds max {max_ratio}x"
                )
            }
            Self::AbsoluteCeilingExceeded { declared_len, ceiling } => {
                write!(f, "declared uncompressed len {declared_len}B exceeds absolute ceiling {ceiling}B")
            }
            Self::RuntimeBudgetExceeded { bytes_produced, max_allowed } => {
                write!(f, "decompression runtime budget exceeded: {bytes_produced}B > {max_allowed}B")
            }
            Self::CorruptedInput { reason } => {
                write!(f, "corrupted compressed input: {reason}")
            }
        }
    }
}

impl std::error::Error for DecompressionCapError {}

/// Configuration policy for decompression safety caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecompressionCapPolicy {
    /// Maximum allowed expansion ratio (e.g. 1024).
    pub max_expansion_ratio: u32,
    /// Absolute ceiling on uncompressed block size in bytes.
    pub max_absolute_bytes: usize,
}

impl Default for DecompressionCapPolicy {
    fn default() -> Self {
        Self {
            max_expansion_ratio: DEFAULT_MAX_EXPANSION_RATIO,
            max_absolute_bytes: DEFAULT_MAX_ABSOLUTE_OUTPUT_BYTES,
        }
    }
}

/// Governor enforcing expansion caps before and during decompression.
pub struct DecompressionCapGovernor {
    policy: DecompressionCapPolicy,
}

impl DecompressionCapGovernor {
    /// Creates a new governor with custom policy.
    #[must_use]
    pub fn new(policy: DecompressionCapPolicy) -> Self {
        Self { policy }
    }

    /// Pre-validates block header metadata before allocating memory.
    pub fn pre_validate_header(
        &self,
        compressed_len: usize,
        declared_uncompressed_len: usize,
    ) -> Result<(), DecompressionCapError> {
        if declared_uncompressed_len > self.policy.max_absolute_bytes {
            return Err(DecompressionCapError::AbsoluteCeilingExceeded {
                declared_len: declared_uncompressed_len,
                ceiling: self.policy.max_absolute_bytes,
            });
        }

        // Avoid division by zero: if compressed_len is 0 and declared is > 0, it's infinite expansion
        let comp_nonzero = compressed_len.max(1);
        let ratio = (declared_uncompressed_len / comp_nonzero) as u32;

        if ratio > self.policy.max_expansion_ratio {
            return Err(DecompressionCapError::ExcessiveDeclaredExpansion {
                declared_len: declared_uncompressed_len,
                compressed_len,
                ratio,
                max_ratio: self.policy.max_expansion_ratio,
            });
        }

        Ok(())
    }

    /// Safely unpacks run-length or raw compressed data under strict output cap monitoring.
    pub fn safe_unpack(
        &self,
        compressed: &[u8],
        declared_len: usize,
    ) -> Result<Vec<u8>, DecompressionCapError> {
        self.pre_validate_header(compressed.len(), declared_len)?;

        let mut output = Vec::with_capacity(declared_len.min(64 * 1024));
        let mut cursor = 0;

        // Structured unpacker: reads tagged sequences [flag: 0 = literal, 1 = repeat]
        while cursor < compressed.len() {
            let tag = compressed[cursor];
            cursor += 1;

            if tag == 0 {
                // Literal run: next 2 bytes length, followed by bytes
                if cursor + 2 > compressed.len() {
                    return Err(DecompressionCapError::CorruptedInput {
                        reason: "truncated literal header".to_string(),
                    });
                }
                let len = u16::from_le_bytes([compressed[cursor], compressed[cursor + 1]]) as usize;
                cursor += 2;
                if cursor + len > compressed.len() {
                    return Err(DecompressionCapError::CorruptedInput {
                        reason: "truncated literal payload".to_string(),
                    });
                }
                if output.len() + len > declared_len {
                    return Err(DecompressionCapError::RuntimeBudgetExceeded {
                        bytes_produced: output.len() + len,
                        max_allowed: declared_len,
                    });
                }
                output.extend_from_slice(&compressed[cursor..cursor + len]);
                cursor += len;
            } else if tag == 1 {
                // Repeat run: next 2 bytes count, next byte value
                if cursor + 3 > compressed.len() {
                    return Err(DecompressionCapError::CorruptedInput {
                        reason: "truncated repeat header".to_string(),
                    });
                }
                let count = u16::from_le_bytes([compressed[cursor], compressed[cursor + 1]]) as usize;
                let val = compressed[cursor + 2];
                cursor += 3;
                if output.len() + count > declared_len {
                    return Err(DecompressionCapError::RuntimeBudgetExceeded {
                        bytes_produced: output.len() + count,
                        max_allowed: declared_len,
                    });
                }
                output.resize(output.len() + count, val);
            } else {
                return Err(DecompressionCapError::CorruptedInput {
                    reason: format!("unknown run tag {tag}"),
                });
            }
        }

        Ok(output)
    }
}
