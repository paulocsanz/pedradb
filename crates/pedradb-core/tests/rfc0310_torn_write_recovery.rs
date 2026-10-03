//! RFC-0310: Torn-Write Crash Recovery & Sector-Cut Resilience Test Suite.
//!
//! Validates atomic recovery guarantees, framing invariants, and bit-flip immunity
//! under arbitrary hardware truncation points and power-loss torn writes.

use pedradb_core::torn_write_recovery_kernel::{
    encode_frame, recover_frames, TornWriteError, HEADER_LEN,
};

#[test]
fn test_atomic_recovery_closed_under_arbitrary_byte_truncations() {
    let mut log = Vec::new();
    let mut frame_boundaries = vec![0];

    // Generate 30 heterogeneous frames
    for i in 1..=30 {
        let payload = format!("val-batch-{:04}-data-{}", i, "x".repeat(i * 7));
        let frame_bytes = encode_frame(i as u64, payload.as_bytes());
        log.extend_from_slice(&frame_bytes);
        frame_boundaries.push(log.len());
    }

    let total_len = log.len();
    assert!(total_len > 1000);

    // Test truncation at EVERY single byte offset from 0 to total_len
    for cut in 0..=total_len {
        let truncated_slice = &log[..cut];
        let report = recover_frames(truncated_slice);

        // Find how many complete frames fit strictly within `cut`
        let mut expected_frames = 0;
        for (idx, &boundary) in frame_boundaries.iter().enumerate().skip(1) {
            if boundary <= cut {
                expected_frames = idx;
            } else {
                break;
            }
        }

        assert_eq!(
            report.frames.len(),
            expected_frames,
            "Mismatch in recovered frame count at cut offset {cut}/{total_len}"
        );
        assert_eq!(
            report.clean_offset, frame_boundaries[expected_frames],
            "Clean offset must match the exact boundary of the last complete frame at cut {cut}"
        );

        // Verify all recovered frames have correct sequence and data
        for (idx, frame) in report.frames.iter().enumerate() {
            let expected_seq = (idx + 1) as u64;
            assert_eq!(frame.seq, expected_seq);
            let expected_payload =
                format!("val-batch-{:04}-data-{}", expected_seq, "x".repeat((expected_seq as usize) * 7));
            assert_eq!(frame.payload, expected_payload.as_bytes());
        }

        let is_exact_boundary = frame_boundaries.contains(&cut);
        if !is_exact_boundary {
            assert!(
                report.torn_detected,
                "Torn write must be detected when cut ({cut}) is not a frame boundary"
            );
        } else {
            assert!(
                !report.torn_detected,
                "No torn write should be flagged on exact frame boundary at cut {cut}"
            );
            assert!(report.reason.is_none());
        }
    }
}

#[test]
fn test_sector_alignment_torn_write_resilience() {
    let mut log = Vec::new();
    for i in 1..=20 {
        let payload = vec![i as u8; 128];
        log.extend_from_slice(&encode_frame(i, &payload));
    }

    // Simulate standard 512B and 4096B sector boundary power cuts where the remainder is zeroed
    for sector_size in [512, 4096] {
        for sector_count in 1..5 {
            let cut = sector_count * sector_size;
            if cut >= log.len() {
                continue;
            }

            let mut corrupted_sector = log[..cut].to_vec();
            // Append a partial torn sector filled with zeros
            corrupted_sector.extend_from_slice(&[0u8; 256]);

            let report = recover_frames(&corrupted_sector);
            assert!(report.torn_detected);
            assert!(report.clean_offset <= cut);
            for (idx, f) in report.frames.iter().enumerate() {
                assert_eq!(f.seq, (idx + 1) as u64);
            }
        }
    }
}

#[test]
fn test_bit_flip_and_magic_corruption_immunity() {
    let payload = b"critical financial ledger commit entry";
    let encoded = encode_frame(100, payload);

    // 1. Bit flip in magic
    let mut corrupt_magic = encoded.clone();
    corrupt_magic[0] ^= 0xFF;
    let r1 = recover_frames(&corrupt_magic);
    assert!(r1.torn_detected);
    assert_eq!(r1.frames.len(), 0);
    assert!(matches!(r1.reason, Some(TornWriteError::InvalidMagic { .. })));

    // 2. Bit flip in length
    let mut corrupt_len = encoded.clone();
    corrupt_len[4] ^= 0x80; // inflate length
    let r2 = recover_frames(&corrupt_len);
    assert!(r2.torn_detected);
    assert_eq!(r2.frames.len(), 0);

    // 3. Bit flip in payload (CRC mismatch)
    let mut corrupt_payload = encoded.clone();
    corrupt_payload[HEADER_LEN + 5] ^= 0x01;
    let r3 = recover_frames(&corrupt_payload);
    assert!(r3.torn_detected);
    assert_eq!(r3.frames.len(), 0);
    assert!(matches!(r3.reason, Some(TornWriteError::CrcMismatch { .. })));
}
