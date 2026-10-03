//! kernel: ephemeral_key_zeroization
//! Volatile memory scrubber and zeroization verification sentinel.
//!
//! Enforces zero-entropy byte wiping resilient against dead-store elimination (DSE)
//! compiler passes, with constant-time non-zero accumulator verification.

use core::sync::atomic::{compiler_fence, Ordering};

/// Typed errors produced during volatile zeroization and verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZeroizationError {
    /// Zeroization verification failed: non-zero byte was found at index.
    NonZeroByteDetected { index: usize, value: u8 },
    /// Memory buffer provided is empty.
    BufferEmpty,
}

impl core::fmt::Display for ZeroizationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NonZeroByteDetected { index, value } => {
                write!(f, "Verification failed: non-zero byte 0x{:02x} at index {}", value, index)
            }
            Self::BufferEmpty => write!(f, "Buffer is empty"),
        }
    }
}

impl std::error::Error for ZeroizationError {}

/// Pure volatile memory scrubber and verification sentinel.
pub struct VolatileScrubber;

impl VolatileScrubber {
    /// Scrub `buf` using volatile compiler fences and black-box side-effects to prevent DSE.
    ///
    /// Returns the total count of bytes zeroized.
    pub fn scrub_bytes(buf: &mut [u8]) -> usize {
        if buf.is_empty() {
            return 0;
        }

        // Establish compiler ordering barrier before write operations
        compiler_fence(Ordering::SeqCst);

        for byte_ref in buf.iter_mut() {
            *byte_ref = 0u8;
            core::hint::black_box(*byte_ref);
        }

        core::hint::black_box(&*buf);

        // Prevent subsequent reordering or elision of preceding writes
        compiler_fence(Ordering::SeqCst);

        buf.len()
    }

    /// Verifies in constant time that all bytes in `buf` are strictly zero.
    ///
    /// Accumulates all bitwise ORs across the entire buffer without early branching
    /// to mitigate timing side-channel leakage.
    pub fn verify_zeroized_constant_time(buf: &[u8]) -> Result<(), ZeroizationError> {
        if buf.is_empty() {
            return Err(ZeroizationError::BufferEmpty);
        }

        let mut accumulator: u8 = 0;
        for &b in buf {
            accumulator |= b;
        }
        core::hint::black_box(accumulator);

        if accumulator == 0 {
            Ok(())
        } else {
            // Find specific index for diagnostic logging only after timing-neutral accumulation
            for (idx, &b) in buf.iter().enumerate() {
                if b != 0 {
                    return Err(ZeroizationError::NonZeroByteDetected {
                        index: idx,
                        value: b,
                    });
                }
            }
            Err(ZeroizationError::NonZeroByteDetected { index: 0, value: accumulator })
        }
    }

    /// Performs volatile scrubbing followed by immediate constant-time verification.
    pub fn scrub_and_verify(buf: &mut [u8]) -> Result<usize, ZeroizationError> {
        let len = Self::scrub_bytes(buf);
        Self::verify_zeroized_constant_time(buf)?;
        Ok(len)
    }

    /// Returns `true` if every byte in `buf` is 0x00.
    #[must_use]
    pub fn is_all_zero(buf: &[u8]) -> bool {
        buf.iter().all(|&b| b == 0)
    }
}

/// RAII Guard that wraps a mutable slice containing sensitive key material
/// and automatically scrubs it on drop to prevent memory retention bugs.
pub struct EphemeralKeyGuard<'a> {
    buffer: &'a mut [u8],
}

impl<'a> EphemeralKeyGuard<'a> {
    pub fn new(buffer: &'a mut [u8]) -> Self {
        Self { buffer }
    }

    pub fn as_slice(&self) -> &[u8] {
        self.buffer
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.buffer
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

impl<'a> Drop for EphemeralKeyGuard<'a> {
    fn drop(&mut self) {
        VolatileScrubber::scrub_bytes(self.buffer);
    }
}

/// Owning RAII buffer that zeroizes on drop.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ZeroizingVec {
    inner: Vec<u8>,
}

impl ZeroizingVec {
    pub fn new(inner: Vec<u8>) -> Self {
        Self { inner }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.inner
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.inner
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl Drop for ZeroizingVec {
    fn drop(&mut self) {
        VolatileScrubber::scrub_bytes(&mut self.inner);
    }
}
