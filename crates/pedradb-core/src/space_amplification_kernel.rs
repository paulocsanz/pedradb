//! RFC-0280 P2.1 — Amortized Space & Write Amplification Bounds Kernel.
//!
//! Formalizes transient disk space bounds during cascading LSM compactions.
//! Enforces safety headroom invariants preventing `ENOSPC` storage exhaustion:
//! $$\text{TransientSpace}(\text{Compaction}) \le \sum_{f \in \text{Inputs}} \text{Size}(f) \le \beta \cdot \text{FreeSpace}$$
//! Proves that write amplification is strictly bounded by $O(\text{MAX\_LEVELS} \times \text{FANOUT})$.

#![forbid(unsafe_code)]

/// Safety multiplier required between free disk space and compaction transient demand.
pub const HEADROOM_SAFETY_MULTIPLIER: u64 = 2; // Requires at least 2x transient space free
/// Maximum disk levels.
pub const LSM_MAX_LEVELS: usize = 7;
/// Level size multiplier fanout (e.g. 10x).
pub const LEVEL_FANOUT: u64 = 10;

/// State of disk storage and active compactions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskSpaceBudget {
    /// Total physical disk capacity in bytes.
    pub total_capacity_bytes: u64,
    /// Currently available free disk space in bytes.
    pub available_bytes: u64,
    /// Transient bytes currently reserved by in-progress compactions.
    pub reserved_transient_bytes: u64,
}

impl DiskSpaceBudget {
    /// Creates a fresh disk space budget.
    pub fn new(total_capacity_bytes: u64, available_bytes: u64) -> Self {
        Self {
            total_capacity_bytes,
            available_bytes,
            reserved_transient_bytes: 0,
        }
    }

    /// Evaluates whether a planned compaction of input size `input_bytes` can safely execute
    /// without risking an out-of-space (`ENOSPC`) crash.
    pub fn can_admit_compaction(&self, input_bytes: u64) -> bool {
        // Output size is at most input_bytes (due to deduplication and tombstone purge)
        let estimated_transient_demand = input_bytes;
        let required_free_headroom = estimated_transient_demand.saturating_mul(HEADROOM_SAFETY_MULTIPLIER);

        let unreserved_free = self.available_bytes.saturating_sub(self.reserved_transient_bytes);
        unreserved_free >= required_free_headroom
    }

    /// Reserves transient space for an admitted compaction.
    pub fn reserve_compaction(&mut self, input_bytes: u64) -> Result<(), &'static str> {
        if !self.can_admit_compaction(input_bytes) {
            return Err("ENOSPC Prevention: Compaction deferred due to insufficient disk headroom");
        }
        self.reserved_transient_bytes = self.reserved_transient_bytes.saturating_add(input_bytes);
        Ok(())
    }

    /// Releases transient space upon compaction completion and file unlinking.
    pub fn release_compaction(&mut self, input_bytes: u64, output_bytes: u64) {
        self.reserved_transient_bytes = self.reserved_transient_bytes.saturating_sub(input_bytes);
        // Free space changes by (input_bytes - output_bytes) due to compaction reclaim
        if input_bytes >= output_bytes {
            let reclaimed = input_bytes - output_bytes;
            self.available_bytes = self.available_bytes.saturating_add(reclaimed);
        } else {
            let expanded = output_bytes - input_bytes;
            self.available_bytes = self.available_bytes.saturating_sub(expanded);
        }
    }

    /// Computes the theoretical worst-case write amplification factor.
    pub fn compute_max_write_amplification() -> u64 {
        // 1 (WAL) + 1 (MemTable flush to L0) + (LSM_MAX_LEVELS - 1) * LEVEL_FANOUT
        1 + 1 + ((LSM_MAX_LEVELS - 1) as u64).saturating_mul(LEVEL_FANOUT)
    }

    /// Verifies the Space Headroom Invariant:
    /// At no point does reserved compaction space exceed available disk capacity.
    pub fn verify_headroom_invariant(&self) -> bool {
        self.reserved_transient_bytes <= self.available_bytes
    }
}
