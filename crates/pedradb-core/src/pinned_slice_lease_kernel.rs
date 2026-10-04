//! RFC-0289: Pinned Slice Lease and Transparent Decoupling Kernel.
//!
//! Enforces bounded lifetime quotas on block cache pinning (PinSlice).
//! Automatically transitions zombie reader leases to isolated private copies
//! (Copy-on-Exceed), freeing global table references and unblocking LSM compaction.

/// Errors occurring during pinned slice lease lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinnedLeaseError {
    ZeroBlockId,
    EmptyPayload,
}

impl std::fmt::Display for PinnedLeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroBlockId => write!(f, "Block ID cannot be 0"),
            Self::EmptyPayload => write!(f, "Payload cannot be empty"),
        }
    }
}

impl std::error::Error for PinnedLeaseError {}

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
    /// Block identifier being pinned.
    pub block_id: u64,
    /// Active payload slice.
    pub payload: Vec<u8>,
    /// State of the lease.
    pub state: LeaseState,
    /// Maximum allowed pin duration in clock ticks before forced decoupling.
    pub max_pin_ticks: u64,
}

impl PinnedSliceLease {
    /// Creates a new pinned shared lease referencing a block cache entry safely.
    pub fn try_new_pinned(
        block_id: u64,
        payload: Vec<u8>,
        acquired_tick: u64,
        max_pin_ticks: u64,
    ) -> Result<Self, PinnedLeaseError> {
        if block_id == 0 {
            return Err(PinnedLeaseError::ZeroBlockId);
        }
        if payload.is_empty() {
            return Err(PinnedLeaseError::EmptyPayload);
        }
        Ok(Self::new_pinned(block_id, payload, acquired_tick, max_pin_ticks))
    }

    /// Creates a new pinned shared lease referencing a block cache entry.
    #[must_use]
    pub fn new_pinned(
        block_id: u64,
        payload: Vec<u8>,
        acquired_tick: u64,
        max_pin_ticks: u64,
    ) -> Self {
        Self {
            block_id,
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
        self.check_and_decouple_evict(current_tick).is_some()
    }

    /// Evaluates current clock tick and decouples if lease duration exceeded.
    /// Returns `Some(block_id)` if decoupling occurred on this tick, signaling
    /// to the caller to decrement the shared pin reference count in block cache.
    pub fn check_and_decouple_evict(&mut self, current_tick: u64) -> Option<u64> {
        match self.state {
            LeaseState::PinnedShared { block_id, acquired_tick } => {
                let elapsed = current_tick.saturating_sub(acquired_tick);
                if elapsed > self.max_pin_ticks
                    || (self.max_pin_ticks == 0 && current_tick >= acquired_tick)
                {
                    let len = self.payload.len();
                    self.state = LeaseState::DetachedPrivateCopy { copied_bytes: len };
                    Some(block_id)
                } else {
                    None
                }
            }
            LeaseState::DetachedPrivateCopy { .. } | LeaseState::Released => None,
        }
    }

    /// Returns the remaining pin duration in ticks before decoupling occurs.
    #[must_use]
    pub fn remaining_pin_ticks(&self, current_tick: u64) -> u64 {
        match self.state {
            LeaseState::PinnedShared { acquired_tick, .. } => {
                let elapsed = current_tick.saturating_sub(acquired_tick);
                self.max_pin_ticks.saturating_sub(elapsed)
            }
            _ => 0,
        }
    }

    /// Returns a slice of the payload if the lease is active (pinned or detached).
    /// Returns `None` if the lease has been released.
    #[must_use]
    pub fn get_slice(&self) -> Option<&[u8]> {
        if self.is_released() {
            None
        } else {
            Some(self.payload.as_slice())
        }
    }

    /// Explicitly releases the lease and immediately reclaims payload memory.
    pub fn release(&mut self) {
        self.state = LeaseState::Released;
        self.payload.clear();
        self.payload.shrink_to_fit();
    }

    /// Explicitly releases the lease and returns `Some(block_id)` if the block
    /// was still pinned in shared cache, so the caller can unpin it immediately.
    pub fn release_and_unpin(&mut self) -> Option<u64> {
        let unpin = match self.state {
            LeaseState::PinnedShared { block_id, .. } => Some(block_id),
            _ => None,
        };
        self.release();
        unpin
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

    /// Checks if the lease has been explicitly released.
    #[must_use]
    pub fn is_released(&self) -> bool {
        matches!(self.state, LeaseState::Released)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pinned_slice_lease_invariants_red_to_green() {
        assert_eq!(
            PinnedSliceLease::try_new_pinned(0, vec![1, 2, 3], 100, 50),
            Err(PinnedLeaseError::ZeroBlockId)
        );

        assert_eq!(
            PinnedSliceLease::try_new_pinned(42, vec![], 100, 50),
            Err(PinnedLeaseError::EmptyPayload)
        );

        let lease = PinnedSliceLease::try_new_pinned(42, vec![1, 2, 3], 100, 50);
        assert!(lease.is_ok());

        let disp = format!("{}", PinnedLeaseError::ZeroBlockId);
        assert!(!disp.is_empty());
    }
}
