//! RFC-0316: Differential Prefix Front-Coding Block Iterator Test Suite
//!
//! Verifies:
//! - Round-trip encoding/decoding of front-coded blocks.
//! - Exact seek traversal over restart points.
//! - Rejection of shared prefix overflows (preventing buffer over-read).
//! - Rejection of corrupt restart offsets.
//! - Rejection of key order violations.

use pedradb_core::front_coding_block_iter_kernel::{
    BlockDecodeError, FrontCodingBlockEncoder, FrontCodingBlockViewer,
};

#[test]
fn test_front_coding_roundtrip_and_seek() {
    let mut encoder = FrontCodingBlockEncoder::new(4);
    let pairs = vec![
        (b"user:0001".to_vec(), b"Alice".to_vec()),
        (b"user:0002".to_vec(), b"Bob".to_vec()),
        (b"user:0003".to_vec(), b"Charlie".to_vec()),
        (b"user:0010".to_vec(), b"Dave".to_vec()),
        (b"user:0020".to_vec(), b"Eve".to_vec()),
    ];

    for (k, v) in &pairs {
        encoder.add(k.clone(), v.clone());
    }

    let raw_block = encoder.encode();
    let viewer = FrontCodingBlockViewer::parse(&raw_block).unwrap();

    assert!(viewer.num_restarts() >= 2);
    assert!(viewer.verify_internal_invariants().unwrap());

    // Verify all entries decoded match original pairs
    let decoded = viewer.decode_all_entries().unwrap();
    assert_eq!(decoded.len(), pairs.len());
    for (i, entry) in decoded.iter().enumerate() {
        assert_eq!(entry.key, pairs[i].0);
        assert_eq!(entry.value, pairs[i].1);
    }

    // Verify seek
    let found = viewer.seek_key(b"user:0003").unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().value, b"Charlie");

    let not_found = viewer.seek_key(b"user:9999").unwrap();
    assert!(not_found.is_none());
}

#[test]
fn test_buffer_overread_prefix_overflow_rejected() {
    let mut encoder = FrontCodingBlockEncoder::new(16);
    encoder.add(b"short".to_vec(), b"val1".to_vec());
    encoder.add(b"longer_key".to_vec(), b"val2".to_vec());
    let mut raw_block = encoder.encode();

    // Corrupt the shared prefix of the second entry:
    // First entry has shared = 0. Find the second entry and increase shared prefix beyond "short".len() (5).
    // The restart array starts at the end. First entry is at offset 0.
    // Lengths: varint(0), varint(5), varint(4), "short", "val1" -> total 1 + 1 + 1 + 5 + 4 = 12 bytes.
    // Second entry starts at byte 12: byte 12 is shared prefix. Change it to 20 (> 5).
    raw_block[12] = 20;

    let viewer = FrontCodingBlockViewer::parse(&raw_block).unwrap();
    let err = viewer.decode_all_entries().unwrap_err();
    match err {
        BlockDecodeError::SharedPrefixOverflow { shared, prev_len } => {
            assert_eq!(shared, 20);
            assert_eq!(prev_len, 5);
        }
        other => panic!("Expected SharedPrefixOverflow, got {:?}", other),
    }
}

#[test]
fn test_corrupt_restart_offset_rejected() {
    let mut encoder = FrontCodingBlockEncoder::new(2);
    encoder.add(b"key1".to_vec(), b"v1".to_vec());
    encoder.add(b"key2".to_vec(), b"v2".to_vec());
    encoder.add(b"key3".to_vec(), b"v3".to_vec());
    let mut raw_block = encoder.encode();

    // The last 4 bytes are num_restarts. Right before it is the restart offset.
    // Set a restart offset pointing way past the data buffer.
    let num_restarts_off = raw_block.len() - 4;
    let restart_off_pos = num_restarts_off - 4;
    raw_block[restart_off_pos..restart_off_pos + 4].copy_from_slice(&99999u32.to_le_bytes());

    let res = FrontCodingBlockViewer::parse(&raw_block);
    assert!(matches!(res, Err(BlockDecodeError::RestartOffsetOutOfBounds { .. })));
}

#[test]
fn test_key_order_violation_rejected() {
    // Manually build an invalid block with keys out of order: "zzz" then "aaa"
    let mut encoder = FrontCodingBlockEncoder::new(16);
    encoder.add(b"zzz".to_vec(), b"val1".to_vec());
    encoder.add(b"aaa".to_vec(), b"val2".to_vec()); // Order violation!
    let raw_block = encoder.encode();

    let viewer = FrontCodingBlockViewer::parse(&raw_block).unwrap();
    let res = viewer.decode_all_entries();
    assert_eq!(res, Err(BlockDecodeError::KeyOrderViolation));
}

#[test]
fn test_front_coding_red_invariants() {
    // 1. Duplicate empty keys must be rejected as KeyOrderViolation
    let mut encoder = FrontCodingBlockEncoder::new(16);
    encoder.add(b"".to_vec(), b"v1".to_vec());
    encoder.add(b"".to_vec(), b"v2".to_vec());
    let raw = encoder.encode();
    let viewer = FrontCodingBlockViewer::parse(&raw).unwrap();
    assert_eq!(viewer.decode_all_entries().unwrap_err(), BlockDecodeError::KeyOrderViolation);

    // 2. Zero restarts with data payload must be rejected
    let bad_raw = vec![1, 2, 3, 4, 5, 0, 0, 0, 0]; // 5 bytes of data, num_restarts = 0
    assert!(matches!(
        FrontCodingBlockViewer::parse(&bad_raw),
        Err(BlockDecodeError::InvalidRestartCount)
    ));

    // 3. Restarts not starting at 0 must be rejected
    let mut encoder2 = FrontCodingBlockEncoder::new(2);
    encoder2.add(b"k1".to_vec(), b"v1".to_vec());
    let mut raw2 = encoder2.encode();
    // Corrupt restarts[0] from 0 to 5
    let num_restarts_off = raw2.len() - 4;
    let r0_off = num_restarts_off - 4;
    raw2[r0_off..r0_off + 4].copy_from_slice(&5u32.to_le_bytes());
    assert!(matches!(
        FrontCodingBlockViewer::parse(&raw2),
        Err(BlockDecodeError::InvalidRestartOffset { .. })
    ));
}

