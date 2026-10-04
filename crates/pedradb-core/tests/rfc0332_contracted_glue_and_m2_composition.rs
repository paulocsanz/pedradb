//! RFC-0332 Verification Suite:
//! 1. Contracted Syscall Glue (`pedradb-spec` ↔ `pedradb-posix` / `pedradb-io-uring`, AGENTS.md §4)
//! 2. End-to-End M2 Chained Invariant Composition (>= 80% Chaining, AGENTS.md §3)
//! 3. Direct-I/O & Kernel CQ Completion Queue Drain Armor
//! 4. Anti-Vacuity Mechanical Mutation Oracles (M1..M5)

use pedradb_spec::composition_m2_kernel::{
    total_chained_atoms, verify_m2_composition_ratio, DirectIoDrainInductiveState,
    M2CompositionViolation, WritePathInductiveState, GRAND_INDUCTIVE_CHAINS,
    MIN_REQUIRED_CHAINED_ATOMS, TOTAL_CATALOG_ATOMIC_FUNCTIONS,
};
use pedradb_spec::syscall_glue_kernel::{
    verify_cqe_drain, verify_cqe_harvest, verify_direct_io_alignment, verify_fadvise_post,
    verify_fadvise_pre, verify_fdatasync_post, verify_fdatasync_pre, verify_fsync_post,
    verify_fsync_pre, verify_mmap_bounds, verify_preallocate_post, verify_preallocate_pre,
    verify_pwrite_post, verify_pwrite_pre, verify_sqe_submission, IoUringViolation,
    PosixSyscallViolation,
};

#[test]
fn test_posix_syscall_glue_contracts_and_boundaries() {
    // 1. pwrite preconditions & boundaries
    assert_eq!(
        verify_pwrite_pre(-1, 64, 0),
        Err(PosixSyscallViolation::InvalidFd { fd: -1 })
    );
    assert_eq!(
        verify_pwrite_pre(5, 0, 100),
        Err(PosixSyscallViolation::ZeroLengthBuffer)
    );
    assert_eq!(
        verify_pwrite_pre(5, 10, u64::MAX - 5),
        Err(PosixSyscallViolation::OffsetOverflow {
            offset: u64::MAX - 5,
            len: 10
        })
    );
    assert!(verify_pwrite_pre(5, 4096, 0).is_ok());

    // 2. pwrite postconditions (no short write accepted without error)
    assert_eq!(
        verify_pwrite_post(4096, 2048),
        Err(PosixSyscallViolation::ShortWrite {
            requested: 4096,
            actual: 2048
        })
    );
    assert_eq!(verify_pwrite_post(4096, 4096), Ok(4096));

    // 3. fdatasync contracts (fail-closed on non-zero return)
    assert_eq!(
        verify_fdatasync_pre(-1),
        Err(PosixSyscallViolation::InvalidFd { fd: -1 })
    );
    assert!(verify_fdatasync_pre(3).is_ok());
    assert_eq!(
        verify_fdatasync_post(-1),
        Err(PosixSyscallViolation::FdatasyncFailed { rc: -1 })
    );
    assert_eq!(
        verify_fdatasync_post(5),
        Err(PosixSyscallViolation::FdatasyncFailed { rc: 5 })
    );
    assert!(verify_fdatasync_post(0).is_ok());

    // 4. fsync contracts
    assert_eq!(
        verify_fsync_pre(-1),
        Err(PosixSyscallViolation::InvalidFd { fd: -1 })
    );
    assert!(verify_fsync_pre(4).is_ok());
    assert_eq!(
        verify_fsync_post(-1),
        Err(PosixSyscallViolation::FsyncFailed { rc: -1 })
    );
    assert!(verify_fsync_post(0).is_ok());

    // 5. preallocate / fallocate contracts
    assert_eq!(
        verify_preallocate_pre(3, 0, 0),
        Err(PosixSyscallViolation::ZeroLengthBuffer)
    );
    assert_eq!(
        verify_preallocate_pre(3, u64::MAX - 10, 20),
        Err(PosixSyscallViolation::OffsetOverflow {
            offset: u64::MAX - 10,
            len: 20
        })
    );
    assert!(verify_preallocate_pre(3, 0, 1024 * 1024).is_ok());
    assert_eq!(
        verify_preallocate_post(-1),
        Err(PosixSyscallViolation::PreallocateFailed { rc: -1 })
    );
    assert!(verify_preallocate_post(0).is_ok());

    // 6. fadvise contracts
    assert_eq!(
        verify_fadvise_pre(-1, 0, 1024),
        Err(PosixSyscallViolation::InvalidFd { fd: -1 })
    );
    assert!(verify_fadvise_pre(3, 0, 1024).is_ok());
    assert_eq!(
        verify_fadvise_post(-1),
        Err(PosixSyscallViolation::FadviseFailed { rc: -1 })
    );
    assert!(verify_fadvise_post(0).is_ok());

    // 7. Direct-I/O 4096B sector alignment
    assert_eq!(
        verify_direct_io_alignment(4095, 0x1000, 4096, 4096),
        Err(PosixSyscallViolation::UnalignedSector {
            offset: 4095,
            ptr_addr: 0x1000,
            len: 4096,
            sector_size: 4096
        })
    );
    assert_eq!(
        verify_direct_io_alignment(4096, 0x1001, 4096, 4096),
        Err(PosixSyscallViolation::UnalignedSector {
            offset: 4096,
            ptr_addr: 0x1001,
            len: 4096,
            sector_size: 4096
        })
    );
    assert_eq!(
        verify_direct_io_alignment(4096, 0x1000, 4095, 4096),
        Err(PosixSyscallViolation::UnalignedSector {
            offset: 4096,
            ptr_addr: 0x1000,
            len: 4095,
            sector_size: 4096
        })
    );
    assert!(verify_direct_io_alignment(8192, 0x2000, 4096, 4096).is_ok());

    // 8. Memory-mapped file bounds
    assert_eq!(
        verify_mmap_bounds(1000, 500, 1200),
        Err(PosixSyscallViolation::MmapBoundsViolation {
            offset: 1000,
            len: 500,
            file_len: 1200
        })
    );
    assert!(verify_mmap_bounds(1000, 200, 1200).is_ok());
}

#[test]
fn test_io_uring_syscall_contracts_and_cq_overflow() {
    // 1. SQE submission invariants
    assert_eq!(
        verify_sqe_submission(0, 0, 0, 128),
        Err(IoUringViolation::ZeroUserData)
    );
    assert_eq!(
        verify_sqe_submission(10, 10, 0, 128),
        Err(IoUringViolation::NonMonotonicUserData { prev: 10, current: 10 })
    );
    assert_eq!(
        verify_sqe_submission(9, 10, 0, 128),
        Err(IoUringViolation::NonMonotonicUserData { prev: 10, current: 9 })
    );
    assert_eq!(
        verify_sqe_submission(11, 10, 128, 128),
        Err(IoUringViolation::SubmissionQueueOverflow { current_depth: 128, capacity: 128 })
    );
    assert!(verify_sqe_submission(11, 10, 10, 128).is_ok());

    // 2. CQE harvest postconditions
    assert_eq!(
        verify_cqe_harvest(-5, 11, 11, false),
        Err(IoUringViolation::NegativeCompletionResult { res: -5 })
    );
    assert_eq!(
        verify_cqe_harvest(4096, 12, 11, false),
        Err(IoUringViolation::TagMismatch { expected: 11, actual: 12 })
    );
    assert_eq!(
        verify_cqe_harvest(4096, 11, 11, true),
        Err(IoUringViolation::CompletionQueueOverflow)
    );
    assert_eq!(verify_cqe_harvest(4096, 11, 11, false), Ok(4096));

    // 3. Complete CQ drain verification
    assert_eq!(
        verify_cqe_drain(20, 19),
        Err(IoUringViolation::IncompleteDrain { expected: 20, harvested: 19 })
    );
    assert!(verify_cqe_drain(20, 20).is_ok());
    assert!(verify_cqe_drain(20, 25).is_ok());
}

#[test]
fn test_m2_composition_ratio_and_grand_inductive_chains() {
    // AGENTS.md §3: Chaining must reach >= 80% (265+/331 atomic functions).
    let total_catalog = TOTAL_CATALOG_ATOMIC_FUNCTIONS;
    assert_eq!(total_catalog, 331);

    let chained = total_chained_atoms();
    assert_eq!(chained, 269);
    assert!(chained >= MIN_REQUIRED_CHAINED_ATOMS);

    let ratio = (chained as f64) / (total_catalog as f64);
    assert!(
        ratio >= 0.80,
        "M2 chaining ratio {:.4} must be >= 0.80 (80%)",
        ratio
    );

    // Verify all 6 Grand Inductive Chains exist and sum to 269
    assert_eq!(GRAND_INDUCTIVE_CHAINS.len(), 6);
    let mut sum = 0;
    for chain in &GRAND_INDUCTIVE_CHAINS {
        assert!(chain.atom_count > 0);
        sum += chain.atom_count;
    }
    assert_eq!(sum, 269);

    assert_eq!(verify_m2_composition_ratio(), Ok(269));
}

#[test]
fn test_write_path_inductive_transitions_anti_vacuity() {
    // Chain 1: Write Path & Durability D1
    let state = WritePathInductiveState::new(1001, 1024);

    // Anti-vacuity M1: Skipping step 1 (attempting ticket alloc without admission)
    assert_eq!(
        state.step_ticket_alloc(1001),
        Err(M2CompositionViolation::BrokenInductiveLink { chain_id: 1, step_index: 2 })
    );

    let s1 = state.step_admission().expect("admission ok");
    let s2 = s1.step_ticket_alloc(1001).expect("ticket ok");
    let s3 = s2.step_batch_framed().expect("batch ok");
    let s4 = s3.step_wal_written().expect("wal write ok");

    // Anti-vacuity M2: Publishing before physical disk sync barrier (G1 violation)
    assert_eq!(
        s4.step_superversion_publish(1001),
        Err(M2CompositionViolation::UnpersistedPublication { ticket: 1001 })
    );

    let s5 = s4.step_fdatasync_barrier().expect("fdatasync ok");

    // Anti-vacuity M3: Sequence discontinuity (publishing older sequence than ticket)
    assert_eq!(
        s5.step_superversion_publish(1000),
        Err(M2CompositionViolation::SequenceDiscontinuity {
            expected_seq: 1001,
            actual_seq: 1000,
        })
    );

    let s6 = s5.step_superversion_publish(1001).expect("publish ok");
    assert_eq!(s6.step, 6);
    assert!(s6.disk_synced);
    assert_eq!(s6.published_seq, 1001);
}

#[test]
fn test_direct_io_drain_safety_under_cancellation_and_overflow() {
    // Chain 6: Direct-I/O & CQ Drain
    let state = DirectIoDrainInductiveState::new(32);

    // Anti-vacuity M4: Ring queue capacity overflow rejected
    assert_eq!(
        state.step_submit(33),
        Err(M2CompositionViolation::BrokenInductiveLink { chain_id: 6, step_index: 1 })
    );

    let s1 = state.step_submit(8).expect("submit 8 ok");
    let s2 = s1.step_detect_overflow(true).expect("overflow detected");

    // Anti-vacuity M5: Incomplete drain rejected fail-closed
    assert_eq!(
        s2.step_drain_all(7),
        Err(M2CompositionViolation::BrokenInductiveLink { chain_id: 6, step_index: 3 })
    );

    let s3 = s2.step_drain_all(8).expect("full drain 8 ok");
    assert_eq!(s3.step, 3);
    assert_eq!(s3.in_flight, 0);
    assert_eq!(s3.drained, 8);
}
