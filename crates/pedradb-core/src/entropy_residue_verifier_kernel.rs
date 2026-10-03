//! kernel: entropy_residue_verifier
//! Shannon entropy and non-zero residue verifier for reclaimed storage slabs and memory buffers.
//!
//! Provides deterministic fixed-point entropy calculation in millibits (1/1000 of a bit)
//! to detect residual cryptographic keys or plaintext fragments in deallocated extents.

use std::fmt;

/// Typed error produced during buffer residue verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidueError {
    /// Non-zero bytes were detected in a buffer required to be pure zero.
    NonZeroResidueDetected { non_zero_count: usize, max_freq: usize },
    /// Shannon entropy exceeded the maximum allowed threshold.
    ExcessiveEntropy { millibits: u32, threshold: u32 },
}

impl fmt::Display for ResidueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonZeroResidueDetected { non_zero_count, max_freq } => {
                write!(
                    f,
                    "Non-zero residue detected: {} non-zero bytes (max freq: {})",
                    non_zero_count, max_freq
                )
            }
            Self::ExcessiveEntropy { millibits, threshold } => {
                write!(
                    f,
                    "Entropy {} millibits exceeded allowed threshold of {} millibits",
                    millibits, threshold
                )
            }
        }
    }
}

impl std::error::Error for ResidueError {}

/// Assessment report for residual entropy analysis on a memory buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntropyResidueReport {
    /// Total buffer size in bytes.
    pub total_bytes: usize,
    /// Number of bytes strictly equal to 0x00.
    pub zero_byte_count: usize,
    /// Number of bytes with non-zero values.
    pub non_zero_byte_count: usize,
    /// Maximum count of any single byte value observed.
    pub max_frequency: usize,
    /// Indicates whether the buffer is 100% pure zeroes (zero entropy).
    pub is_pure_zero: bool,
    /// Estimated Shannon entropy in millibits per byte (0 to 8000 millibits).
    pub estimated_entropy_millibits: u32,
}

/// Verifies memory slabs and reclaimed extents against residual information leakage.
pub struct EntropyResidueVerifier;

impl EntropyResidueVerifier {
    /// Evaluates the byte frequency distribution and Shannon entropy of `buf`.
    pub fn analyze(buf: &[u8]) -> EntropyResidueReport {
        if buf.is_empty() {
            return EntropyResidueReport {
                total_bytes: 0,
                zero_byte_count: 0,
                non_zero_byte_count: 0,
                max_frequency: 0,
                is_pure_zero: true,
                estimated_entropy_millibits: 0,
            };
        }

        let mut counts = [0usize; 256];
        for &b in buf {
            counts[b as usize] += 1;
        }

        let zero_byte_count = counts[0];
        let total_bytes = buf.len();
        let non_zero_byte_count = total_bytes.saturating_sub(zero_byte_count);
        let is_pure_zero = zero_byte_count == total_bytes;

        let mut max_frequency = 0usize;
        for &c in &counts {
            if c > max_frequency {
                max_frequency = c;
            }
        }

        let estimated_entropy_millibits = if is_pure_zero {
            0
        } else {
            Self::compute_shannon_entropy_millibits(&counts, total_bytes)
        };

        let report = EntropyResidueReport {
            total_bytes,
            zero_byte_count,
            non_zero_byte_count,
            max_frequency,
            is_pure_zero,
            estimated_entropy_millibits,
        };

        debug_assert!(Self::verify_internal_invariants(&report));
        report
    }

    /// Determines if the buffer is clean enough to be safely recycled without data leakage.
    /// When `max_allowed_millibits == 0`, requires strict 100% pure zero bytes.
    #[must_use]
    pub fn is_clean_for_reuse(buf: &[u8], max_allowed_millibits: u32) -> bool {
        let report = Self::analyze(buf);
        if max_allowed_millibits == 0 {
            return report.is_pure_zero;
        }
        report.estimated_entropy_millibits <= max_allowed_millibits
    }

    /// Verifies buffer cleanliness returning typed error on violation.
    pub fn verify_buffer_cleanliness(buf: &[u8], max_allowed_millibits: u32) -> Result<(), ResidueError> {
        let report = Self::analyze(buf);
        if max_allowed_millibits == 0 && !report.is_pure_zero {
            return Err(ResidueError::NonZeroResidueDetected {
                non_zero_count: report.non_zero_byte_count,
                max_freq: report.max_frequency,
            });
        }
        if report.estimated_entropy_millibits > max_allowed_millibits {
            return Err(ResidueError::ExcessiveEntropy {
                millibits: report.estimated_entropy_millibits,
                threshold: max_allowed_millibits,
            });
        }
        Ok(())
    }

    /// Verifies mathematical consistency of the entropy residue report.
    #[must_use]
    pub fn verify_internal_invariants(report: &EntropyResidueReport) -> bool {
        if report.zero_byte_count.saturating_add(report.non_zero_byte_count) != report.total_bytes {
            return false;
        }
        if report.is_pure_zero && (report.non_zero_byte_count != 0 || report.estimated_entropy_millibits != 0) {
            return false;
        }
        if report.estimated_entropy_millibits > 8000 {
            return false;
        }
        true
    }

    /// Public helper for 64-bit entropy verification on byte distribution counts.
    pub fn compute_entropy_for_counts(counts: &[usize; 256], n: usize) -> u32 {
        Self::compute_shannon_entropy_millibits(counts, n)
    }

    /// Computes Shannon entropy in millibits using 64-bit precision to prevent truncation on large extents:
    /// H(X) = log2(N) - (1/N) * sum(c_i * log2(c_i)).
    fn compute_shannon_entropy_millibits(counts: &[usize; 256], n: usize) -> u32 {
        if n == 0 {
            return 0;
        }

        let total_c: usize = counts.iter().sum();
        if total_c == 0 || counts.iter().any(|&c| c == total_c) {
            return 0;
        }

        let n_u64 = n as u64;
        let log2_n = Self::integer_log2_millibits_u64(n_u64);
        let mut sum_c_log_c: u128 = 0;

        for &c in counts {
            if c > 0 {
                let log_c = Self::integer_log2_millibits_u64(c as u64) as u128;
                sum_c_log_c = sum_c_log_c.saturating_add((c as u128).saturating_mul(log_c));
            }
        }

        let avg_c_log_c = (sum_c_log_c / (n_u64 as u128)) as u64;
        log2_n.saturating_sub(avg_c_log_c).min(8000) as u32
    }

    /// Deterministic integer log2 in millibits (scaled by 1000) with 64-bit precision.
    fn integer_log2_millibits_u64(x: u64) -> u64 {
        if x <= 1 {
            return 0;
        }

        let leading = 63 - x.leading_zeros();
        let base_milli = (leading as u64).saturating_mul(1000);
        let remainder = x.saturating_sub(1u64 << leading);
        let fraction_milli = ((remainder as u128) * 1000 / (1u128 << leading)) as u64;

        base_milli.saturating_add(fraction_milli)
    }
}
