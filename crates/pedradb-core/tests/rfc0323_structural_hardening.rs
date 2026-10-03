//! RFC-0323: Structural Hardening, Bug Class Eradication & Typestate Verification Suite.
//!
//! Mechanically proves the 5 Structural Impossibility Barriers:
//! 1. Barreira 1: Typestate Pattern & RAII Quiescence Leases.
//! 2. Barreira 2: Strong Domain Types (`NonZeroU64`, `NonNilUuid`).
//! 3. Barreira 3: Closed Structs & Smart Constructors (validating invariants upon construction).
//! 4. Barreira 4: Complete Float Sanitization (`FiniteF64`).
//! 5. Barreira 5: Global structural verification oracle.

#![forbid(unsafe_code)]

use pedradb_core::structural_hardening_kernel::{
    verify_structural_invariants, ClosedAdaptiveBloomBudget, ClosedParallelSubcompactionSlice,
    ClosedTombstoneDrainConfig, FileNumber, FiniteF64, Generation, MmapTypestateRegion,
    NonNilUuid, SequenceNumber, SstId, StructuralHardeningError,
};

#[test]
fn rfc0323_barreira_2_domain_types_reject_zero_and_nil() {
    // 1. FileNumber rejects 0
    assert_eq!(
        FileNumber::try_new(0),
        Err(StructuralHardeningError::ZeroIdentifierHazard { entity: "FileNumber" })
    );
    let fn1 = FileNumber::try_new(1048576).expect("valid file number");
    assert_eq!(fn1.as_u64(), 1048576);

    // 2. SequenceNumber rejects 0
    assert_eq!(
        SequenceNumber::try_new(0),
        Err(StructuralHardeningError::ZeroIdentifierHazard { entity: "SequenceNumber" })
    );
    let seq1 = SequenceNumber::try_new(42).expect("valid sequence");
    assert_eq!(seq1.as_u64(), 42);

    // 3. Generation rejects 0
    assert_eq!(
        Generation::try_new(0),
        Err(StructuralHardeningError::ZeroIdentifierHazard { entity: "Generation" })
    );
    let gen1 = Generation::try_new(1).expect("valid generation");
    assert_eq!(gen1.as_u64(), 1);

    // 4. SstId rejects 0
    assert_eq!(
        SstId::try_new(0),
        Err(StructuralHardeningError::ZeroIdentifierHazard { entity: "SstId" })
    );
    let sst1 = SstId::try_new(999).expect("valid sst id");
    assert_eq!(sst1.as_u64(), 999);

    // 5. NonNilUuid rejects all-zero UUID
    assert_eq!(
        NonNilUuid::try_new([0u8; 16]),
        Err(StructuralHardeningError::NilUuidHazard)
    );
    let valid_uuid = NonNilUuid::try_new([
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
        0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
    ]).expect("valid non-nil uuid");
    assert_eq!(valid_uuid.as_bytes()[0], 0x01);

    // Slice decoding checks length bounds
    assert_eq!(
        NonNilUuid::try_from_slice(&[1, 2, 3]),
        Err(StructuralHardeningError::SliceDecodeLengthUnderflow { expected: 16, actual: 3 })
    );
}

#[test]
fn rfc0323_barreira_4_finite_f64_sanitization() {
    // Rejects NaN, +Inf, -Inf
    assert!(FiniteF64::try_new(f64::NAN).is_err());
    assert!(FiniteF64::try_new(f64::INFINITY).is_err());
    assert!(FiniteF64::try_new(f64::NEG_INFINITY).is_err());

    // Valid values work
    let a = FiniteF64::try_new(12.5).expect("valid float");
    let b = FiniteF64::try_new(2.5).expect("valid float");

    assert_eq!(a.get(), 12.5);
    assert_eq!(b.get(), 2.5);

    // Checked arithmetic
    assert_eq!(a.checked_add(b).unwrap().get(), 15.0);
    assert_eq!(a.checked_sub(b).unwrap().get(), 10.0);
    assert_eq!(a.checked_mul(b).unwrap().get(), 31.25);
    assert_eq!(a.checked_div(b).unwrap().get(), 5.0);

    // Division by zero fails closed
    let zero = FiniteF64::try_new(0.0).expect("zero is finite");
    assert!(a.checked_div(zero).is_err());
}

#[test]
fn rfc0323_barreira_3_smart_constructors_enforce_invariants() {
    let half = FiniteF64::try_new(0.5).unwrap();

    // 1. ClosedTombstoneDrainConfig
    // Invariant: tombstones <= total
    assert!(matches!(
        ClosedTombstoneDrainConfig::try_new(100, 101, half),
        Err(StructuralHardeningError::CorruptedRecordAccounting { tombstones: 101, total: 100 })
    ));

    // Valid configuration
    let cfg = ClosedTombstoneDrainConfig::try_new(1000, 600, half).expect("valid drain cfg");
    assert_eq!(cfg.total_records(), 1000);
    assert_eq!(cfg.tombstone_records(), 600);
    assert_eq!(cfg.live_records(), 400);
    assert!(cfg.should_drain()); // 600/1000 = 0.6 >= 0.5

    let cfg_low = ClosedTombstoneDrainConfig::try_new(1000, 200, half).expect("valid drain cfg");
    assert!(!cfg_low.should_drain()); // 200/1000 = 0.2 < 0.5

    // 2. ClosedParallelSubcompactionSlice
    // Invariant: start < end
    assert_eq!(
        ClosedParallelSubcompactionSlice::try_new(b"zebra".to_vec(), b"apple".to_vec(), 0, 4),
        Err(StructuralHardeningError::InvertedSliceBounds)
    );
    assert_eq!(
        ClosedParallelSubcompactionSlice::try_new(b"same".to_vec(), b"same".to_vec(), 0, 4),
        Err(StructuralHardeningError::InvertedSliceBounds)
    );

    // Invariant: partition_idx < total_partitions
    assert!(matches!(
        ClosedParallelSubcompactionSlice::try_new(b"a".to_vec(), b"b".to_vec(), 4, 4),
        Err(StructuralHardeningError::PartitionOutOfBounds { index: 4, total: 4 })
    ));

    let slice = ClosedParallelSubcompactionSlice::try_new(b"apple".to_vec(), b"banana".to_vec(), 1, 3)
        .expect("valid slice");
    assert_eq!(slice.start_bound(), b"apple");
    assert_eq!(slice.end_bound(), b"banana");
    assert_eq!(slice.output_partition_idx(), 1);
    assert_eq!(slice.total_partitions(), 3);

    // 3. ClosedAdaptiveBloomBudget
    // Rejects zero total_keys
    assert!(ClosedAdaptiveBloomBudget::try_new(0, 1024, 0.01).is_err());
    // Rejects zero ram_budget_bytes
    assert!(ClosedAdaptiveBloomBudget::try_new(1000, 0, 0.01).is_err());
    // Rejects fpp outside (0, 1)
    assert!(ClosedAdaptiveBloomBudget::try_new(1000, 1024, 0.0).is_err());
    assert!(ClosedAdaptiveBloomBudget::try_new(1000, 1024, 1.0).is_err());
    assert!(ClosedAdaptiveBloomBudget::try_new(1000, 1024, 1.5).is_err());

    let bloom_budget = ClosedAdaptiveBloomBudget::try_new(1_000_000, 2_000_000, 0.01)
        .expect("valid bloom budget");
    assert_eq!(bloom_budget.total_keys(), 1_000_000);
    assert_eq!(bloom_budget.ram_budget_bytes(), 2_000_000);
    assert_eq!(bloom_budget.bits_per_key().get(), 16.0); // (2_000_000 * 8) / 1_000_000 = 16.0
}

#[test]
fn rfc0323_barreira_1_typestate_pattern_quiescence() {
    let fn1 = FileNumber::try_new(777).unwrap();
    let active_region = MmapTypestateRegion::try_new(fn1);
    assert_eq!(active_region.file_number(), fn1);

    // Acquire reader lease
    let lease1 = active_region.try_acquire_lease().expect("lease acquire ok");
    let lease2 = active_region.try_acquire_lease().expect("lease acquire ok");

    // Begin quiescence: consumes active_region by value into quiescing_region
    let quiescing_region = active_region.begin_quiescence();
    assert_eq!(quiescing_region.active_readers(), 2);

    // Attempting to poll quiescence while leases are held fails safely
    let poll_result = quiescing_region.poll_quiesced();
    assert!(poll_result.is_err());
    let (quiescing_region, err) = poll_result.unwrap_err();
    assert_eq!(err, StructuralHardeningError::QuiescenceDrainPending { active_readers: 2 });

    // Drop one lease
    drop(lease1);
    assert_eq!(quiescing_region.active_readers(), 1);

    // Drop second lease: all readers drained
    drop(lease2);
    assert_eq!(quiescing_region.active_readers(), 0);

    // Now poll quiesced succeeds and transitions into ReclaimedRegion
    let reclaimed_region = quiescing_region.poll_quiesced().expect("quiescence complete");
    assert!(reclaimed_region.is_reclaimed());
    assert_eq!(reclaimed_region.file_number(), fn1);
}

#[test]
fn rfc0323_global_structural_verification_oracle() {
    assert!(verify_structural_invariants(), "Global structural invariants must hold");
}
