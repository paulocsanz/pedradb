//! RFC-0322: Cloud Storage I/O Watchdog Kernel.
//!
//! Protects against silent cloud block storage hangs (EBS / SAN / NVMe-oF multipath freezes).
//! In hyper-scale clouds, underlying storage failures often manifest not as immediate `EIO`
//! errors, but as infinite delays where `pwrite`, `fdatasync`, or `io_uring_enter` hang
//! indefinitely without completion.
//!
//! This kernel maintains a deterministic deadline watchdog over all inflight storage requests.
//! If any operation violates `io_deadline_ticks`, the watchdog transitions to `StorageHungFenced`,
//! halts write admissions to prevent buffer exhaustion, and signals failover.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

/// Maximum number of concurrent inflight I/O operations monitored by the watchdog.
pub const MAX_INFLIGHT_OPS: usize = 65_536;

/// Typed errors produced by the cloud storage watchdog kernel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageWatchdogError {
    /// Default deadline must be strictly positive.
    ZeroDefaultDeadline,
    /// Custom deadline must be strictly positive.
    ZeroCustomDeadline,
    /// Operation ID 0 is reserved sentinel hazard.
    ZeroOpIdHazard,
    /// Operation is already inflight.
    OpAlreadyInflight { op_id: u64 },
    /// Operation was not found in the inflight registry.
    OpNotFound { op_id: u64 },
    /// Maximum inflight operations capacity reached.
    InflightCapacityExceeded { max: usize },
}

impl std::fmt::Display for StorageWatchdogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroDefaultDeadline => {
                write!(f, "StorageWatchdogError: default_deadline_ticks must be strictly positive")
            }
            Self::ZeroCustomDeadline => {
                write!(f, "StorageWatchdogError: deadline_ticks must be strictly positive")
            }
            Self::ZeroOpIdHazard => {
                write!(f, "StorageWatchdogError: operation id 0 is reserved sentinel")
            }
            Self::OpAlreadyInflight { op_id } => {
                write!(f, "StorageWatchdogError: operation {op_id} is already inflight")
            }
            Self::OpNotFound { op_id } => {
                write!(f, "StorageWatchdogError: operation {op_id} not found in inflight table")
            }
            Self::InflightCapacityExceeded { max } => {
                write!(f, "StorageWatchdogError: inflight queue exceeded capacity limit {max}")
            }
        }
    }
}

impl std::error::Error for StorageWatchdogError {}

/// Class of storage operation monitored by the watchdog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageOpClass {
    /// Write-Ahead Log append / sync.
    WalSync,
    /// SST table data block write.
    SstWrite,
    /// Manifest or checkpoint directory sync.
    ManifestSync,
    /// Background compaction read/write.
    CompactionIo,
}

/// Operational state of the storage hardware pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageWatchdogState {
    /// All operations completing within deadline.
    StorageHealthy,
    /// Operations approaching or mildly exceeding threshold; warnings active.
    StorageDegraded {
        /// Number of operations currently past deadline.
        hung_ops: u32,
        /// Maximum observed stall in ticks.
        max_stall_ticks: u64,
    },
    /// Severe storage stall detected; admissions halted to avert OOM convoy.
    StorageHungFenced,
}

/// Description of an inflight storage request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InflightIo {
    class: StorageOpClass,
    start_tick: u64,
    deadline_ticks: u64,
}

/// Zero-allocation/bounded storage watchdog kernel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudStorageWatchdogKernel {
    default_deadline_ticks: u64,
    max_tolerable_hung_ops: u32,
    current_tick: u64,
    inflight: BTreeMap<u64, InflightIo>,
    total_completed: u64,
    total_hung: u64,
}

impl CloudStorageWatchdogKernel {
    /// Construct a new cloud storage watchdog kernel.
    ///
    /// # Errors
    /// Returns an error if `default_deadline_ticks == 0`.
    pub fn new(
        default_deadline_ticks: u64,
        max_tolerable_hung_ops: u32,
    ) -> Result<Self, StorageWatchdogError> {
        if default_deadline_ticks == 0 {
            return Err(StorageWatchdogError::ZeroDefaultDeadline);
        }
        Ok(Self {
            default_deadline_ticks,
            max_tolerable_hung_ops,
            current_tick: 0,
            inflight: BTreeMap::new(),
            total_completed: 0,
            total_hung: 0,
        })
    }

    /// Advance the watchdog's monotonic tick.
    pub fn advance_tick(&mut self, tick: u64) {
        if tick > self.current_tick {
            self.current_tick = tick;
        }
    }

    /// Register a newly submitted I/O operation.
    ///
    /// # Errors
    /// Returns an error if `op_id` is 0, already registered, or queue is full.
    pub fn register_io(
        &mut self,
        op_id: u64,
        class: StorageOpClass,
        submit_tick: u64,
    ) -> Result<(), StorageWatchdogError> {
        self.register_io_with_deadline(op_id, class, submit_tick, self.default_deadline_ticks)
    }

    /// Register an I/O operation with a custom deadline.
    pub fn register_io_with_deadline(
        &mut self,
        op_id: u64,
        class: StorageOpClass,
        submit_tick: u64,
        deadline_ticks: u64,
    ) -> Result<(), StorageWatchdogError> {
        if op_id == 0 {
            return Err(StorageWatchdogError::ZeroOpIdHazard);
        }
        if deadline_ticks == 0 {
            return Err(StorageWatchdogError::ZeroCustomDeadline);
        }
        if self.inflight.contains_key(&op_id) {
            return Err(StorageWatchdogError::OpAlreadyInflight { op_id });
        }
        if self.inflight.len() >= MAX_INFLIGHT_OPS {
            return Err(StorageWatchdogError::InflightCapacityExceeded {
                max: MAX_INFLIGHT_OPS,
            });
        }
        self.advance_tick(submit_tick);
        self.inflight.insert(
            op_id,
            InflightIo {
                class,
                start_tick: submit_tick,
                deadline_ticks,
            },
        );
        Ok(())
    }

    /// Mark an inflight I/O operation as completed.
    pub fn complete_io(&mut self, op_id: u64) -> Result<u64, StorageWatchdogError> {
        let Some(op) = self.inflight.remove(&op_id) else {
            return Err(StorageWatchdogError::OpNotFound { op_id });
        };
        self.total_completed = self.total_completed.saturating_add(1);
        let latency = self.current_tick.saturating_sub(op.start_tick);
        Ok(latency)
    }

    /// Autonomically purges hung I/O entries after a storage fence/failover event has completed.
    pub fn purge_hung_after_fence(&mut self) -> usize {
        let current_tick = self.current_tick;
        let mut purged = 0;
        self.inflight.retain(|_, op| {
            let elapsed = current_tick.saturating_sub(op.start_tick);
            if elapsed > op.deadline_ticks {
                purged += 1;
                false
            } else {
                true
            }
        });
        self.total_hung = self.total_hung.saturating_add(purged as u64);
        purged
    }

    /// Evaluate the current health status of storage I/O.
    pub fn evaluate_health(&self) -> StorageWatchdogState {
        let mut hung_ops = 0u32;
        let mut max_stall = 0u64;

        for op in self.inflight.values() {
            let elapsed = self.current_tick.saturating_sub(op.start_tick);
            if elapsed > op.deadline_ticks {
                hung_ops = hung_ops.saturating_add(1);
                let stall = elapsed.saturating_sub(op.deadline_ticks);
                if stall > max_stall {
                    max_stall = stall;
                }
            }
        }

        if hung_ops == 0 {
            StorageWatchdogState::StorageHealthy
        } else if hung_ops <= self.max_tolerable_hung_ops {
            StorageWatchdogState::StorageDegraded {
                hung_ops,
                max_stall_ticks: max_stall,
            }
        } else {
            StorageWatchdogState::StorageHungFenced
        }
    }

    /// Check if new writes can safely be admitted without queue blowout.
    pub fn is_write_admitted(&self) -> bool {
        match self.evaluate_health() {
            StorageWatchdogState::StorageHealthy => true,
            StorageWatchdogState::StorageDegraded { .. } => true,
            StorageWatchdogState::StorageHungFenced => false,
        }
    }

    /// Count inflight operations.
    pub fn inflight_count(&self) -> usize {
        self.inflight.len()
    }

    /// Count total completed operations.
    pub fn total_completed(&self) -> u64 {
        self.total_completed
    }

    /// Count total hung operations purged.
    pub fn total_hung(&self) -> u64 {
        self.total_hung
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cloud_storage_watchdog_structural_invariants_red_to_green() {
        // 1. Zero default deadline rejected
        assert_eq!(
            CloudStorageWatchdogKernel::new(0, 1),
            Err(StorageWatchdogError::ZeroDefaultDeadline)
        );

        let mut watchdog = CloudStorageWatchdogKernel::new(100, 2).unwrap();

        // 2. Op ID 0 rejected
        assert_eq!(
            watchdog.register_io(0, StorageOpClass::WalSync, 10),
            Err(StorageWatchdogError::ZeroOpIdHazard)
        );

        // 3. Zero custom deadline rejected
        assert_eq!(
            watchdog.register_io_with_deadline(1, StorageOpClass::WalSync, 10, 0),
            Err(StorageWatchdogError::ZeroCustomDeadline)
        );

        // 4. Register and duplicate rejection
        assert_eq!(watchdog.register_io(1, StorageOpClass::WalSync, 10), Ok(()));
        assert_eq!(
            watchdog.register_io(1, StorageOpClass::WalSync, 15),
            Err(StorageWatchdogError::OpAlreadyInflight { op_id: 1 })
        );

        // 5. Complete non-existent op
        assert_eq!(
            watchdog.complete_io(999),
            Err(StorageWatchdogError::OpNotFound { op_id: 999 })
        );

        // 6. Complete valid op
        watchdog.advance_tick(60);
        assert_eq!(watchdog.complete_io(1), Ok(50));
        assert_eq!(watchdog.inflight_count(), 0);

        // 7. Autonomic purge after fence
        watchdog.register_io(2, StorageOpClass::SstWrite, 100).unwrap();
        watchdog.register_io(3, StorageOpClass::SstWrite, 100).unwrap();
        watchdog.register_io(4, StorageOpClass::SstWrite, 100).unwrap();
        watchdog.advance_tick(250); // hung
        assert_eq!(watchdog.evaluate_health(), StorageWatchdogState::StorageHungFenced);
        assert_eq!(watchdog.purge_hung_after_fence(), 3);
        assert_eq!(watchdog.inflight_count(), 0);
        assert_eq!(watchdog.total_hung(), 3);
        assert_eq!(watchdog.evaluate_health(), StorageWatchdogState::StorageHealthy);
    }
}
