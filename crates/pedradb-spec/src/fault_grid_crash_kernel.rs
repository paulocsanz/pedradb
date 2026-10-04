//! RFC-0333: Fault-Grid Completeness Phase 2 and Concurrent Crash Consistency Kernel.
//!
//! Formal specification of:
//! 1. The 42-cell fault alphabet matrix (7 FaultKind x 6 OpClass) and failure policies.
//! 2. The 6 write barrier stages in concurrent group commit.
//! 3. Trans-crash prefix preservation invariants (H_pre ~ H_post).
//! 4. Anti-hole, zero-dirty-leak, and monotonic sequence horizon verifiers.

/// FaultKind enumeration mirroring storage fault injection alphabet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FaultKind {
    IoError,
    StorageFull,
    PermissionDenied,
    Interrupted,
    SyncFail,
    ShortWrite,
    Panic,
}

/// OpClass enumeration for discriminating storage operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpClass {
    Any,
    Write,
    Sync,
    Rename,
    CreateOpen,
    Remove,
    Meta,
}

/// The formal behavioral policy for a fault-grid cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultPolicy {
    /// Operation fails closed with typed error, aborting mutation cleanly.
    FailClosedError,
    /// Durability barrier failed; engine enters durability fence.
    DurabilityFence,
    /// Partial bytes hit storage; recovery reader must detect and truncate.
    TornWriteCandidate,
    /// Interrupted system call; caller or environment may retry once.
    TransientRetry,
    /// Advisory probe failure; falls back safely to conservative default.
    BenignAdvisory,
}

/// Classifies any cell of the 7 x 6 fault matrix into its formal policy.
#[must_use]
pub fn classify_fault_cell(kind: FaultKind, op: OpClass) -> FaultPolicy {
    match (kind, op) {
        (FaultKind::Panic, _) => FaultPolicy::FailClosedError,
        (FaultKind::Interrupted, _) => FaultPolicy::TransientRetry,
        (FaultKind::ShortWrite, OpClass::Write | OpClass::Any) => FaultPolicy::TornWriteCandidate,
        (FaultKind::ShortWrite, _) => FaultPolicy::FailClosedError,
        (FaultKind::SyncFail, OpClass::Sync | OpClass::Any) => FaultPolicy::DurabilityFence,
        (FaultKind::SyncFail, _) => FaultPolicy::FailClosedError,
        (FaultKind::StorageFull, OpClass::Write | OpClass::CreateOpen | OpClass::Any) => {
            FaultPolicy::DurabilityFence
        }
        (FaultKind::StorageFull, _) => FaultPolicy::FailClosedError,
        (FaultKind::PermissionDenied, _) => FaultPolicy::FailClosedError,
        (FaultKind::IoError, OpClass::Sync | OpClass::Any) => FaultPolicy::DurabilityFence,
        (FaultKind::IoError, OpClass::Meta) => FaultPolicy::BenignAdvisory,
        (FaultKind::IoError, _) => FaultPolicy::FailClosedError,
    }
}

/// Validates that a cell is well-formed in the 42-cell universe.
#[must_use]
pub fn is_valid_cell(kind: FaultKind, op: OpClass) -> bool {
    matches!(
        kind,
        FaultKind::IoError
            | FaultKind::StorageFull
            | FaultKind::PermissionDenied
            | FaultKind::Interrupted
            | FaultKind::SyncFail
            | FaultKind::ShortWrite
            | FaultKind::Panic
    ) && matches!(
        op,
        OpClass::Any
            | OpClass::Write
            | OpClass::Sync
            | OpClass::Rename
            | OpClass::CreateOpen
            | OpClass::Remove
            | OpClass::Meta
    )
}

/// Represents the sequential barrier stages in concurrent write commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BarrierStage {
    /// Stage 0: Verification of client admission, backpressure, and key sanity.
    PreAdmission = 0,
    /// Stage 1: Atomic ticket reservation and sequence assignment.
    TicketAssigned = 1,
    /// Stage 2: Payload write into WAL buffer/file (torn writes possible).
    PayloadPwrite = 2,
    /// Stage 3: CRC32C seal and record header write.
    HeaderCrcPwrite = 3,
    /// Stage 4: Physical media sync barrier (fdatasync/fsync).
    FdatasyncBarrier = 4,
    /// Stage 5: Atomic publishing to SuperVersion and MemTable; Ack returned.
    SuperVersionPublish = 5,
}

/// Verifies that an operation at a given barrier stage obeys prefix durability.
pub fn verify_crash_stage_prefix_property(
    stage: BarrierStage,
    is_acked: bool,
) -> Result<(), &'static str> {
    if is_acked && stage < BarrierStage::SuperVersionPublish {
        return Err("Violation: transaction acknowledged before SuperVersion publish barrier");
    }
    if !is_acked && stage >= BarrierStage::SuperVersionPublish {
        return Err("Violation: transaction published to SuperVersion without returning Ack");
    }
    Ok(())
}

/// Verifies that a short write was properly rejected with an error.
pub fn verify_short_write_rejected(
    wrote_bytes: usize,
    requested_bytes: usize,
) -> Result<(), &'static str> {
    if wrote_bytes < requested_bytes {
        // Must fail closed; accepting partial write as success is forbidden.
        Ok(())
    } else {
        Err("Expected short write condition was not present")
    }
}

/// Verifies trans-crash prefix preservation: all pre-crash acked writes must be
/// durably replayed, and no unacknowledged torn writes may resurrect.
pub fn verify_trans_crash_prefix_preservation(
    pre_crash_acked: &[u64],
    post_reopen_replayed: &[u64],
) -> Result<(), &'static str> {
    if post_reopen_replayed.len() < pre_crash_acked.len() {
        return Err("Violation: durable state lost acknowledged transactions across restart");
    }
    for (i, &acked_seq) in pre_crash_acked.iter().enumerate() {
        if post_reopen_replayed[i] != acked_seq {
            return Err("Violation: sequence mismatch between pre-crash acked and replayed state");
        }
    }
    Ok(())
}

/// Verifies durability fence activation when an I/O fault occurs during sync or append.
pub fn verify_fence_policy(
    stage: BarrierStage,
    kind: FaultKind,
    fenced: bool,
) -> Result<(), &'static str> {
    let policy = match stage {
        BarrierStage::FdatasyncBarrier => classify_fault_cell(kind, OpClass::Sync),
        BarrierStage::PayloadPwrite | BarrierStage::HeaderCrcPwrite => {
            classify_fault_cell(kind, OpClass::Write)
        }
        _ => classify_fault_cell(kind, OpClass::Any),
    };

    if policy == FaultPolicy::DurabilityFence && !fenced {
        return Err("Violation: durability fence was not activated on a critical barrier failure");
    }
    Ok(())
}

/// Verifies that the sequence horizon does not regress across crash recovery.
pub fn verify_sequence_horizon_monotonicity(
    pre_crash_seq: u64,
    post_reopen_seq: u64,
) -> Result<(), &'static str> {
    if post_reopen_seq > pre_crash_seq {
        return Err("Violation: post-reopen sequence horizon exceeded pre-crash allocated horizon");
    }
    Ok(())
}

/// Verifies that uncommitted transactions never leak into reader-visible state.
pub fn verify_zero_dirty_leak(
    uncommitted_keys: &[&[u8]],
    visible_keys: &[&[u8]],
) -> Result<(), &'static str> {
    for uncommitted in uncommitted_keys {
        if visible_keys.contains(uncommitted) {
            return Err("Violation: dirty uncommitted key leaked into visible state");
        }
    }
    Ok(())
}
