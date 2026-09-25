//! RFC-0281 Test Suite:
//! - P0.1: Bloom Filter Zero False Negative & Monotonicity
//! - P0.2: Dual-Log Recovery & Anti-Zombie Resurrection
//! - P1.1: Tombstone Soundness & Unmasking Prevention
//! - P1.2: Strict Weak Ordering Key Comparator Axioms
//! - P2.1: Direct I/O 4096-Byte Alignment & DMA Contract

use std::cmp::Ordering;

use pedradb_core::bloom_soundness_kernel::{
    BloomSoundnessOracle, BloomSoundnessViolation, VerifiedBloomBitset,
};
use pedradb_core::comparator_axiom_kernel::{
    AxiomaticComparatorVerifier, ByteLexicographicalComparator, KeyComparator, PrefixComparator,
};
use pedradb_core::direct_io_contract_kernel::{
    verify_direct_io_request, AlignedDirectBuffer, DirectIoAlignmentViolation,
    DIRECT_IO_PAGE_ALIGNMENT, DIRECT_IO_SECTOR_ALIGNMENT,
};
use pedradb_core::dual_log_recovery_kernel::{
    DualLogRecoveryGate, DualLogRecoveryViolation, ManifestRecoveryAnchor, WalRecoveryRecord,
    WalReplayDecision,
};
use pedradb_core::tombstone_soundness_kernel::{
    MockLevelCatalog, TombstonePurgeDecision, TombstonePurgeOracle, TombstoneSoundnessViolation,
};

#[test]
fn test_bloom_zero_false_negatives_and_monotonicity() {
    let mut bitset = VerifiedBloomBitset::new(4096, 7).expect("Bitset creation should succeed");

    // Keys to insert
    let keys: Vec<Vec<u8>> = (0..250)
        .map(|i| format!("test_key_{i:06}").into_bytes())
        .collect();

    for key in &keys {
        bitset.insert_key(key);
    }

    // 1. Zero False Negative verification
    assert!(
        BloomSoundnessOracle::verify_zero_false_negatives(&bitset, &keys).is_ok(),
        "Bloom filter must satisfy Zero False Negatives for all inserted keys"
    );

    // 2. Monotonicity verification on expanding set
    let snapshot_before = bitset.clone();
    let extra_keys: Vec<Vec<u8>> = (250..500)
        .map(|i| format!("test_key_{i:06}").into_bytes())
        .collect();

    for key in &extra_keys {
        bitset.insert_key(key);
    }

    assert!(
        BloomSoundnessOracle::verify_monotonicity(&snapshot_before, &bitset).is_ok(),
        "Bitset growth must be strictly monotonic (no bit clearing)"
    );

    // 3. Inject a false negative bug (clear a bit) and prove oracle detects it
    let mut corrupted = bitset.clone();
    corrupted.raw_bytes[0] = 0; // Clear first byte (bits 0..8)
    let violation = BloomSoundnessOracle::verify_zero_false_negatives(&corrupted, &keys);
    assert!(
        matches!(
            violation,
            Err(BloomSoundnessViolation::FalseNegativeDetected { .. })
        ),
        "Oracle must catch injected false negatives"
    );

    // 4. Bound validation on probe count
    assert!(
        matches!(
            VerifiedBloomBitset::new(1024, 0),
            Err(BloomSoundnessViolation::InvalidProbeCount { .. })
        ),
        "k=0 must be rejected"
    );
    assert!(
        matches!(
            VerifiedBloomBitset::new(1024, 31),
            Err(BloomSoundnessViolation::InvalidProbeCount { .. })
        ),
        "k > MAX_K must be rejected"
    );
}

#[test]
fn test_dual_log_recovery_anti_zombie_resurrection() {
    let anchor = ManifestRecoveryAnchor {
        min_log_number: 10,
        earliest_readable_seq: 500,
        manifest_head_seq: 500,
    };

    let mut gate = DualLogRecoveryGate::new(anchor);

    let wal_stream = vec![
        // Record 1: from dead/obsolete WAL segment 8 (< min_log_number 10)
        WalRecoveryRecord {
            segment_id: 8,
            seq: 200,
            key: b"zombie_key_1".to_vec(),
            value: Some(b"zombie_val_1".to_vec()),
        },
        // Record 2: from dead/obsolete WAL segment 9 (< min_log_number 10)
        WalRecoveryRecord {
            segment_id: 9,
            seq: 450,
            key: b"zombie_key_2".to_vec(),
            value: Some(b"zombie_val_2".to_vec()),
        },
        // Record 3: in active segment 10, but already consolidated to SST (seq 480 <= 500)
        WalRecoveryRecord {
            segment_id: 10,
            seq: 480,
            key: b"already_flushed_key".to_vec(),
            value: Some(b"old_val".to_vec()),
        },
        // Record 4: exactly at cutoff seq 500 (already in SST)
        WalRecoveryRecord {
            segment_id: 10,
            seq: 500,
            key: b"cutoff_key".to_vec(),
            value: Some(b"cutoff_val".to_vec()),
        },
        // Record 5: post-flush live mutation 501 (MUST be replayed)
        WalRecoveryRecord {
            segment_id: 10,
            seq: 501,
            key: b"live_key_alpha".to_vec(),
            value: Some(b"alpha_val".to_vec()),
        },
        // Record 6: post-flush live mutation 502 (MUST be replayed)
        WalRecoveryRecord {
            segment_id: 10,
            seq: 502,
            key: b"live_key_beta".to_vec(),
            value: Some(b"beta_val".to_vec()),
        },
        // Record 7: next active segment 11, live mutation 503 (tombstone)
        WalRecoveryRecord {
            segment_id: 11,
            seq: 503,
            key: b"live_key_alpha".to_vec(),
            value: None, // Tombstone deletion
        },
    ];

    // Check individual decision contracts
    assert_eq!(
        gate.decide(&wal_stream[0]),
        WalReplayDecision::SkipObsoleteSegment
    );
    assert_eq!(
        gate.decide(&wal_stream[1]),
        WalReplayDecision::SkipObsoleteSegment
    );
    assert_eq!(
        gate.decide(&wal_stream[2]),
        WalReplayDecision::SkipConsolidatedSeq
    );
    assert_eq!(
        gate.decide(&wal_stream[3]),
        WalReplayDecision::SkipConsolidatedSeq
    );
    assert_eq!(
        gate.decide(&wal_stream[4]),
        WalReplayDecision::ApplyToMemTable
    );
    assert_eq!(
        gate.decide(&wal_stream[5]),
        WalReplayDecision::ApplyToMemTable
    );
    assert_eq!(
        gate.decide(&wal_stream[6]),
        WalReplayDecision::ApplyToMemTable
    );

    // Execute full recovery
    let state = gate
        .execute_verified_replay(&wal_stream)
        .expect("Dual log replay should succeed");

    // Zombie keys MUST NOT exist
    assert!(!state.contains_key(&b"zombie_key_1"[..]));
    assert!(!state.contains_key(&b"zombie_key_2"[..]));
    assert!(!state.contains_key(&b"already_flushed_key"[..]));
    assert!(!state.contains_key(&b"cutoff_key"[..]));

    // Live mutations must be properly reflected
    assert_eq!(
        state.get(&b"live_key_alpha"[..]),
        Some(&None),
        "live_key_alpha was deleted by tombstone at seq 503"
    );
    assert_eq!(
        state.get(&b"live_key_beta"[..]),
        Some(&Some(b"beta_val".to_vec()))
    );
    assert_eq!(gate.last_applied_seq(), 503);

    // Monotonicity violation check
    let non_monotonic_stream = vec![
        WalRecoveryRecord {
            segment_id: 10,
            seq: 600,
            key: b"k1".to_vec(),
            value: Some(b"v1".to_vec()),
        },
        WalRecoveryRecord {
            segment_id: 10,
            seq: 599, // Goes backward!
            key: b"k2".to_vec(),
            value: Some(b"v2".to_vec()),
        },
    ];
    let err = gate.execute_verified_replay(&non_monotonic_stream);
    assert!(
        matches!(
            err,
            Err(DualLogRecoveryViolation::NonMonotonicWalSequence { .. })
        ),
        "Non-monotonic WAL replay must be rejected fail-closed"
    );
}

#[test]
fn test_tombstone_soundness_and_unmasking_prevention() {
    let mut catalog = MockLevelCatalog::default();
    // Level 5 contains an older version of "key_shadowed"
    catalog
        .levels
        .entry(5)
        .or_default()
        .insert(b"key_shadowed".to_vec());

    // 1. Safe purge case: key_clean at level 3, no snapshots, no lower levels contain it
    let safe_decision = TombstonePurgeOracle::evaluate_purge(
        b"key_clean",
        100, // tombstone_seq
        3,   // current_level
        6,   // max_level
        &[], // active_snapshots
        &catalog,
    );
    assert_eq!(safe_decision, TombstonePurgeDecision::SafeToPurge);
    assert!(TombstonePurgeOracle::verify_compaction_purge(
        b"key_clean",
        100,
        3,
        6,
        &[],
        &catalog,
        true // purged
    )
    .is_ok());

    // 2. Unmasking hazard: key_shadowed has a version in level 5, purging at level 2
    let unmask_decision = TombstonePurgeOracle::evaluate_purge(
        b"key_shadowed",
        100,
        2,
        6,
        &[],
        &catalog,
    );
    assert_eq!(
        unmask_decision,
        TombstonePurgeDecision::RetainForShadowedKey { shadow_level: 5 }
    );
    let err_unmask = TombstonePurgeOracle::verify_compaction_purge(
        b"key_shadowed",
        100,
        2,
        6,
        &[],
        &catalog,
        true,
    );
    assert!(
        matches!(
            err_unmask,
            Err(TombstoneSoundnessViolation::OlderVersionUnmasked {
                compaction_level: 2,
                shadow_level: 5,
                ..
            })
        ),
        "Purging tombstone when shadows exist must trigger unmasking violation"
    );

    // 3. Snapshot visibility hazard: snapshot seq 120 exists (>= tombstone seq 100)
    let snap_decision = TombstonePurgeOracle::evaluate_purge(
        b"key_clean",
        100,
        3,
        6,
        &[120], // active snapshot
        &catalog,
    );
    assert_eq!(
        snap_decision,
        TombstonePurgeDecision::RetainForSnapshot {
            blocking_snapshot: 120
        }
    );
    let err_snap = TombstonePurgeOracle::verify_compaction_purge(
        b"key_clean",
        100,
        3,
        6,
        &[120],
        &catalog,
        true,
    );
    assert!(
        matches!(
            err_snap,
            Err(TombstoneSoundnessViolation::SnapshotViewCorrupted {
                tombstone_seq: 100,
                snapshot_seq: 120,
                ..
            })
        ),
        "Purging tombstone visible to active snapshots must trigger snapshot violation"
    );
}

#[test]
fn test_comparator_strict_weak_ordering_axioms() {
    let adversarial_keys = AxiomaticComparatorVerifier::generate_adversarial_key_corpus();

    // 1. Verify standard ByteLexicographicalComparator
    let byte_cmp = ByteLexicographicalComparator;
    assert!(
        AxiomaticComparatorVerifier::verify_strict_weak_ordering(&byte_cmp, &adversarial_keys)
            .is_ok(),
        "ByteLexicographicalComparator must satisfy all 4 Strict Weak Ordering axioms"
    );

    // 2. Verify PrefixComparator
    let prefix_cmp = PrefixComparator { prefix_len: 4 };
    assert!(
        AxiomaticComparatorVerifier::verify_strict_weak_ordering(&prefix_cmp, &adversarial_keys)
            .is_ok(),
        "PrefixComparator must satisfy all 4 Strict Weak Ordering axioms"
    );

    // 3. Verify that an axiomatic violation is properly detected by a broken comparator
    struct BrokenNonTransitiveComparator;
    impl KeyComparator for BrokenNonTransitiveComparator {
        fn compare(&self, a: &[u8], b: &[u8]) -> Ordering {
            // Intentionally broken rock-paper-scissors comparison:
            // a < b < c < a
            if a == b"rock" && b == b"paper" {
                Ordering::Less
            } else if a == b"paper" && b == b"scissors" {
                Ordering::Less
            } else if a == b"scissors" && b == b"rock" {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        }
    }

    let broken_samples = vec![b"rock".to_vec(), b"paper".to_vec(), b"scissors".to_vec()];
    let broken_res = AxiomaticComparatorVerifier::verify_strict_weak_ordering(
        &BrokenNonTransitiveComparator,
        &broken_samples,
    );
    assert!(
        broken_res.is_err(),
        "Verifier must catch non-transitive or cyclic comparator violations"
    );
}

#[test]
fn test_direct_io_4096_alignment_and_buffer_contract() {
    // 1. Test verify_direct_io_request
    assert!(
        verify_direct_io_request(4096, 8192, 4096, DIRECT_IO_PAGE_ALIGNMENT).is_ok(),
        "Valid 4096-aligned request must pass"
    );
    assert!(
        verify_direct_io_request(512, 1024, 512, DIRECT_IO_SECTOR_ALIGNMENT).is_ok(),
        "Valid 512-aligned request must pass"
    );

    // Misaligned buffer pointer
    assert!(matches!(
        verify_direct_io_request(4097, 8192, 4096, DIRECT_IO_PAGE_ALIGNMENT),
        Err(DirectIoAlignmentViolation::MisalignedBufferPointer { .. })
    ));

    // Misaligned file offset
    assert!(matches!(
        verify_direct_io_request(4096, 4095, 4096, DIRECT_IO_PAGE_ALIGNMENT),
        Err(DirectIoAlignmentViolation::MisalignedFileOffset { .. })
    ));

    // Misaligned transfer length
    assert!(matches!(
        verify_direct_io_request(4096, 8192, 4000, DIRECT_IO_PAGE_ALIGNMENT),
        Err(DirectIoAlignmentViolation::MisalignedTransferLength { .. })
    ));

    // Zero length
    assert!(matches!(
        verify_direct_io_request(4096, 8192, 0, DIRECT_IO_PAGE_ALIGNMENT),
        Err(DirectIoAlignmentViolation::ZeroLengthTransfer)
    ));

    // 2. Test AlignedDirectBuffer container
    let mut buf = AlignedDirectBuffer::allocate(5000, DIRECT_IO_PAGE_ALIGNMENT);
    assert_eq!(
        buf.aligned_address() % DIRECT_IO_PAGE_ALIGNMENT,
        0,
        "Buffer address must be 4096-byte aligned"
    );
    assert_eq!(
        buf.len(),
        8192,
        "Capacity 5000 must round up to 8192 (2 * 4096)"
    );

    // Verify self-conformance
    assert!(buf
        .verify_conformance(4096, DIRECT_IO_PAGE_ALIGNMENT)
        .is_ok());

    // Write and read data safely
    let slice = buf.as_mut_slice();
    slice[0] = 0xAA;
    slice[8191] = 0xBB;
    assert_eq!(buf.as_slice()[0], 0xAA);
    assert_eq!(buf.as_slice()[8191], 0xBB);
}
