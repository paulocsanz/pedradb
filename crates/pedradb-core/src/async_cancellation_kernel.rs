//! Async Cancellation Safety and Leader Handover Kernel (RFC-0284 Pilar 10).
//!
//! Protects group commit pipelines against deadlocks, orphaned slots, and WAL corruption
//! when client threads or async tasks are cancelled (e.g. via RPC timeout or SIGINT).
//!
//! Guarantees:
//! 1. Lock-free slot status transition: `Waiting -> Cancelled` or `Waiting -> Committed`.
//! 2. Cancelled follower payload is skipped during WAL assembly without leaving unwritten holes.
//! 3. If the group leader cancels, it atomically transfers leadership to the first active follower.

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// Operational lifecycle state of a participant slot in a group commit batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CommitSlotState {
    /// Task has enrolled and is waiting for leader commit.
    Waiting = 0,
    /// Task was cancelled (timeout / abort) prior to WAL sync.
    Cancelled = 1,
    /// Task was committed and synced successfully.
    Committed = 2,
    /// Task was elected as the new group leader.
    ElectedLeader = 3,
}

impl CommitSlotState {
    fn from_u8(val: u8) -> Self {
        match val {
            0 => Self::Waiting,
            1 => Self::Cancelled,
            2 => Self::Committed,
            3 => Self::ElectedLeader,
            _ => unreachable!(),
        }
    }
}

/// Handle held by an enrolled client task.
pub struct CommitSlot {
    /// Slot identifier.
    pub slot_id: u64,
    /// Payload bytes to be persisted.
    pub payload: Vec<u8>,
    /// Atomic state shared between client task and group leader.
    state: Arc<AtomicU8>,
}

impl CommitSlot {
    /// Creates a new slot in `Waiting` state.
    pub fn new(slot_id: u64, payload: Vec<u8>) -> Self {
        Self {
            slot_id,
            payload,
            state: Arc::new(AtomicU8::new(CommitSlotState::Waiting as u8)),
        }
    }

    /// Read current state.
    pub fn current_state(&self) -> CommitSlotState {
        CommitSlotState::from_u8(self.state.load(Ordering::Acquire))
    }

    /// Attempts to cancel this slot asynchronously.
    ///
    /// Returns true if cancelled cleanly before commit, or false if already committed/promoted.
    pub fn try_cancel(&self) -> bool {
        self.state
            .compare_exchange(
                CommitSlotState::Waiting as u8,
                CommitSlotState::Cancelled as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    /// Group leader attempts to commit this slot.
    pub fn mark_committed(&self) -> bool {
        self.state
            .compare_exchange(
                CommitSlotState::Waiting as u8,
                CommitSlotState::Committed as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    /// Promotes this slot to become the new group leader.
    pub fn promote_to_leader(&self) -> bool {
        self.state
            .compare_exchange(
                CommitSlotState::Waiting as u8,
                CommitSlotState::ElectedLeader as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
}

/// Pipeline reconciler for group commit execution under cancellation.
pub struct GroupCommitReconciler;

impl GroupCommitReconciler {
    /// Assembles batch payload for WAL sync, filtering out cancelled slots.
    pub fn assemble_active_batch<'a>(
        slots: &'a [CommitSlot],
    ) -> (Vec<&'a [u8]>, Vec<u64>) {
        let mut active_payloads = Vec::new();
        let mut committed_ids = Vec::new();

        for slot in slots {
            if slot.mark_committed() {
                active_payloads.push(slot.payload.as_slice());
                committed_ids.push(slot.slot_id);
            }
        }

        (active_payloads, committed_ids)
    }

    /// When current leader aborts, finds the first waiting follower and transfers leadership.
    pub fn handover_leadership<'a>(
        followers: &'a [CommitSlot],
    ) -> Option<&'a CommitSlot> {
        for slot in followers {
            if slot.promote_to_leader() {
                return Some(slot);
            }
        }
        None
    }
}
