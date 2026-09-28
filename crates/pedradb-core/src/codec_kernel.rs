//! Safe cursor and zero-panic decoding primitives (RFC-0298 P0.1).
//!
//! Replaces bare slice indexing and unchecked arithmetic in I/O decoders
//! with checked arithmetic, bounded allocations, and strictly validated buffers.

#![forbid(unsafe_code)]

use std::fmt;

/// Errors arising during safe deserialization of disk and wire formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Unexpected end of input while reading required bytes.
    UnexpectedEof {
        /// Number of bytes requested.
        needed: usize,
        /// Number of bytes remaining in the buffer.
        remaining: usize,
    },
    /// An arithmetic calculation during slicing or offset advancing would overflow.
    IntegerOverflow,
    /// Unconsumed trailing bytes when strict frame termination was expected.
    TrailingGarbage {
        /// Number of unconsumed bytes.
        remaining: usize,
    },
    /// Field length exceeds maximum permitted or remaining buffer capacity.
    LengthExceedsCapacity {
        /// Specified length.
        len: usize,
        /// Maximum allowable or remaining bytes.
        max_allowed: usize,
    },
    /// Invalid magic bytes or format tag.
    InvalidTag {
        /// Tag byte encountered.
        tag: u8,
    },
    /// Generic corruption error message.
    Corrupt(String),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof { needed, remaining } => {
                write!(f, "unexpected EOF: needed {needed} bytes, only {remaining} available")
            }
            Self::IntegerOverflow => write!(f, "integer overflow in buffer offset arithmetic"),
            Self::TrailingGarbage { remaining } => {
                write!(f, "trailing garbage detected: {remaining} unconsumed bytes")
            }
            Self::LengthExceedsCapacity { len, max_allowed } => {
                write!(f, "length {len} exceeds maximum allowed {max_allowed}")
            }
            Self::InvalidTag { tag } => write!(f, "invalid wire/format tag: {tag:#04x}"),
            Self::Corrupt(msg) => write!(f, "format corruption: {msg}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// A bounds-checked cursor over an immutable byte slice.
#[derive(Clone, Copy, Debug)]
pub struct SafeCursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> SafeCursor<'a> {
    /// Creates a new cursor starting at offset 0.
    #[must_use]
    #[inline]
    pub const fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Current read position in the underlying slice.
    #[must_use]
    #[inline]
    pub const fn position(&self) -> usize {
        self.pos
    }

    /// Sets cursor position with bounds checking.
    pub fn set_position(&mut self, pos: usize) -> Result<(), DecodeError> {
        if pos <= self.buf.len() {
            self.pos = pos;
            Ok(())
        } else {
            Err(DecodeError::UnexpectedEof {
                needed: pos,
                remaining: self.buf.len(),
            })
        }
    }

    /// Number of bytes remaining to be read.
    #[must_use]
    #[inline]
    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    /// True if the cursor has reached or exceeded the buffer end.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.pos >= self.buf.len()
    }

    /// True if the cursor has reached or exceeded the buffer end (alias for is_empty).
    #[must_use]
    #[inline]
    pub fn is_eof(&self) -> bool {
        self.is_empty()
    }

    /// Returns the remaining slice of unread bytes.
    #[must_use]
    #[inline]
    pub fn remaining_slice(&self) -> &'a [u8] {
        if self.pos >= self.buf.len() {
            &[]
        } else {
            &self.buf[self.pos..]
        }
    }

    /// Total underlying slice length.
    #[must_use]
    #[inline]
    pub const fn total_len(&self) -> usize {
        self.buf.len()
    }

    /// Reads exactly `len` bytes from the cursor.
    pub fn read_exact(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        let new_pos = self.pos.checked_add(len).ok_or(DecodeError::IntegerOverflow)?;
        if new_pos > self.buf.len() {
            return Err(DecodeError::UnexpectedEof {
                needed: len,
                remaining: self.remaining(),
            });
        }
        let slice = &self.buf[self.pos..new_pos];
        self.pos = new_pos;
        Ok(slice)
    }

    /// Advances the cursor by `len` bytes without returning a slice.
    pub fn advance(&mut self, len: usize) -> Result<(), DecodeError> {
        let new_pos = self.pos.checked_add(len).ok_or(DecodeError::IntegerOverflow)?;
        if new_pos > self.buf.len() {
            return Err(DecodeError::UnexpectedEof {
                needed: len,
                remaining: self.remaining(),
            });
        }
        self.pos = new_pos;
        Ok(())
    }

    /// Reads a single byte.
    pub fn read_u8(&mut self) -> Result<u8, DecodeError> {
        let slice = self.read_exact(1)?;
        Ok(slice[0])
    }

    /// Peeks a single byte without advancing the cursor.
    pub fn peek_u8(&self) -> Result<u8, DecodeError> {
        if self.pos < self.buf.len() {
            Ok(self.buf[self.pos])
        } else {
            Err(DecodeError::UnexpectedEof {
                needed: 1,
                remaining: 0,
            })
        }
    }

    /// Reads a boolean (0 = false, non-zero = true).
    pub fn read_bool(&mut self) -> Result<bool, DecodeError> {
        Ok(self.read_u8()? != 0)
    }

    /// Reads a 16-bit little-endian integer.
    pub fn read_u16_le(&mut self) -> Result<u16, DecodeError> {
        let b = self.read_exact(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    /// Reads a 32-bit little-endian integer.
    pub fn read_u32_le(&mut self) -> Result<u32, DecodeError> {
        let b = self.read_exact(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Reads a 64-bit little-endian integer.
    pub fn read_u64_le(&mut self) -> Result<u64, DecodeError> {
        let b = self.read_exact(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    /// Reads a 128-bit little-endian integer.
    pub fn read_u128_le(&mut self) -> Result<u128, DecodeError> {
        let b = self.read_exact(16)?;
        let mut arr = [0u8; 16];
        arr.copy_from_slice(b);
        Ok(u128::from_le_bytes(arr))
    }

    /// Reads a 16-bit big-endian integer.
    pub fn read_u16_be(&mut self) -> Result<u16, DecodeError> {
        let b = self.read_exact(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    /// Reads a 32-bit big-endian integer.
    pub fn read_u32_be(&mut self) -> Result<u32, DecodeError> {
        let b = self.read_exact(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Reads a 64-bit big-endian integer.
    pub fn read_u64_be(&mut self) -> Result<u64, DecodeError> {
        let b = self.read_exact(8)?;
        Ok(u64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    /// Reads a 128-bit big-endian integer.
    pub fn read_u128_be(&mut self) -> Result<u128, DecodeError> {
        let b = self.read_exact(16)?;
        let mut arr = [0u8; 16];
        arr.copy_from_slice(b);
        Ok(u128::from_be_bytes(arr))
    }

    /// Reads a `u32 LE` length prefix followed by that many bytes.
    /// Rejects lengths exceeding `max_len` or remaining bytes.
    pub fn read_length_prefixed_bytes(&mut self, max_len: usize) -> Result<&'a [u8], DecodeError> {
        let len = self.read_u32_le()? as usize;
        let rem = self.remaining();
        if len > max_len || len > rem {
            return Err(DecodeError::LengthExceedsCapacity {
                len,
                max_allowed: max_len.min(rem),
            });
        }
        self.read_exact(len)
    }

    /// Reads a `u64 LE` length prefix followed by that many bytes.
    pub fn read_length_prefixed_bytes_u64(&mut self, max_len: usize) -> Result<&'a [u8], DecodeError> {
        let len_u64 = self.read_u64_le()?;
        let len = usize::try_from(len_u64).map_err(|_| DecodeError::IntegerOverflow)?;
        let rem = self.remaining();
        if len > max_len || len > rem {
            return Err(DecodeError::LengthExceedsCapacity {
                len,
                max_allowed: max_len.min(rem),
            });
        }
        self.read_exact(len)
    }

    /// Enforces that the cursor has consumed the entire buffer.
    /// Returns `Err(TrailingGarbage)` if any bytes remain.
    pub fn ensure_fully_consumed(&self) -> Result<(), DecodeError> {
        if !self.is_empty() {
            Err(DecodeError::TrailingGarbage {
                remaining: self.remaining(),
            })
        } else {
            Ok(())
        }
    }

    /// Alias for [`Self::ensure_fully_consumed`].
    pub fn ensure_exact_exhaustion(&self) -> Result<(), DecodeError> {
        self.ensure_fully_consumed()
    }

    /// Calculates safe `with_capacity` bound preventing pre-allocation DoS (RFC-0298 P1.1).
    pub fn bounded_capacity(
        count: usize,
        min_elem_size: usize,
        rem: usize,
        hard_cap: usize,
    ) -> Result<usize, DecodeError> {
        let min_size = min_elem_size.max(1);
        let max_possible = rem / min_size;
        if count > max_possible || count > hard_cap {
            return Err(DecodeError::LengthExceedsCapacity {
                len: count,
                max_allowed: max_possible.min(hard_cap),
            });
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_cursor_basics() {
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let mut cur = SafeCursor::new(&data);
        assert_eq!(cur.read_u8().unwrap(), 1);
        assert_eq!(cur.read_u16_le().unwrap(), 0x0302);
        assert_eq!(cur.read_u32_be().unwrap(), 0x0405_0607);
        assert_eq!(cur.remaining(), 3);
        assert_eq!(cur.read_exact(3).unwrap(), &[8, 9, 10]);
        assert!(cur.ensure_fully_consumed().is_ok());
    }

    #[test]
    fn test_safe_cursor_overflow_and_eof() {
        let data = [1u8, 2, 3];
        let mut cur = SafeCursor::new(&data);
        assert_eq!(cur.read_u8().unwrap(), 1);
        assert!(matches!(
            cur.read_exact(5),
            Err(DecodeError::UnexpectedEof { .. })
        ));
        assert!(matches!(
            cur.read_exact(usize::MAX),
            Err(DecodeError::IntegerOverflow)
        ));
    }

    #[test]
    fn test_trailing_garbage() {
        let data = [1u8, 2, 3];
        let mut cur = SafeCursor::new(&data);
        assert_eq!(cur.read_u8().unwrap(), 1);
        assert!(matches!(
            cur.ensure_fully_consumed(),
            Err(DecodeError::TrailingGarbage { remaining: 2 })
        ));
    }
}
