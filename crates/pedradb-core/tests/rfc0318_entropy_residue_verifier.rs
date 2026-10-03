//! RFC-0318: Shannon Entropy Residue Verifier test suite.
//!
//! Verifies:
//! - Pure zero buffer yields exactly 0 millibits entropy and passes clean reuse.
//! - Non-zero residue increases entropy and triggers reuse rejection.
//! - Uniform high-entropy buffer (simulated key material) exhibits near 8000 millibits entropy.
//! - Mathematical consistency invariants of the EntropyResidueReport.

use pedradb_core::entropy_residue_verifier_kernel::EntropyResidueVerifier;

#[test]
fn test_pure_zero_buffer_entropy_zero() {
    let zeros = [0u8; 1024];
    let report = EntropyResidueVerifier::analyze(&zeros);

    assert_eq!(report.total_bytes, 1024);
    assert_eq!(report.zero_byte_count, 1024);
    assert_eq!(report.non_zero_byte_count, 0);
    assert_eq!(report.max_frequency, 1024);
    assert!(report.is_pure_zero);
    assert_eq!(report.estimated_entropy_millibits, 0);
    assert!(EntropyResidueVerifier::verify_internal_invariants(&report));
    assert!(EntropyResidueVerifier::is_clean_for_reuse(&zeros, 0));
}

#[test]
fn test_high_entropy_key_material_detection() {
    // 256 distinct bytes repeated 4 times = perfectly uniform distribution
    let mut high_entropy = Vec::with_capacity(1024);
    for _ in 0..4 {
        for b in 0..=255u8 {
            high_entropy.push(b);
        }
    }

    let report = EntropyResidueVerifier::analyze(&high_entropy);
    assert_eq!(report.total_bytes, 1024);
    assert_eq!(report.zero_byte_count, 4);
    assert_eq!(report.non_zero_byte_count, 1020);
    assert_eq!(report.max_frequency, 4);
    assert!(!report.is_pure_zero);
    // Theoretical max entropy is 8.000 bits = 8000 millibits
    assert!(
        report.estimated_entropy_millibits >= 7800,
        "Entropy must be near 8000 millibits for uniform data, got {}",
        report.estimated_entropy_millibits
    );
    assert!(EntropyResidueVerifier::verify_internal_invariants(&report));

    // Must be rejected as not clean for reuse with 0 tolerance
    assert!(!EntropyResidueVerifier::is_clean_for_reuse(&high_entropy, 100));
}

#[test]
fn test_sparse_non_zero_residue_detection() {
    let mut sparse = vec![0u8; 1000];
    sparse[500] = 0xAB; // Single residual non-zero byte

    let report = EntropyResidueVerifier::analyze(&sparse);
    assert_eq!(report.total_bytes, 1000);
    assert_eq!(report.zero_byte_count, 999);
    assert_eq!(report.non_zero_byte_count, 1);
    assert!(!report.is_pure_zero);
    assert!(EntropyResidueVerifier::verify_internal_invariants(&report));
    assert!(!EntropyResidueVerifier::is_clean_for_reuse(&sparse, 0));
}

#[test]
fn test_empty_buffer_entropy() {
    let report = EntropyResidueVerifier::analyze(&[]);
    assert_eq!(report.total_bytes, 0);
    assert_eq!(report.estimated_entropy_millibits, 0);
    assert!(report.is_pure_zero);
    assert!(EntropyResidueVerifier::verify_internal_invariants(&report));
}

#[test]
fn test_entropy_residue_red_invariants() {
    use pedradb_core::entropy_residue_verifier_kernel::ResidueError;

    // 1. Error implements std::error::Error
    let err: Box<dyn std::error::Error> = Box::new(ResidueError::ExcessiveEntropy {
        millibits: 500,
        threshold: 100,
    });
    assert!(!err.to_string().is_empty());

    // 2. Critical: verify_buffer_cleanliness with threshold 0 must reject ANY non-zero byte
    let mut sparse_large = vec![0u8; 100_000];
    sparse_large[42] = 0x01; // single non-zero byte in large buffer
    let res = EntropyResidueVerifier::verify_buffer_cleanliness(&sparse_large, 0);
    assert!(matches!(res, Err(ResidueError::NonZeroResidueDetected { .. })));
    assert!(!EntropyResidueVerifier::is_clean_for_reuse(&sparse_large, 0));

    // 3. 64-bit log2 verification avoids 32-bit truncation
    let counts = [0usize; 256];
    let entropy_large = EntropyResidueVerifier::compute_entropy_for_counts(&counts, 5_000_000_000usize);
    assert_eq!(entropy_large, 0);
}

