//! RFC-0324: WAL Crash Recovery Reconciler Verification Suite.
//!
//! Mechanically verifies deterministic crash recovery across torn writes,
//! short writes, preallocated trailing zeros, Sentinel Seals, and mid-log corruption.

#![forbid(unsafe_code)]

use pedradb_core::wal_crash_recovery_reconciler_kernel::{
    RecoveryAction, WalCrashRecoveryReconciler, WalRecoveryError, WAL_RECORD_MAGIC,
    WAL_SENTINEL_SEAL_MAGIC,
};

fn encode_record_header(magic: u32, seq: u64, len: u32) -> [u8; 16] {
    let mut buf = [0u8; 16];
    buf[0..4].copy_from_slice(&magic.to_le_bytes());
    buf[4..12].copy_from_slice(&seq.to_le_bytes());
    buf[12..16].copy_from_slice(&len.to_le_bytes());
    buf
}

#[test]
fn rfc0324_recovery_reconciler_happy_path() {
    let mut reconciler = WalCrashRecoveryReconciler::new();
    let file_size = 1024u64;

    // Record 1 at offset 0
    let h1 = encode_record_header(WAL_RECORD_MAGIC, 1, 32);
    let a1 = reconciler
        .reconcile_chunk(0, &h1, true, file_size)
        .expect("record 1 ok");
    assert_eq!(
        a1,
        RecoveryAction::AcceptRecord {
            seq: 1,
            payload_len: 32,
            next_offset: 48,
        }
    );
    assert_eq!(reconciler.last_valid_seq(), 1);
    assert_eq!(reconciler.last_valid_offset(), 48);
    assert_eq!(reconciler.total_valid_records(), 1);

    // Record 2 at offset 48
    let h2 = encode_record_header(WAL_RECORD_MAGIC, 2, 64);
    let a2 = reconciler
        .reconcile_chunk(48, &h2, true, file_size)
        .expect("record 2 ok");
    assert_eq!(
        a2,
        RecoveryAction::AcceptRecord {
            seq: 2,
            payload_len: 64,
            next_offset: 128,
        }
    );
    assert_eq!(reconciler.last_valid_seq(), 2);
    assert_eq!(reconciler.last_valid_offset(), 128);
    assert_eq!(reconciler.total_valid_records(), 2);

    // Clean EOF at offset 1024
    let eof = reconciler
        .reconcile_chunk(1024, &[], true, file_size)
        .expect("clean eof ok");
    assert_eq!(
        eof,
        RecoveryAction::CleanEof {
            total_records: 2,
            final_offset: 128,
        }
    );
}

#[test]
fn rfc0324_recovery_reconciler_trailing_zeros_and_torn_tail() {
    let mut reconciler = WalCrashRecoveryReconciler::new();
    let file_size = 4096u64;

    // Valid record 1
    let h1 = encode_record_header(WAL_RECORD_MAGIC, 10, 16);
    reconciler
        .reconcile_chunk(0, &h1, true, file_size)
        .expect("record 1 ok");

    // Trailing preallocated zeros from offset 32 to 4096
    let zeros = [0u8; 16];
    let action_zeros = reconciler
        .reconcile_chunk(32, &zeros, true, file_size)
        .expect("zeros handled");
    assert_eq!(
        action_zeros,
        RecoveryAction::TrailingZerosTruncate {
            valid_offset: 32,
            zero_bytes: 4096 - 32,
        }
    );

    // Partial short torn write at tail (e.g. power failure during 16-byte header write)
    let mut torn_reconciler = WalCrashRecoveryReconciler::new();
    torn_reconciler
        .reconcile_chunk(0, &h1, true, 40)
        .expect("record 1 ok");
    let short_header = [0x50, 0x45, 0x44]; // only 3 bytes written
    let action_torn = torn_reconciler
        .reconcile_chunk(32, &short_header, true, 40)
        .expect("torn tail handled");
    assert!(matches!(action_torn, RecoveryAction::TornTailTruncate { .. }));
}

#[test]
fn rfc0324_recovery_reconciler_sentinel_seal_and_midlog_quarantine() {
    let mut reconciler = WalCrashRecoveryReconciler::new();
    let file_size = 8192u64;

    // Record 1
    let h1 = encode_record_header(WAL_RECORD_MAGIC, 5, 20);
    reconciler
        .reconcile_chunk(0, &h1, true, file_size)
        .expect("rec 1 ok");

    // Sentinel Seal at offset 36
    let h_seal = encode_record_header(WAL_SENTINEL_SEAL_MAGIC, 5, 0);
    let seal_action = reconciler
        .reconcile_chunk(36, &h_seal, true, file_size)
        .expect("seal ok");
    assert_eq!(
        seal_action,
        RecoveryAction::SentinelSealEncountered {
            seal_seq: 5,
            logical_offset: 36,
        }
    );
    assert!(reconciler.is_sealed());

    // Subsequent chunk after seal returns CleanEof immediately
    let post_seal = reconciler
        .reconcile_chunk(52, &h1, true, file_size)
        .expect("post seal eof ok");
    assert!(matches!(post_seal, RecoveryAction::CleanEof { .. }));

    // Mid-log damage quarantine test (reconciler with mid-log corrupt magic)
    let mut corrupted_reconciler = WalCrashRecoveryReconciler::new();
    let h_corrupt = [0xFF; 16]; // corrupted magic
    let action_quarantine = corrupted_reconciler
        .reconcile_chunk(0, &h_corrupt, true, 65536)
        .expect("quarantined");
    assert!(matches!(action_quarantine, RecoveryAction::QuarantinedRegion { .. }));
    assert_eq!(corrupted_reconciler.quarantined_regions_count(), 1);

    // Non-monotonic sequence rejection
    let mut seq_reconciler = WalCrashRecoveryReconciler::new();
    let h_first = encode_record_header(WAL_RECORD_MAGIC, 100, 10);
    seq_reconciler
        .reconcile_chunk(0, &h_first, true, 1024)
        .expect("first ok");
    let h_regressive = encode_record_header(WAL_RECORD_MAGIC, 99, 10);
    let regr_err = seq_reconciler
        .reconcile_chunk(26, &h_regressive, true, 1024)
        .expect_err("should reject non-monotonic sequence");
    assert!(matches!(regr_err, WalRecoveryError::NonMonotonicSequence { .. }));
}
