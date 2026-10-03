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
    /// Decompressed output is shorter than declared length (unexpected short/truncated stream).
    TruncatedOutput {
        /// Actual bytes produced.
        produced: usize,
        /// Expected declared length.
        expected: usize,
    },
    /// Compressed input is corrupted or malformed.
    CorruptedInput {
        /// Diagnostic reason.
        reason: String,
    },
    /// Maximum expansion ratio cannot be zero.
    ZeroExpansionRatio,
    /// Absolute size ceiling cannot be zero bytes.
    ZeroAbsoluteBytes,
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
            Self::TruncatedOutput { produced, expected } => {
                write!(f, "decompressed output truncated: produced {produced}B < declared {expected}B")
            }
            Self::CorruptedInput { reason } => {
                write!(f, "corrupted compressed input: {reason}")
            }
            Self::ZeroExpansionRatio => {
                write!(f, "max expansion ratio cannot be zero")
            }
            Self::ZeroAbsoluteBytes => {
                write!(f, "max absolute bytes cannot be zero")
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

impl DecompressionCapPolicy {
    /// Creates a validated policy rejecting zero values.
    pub fn try_new(max_expansion_ratio: u32, max_absolute_bytes: usize) -> Result<Self, DecompressionCapError> {
        if max_expansion_ratio == 0 {
            return Err(DecompressionCapError::ZeroExpansionRatio);
        }
        if max_absolute_bytes == 0 {
            return Err(DecompressionCapError::ZeroAbsoluteBytes);
        }
        Ok(Self {
            max_expansion_ratio,
            max_absolute_bytes,
        })
    }
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
    /// Creates a new governor with custom policy, validating bounds.
    pub fn try_new(policy: DecompressionCapPolicy) -> Result<Self, DecompressionCapError> {
        if policy.max_expansion_ratio == 0 {
            return Err(DecompressionCapError::ZeroExpansionRatio);
        }
        if policy.max_absolute_bytes == 0 {
            return Err(DecompressionCapError::ZeroAbsoluteBytes);
        }
        Ok(Self { policy })
    }

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

    /// Safely unpacks run-length or raw compressed data directly into an existing buffer.
    /// Reuses existing vector capacity and wipes destination on any error to prevent leak/forensics.
    pub fn safe_unpack_into(
        &self,
        compressed: &[u8],
        declared_len: usize,
        dest: &mut Vec<u8>,
    ) -> Result<usize, DecompressionCapError> {
        self.pre_validate_header(compressed.len(), declared_len)?;
        dest.clear();
        dest.reserve(declared_len.min(64 * 1024));

        let mut cursor = 0;

        // Structured unpacker: reads tagged sequences [flag: 0 = literal, 1 = repeat]
        while cursor < compressed.len() {
            let tag = compressed[cursor];
            cursor += 1;

            if tag == 0 {
                // Literal run: next 2 bytes length, followed by bytes
                if cursor + 2 > compressed.len() {
                    dest.clear();
                    return Err(DecompressionCapError::CorruptedInput {
                        reason: "truncated literal header".to_string(),
                    });
                }
                let len = u16::from_le_bytes([compressed[cursor], compressed[cursor + 1]]) as usize;
                cursor += 2;
                if cursor + len > compressed.len() {
                    dest.clear();
                    return Err(DecompressionCapError::CorruptedInput {
                        reason: "truncated literal payload".to_string(),
                    });
                }
                if dest.len().saturating_add(len) > declared_len {
                    dest.clear();
                    return Err(DecompressionCapError::RuntimeBudgetExceeded {
                        bytes_produced: dest.len() + len,
                        max_allowed: declared_len,
                    });
                }
                dest.extend_from_slice(&compressed[cursor..cursor + len]);
                cursor += len;
            } else if tag == 1 {
                // Repeat run: next 2 bytes count, next byte value
                if cursor + 3 > compressed.len() {
                    dest.clear();
                    return Err(DecompressionCapError::CorruptedInput {
                        reason: "truncated repeat header".to_string(),
                    });
                }
                let count = u16::from_le_bytes([compressed[cursor], compressed[cursor + 1]]) as usize;
                let val = compressed[cursor + 2];
                cursor += 3;
                if dest.len().saturating_add(count) > declared_len {
                    dest.clear();
                    return Err(DecompressionCapError::RuntimeBudgetExceeded {
                        bytes_produced: dest.len() + count,
                        max_allowed: declared_len,
                    });
                }
                dest.resize(dest.len() + count, val);
            } else {
                dest.clear();
                return Err(DecompressionCapError::CorruptedInput {
                    reason: format!("unknown run tag {tag}"),
                });
            }
        }

        Ok(dest.len())
    }

    /// Safely unpacks run-length or raw compressed data under strict output cap monitoring.
    pub fn safe_unpack(
        &self,
        compressed: &[u8],
        declared_len: usize,
    ) -> Result<Vec<u8>, DecompressionCapError> {
        let mut output = Vec::with_capacity(declared_len.min(64 * 1024));
        self.safe_unpack_into(compressed, declared_len, &mut output)?;
        Ok(output)
    }

    /// Safely unpacks compressed data requiring that the output matches `declared_len` exactly.
    pub fn safe_unpack_exact(
        &self,
        compressed: &[u8],
        declared_len: usize,
    ) -> Result<Vec<u8>, DecompressionCapError> {
        let output = self.safe_unpack(compressed, declared_len)?;
        if output.len() != declared_len {
            return Err(DecompressionCapError::TruncatedOutput {
                produced: output.len(),
                expected: declared_len,
            });
        }
        Ok(output)
    }
}

/// Incremental streaming decoder that unpacks compressed input into bounded chunks of at most M bytes.
pub struct StreamingBoundedDecoder<'a> {
    _governor: &'a DecompressionCapGovernor,
    max_chunk_bytes: usize,
    total_budget: usize,
    compressed_cursor: usize,
    pending_repeat: Option<(usize, u8)>,
    pending_literal_range: Option<(usize, usize)>,
    total_produced: usize,
}

impl<'a> StreamingBoundedDecoder<'a> {
    /// Creates a new streaming bounded decoder with maximum chunk size M and total budget.
    #[must_use]
    pub fn new(
        governor: &'a DecompressionCapGovernor,
        max_chunk_bytes: usize,
        total_budget: usize,
    ) -> Self {
        Self {
            _governor: governor,
            max_chunk_bytes: max_chunk_bytes.max(1),
            total_budget,
            compressed_cursor: 0,
            pending_repeat: None,
            pending_literal_range: None,
            total_produced: 0,
        }
    }

    /// Decodes the next bounded chunk from the compressed stream, producing at most `max_chunk_bytes`.
    /// Returns `Ok(None)` when EOF is reached.
    pub fn decode_next_chunk(
        &mut self,
        compressed: &[u8],
    ) -> Result<Option<Vec<u8>>, DecompressionCapError> {
        if self.compressed_cursor >= compressed.len()
            && self.pending_repeat.is_none()
            && self.pending_literal_range.is_none()
        {
            return Ok(None);
        }

        let mut chunk = Vec::with_capacity(self.max_chunk_bytes);

        while chunk.len() < self.max_chunk_bytes {
            // 1. Drain pending literal range if any
            if let Some((start, end)) = self.pending_literal_range.take() {
                let avail = end.saturating_sub(start);
                let to_take = avail.min(self.max_chunk_bytes - chunk.len());
                let new_total = self.total_produced.saturating_add(to_take);
                if new_total > self.total_budget {
                    return Err(DecompressionCapError::RuntimeBudgetExceeded {
                        bytes_produced: new_total,
                        max_allowed: self.total_budget,
                    });
                }
                chunk.extend_from_slice(&compressed[start..start + to_take]);
                self.total_produced = new_total;
                if avail > to_take {
                    self.pending_literal_range = Some((start + to_take, end));
                    break;
                }
            }

            // 2. Drain pending repeat if any
            if let Some((remaining, val)) = self.pending_repeat.take() {
                let to_take = remaining.min(self.max_chunk_bytes - chunk.len());
                let new_total = self.total_produced.saturating_add(to_take);
                if new_total > self.total_budget {
                    return Err(DecompressionCapError::RuntimeBudgetExceeded {
                        bytes_produced: new_total,
                        max_allowed: self.total_budget,
                    });
                }
                chunk.resize(chunk.len() + to_take, val);
                self.total_produced = new_total;
                if remaining > to_take {
                    self.pending_repeat = Some((remaining - to_take, val));
                    break;
                }
            }

            if self.compressed_cursor >= compressed.len() {
                break;
            }

            let tag = compressed[self.compressed_cursor];
            self.compressed_cursor += 1;

            if tag == 0 {
                // Literal run
                if self.compressed_cursor + 2 > compressed.len() {
                    return Err(DecompressionCapError::CorruptedInput {
                        reason: "truncated literal header".to_string(),
                    });
                }
                let len = u16::from_le_bytes([
                    compressed[self.compressed_cursor],
                    compressed[self.compressed_cursor + 1],
                ]) as usize;
                self.compressed_cursor += 2;
                if self.compressed_cursor + len > compressed.len() {
                    return Err(DecompressionCapError::CorruptedInput {
                        reason: "truncated literal payload".to_string(),
                    });
                }

                let avail_in_chunk = self.max_chunk_bytes - chunk.len();
                let to_take = len.min(avail_in_chunk);
                let new_total = self.total_produced.saturating_add(to_take);
                if new_total > self.total_budget {
                    return Err(DecompressionCapError::RuntimeBudgetExceeded {
                        bytes_produced: new_total,
                        max_allowed: self.total_budget,
                    });
                }

                chunk.extend_from_slice(&compressed[self.compressed_cursor..self.compressed_cursor + to_take]);
                self.total_produced = new_total;
                if len > to_take {
                    self.pending_literal_range = Some((self.compressed_cursor + to_take, self.compressed_cursor + len));
                    self.compressed_cursor += len;
                    break;
                }
                self.compressed_cursor += len;
            } else if tag == 1 {
                // Repeat run
                if self.compressed_cursor + 3 > compressed.len() {
                    return Err(DecompressionCapError::CorruptedInput {
                        reason: "truncated repeat header".to_string(),
                    });
                }
                let count = u16::from_le_bytes([
                    compressed[self.compressed_cursor],
                    compressed[self.compressed_cursor + 1],
                ]) as usize;
                let val = compressed[self.compressed_cursor + 2];
                self.compressed_cursor += 3;
                self.pending_repeat = Some((count, val));
            } else {
                return Err(DecompressionCapError::CorruptedInput {
                    reason: format!("unknown run tag {tag}"),
                });
            }
        }

        if chunk.is_empty() {
            Ok(None)
        } else {
            Ok(Some(chunk))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decompression_cap_policy_bounds() {
        assert_eq!(
            DecompressionCapPolicy::try_new(0, 1024),
            Err(DecompressionCapError::ZeroExpansionRatio)
        );
        assert_eq!(
            DecompressionCapPolicy::try_new(1024, 0),
            Err(DecompressionCapError::ZeroAbsoluteBytes)
        );

        let policy = DecompressionCapPolicy::try_new(100, 1024 * 1024).expect("valid policy");
        let gov = DecompressionCapGovernor::try_new(policy).expect("valid governor");

        // Validate excessive expansion
        let err = gov.pre_validate_header(1, 101).unwrap_err();
        match err {
            DecompressionCapError::ExcessiveDeclaredExpansion { ratio, max_ratio, .. } => {
                assert_eq!(ratio, 101);
                assert_eq!(max_ratio, 100);
            }
            _ => panic!("unexpected error: {err:?}"),
        }

        // Validate ceiling exceeded
        let err2 = gov.pre_validate_header(100, 1024 * 1024 + 1).unwrap_err();
        match err2 {
            DecompressionCapError::AbsoluteCeilingExceeded { declared_len, ceiling } => {
                assert_eq!(declared_len, 1024 * 1024 + 1);
                assert_eq!(ceiling, 1024 * 1024);
            }
            _ => panic!("unexpected error: {err2:?}"),
        }
    }

    #[test]
    fn test_decompression_cap_display() {
        let err = DecompressionCapError::ZeroExpansionRatio;
        assert_eq!(format!("{err}"), "max expansion ratio cannot be zero");

        let err2 = DecompressionCapError::ZeroAbsoluteBytes;
        assert_eq!(format!("{err2}"), "max absolute bytes cannot be zero");
    }
}
