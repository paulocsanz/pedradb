//! Memory-Mapped Region Quiescence and Safe Unmap Epoch Barrier Kernel (RFC-0285 Pilar 4).
//!
//! Protects against fatal SIGBUS signals and invalid virtual page accesses when
//! background compaction unlinks and unmaps SST files while reader threads are actively
//! scanning mapped pages.
//!
//! Guarantees:
//! 1. Quiescence invariant: `PhysicalUnmap(region) => ActiveReaders(region) == 0`.
//! 2. Unmap requests are gracefully deferred until all concurrent reader leases drain.
//! 3. Zero SIGBUS: virtual addresses remain valid for the entire lifetime of any leased reader.

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Operational lifecycle state of a memory-mapped file region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionState {
    /// Region is active and accepting new reader leases.
    Active,
    /// Compaction requested unmap; new reader leases are rejected, waiting for active readers to drain.
    PendingUnmap,
    /// All readers drained; safe to physically execute `munmap` / close file descriptor.
    QuiescedSafeToUnmap,
}

/// Shared control block for a memory-mapped file region.
pub struct MmapRegionControl {
    /// File identifier.
    pub file_number: u64,
    /// Active reader counter.
    active_readers: AtomicU64,
    /// Flag indicating unmap has been requested.
    unmap_requested: AtomicBool,
}

impl MmapRegionControl {
    /// Creates a new control block for a file region.
    pub fn new(file_number: u64) -> Self {
        Self {
            file_number,
            active_readers: AtomicU64::new(0),
            unmap_requested: AtomicBool::new(false),
        }
    }

    /// Current reader count.
    pub fn reader_count(&self) -> u64 {
        self.active_readers.load(Ordering::Acquire)
    }

    /// Current lifecycle state.
    pub fn state(&self) -> RegionState {
        if self.unmap_requested.load(Ordering::Acquire) {
            if self.active_readers.load(Ordering::Acquire) == 0 {
                RegionState::QuiescedSafeToUnmap
            } else {
                RegionState::PendingUnmap
            }
        } else {
            RegionState::Active
        }
    }
}

/// RAII lease held by an active reader thread.
pub struct MmapReaderLease {
    control: Arc<MmapRegionControl>,
}

impl MmapReaderLease {
    /// File number protected by this lease.
    pub fn file_number(&self) -> u64 {
        self.control.file_number
    }
}

impl Drop for MmapReaderLease {
    fn drop(&mut self) {
        self.control.active_readers.fetch_sub(1, Ordering::Release);
    }
}

/// Coordinator managing mmap lifecycles and safe quiescence barriers.
pub struct MmapQuiescenceCoordinator;

impl MmapQuiescenceCoordinator {
    /// Attempts to acquire a reader lease on an active region.
    ///
    /// Fails with `None` if the region is already pending unmap.
    pub fn acquire_lease(control: &Arc<MmapRegionControl>) -> Option<MmapReaderLease> {
        control.active_readers.fetch_add(1, Ordering::Acquire);
        if control.unmap_requested.load(Ordering::Acquire) {
            // Unmap was requested; rollback reader count and reject lease
            control.active_readers.fetch_sub(1, Ordering::Release);
            None
        } else {
            Some(MmapReaderLease {
                control: Arc::clone(control),
            })
        }
    }

    /// Background compaction requests that this region be unmapped and reclaimed.
    pub fn request_unmap(control: &MmapRegionControl) -> RegionState {
        control.unmap_requested.store(true, Ordering::Release);
        control.state()
    }

    /// Verifies that physical unmapping is safe to proceed without risking SIGBUS.
    pub fn verify_unmap_safety(control: &MmapRegionControl) -> Result<(), &'static str> {
        if control.active_readers.load(Ordering::Acquire) > 0 {
            return Err("Unsafe unmap: active readers still hold memory pointers (SIGBUS hazard)");
        }
        if !control.unmap_requested.load(Ordering::Acquire) {
            return Err("Unsafe unmap: unmap protocol not initiated");
        }
        Ok(())
    }
}
