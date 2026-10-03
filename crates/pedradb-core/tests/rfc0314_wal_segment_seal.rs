//! RFC-0314: Atomic WAL Segment Seal and Preallocated Extent Reconciliation Test Suite.
//!
//! Validates mathematical properties:
//! - Strict conservation: `logical_bytes <= allocated_bytes`.
//! - Sentinel seal frame encoding with verified CRC32C.
//! - Clean trailing zero classification (eliminates false corruption alarms on recovery).
//! - Single bit-flip detection on seal headers.

use pedradb_core::wal_segment_seal_kernel::{
    SealParseResult, WalPreallocationTracker, WalSealError, SEAL_FRAME_LEN,
};

#[test]
fn test_wal_preallocation_headroom_and_append() {
    let mut tracker = WalPreallocationTracker::new(1024);
    assert_eq!(tracker.allocated_bytes(), 1024);
    assert_eq!(tracker.logical_bytes(), 0);
    assert_eq!(tracker.remaining_headroom(), 1024);
    assert!(!tracker.is_sealed());

    // Valid appends
    assert!(tracker.can_append(256));
    assert!(tracker.advance_logical(256).is_ok());
    assert_eq!(tracker.logical_bytes(), 256);
    assert_eq!(tracker.remaining_headroom(), 768);

    assert!(tracker.can_append(512));
    assert!(tracker.advance_logical(512).is_ok());
    assert_eq!(tracker.logical_bytes(), 768);
    assert_eq!(tracker.remaining_headroom(), 256);

    // Append that exceeds capacity is rejected
    assert!(!tracker.can_append(300));
    assert!(tracker.advance_logical(300).is_err());
    assert_eq!(tracker.logical_bytes(), 768);
}

#[test]
fn test_wal_sentinel_seal_encode_and_parse() {
    let mut tracker = WalPreallocationTracker::new(1024);
    tracker.advance_logical(500).unwrap();

    // Encode seal frame
    let seal_bytes = tracker.encode_seal().unwrap();
    assert_eq!(seal_bytes.len(), SEAL_FRAME_LEN);
    assert!(tracker.is_sealed());
    assert_eq!(tracker.logical_bytes(), 500 + SEAL_FRAME_LEN as u64);

    // Any subsequent append must be rejected
    assert!(!tracker.can_append(10));
    assert!(tracker.advance_logical(10).is_err());

    // Re-sealing must be rejected
    assert!(tracker.encode_seal().is_err());

    // Parse the encoded seal
    let parse_res = WalPreallocationTracker::parse_frame_or_seal(&seal_bytes);
    assert_eq!(
        parse_res,
        SealParseResult::ValidSeal {
            logical_eof: 500,
        }
    );
}

#[test]
fn test_wal_trailing_zeroes_reconciliation() {
    // Synthesize physical preallocated extent of 1024 bytes
    let mut file_buf = vec![0u8; 1024];

    // Logical data: 200 bytes
    for i in 0..200 {
        file_buf[i] = (i % 255) as u8;
    }

    // Seal frame at offset 200
    let mut tracker = WalPreallocationTracker::new(1024);
    tracker.advance_logical(200).unwrap();
    let seal_bytes = tracker.encode_seal().unwrap();
    file_buf[200..216].copy_from_slice(&seal_bytes);

    // Verify seal parse at offset 200
    let seal_result = WalPreallocationTracker::parse_frame_or_seal(&file_buf[200..216]);
    assert_eq!(seal_result, SealParseResult::ValidSeal { logical_eof: 200 });

    // Verify trailing bytes [216, 1024) are recognized as valid unwritten zeroes
    let trailing_result = WalPreallocationTracker::parse_frame_or_seal(&file_buf[216..1024]);
    assert_eq!(trailing_result, SealParseResult::TrailingZeroes);

    // Direct helper check
    assert!(WalPreallocationTracker::verify_trailing_zeroes(&file_buf[216..1024]));
}

#[test]
fn test_corrupt_seal_crc_detection() {
    let mut tracker = WalPreallocationTracker::new(1024);
    tracker.advance_logical(100).unwrap();
    let seal_bytes = tracker.encode_seal().unwrap();

    // 1. Bit flip in magic
    let mut bad_magic = seal_bytes;
    bad_magic[0] ^= 0x01;
    let res1 = WalPreallocationTracker::parse_frame_or_seal(&bad_magic);
    assert!(matches!(res1, SealParseResult::CorruptFrame(msg) if msg.contains("Invalid seal magic")));

    // 2. Bit flip in CRC
    let mut bad_crc = seal_bytes;
    bad_crc[15] ^= 0x01;
    let res2 = WalPreallocationTracker::parse_frame_or_seal(&bad_crc);
    assert!(matches!(res2, SealParseResult::CorruptFrame(msg) if msg.contains("CRC mismatch")));

    // 3. Bit flip in logical offset (CRC mismatch)
    let mut bad_offset = seal_bytes;
    bad_offset[4] ^= 0x01;
    let res3 = WalPreallocationTracker::parse_frame_or_seal(&bad_offset);
    assert!(matches!(res3, SealParseResult::CorruptFrame(msg) if msg.contains("CRC mismatch")));
}

#[test]
fn test_wal_segment_seal_typed_errors_and_extent_green() {
    // 1. try_new rejects 0 allocated bytes
    assert_eq!(
        WalPreallocationTracker::try_new(0).err(),
        Some(WalSealError::InvalidPreallocationSize(0))
    );
    let mut tracker = WalPreallocationTracker::try_new(1024).expect("valid tracker");

    // 2. advance_logical typed error on overflow
    assert_eq!(
        tracker.advance_logical(2000).err(),
        Some(WalSealError::ExceedsCapacity { requested: 2000, available: 1024 })
    );

    tracker.advance_logical(500).unwrap();
    let seal_bytes = tracker.encode_seal().unwrap();

    // 3. Reseal gives AlreadySealed typed error
    assert_eq!(tracker.encode_seal().err(), Some(WalSealError::AlreadySealed));

    // 4. Invariant 2: parse_and_verify_seal_extent checks that bytes AFTER seal are all zero
    let mut clean_extent = vec![0u8; 100];
    clean_extent[0..16].copy_from_slice(&seal_bytes);
    assert_eq!(
        WalPreallocationTracker::parse_and_verify_seal_extent(&clean_extent),
        SealParseResult::ValidSeal { logical_eof: 500 }
    );

    // Corrupted extent: non-zero garbage after seal frame
    let mut dirty_extent = clean_extent.clone();
    dirty_extent[50] = 0xAA;
    let dirty_res = WalPreallocationTracker::parse_and_verify_seal_extent(&dirty_extent);
    assert!(
        matches!(dirty_res, SealParseResult::CorruptFrame(msg) if msg.contains("trailing")),
        "Must detect non-zero corruption after seal frame"
    );

    // 5. Recovery truncation offset determination
    let valid_seal = SealParseResult::ValidSeal { logical_eof: 500 };
    assert_eq!(
        WalPreallocationTracker::determine_recovery_truncation_offset(&valid_seal, 500),
        Some(500 + SEAL_FRAME_LEN as u64)
    );

    let trailing_zeroes = SealParseResult::TrailingZeroes;
    assert_eq!(
        WalPreallocationTracker::determine_recovery_truncation_offset(&trailing_zeroes, 400),
        Some(400)
    );

    let corrupt = SealParseResult::CorruptFrame("corrupt".into());
    assert_eq!(
        WalPreallocationTracker::determine_recovery_truncation_offset(&corrupt, 400),
        None
    );
}

