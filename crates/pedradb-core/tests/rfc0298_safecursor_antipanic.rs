//! Adversarial negative regression suite for RFC-0298 SafeCursor and zero-panic decoders.
//!
//! Validates that corrupt, truncated, and overflowed inputs return clean `Result::Err`
//! and NEVER trigger unwraps, bare slice panics, or unauthenticated heap exhaustions.

#![forbid(unsafe_code)]

use pedradb_core::bloom::BloomFilter;
use pedradb_core::codec::{DecodeError, SafeCursor};
use pedradb_core::history::verify_bloom_sidecar;
use pedradb_core::prefix_delta_restart_kernel::PrefixDeltaBlock;

#[test]
fn test_safecursor_truncated_primitives() {
    let empty: [u8; 0] = [];
    let mut cur = SafeCursor::new(&empty);
    assert!(cur.read_u8().is_err());
    assert!(cur.read_u16_le().is_err());
    assert!(cur.read_u32_le().is_err());
    assert!(cur.read_u64_le().is_err());
    assert!(cur.read_u128_le().is_err());

    let three_bytes = [1, 2, 3];
    let mut cur = SafeCursor::new(&three_bytes);
    assert_eq!(cur.read_u8().unwrap(), 1);
    assert_eq!(cur.read_u16_le().unwrap(), 0x0302);
    assert!(cur.read_u32_le().is_err());
    assert!(cur.read_u64_le().is_err());
}

#[test]
fn test_safecursor_length_prefixed_bytes_overflow() {
    // Length prefix specifies 1000 bytes, but buffer only has 4 bytes
    let mut buf = Vec::new();
    buf.extend_from_slice(&1000u32.to_le_bytes());
    buf.extend_from_slice(&[0xaa; 4]);

    let mut cur = SafeCursor::new(&buf);
    let res = cur.read_length_prefixed_bytes(1000);
    assert!(matches!(res, Err(DecodeError::LengthExceedsCapacity { .. })));

    // Length prefix specifies more than max_len limit
    let mut cur = SafeCursor::new(&buf);
    let res = cur.read_length_prefixed_bytes(500);
    assert!(matches!(res, Err(DecodeError::LengthExceedsCapacity { .. })));
}

#[test]
fn test_safecursor_bounded_capacity_dos_prevention() {
    // Untrusted count 10,000,000 with only 16 bytes remaining
    let rem = 16;
    let res = SafeCursor::bounded_capacity(10_000_000, 4, rem, 100_000);
    assert!(matches!(res, Err(DecodeError::LengthExceedsCapacity { .. })));

    // Count within remaining bounds
    let ok = SafeCursor::bounded_capacity(4, 4, rem, 100_000).unwrap();
    assert_eq!(ok, 4);

    // Count exceeding hard cap
    let res = SafeCursor::bounded_capacity(100_001, 1, 1_000_000, 100_000);
    assert!(matches!(res, Err(DecodeError::LengthExceedsCapacity { .. })));
}

#[test]
fn test_safecursor_trailing_garbage_detection() {
    let buf = [1, 2, 3, 4, 5];
    let mut cur = SafeCursor::new(&buf);
    assert_eq!(cur.read_u8().unwrap(), 1);
    assert_eq!(cur.read_u8().unwrap(), 2);
    // 3 bytes remain unconsumed
    assert!(matches!(
        cur.ensure_fully_consumed(),
        Err(DecodeError::TrailingGarbage { remaining: 3 })
    ));
    assert!(matches!(
        cur.ensure_exact_exhaustion(),
        Err(DecodeError::TrailingGarbage { remaining: 3 })
    ));
}

#[test]
fn test_bloom_decode_adversarial_inputs() {
    // Empty buffer is safely treated as always_true (zero-panic)
    let empty_filter = BloomFilter::decode(&[]).unwrap();
    assert!(empty_filter.may_contain(b"any_key"));

    // Truncated (less than 12 bytes header)
    assert!(BloomFilter::decode(&[0x01, 0x02, 0x03]).is_err());
    assert!(BloomFilter::decode(&[0x01; 11]).is_err());

    // Wrong tag / version
    let bad_tag = [0x99; 30];
    assert!(BloomFilter::decode(&bad_tag).is_err());

    // Valid header but bit_bytes length exceeds buffer
    let mut corrupt = Vec::new();
    corrupt.push(0x01); // tag
    corrupt.extend_from_slice(&10u32.to_le_bytes()); // num_probes
    corrupt.extend_from_slice(&1000000u32.to_le_bytes()); // bit_bytes (huge!)
    corrupt.extend_from_slice(&1u32.to_le_bytes()); // partitions
    corrupt.extend_from_slice(&[0u8; 10]); // only 10 bytes payload
    assert!(BloomFilter::decode(&corrupt).is_err());
}

#[test]
fn test_prefix_delta_restart_adversarial_inputs() {
    // Truncated block (less than restart_len u32)
    assert!(PrefixDeltaBlock::decode_and_verify(&[0x01, 0x02], 16).is_err());

    // Restart count specified as huge number
    let mut corrupt = Vec::new();
    corrupt.extend_from_slice(&[0u8; 10]); // data
    corrupt.extend_from_slice(&1000000u32.to_le_bytes()); // num_restarts = 1,000,000
    assert!(PrefixDeltaBlock::decode_and_verify(&corrupt, 16).is_err());

    // Restart offset beyond block length
    let mut corrupt = Vec::new();
    corrupt.extend_from_slice(&[0u8; 10]);
    corrupt.extend_from_slice(&500u32.to_le_bytes()); // restart offset 500
    corrupt.extend_from_slice(&1u32.to_le_bytes()); // num_restarts = 1
    assert!(PrefixDeltaBlock::decode_and_verify(&corrupt, 16).is_err());

    // Corrupt entry record length prefix
    let mut corrupt = Vec::new();
    corrupt.extend_from_slice(&0u32.to_le_bytes()); // shared = 0
    corrupt.extend_from_slice(&50000u32.to_le_bytes()); // non_shared = 50,000 (exceeds block)
    corrupt.extend_from_slice(&0u32.to_le_bytes()); // val_len = 0
    corrupt.extend_from_slice(&0u32.to_le_bytes()); // restart[0] = 0
    corrupt.extend_from_slice(&1u32.to_le_bytes()); // num_restarts = 1
    assert!(PrefixDeltaBlock::decode_and_verify(&corrupt, 16).is_err());

    // Zero restart interval must not cause division-by-zero panic
    assert!(PrefixDeltaBlock::decode_and_verify(&corrupt, 0).is_err());
    let entry = pedradb_core::prefix_delta_restart_kernel::BlockKvEntry {
        key: b"k".to_vec(),
        val: b"v".to_vec(),
    };
    let _ = PrefixDeltaBlock::encode_block(&[entry.clone()], 0);

    // Phantom extra restart points beyond entries must be rejected
    let valid_block = PrefixDeltaBlock::encode_block(&[entry.clone()], 16);
    // Tamper with valid block: declare 2 restart points instead of 1, pointing to offset 0 and 0
    let tampered = valid_block.clone();
    let old_trailer_start = tampered.len() - 8; // restart[0] (4B) + num_restarts (4B)
    let payload = tampered[..old_trailer_start].to_vec();
    let mut tampered_block = payload;
    tampered_block.extend_from_slice(&0u32.to_le_bytes()); // restart[0] = 0
    tampered_block.extend_from_slice(&0u32.to_le_bytes()); // restart[1] = 0 (duplicate / phantom)
    tampered_block.extend_from_slice(&2u32.to_le_bytes()); // num_restarts = 2
    assert!(PrefixDeltaBlock::decode_and_verify(&tampered_block, 16).is_err(), "Phantom or non-monotonic restart offsets must be rejected");
}

#[test]
fn test_bloom_sidecar_adversarial_inputs() {
    // Truncated header (less than 8 bytes magic + 4 bytes len)
    let short = [0u8; 6];
    assert!(verify_bloom_sidecar(&short).is_err());

    // Bad magic
    let bad_magic = [0xff; 30];
    assert!(verify_bloom_sidecar(&bad_magic).is_err());

    // Valid magic but payload length exceeds remaining slice
    let mut corrupt = Vec::new();
    corrupt.extend_from_slice(b"PEDRABLM");
    corrupt.extend_from_slice(&99999u32.to_le_bytes());
    corrupt.extend_from_slice(&[0xaa; 10]);
    assert!(verify_bloom_sidecar(&corrupt).is_err());
}
