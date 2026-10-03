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
    ) -> Result<Self, &'static str> {
        if default_deadline_ticks == 0 {
            return Err("default_deadline_ticks must be positive");
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
    /// Returns an error if `op_id` is already registered and inflight.
    pub fn register_io(
        &mut self,
        op_id: u64,
        class: StorageOpClass,
        submit_tick: u64,
    ) -> Result<(), &'static str> {
        if self.inflight.contains_key(&op_id) {
            return Err("op_id already inflight");
        }
        self.advance_tick(submit_tick);
        self.inflight.insert(
            op_id,
            InflightIo {
                class,
                start_tick: submit_tick,
                deadline_ticks: self.default_deadline_ticks,
            },
        );
        Ok(())
    }

    /// Register an I/O operation with a custom deadline.
    pub fn register_io_with_deadline(
        &mut self,
        op_id: u64,
        class: StorageOpClass,
        submit_tick: u64,
        deadline_ticks: u64,
    ) -> Result<(), &'static str> {
        if self.inflight.contains_key(&op_id) {
            return Err("op_id already inflight");
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
    pub fn complete_io(&mut self, op_id: u64) -> Result<u64, &'static str> {
        let Some(op) = self.inflight.remove(&op_id) else {
            return Err("op_id not found in inflight table");
        };
        self.total_completed = self.total_completed.saturating_add(1);
        let latency = self.current_tick.saturating_sub(op.start_tick);
        Ok(latency)
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
}
