//! RFC-0326: POSIX Syscall Contract Verification Suite.
//!
//! Mechanically verifies that POSIX syscall errors (ENOSPC, EIO, EINTR, short writes)
//! deterministically fence the engine and fail closed (RFC-0273 Rule 4).

#![forbid(unsafe_code)]

use pedradb_core::posix_syscall_contract_kernel::{
    PosixContractError, PosixSyscallContract, POSIX_EIO, POSIX_ENOSPC, POSIX_EROFS,
};

#[test]
fn rfc0326_posix_contract_pwrite_lifecycle() {
    let mut contract = PosixSyscallContract::new("/var/data/wal/000001.wal");
    assert!(!contract.is_durability_fenced());
    assert_eq!(contract.total_short_writes(), 0);

    // 1. Successful full write
    let written = contract
        .evaluate_write_result(3, 4096, Ok(4096))
        .expect("write ok");
    assert_eq!(written, 4096);
    assert!(!contract.is_durability_fenced());

    // 2. Short write hazard detected
    let short_err = contract.evaluate_write_result(3, 4096, Ok(1024));
    assert_eq!(
        short_err,
        Err(PosixContractError::ShortWriteHazard {
            requested: 4096,
            written: 1024
        })
    );
    assert_eq!(contract.total_short_writes(), 1);

    // 3. ENOSPC error activates durability fence
    let enospc_err = contract.evaluate_write_result(3, 4096, Err(POSIX_ENOSPC));
    assert!(matches!(enospc_err, Err(PosixContractError::StorageCapacityExhausted { .. })));
    assert!(contract.is_durability_fenced());

    // 4. Subsequent writes blocked by durability fence
    let blocked_err = contract.evaluate_write_result(3, 4096, Ok(4096));
    assert_eq!(
        blocked_err,
        Err(PosixContractError::SyscallBlockedByDurabilityFence { op: "pwrite" })
    );

    // 5. Clear fence post-recovery
    contract.clear_fence_post_recovery();
    assert!(!contract.is_durability_fenced());

    // 6. EIO error activates durability fence
    let eio_err = contract.evaluate_write_result(3, 4096, Err(POSIX_EIO));
    assert!(matches!(eio_err, Err(PosixContractError::HardwareDegraded { .. })));
    assert!(contract.is_durability_fenced());
}

#[test]
fn rfc0326_posix_contract_fdatasync_and_eintr() {
    let mut contract = PosixSyscallContract::new("/var/data/sst/000042.sst");

    // 1. Clean fdatasync
    assert!(contract.evaluate_sync_result(5, Ok(())).is_ok());

    // 2. Sync failure with EROFS fences the file
    let sync_err = contract.evaluate_sync_result(5, Err(POSIX_EROFS));
    assert!(matches!(sync_err, Err(PosixContractError::HardwareDegraded { .. })));
    assert!(contract.is_durability_fenced());

    // 3. Attempting sync while fenced fails immediately
    let fenced_sync = contract.evaluate_sync_result(5, Ok(()));
    assert_eq!(
        fenced_sync,
        Err(PosixContractError::SyscallBlockedByDurabilityFence { op: "fdatasync" })
    );

    // 4. EINTR retry bounds
    assert!(contract.check_eintr_retry(0).is_ok());
    assert!(contract.check_eintr_retry(15).is_ok());
    assert_eq!(
        contract.check_eintr_retry(16),
        Err(PosixContractError::InterruptedExhausted { attempts: 16 })
    );
}
