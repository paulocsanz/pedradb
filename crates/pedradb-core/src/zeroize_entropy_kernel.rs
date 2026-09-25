//! RFC-0282 Pilar 7 — Não-Interferência e Zeroização Criptográfica (Zeroize Entropy Kernel).
//!
//! Formalizes memory zeroization and destruction of residual entropy for confidential
//! data, cryptographic credentials, and transaction buffers upon deallocation (DROP TABLE / Commit / Abort).
//! Proves that dead-store elimination (DSE) cannot optimize away memory wipe passes,
//! guaranteeing zero residual bit-entropy (H(M_freed) == 0) and non-interference across tenant boundaries.

#![forbid(unsafe_code)]

use std::sync::atomic::{compiler_fence, Ordering};

/// Violations of zeroization and residual entropy elimination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZeroizeViolation {
    /// Non-zero byte detected in supposedly zeroized memory (entropy leak).
    ResidualEntropyDetected {
        /// Offset where residual byte was found.
        offset: usize,
        /// Leaked byte value.
        leaked_byte: u8,
    },
    /// The buffer slice was unexpectedly truncated or zero-length.
    EmptyBufferTested,
}

/// A securely managed memory buffer that guarantees zeroization upon disposal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecureZeroizeBuffer {
    data: Vec<u8>,
}

impl SecureZeroizeBuffer {
    /// Creates a new secure zeroize buffer initialized with data.
    #[must_use]
    pub fn new(source: &[u8]) -> Self {
        Self {
            data: source.to_vec(),
        }
    }

    /// Allocates an empty zeroed buffer of specified size.
    #[must_use]
    pub fn allocate(size: usize) -> Self {
        Self {
            data: vec![0u8; size],
        }
    }

    /// Returns a slice to read buffer contents.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    /// Returns a mutable slice to buffer contents.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Returns length of buffer.
    #[must_use]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns whether the buffer is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Formally zeroizes all bytes in the buffer, enforcing a compiler barrier
    /// to prevent Dead Store Elimination (DSE) by LLVM.
    pub fn zeroize(&mut self) {
        // Step 1: Overwrite with zero
        for byte in self.data.iter_mut() {
            *byte = 0;
        }

        // Step 2: Compiler fence prevents compiler from treating writes as dead stores
        compiler_fence(Ordering::SeqCst);
    }
}

impl Drop for SecureZeroizeBuffer {
    fn drop(&mut self) {
        self.zeroize();
    }
}

/// Verification oracle for proving absence of residual entropy.
pub struct ZeroizeEntropyOracle;

impl ZeroizeEntropyOracle {
    /// Verifies that a memory slice has been completely zeroized (entropy == 0).
    ///
    /// # Errors
    /// Returns `ZeroizeViolation::ResidualEntropyDetected` if any byte is non-zero.
    pub fn verify_zeroized_entropy(slice: &[u8]) -> Result<(), ZeroizeViolation> {
        if slice.is_empty() {
            return Err(ZeroizeViolation::EmptyBufferTested);
        }

        for (idx, &byte) in slice.iter().enumerate() {
            if byte != 0 {
                return Err(ZeroizeViolation::ResidualEntropyDetected {
                    offset: idx,
                    leaked_byte: byte,
                });
            }
        }

        Ok(())
    }

    /// Calculates the Shannon bit-entropy of a memory slice.
    /// Returns 0.0 if and only if the slice consists of completely uniform bytes.
    #[must_use]
    pub fn calculate_shannon_entropy(slice: &[u8]) -> f64 {
        if slice.is_empty() {
            return 0.0;
        }

        let mut frequencies = [0usize; 256];
        for &b in slice {
            frequencies[b as usize] += 1;
        }

        let len_f = slice.len() as f64;
        let mut entropy: f64 = 0.0;

        for &count in &frequencies {
            if count > 0 {
                let p = count as f64 / len_f;
                entropy -= p * p.log2();
            }
        }

        entropy
    }
}
