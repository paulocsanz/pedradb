//! Integration test suite for RFC-0336:
//! Miri UB-Zero Certification, Pointer Provenance, and Memory Safety Contracts for Unsafe Islands.

use pedradb_spec::unsafe_provenance_kernel::{
    verify_anti_vacuity_mutants_abatement, verify_buffer_bounds, verify_cqe_user_data_unique,
    verify_handle_generation_valid, verify_non_overlapping_spans, verify_pointer_alignment,
    verify_valid_file_descriptor, MAX_KEY_BYTES, MAX_VALUE_BYTES, SECTOR_ALIGN_4096,
    SECTOR_ALIGN_512,
};

#[test]
fn test_pointer_alignment_and_sector_bounds_direct_io() {
    // 1. Valid sector alignments (512 and 4096 bytes)
    for mult in 1..=16 {
        let addr_512 = mult * SECTOR_ALIGN_512;
        assert!(
            verify_pointer_alignment(addr_512, SECTOR_ALIGN_512).is_ok(),
            "Address {:#x} must satisfy 512-byte sector alignment",
            addr_512
        );
        let addr_4096 = mult * SECTOR_ALIGN_4096;
        assert!(
            verify_pointer_alignment(addr_4096, SECTOR_ALIGN_4096).is_ok(),
            "Address {:#x} must satisfy 4096-byte sector alignment",
            addr_4096
        );
    }

    // 2. Reject misaligned pointers (unaligned addresses must fail closed)
    let misaligned_offsets = [1, 2, 3, 7, 15, 31, 63, 127, 255, 511];
    for offset in misaligned_offsets {
        let addr = 4096 + offset;
        assert!(
            verify_pointer_alignment(addr, SECTOR_ALIGN_4096).is_err(),
            "Misaligned address {:#x} must be rejected for 4096-byte alignment",
            addr
        );
        assert!(
            verify_pointer_alignment(512 + offset, SECTOR_ALIGN_512).is_err(),
            "Misaligned address {:#x} must be rejected for 512-byte alignment",
            512 + offset
        );
    }

    // 3. Real memory allocation alignment verification
    let mut aligned_buf = vec![0u8; 8192];
    let buf_addr = aligned_buf.as_mut_ptr() as usize;
    // Alignments up to standard system alignment must succeed or be cleanly detected
    let sys_align = std::mem::align_of::<u8>();
    assert!(verify_pointer_alignment(buf_addr, sys_align).is_ok());
}

#[test]
fn test_slice_capacity_bounds_and_overflow_prevention() {
    // 1. Within legitimate capacity limits
    assert!(verify_buffer_bounds(0, MAX_KEY_BYTES).is_ok());
    assert!(verify_buffer_bounds(1024, MAX_KEY_BYTES).is_ok());
    assert!(verify_buffer_bounds(MAX_KEY_BYTES, MAX_KEY_BYTES).is_ok());

    assert!(verify_buffer_bounds(0, MAX_VALUE_BYTES).is_ok());
    assert!(verify_buffer_bounds(4096, MAX_VALUE_BYTES).is_ok());
    assert!(verify_buffer_bounds(MAX_VALUE_BYTES, MAX_VALUE_BYTES).is_ok());

    // 2. Beyond capacity limits (must fail closed before constructing slice)
    assert!(
        verify_buffer_bounds(MAX_KEY_BYTES + 1, MAX_KEY_BYTES).is_err(),
        "Key length exceeding MAX_KEY_BYTES must fail closed"
    );
    assert!(
        verify_buffer_bounds(MAX_VALUE_BYTES + 1, MAX_VALUE_BYTES).is_err(),
        "Value length exceeding MAX_VALUE_BYTES must fail closed"
    );

    // 3. Prevent usize overflow attacks
    assert!(verify_buffer_bounds(usize::MAX, MAX_KEY_BYTES).is_err());
    assert!(verify_buffer_bounds(usize::MAX, MAX_VALUE_BYTES).is_err());
}

#[test]
fn test_non_overlapping_memory_spans_contract() {
    // 1. Disjoint spans: [1000, 1100) and [2000, 2100)
    assert!(
        verify_non_overlapping_spans(1000, 100, 2000, 100).is_ok(),
        "Completely disjoint spans must succeed"
    );

    // 2. Abutting spans: [1000, 1100) and [1100, 1200) -> strictly disjoint
    assert!(
        verify_non_overlapping_spans(1000, 100, 1100, 100).is_ok(),
        "Strictly abutting spans (dst == src + len) must succeed without overlap"
    );
    assert!(
        verify_non_overlapping_spans(1100, 100, 1000, 100).is_ok(),
        "Strictly abutting spans (src == dst + len) must succeed without overlap"
    );

    // 3. Overlapping spans: [1000, 1100) and [1050, 1150)
    assert!(
        verify_non_overlapping_spans(1000, 100, 1050, 100).is_err(),
        "Partially overlapping spans must be rejected before copy_nonoverlapping"
    );
    assert!(
        verify_non_overlapping_spans(1050, 100, 1000, 100).is_err(),
        "Reverse partially overlapping spans must be rejected before copy_nonoverlapping"
    );

    // 4. Identical spans: [1000, 1100) and [1000, 1100)
    assert!(
        verify_non_overlapping_spans(1000, 100, 1000, 100).is_err(),
        "Identical memory ranges must be rejected"
    );

    // 5. Zero-length spans are vacously non-overlapping
    assert!(verify_non_overlapping_spans(1000, 0, 1000, 0).is_ok());
    assert!(verify_non_overlapping_spans(1000, 0, 1000, 100).is_ok());
}

#[test]
fn test_posix_fd_and_handle_generation_freshness() {
    // 1. File descriptors: valid vs negative/invalid
    assert!(verify_valid_file_descriptor(0).is_ok()); // stdin
    assert!(verify_valid_file_descriptor(1).is_ok()); // stdout
    assert!(verify_valid_file_descriptor(2).is_ok()); // stderr
    assert!(verify_valid_file_descriptor(42).is_ok());

    assert!(
        verify_valid_file_descriptor(-1).is_err(),
        "EBADF / negative fd must fail closed before FFI syscall"
    );
    assert!(
        verify_valid_file_descriptor(-99).is_err(),
        "Negative fd must fail closed"
    );

    // 2. Handle generation validation (slot and generation freshness)
    // Matching active generation is admitted
    assert!(verify_handle_generation_valid(0, 1, 1).is_ok());
    assert!(verify_handle_generation_valid(15, 100, 100).is_ok());

    // Stale generation (e.g. reused slot) is rejected
    assert!(
        verify_handle_generation_valid(15, 99, 100).is_err(),
        "Stale handle generation must be rejected"
    );
    // Generation 0 (reserved / uninitialized) is rejected
    assert!(
        verify_handle_generation_valid(15, 0, 100).is_err(),
        "Generation 0 must be rejected"
    );

    // 3. Unique user_data for concurrent io_uring SQEs
    let pending_sqes = [1001u64, 1002, 1003];
    assert!(verify_cqe_user_data_unique(&pending_sqes, 1004).is_ok());
    assert!(
        verify_cqe_user_data_unique(&pending_sqes, 1002).is_err(),
        "Reused user_data in flight must be rejected"
    );
}

#[test]
fn test_anti_vacuity_mutants_m1_m5_abatement() {
    // Verify that mechanical oracles kill mutants M1..M5
    for mutant_id in 1..=5 {
        assert!(
            verify_anti_vacuity_mutants_abatement(mutant_id),
            "Anti-vacuity mutant M{} must be abated by formal oracles",
            mutant_id
        );
    }
    // Unknown mutant IDs are not admitted
    assert!(!verify_anti_vacuity_mutants_abatement(0));
    assert!(!verify_anti_vacuity_mutants_abatement(6));
}
