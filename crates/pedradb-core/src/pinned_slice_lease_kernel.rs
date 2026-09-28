//! RFC-0289: Pinned Slice Lease and Transparent Decoupling Kernel.
//!
//! Enforces bounded lifetime quotas on block cache pinning (PinSlice).
//! Automatically transitions zombie reader leases to isolated private copies
//! (Copy-on-Exceed), freeing global table references and unblocking LSM compaction.

/// Status of a pinned slice lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseState {
    /// Zero-copy pointer pinned into shared block cache.
    PinnedShared {
        /// Identifier of the pinned block in block cache.
        block_id: u64,
        /// Clock tick when pin was acquired.
        acquired_tick: u64,
    },
    /// Expired lease that was automatically decoupled to private memory.
    DetachedPrivateCopy {
        /// Byte size of the private copy.
        copied_bytes: usize,
    },
    /// Explicitly released lease.
    Released,
}

/// A pinned slice lease handle holding payload data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedSliceLease {
    /// Active payload slice.
    pub payload: Vec<u8>,
    /// State of the lease.
    pub state: LeaseState,
    /// Maximum allowed pin duration in clock ticks before forced decoupling.
    pub max_pin_ticks: u64,
}

impl PinnedSliceLease {
    /// Creates a new pinned shared lease referencing a block cache entry.
    #[must_use]
    pub fn new_pinned(
        block_id: u64,
        payload: Vec<u8>,
        acquired_tick: u64,
        max_pin_ticks: u64,
    ) -> Self {
        Self {
            payload,
            state: LeaseState::PinnedShared {
                block_id,
                acquired_tick,
            },
            max_pin_ticks,
        }
    }

    /// Evaluates current clock tick. If the lease duration exceeds `max_pin_ticks`,
    /// automatically decouples the lease from shared cache into private memory.
    /// Returns `true` if decoupling occurred on this tick.
    pub fn check_and_maybe_decouple(&mut self, current_tick: u64) -> bool {
        match self.state {
            LeaseState::PinnedShared { acquired_tick, .. } => {
                let elapsed = current_tick.saturating_sub(acquired_tick);
                if elapsed > self.max_pin_ticks {
                    // Decouple: Payload is already owned in this handle,
                    // we transition the state to signal that the shared block cache reference is released.
                    let len = self.payload.len();
                    self.state = LeaseState::DetachedPrivateCopy { copied_bytes: len };
                    true
                } else {
                    false
                }
            }
            LeaseState::DetachedPrivateCopy { .. } | LeaseState::Released => false,
        }
    }

    /// Explicitly releases the lease.
    pub fn release(&mut self) {
        self.state = LeaseState::Released;
    }

    /// Checks if the lease is currently in shared pinned state.
    #[must_use]
    pub fn is_pinned_shared(&self) -> bool {
        matches!(self.state, LeaseState::PinnedShared { .. })
    }

    /// Checks if the lease has been decoupled into private memory.
    #[must_use]
    pub fn is_detached_private(&self) -> bool {
        matches!(self.state, LeaseState::DetachedPrivateCopy { .. })
    }
}
