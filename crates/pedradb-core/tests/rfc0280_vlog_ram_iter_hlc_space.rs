//! RFC-0280 Test Suite:
//! - P0.1: VLog Referential Integrity & Zero Dangling Blob Pointers
//! - P0.2: RAM Pre-Flush Barrier & DRAM Bit-Rot Protection
//! - P1.1: Iterator Pinning & Hazard Reference Counting
//! - P1.2: Hybrid Logical Clock & Time-Travel Immunity
//! - P2.1: Amortized Space Bounds & ENOSPC Prevention

use pedradb_core::iterator_pinning_kernel::{
    verify_iterator_step_monotonicity, IteratorPinningManager,
};
use pedradb_core::monotonic_clock_kernel::MonotonicClock;
use pedradb_core::ram_preflush_barrier_kernel::{
    verify_preflush_barrier, PreFlushBarrierResult, StagedFlushEntry,
};
use pedradb_core::space_amplification_kernel::DiskSpaceBudget;
use pedradb_core::vlog_integrity_kernel::VlogStore;

#[test]
fn test_vlog_referential_integrity_and_gc_atomic_swap() {
    let mut vlog_store = VlogStore::new(1);
    let crc_fn = |data: &[u8]| crc32c::crc32c(data);

    // Write 3 large blobs to VLog
    let p1 = vlog_store.append(b"blob_payload_alpha", crc_fn(b"blob_payload_alpha")).unwrap();
    let p2 = vlog_store.append(b"blob_payload_beta", crc_fn(b"blob_payload_beta")).unwrap();
    let p3 = vlog_store.append(b"blob_payload_gamma", crc_fn(b"blob_payload_gamma")).unwrap();

    let lsm_pointers = vec![p1, p2, p3];
    assert!(
        vlog_store.verify_referential_integrity(&lsm_pointers),
        "Initial VLog pointers must satisfy 100% referential integrity"
    );

    // Simulate Garbage Collection: migrate p1 and p3 to generation 2
    let live_to_migrate = vec![p1, p3];
    let new_pointers = vlog_store.gc_migrate_blobs(2, &live_to_migrate).unwrap();
    assert_eq!(new_pointers.len(), 2);

    // Verify migrated pointers are sound in the new VLog file
    assert!(
        vlog_store.verify_referential_integrity(&new_pointers),
        "Migrated VLog pointers must be valid in the new generation file"
    );

    // Corrupt an offset in a pointer: verify that referential integrity catches the violation
    let mut corrupted_ptr = p2;
    corrupted_ptr.offset += 99999;
    assert!(
        !vlog_store.verify_referential_integrity(&[corrupted_ptr]),
        "Corrupted or out-of-bounds blob pointer must be detected"
    );
}

#[test]
fn test_ram_preflush_barrier_bitrot_and_inversion_detection() {
    let crc_fn = |data: &[u8]| crc32c::crc32c(data);

    // 1. Valid sorted entries
    let entries = vec![
        StagedFlushEntry::new(b"key_a".to_vec(), 100, b"val_100".to_vec(), crc_fn),
        StagedFlushEntry::new(b"key_b".to_vec(), 90, b"val_90".to_vec(), crc_fn),
        StagedFlushEntry::new(b"key_b".to_vec(), 80, b"val_80".to_vec(), crc_fn),
        StagedFlushEntry::new(b"key_c".to_vec(), 50, b"val_50".to_vec(), crc_fn),
    ];
    assert_eq!(
        verify_preflush_barrier(&entries, crc_fn),
        PreFlushBarrierResult::Pass
    );

    // 2. Corrupt DRAM payload (simulating bit flip in RAM)
    let mut corrupted_entries = entries.clone();
    corrupted_entries[1].value[0] ^= 0x01; // flip 1 bit in RAM
    match verify_preflush_barrier(&corrupted_entries, crc_fn) {
        PreFlushBarrierResult::PayloadChecksumCorrupted { index } => assert_eq!(index, 1),
        _ => panic!("Pre-flush barrier must intercept DRAM bit-rot"),
    }

    // 3. Corrupt Comparator Order (keys out of order due to index corruption)
    let mut inverted_entries = entries.clone();
    inverted_entries.swap(0, 1); // "key_b" before "key_a"
    match verify_preflush_barrier(&inverted_entries, crc_fn) {
        PreFlushBarrierResult::KeyOrderInversion { index } => assert_eq!(index, 0),
        _ => panic!("Pre-flush barrier must intercept key comparator inversion"),
    }
}

#[test]
fn test_iterator_pinning_and_concurrent_unlink_retention() {
    let mut pinning = IteratorPinningManager::new();

    // Iterator 1 opens and pins SST files [10, 11, 12]
    pinning.open_iterator(1, &[10, 11, 12]).unwrap();

    // Background compaction marks SST file 11 as obsolete
    let unlinked_immediately = pinning.mark_file_obsolete(11);
    assert!(
        !unlinked_immediately,
        "File 11 must NOT be physically unlinked while pinned by active iterator"
    );
    assert!(
        pinning.is_file_pinned(11),
        "File 11 must be retained by active iterator ref"
    );

    // Iterator 2 opens and also pins SST file 11
    pinning.open_iterator(2, &[11]).unwrap();

    // Iterator 1 finishes and closes
    let unlinked_on_close_1 = pinning.close_iterator(1);
    assert!(
        !unlinked_on_close_1.contains(&11),
        "File 11 still pinned by Iterator 2, cannot be unlinked yet"
    );

    // Iterator 2 finishes and closes
    let unlinked_on_close_2 = pinning.close_iterator(2);
    assert!(
        unlinked_on_close_2.contains(&11),
        "File 11 must now be unlinked since ref count reached 0"
    );
    assert!(
        pinning.can_safely_unlink(11),
        "File 11 is now safe to physically unlink"
    );

    // Verify step monotonicity
    let monotonic_scan = vec![10, 20, 30, 45, 100];
    assert!(verify_iterator_step_monotonicity(&monotonic_scan));

    let non_monotonic_scan = vec![10, 20, 15, 30];
    assert!(!verify_iterator_step_monotonicity(&non_monotonic_scan));
}

#[test]
fn test_monotonic_clock_time_travel_immunity() {
    let mut clock = MonotonicClock::new(1_000_000);

    // Physical time advances normally
    let t1 = clock.now(1_000_100);
    assert_eq!(t1, 1_000_100);

    // Physical time jumps backwards by 500 units (NTP step / leap second)
    let t2 = clock.now(1_000_050);
    // Monotonic clock MUST advance logically: max(1_000_050, 1_000_100 + 1)
    assert_eq!(t2, 1_000_101);
    assert!(t2 > t1);

    // TTL check
    let entry_created = 1_000_000;
    let ttl_duration = 500;
    // Current time is 1_000_101 < 1_000_500: not expired
    assert!(!clock.is_ttl_expired(entry_created, ttl_duration));

    // Time advances past expiry
    clock.now(1_000_600);
    assert!(clock.is_ttl_expired(entry_created, ttl_duration));

    // Verify complete sequence
    let erratic_physical_inputs = vec![100, 200, 150, 300, 250, 250, 400];
    assert!(MonotonicClock::verify_monotonicity_sequence(&erratic_physical_inputs));
}

#[test]
fn test_space_amplification_enospc_prevention() {
    // Total disk: 100 GiB, Available: 10 GiB
    let mut budget = DiskSpaceBudget::new(100 * 1024 * 1024 * 1024, 10 * 1024 * 1024 * 1024);

    // Compaction A demands 2 GiB input. Headroom requires 2x (4 GiB).
    // Available is 10 GiB >= 4 GiB -> Can admit!
    let input_a = 2 * 1024 * 1024 * 1024;
    assert!(budget.can_admit_compaction(input_a));
    budget.reserve_compaction(input_a).unwrap();

    // Compaction B demands 5 GiB input. Headroom requires 10 GiB.
    // Unreserved is (10 - 2) = 8 GiB < 10 GiB -> CANNOT ADMIT!
    let input_b = 5 * 1024 * 1024 * 1024;
    assert!(!budget.can_admit_compaction(input_b));
    assert!(budget.reserve_compaction(input_b).is_err());

    // Release Compaction A: input 2 GiB, compacted output 1.5 GiB (0.5 GiB reclaimed)
    let output_a = (1.5 * (1024.0 * 1024.0 * 1024.0)) as u64;
    budget.release_compaction(input_a, output_a);
    assert!(budget.verify_headroom_invariant());

    // Worst-case write amplification bound check
    let max_wa = DiskSpaceBudget::compute_max_write_amplification();
    assert!(max_wa <= 100, "Write amplification must be mathematically bounded");
}
