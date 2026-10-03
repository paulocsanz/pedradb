//! RFC-0327: WAL Archive Healing Verification Suite.
//!
//! Mechanically verifies that rotated archived WAL segments containing
//! torn tail fragments or preallocated zeros are healed deterministically (F-CAMP-3).

#![forbid(unsafe_code)]

use pedradb_core::wal_archive_healing_kernel::{
    ArchiveHealError, ArchiveHealOutcome, WalArchiveHealer, WAL_RECORD_MAGIC,
};

fn encode_test_frame(seq: u64, payload_len: u32) -> Vec<u8> {
    let mut frame = Vec::with_capacity(16 + payload_len as usize);
    frame.extend_from_slice(&WAL_RECORD_MAGIC.to_le_bytes());
    frame.extend_from_slice(&seq.to_le_bytes());
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.resize(16 + payload_len as usize, 0xAA);
    frame
}

#[test]
fn rfc0327_archive_healing_clean_and_zero_padded() {
    // 1. Zero archive length rejected
    assert_eq!(
        WalArchiveHealer::inspect_and_heal(&[]),
        Err(ArchiveHealError::ZeroArchiveLength)
    );

    // 2. Clean 2-record segment
    let mut buf = Vec::new();
    buf.extend_from_slice(&encode_test_frame(1, 32));
    buf.extend_from_slice(&encode_test_frame(2, 48));

    let clean = WalArchiveHealer::inspect_and_heal(&buf).expect("clean archive ok");
    assert_eq!(
        clean,
        ArchiveHealOutcome::CleanUnchanged {
            total_records: 2,
            valid_length: (16 + 32 + 16 + 48) as u64,
            last_seq: 2,
        }
    );

    // 3. Preallocated zeros after valid records
    let mut zero_padded = buf.clone();
    zero_padded.resize(zero_padded.len() + 4096, 0);

    let pruned = WalArchiveHealer::inspect_and_heal(&zero_padded).expect("zero padding ok");
    assert_eq!(
        pruned,
        ArchiveHealOutcome::PreallocatedZerosPruned {
            healed_length: buf.len() as u64,
            zero_bytes: 4096,
            valid_records: 2,
        }
    );
}

#[test]
fn rfc0327_archive_healing_torn_tail_fragment() {
    // Construct valid segment with 2 records
    let mut buf = Vec::new();
    buf.extend_from_slice(&encode_test_frame(10, 20)); // offset 0..36
    buf.extend_from_slice(&encode_test_frame(11, 20)); // offset 36..72

    let valid_len = buf.len() as u64;

    // Simulate F-CAMP-3: ShortWrite/Interrupted wrote only 8 bytes of the 3rd header
    buf.extend_from_slice(&WAL_RECORD_MAGIC.to_le_bytes());
    buf.extend_from_slice(&12u32.to_le_bytes()); // truncated header!

    let healed = WalArchiveHealer::inspect_and_heal(&buf).expect("healing ok");
    assert_eq!(
        healed,
        ArchiveHealOutcome::HealedTornTail {
            original_length: buf.len() as u64,
            healed_length: valid_len,
            discarded_bytes: 8,
            valid_records: 2,
            last_seq: 11,
        }
    );

    // Non-monotonic sequence is rejected
    let mut regressive = Vec::new();
    regressive.extend_from_slice(&encode_test_frame(10, 16));
    regressive.extend_from_slice(&encode_test_frame(9, 16)); // regression!
    assert!(matches!(
        WalArchiveHealer::inspect_and_heal(&regressive),
        Err(ArchiveHealError::NonMonotonicSequence { .. })
    ));
}
