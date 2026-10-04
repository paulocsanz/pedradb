//! RFC-0311: Deterministic Verification Suite for SST Block Decompression Bomb & Varint Safe Parser Guard.
//!
//! Enforces zero-twin production verification of:
//! 1. Memory exhaustion prevention against malicious/corrupted SST blocks (DoS/OOM).
//! 2. Expansion ratio explosion ceilings (zip bombs).
//! 3. Zero-allocation rejection before CRC validation and bounded size checks.
//! 4. Malformed and adversarial restart array detection (overflows, out-of-bounds, non-monotonic).
//! 5. Constant-time bounded varint parser robustness against infinite continuation bits and overflows.

use pedradb_core::sst_block_decompression_guard_kernel::{
    BlockDecompressionGuardConfig, DecompressionGuardError, SafeBlockDecoder,
};

#[test]
fn test_valid_block_decompression_and_restart_extraction() {
    let mut plain_block = Vec::new();
    let entries = [
        (b"user_alpha".to_vec(), b"val_alpha".to_vec()),
        (b"user_beta".to_vec(), b"val_beta".to_vec()),
        (b"user_gamma".to_vec(), b"val_gamma".to_vec()),
    ];

    let mut restart_offsets = Vec::new();
    for (k, v) in &entries {
        restart_offsets.push(plain_block.len() as u32);
        plain_block.extend_from_slice(&(k.len() as u32).to_le_bytes());
        plain_block.extend_from_slice(k);
        plain_block.extend_from_slice(&(v.len() as u32).to_le_bytes());
        plain_block.extend_from_slice(v);
    }

    // Append restart array: [offset_0, offset_1, ..., num_restarts]
    for &off in &restart_offsets {
        plain_block.extend_from_slice(&off.to_le_bytes());
    }
    plain_block.extend_from_slice(&(restart_offsets.len() as u32).to_le_bytes());

    // Compress plain_block with LZ4 prepended with uncompressed size
    let lz4_payload = lz4_flex::compress_prepend_size(&plain_block);

    // Append CRC32C trailer
    let crc = crc32c::crc32c(&lz4_payload);
    let mut raw_block = lz4_payload.clone();
    raw_block.extend_from_slice(&crc.to_le_bytes());

    // 1. Verify CRC trailer
    let verified_body = SafeBlockDecoder::split_and_verify_crc(&raw_block, true)
        .expect("CRC validation must succeed on pristine block");
    assert_eq!(verified_body, lz4_payload.as_slice());

    // 2. Decompress bounded
    let config = BlockDecompressionGuardConfig::default();
    let mut scratch = Vec::new();
    let decompressed_len = SafeBlockDecoder::decompress_lz4_into(verified_body, &config, &mut scratch)
        .expect("Decompression must succeed");
    assert_eq!(decompressed_len, plain_block.len());
    assert_eq!(scratch, plain_block);

    // 3. Extract and verify restarts
    let extracted_restarts = SafeBlockDecoder::verify_and_extract_restarts(&scratch)
        .expect("Restart extraction must succeed");
    assert_eq!(extracted_restarts, restart_offsets);
}

#[test]
fn test_decompression_bomb_rejection_without_allocation() {
    let config = BlockDecompressionGuardConfig {
        max_uncompressed_bytes: 4 * 1024 * 1024, // 4 MiB test limit
        max_expansion_ratio: 256,
        verify_crc: true,
    };

    // Craft a forged LZ4 header declaring 128 MiB uncompressed size
    let forged_uncompressed_size: u32 = 128 * 1024 * 1024;
    let mut payload = forged_uncompressed_size.to_le_bytes().to_vec();
    // Tiny dummy compressed stream
    payload.extend_from_slice(&[0x00, 0x01, 0x02, 0x03]);

    let mut scratch = Vec::new();
    let result = SafeBlockDecoder::decompress_lz4_into(&payload, &config, &mut scratch);

    match result {
        Err(DecompressionGuardError::ExceedsMaxUncompressedSize { declared, limit }) => {
            assert_eq!(declared, 128 * 1024 * 1024);
            assert_eq!(limit, 4 * 1024 * 1024);
        }
        other => panic!("Expected ExceedsMaxUncompressedSize, got: {other:?}"),
    }

    // Mathematical Invariant: Scratch buffer remains unallocated (zero bytes)
    assert!(scratch.is_empty(), "Scratch must NOT allocate on size rejection!");
}

#[test]
fn test_expansion_ratio_explosion_rejection() {
    let config = BlockDecompressionGuardConfig {
        max_uncompressed_bytes: 64 * 1024 * 1024,
        max_expansion_ratio: 256,
        verify_crc: true,
    };

    // Compressed payload of 100 bytes declaring 100_000 bytes uncompressed (1000x ratio)
    let declared_size: u32 = 100_000;
    let mut payload = declared_size.to_le_bytes().to_vec();
    payload.resize(104, 0xAA); // 4 bytes size + 100 bytes payload

    let mut scratch = Vec::new();
    let result = SafeBlockDecoder::decompress_lz4_into(&payload, &config, &mut scratch);

    match result {
        Err(DecompressionGuardError::ExpansionRatioExplosion { declared, compressed, ratio, max_ratio }) => {
            assert_eq!(declared, 100_000);
            assert_eq!(compressed, 104);
            assert!(ratio > 256);
            assert_eq!(max_ratio, 256);
        }
        other => panic!("Expected ExpansionRatioExplosion, got: {other:?}"),
    }
}

#[test]
fn test_crc_tamper_rejection() {
    let data = b"sample uncorrupted data payload";
    let crc = crc32c::crc32c(data);
    let mut raw = data.to_vec();
    raw.extend_from_slice(&crc.to_le_bytes());

    // Single bit flip in data
    raw[5] ^= 0x01;
    let err = SafeBlockDecoder::split_and_verify_crc(&raw, true)
        .expect_err("Tampered data must fail CRC check");

    assert!(matches!(err, DecompressionGuardError::CrcMismatch { .. }));

    // Truncated block too short for 4-byte CRC trailer (< 4 bytes)
    let too_short = &raw[..3];
    let trunc_err = SafeBlockDecoder::split_and_verify_crc(too_short, true)
        .expect_err("Truncated block must fail CRC check");
    assert!(matches!(trunc_err, DecompressionGuardError::TruncatedCrc { .. }));
}

#[test]
fn test_restart_array_malformations() {
    // 1. Block too short
    let err = SafeBlockDecoder::verify_and_extract_restarts(&[0x01, 0x02])
        .expect_err("Block under 4 bytes must fail");
    assert!(matches!(err, DecompressionGuardError::MalformedRestartArray { .. }));

    // 2. num_restarts multiplication overflow or exceeding block length
    let mut bad_block = vec![0u8; 16];
    let huge_restarts = 0xFFFF_FFFFu32;
    bad_block[12..16].copy_from_slice(&huge_restarts.to_le_bytes());
    let err = SafeBlockDecoder::verify_and_extract_restarts(&bad_block)
        .expect_err("Huge restart count must fail");
    assert!(matches!(err, DecompressionGuardError::MalformedRestartArray { .. }));

    // 3. Restart offset points into restart array
    let mut bad_offset_block = Vec::new();
    bad_offset_block.extend_from_slice(&[0u8; 10]); // 10 bytes payload
    bad_offset_block.extend_from_slice(&15u32.to_le_bytes()); // offset 15 > payload len 10
    bad_offset_block.extend_from_slice(&1u32.to_le_bytes()); // 1 restart
    let err = SafeBlockDecoder::verify_and_extract_restarts(&bad_offset_block)
        .expect_err("Offset pointing past payload must fail");
    assert!(matches!(err, DecompressionGuardError::MalformedRestartArray { .. }));

    // 4. Non-monotonic restart offsets
    let mut non_mono_block = Vec::new();
    non_mono_block.extend_from_slice(&[0u8; 20]); // 20 bytes payload
    non_mono_block.extend_from_slice(&10u32.to_le_bytes()); // offset 0: 10
    non_mono_block.extend_from_slice(&5u32.to_le_bytes());  // offset 1: 5 (descending!)
    non_mono_block.extend_from_slice(&2u32.to_le_bytes());  // 2 restarts
    let err = SafeBlockDecoder::verify_and_extract_restarts(&non_mono_block)
        .expect_err("Descending restart offsets must fail");
    assert!(matches!(err, DecompressionGuardError::MalformedRestartArray { .. }));
}

#[test]
fn test_varint_bounded_parser_robustness() {
    // Valid varint32
    let (v, len) = SafeBlockDecoder::decode_varint32(&[0x05]).expect("Single byte varint");
    assert_eq!(v, 5);
    assert_eq!(len, 1);

    let (v, len) = SafeBlockDecoder::decode_varint32(&[0xAC, 0x02]).expect("Two byte varint");
    assert_eq!(v, 300);
    assert_eq!(len, 2);

    let (v, len) = SafeBlockDecoder::decode_varint32(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]).expect("Max u32 varint");
    assert_eq!(v, u32::MAX);
    assert_eq!(len, 5);

    // Infinite continuation bit loop attack
    let infinite_varint = [0x80, 0x80, 0x80, 0x80, 0x80, 0x80];
    let err = SafeBlockDecoder::decode_varint32(&infinite_varint)
        .expect_err("Varint continuation exceeding 5 bytes must fail");
    assert_eq!(err, DecompressionGuardError::VarintOverflow);

    // Truncated varint stream
    let truncated_varint = [0x80];
    let err = SafeBlockDecoder::decode_varint32(&truncated_varint)
        .expect_err("Truncated varint must fail");
    assert_eq!(err, DecompressionGuardError::VarintOverflow);

    // Valid varint64
    let (v64, len64) = SafeBlockDecoder::decode_varint64(&[
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01,
    ])
    .expect("Max u64 varint");
    assert_eq!(v64, u64::MAX);
    assert_eq!(len64, 10);
}

#[test]
fn test_restart_array_zero_restarts_on_non_empty_block_rejected() {
    let mut non_empty_block = vec![1, 2, 3, 4, 5, 6, 7, 8];
    // num_restarts = 0 at tail
    non_empty_block.extend_from_slice(&0u32.to_le_bytes());
    let err = SafeBlockDecoder::verify_and_extract_restarts(&non_empty_block)
        .expect_err("Non-empty block with 0 restarts must be rejected");
    match err {
        DecompressionGuardError::MalformedRestartArray { reason } => {
            assert!(reason.contains("0 restarts") || reason.contains("restart"));
        }
        other => panic!("Expected MalformedRestartArray, got: {other:?}"),
    }
}

#[test]
fn test_restart_array_non_zero_first_offset_rejected() {
    let mut bad_first_block = Vec::new();
    bad_first_block.extend_from_slice(&[0u8; 20]); // payload
    bad_first_block.extend_from_slice(&5u32.to_le_bytes()); // first restart is 5 != 0
    bad_first_block.extend_from_slice(&1u32.to_le_bytes()); // 1 restart
    let err = SafeBlockDecoder::verify_and_extract_restarts(&bad_first_block)
        .expect_err("First restart offset != 0 must be rejected");
    match err {
        DecompressionGuardError::MalformedRestartArray { reason } => {
            assert_eq!(reason, "First restart offset must be 0");
        }
        other => panic!("Expected MalformedRestartArray, got: {other:?}"),
    }
}

#[test]
fn test_restart_array_duplicate_offset_rejected() {
    let mut dup_block = Vec::new();
    dup_block.extend_from_slice(&[0u8; 20]); // payload
    dup_block.extend_from_slice(&0u32.to_le_bytes()); // offset 0: 0
    dup_block.extend_from_slice(&0u32.to_le_bytes()); // offset 1: 0 (duplicate!)
    dup_block.extend_from_slice(&2u32.to_le_bytes()); // 2 restarts
    let err = SafeBlockDecoder::verify_and_extract_restarts(&dup_block)
        .expect_err("Duplicate restart offsets must be rejected");
    match err {
        DecompressionGuardError::MalformedRestartArray { reason } => {
            assert_eq!(reason, "Restart offsets must be strictly monotonically increasing");
        }
        other => panic!("Expected MalformedRestartArray, got: {other:?}"),
    }
}

