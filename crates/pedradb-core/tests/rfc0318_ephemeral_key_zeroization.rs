//! RFC-0318: Volatile Key Zeroization Sentinel test suite.
//!
//! Verifies:
//! - Complete volatile zeroization across buffer extents.
//! - Constant-time detection of non-zero residual bytes.
//! - Correct index reporting without timing early-exit side channels.
//! - Combined scrub_and_verify lifecycle.

use pedradb_core::ephemeral_key_zeroization_kernel::{
    VolatileScrubber, ZeroizationError,
};

#[test]
fn test_volatile_scrub_and_verification_green() {
    let mut secret_key = [0x5Au8; 64];
    assert!(!VolatileScrubber::is_all_zero(&secret_key));

    let scrubbed = VolatileScrubber::scrub_and_verify(&mut secret_key).expect("scrub and verify");
    assert_eq!(scrubbed, 64);
    assert!(VolatileScrubber::is_all_zero(&secret_key));
    assert!(secret_key.iter().all(|&b| b == 0));
}

#[test]
fn test_constant_time_detection_of_corrupt_residuals() {
    let mut buf = [0u8; 128];
    assert!(VolatileScrubber::verify_zeroized_constant_time(&buf).is_ok());

    // Single non-zero byte at index 0
    buf[0] = 0x01;
    assert_eq!(
        VolatileScrubber::verify_zeroized_constant_time(&buf),
        Err(ZeroizationError::NonZeroByteDetected { index: 0, value: 0x01 })
    );

    // Single non-zero byte in the middle
    buf[0] = 0;
    buf[63] = 0xFF;
    assert_eq!(
        VolatileScrubber::verify_zeroized_constant_time(&buf),
        Err(ZeroizationError::NonZeroByteDetected { index: 63, value: 0xFF })
    );

    // Single non-zero byte at the end
    buf[63] = 0;
    buf[127] = 0x42;
    assert_eq!(
        VolatileScrubber::verify_zeroized_constant_time(&buf),
        Err(ZeroizationError::NonZeroByteDetected { index: 127, value: 0x42 })
    );
}

#[test]
fn test_empty_buffer_handling() {
    let mut empty: [u8; 0] = [];
    assert_eq!(VolatileScrubber::scrub_bytes(&mut empty), 0);
    assert_eq!(
        VolatileScrubber::verify_zeroized_constant_time(&empty),
        Err(ZeroizationError::BufferEmpty)
    );
}

#[test]
fn test_ephemeral_key_guard_raii_zeroization_red() {
    use pedradb_core::ephemeral_key_zeroization_kernel::EphemeralKeyGuard;

    // 1. Error implements std::error::Error
    let err: Box<dyn std::error::Error> = Box::new(ZeroizationError::BufferEmpty);
    assert!(!err.to_string().is_empty());

    // 2. RAII Zeroization Guard: drops and scrubs buffer automatically
    let mut key_material = vec![0xDE, 0xAD, 0xBE, 0xEF, 0xAA, 0x55];
    {
        let mut guard = EphemeralKeyGuard::new(&mut key_material);
        assert_eq!(guard.as_slice(), &[0xDE, 0xAD, 0xBE, 0xEF, 0xAA, 0x55]);
        // Modify via guard
        guard.as_mut_slice()[0] = 0x42;
    } // guard dropped here

    // Key material must be 100% zeroed out automatically!
    assert!(VolatileScrubber::is_all_zero(&key_material));
    assert_eq!(key_material, vec![0, 0, 0, 0, 0, 0]);
}

