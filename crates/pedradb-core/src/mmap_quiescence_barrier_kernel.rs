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

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Maximum active readers per mmap region before backpressure/fail-closed kicks in.
pub const MAX_ACTIVE_READERS: u64 = 10_000_000;

/// Invariant violations and errors for mmap quiescence barriers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MmapQuiescenceViolation {
    /// SST file number must be non-zero (0 is reserved/nil).
    InvalidFileNumber,
    /// Physical unmap requested when region is already unmapped.
    DoubleUnmapHazard,
    /// Active readers still present while unmap was attempted (SIGBUS hazard).
    ActiveReadersSigbusHazard { active_readers: u64 },
    /// Physical unmap attempted before `request_unmap` was initiated.
    ProtocolNotInitiated,
    /// Concurrent reader count exceeded safety ceiling (overflow hazard).
    ReaderLimitExceeded { current: u64, max: u64 },
    /// Cannot acquire lease: region is pending unmap.
    RegionPendingUnmap,
    /// Cannot acquire lease: region has already been physically unmapped.
    RegionUnmapped,
}

impl fmt::Display for MmapQuiescenceViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFileNumber => {
                write!(f, "Invalid SST file number: 0 is reserved/nil")
            }
            Self::DoubleUnmapHazard => {
                write!(
                    f,
                    "Unsafe unmap: region already physically unmapped (double-unmap hazard)"
                )
            }
            Self::ActiveReadersSigbusHazard { active_readers } => {
                write!(
                    f,
                    "Unsafe unmap: {active_readers} active readers still hold memory pointers (SIGBUS hazard)"
                )
            }
            Self::ProtocolNotInitiated => {
                write!(f, "Unsafe unmap: unmap protocol not initiated")
            }
            Self::ReaderLimitExceeded { current, max } => {
                write!(
                    f,
                    "Active reader limit exceeded: {current} >= {max} (overflow hazard)"
                )
            }
            Self::RegionPendingUnmap => {
                write!(f, "Cannot acquire reader lease: region is pending unmap")
            }
            Self::RegionUnmapped => {
                write!(f, "Cannot acquire reader lease: region is already unmapped")
            }
        }
    }
}

impl std::error::Error for MmapQuiescenceViolation {}

/// Operational lifecycle state of a memory-mapped file region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionState {
    /// Region is active and accepting new reader leases.
    Active,
    /// Compaction requested unmap; new reader leases are rejected, waiting for active readers to drain.
    PendingUnmap,
    /// All readers drained; safe to physically execute `munmap` / close file descriptor.
    QuiescedSafeToUnmap,
    /// Region has been physically unmapped; all further leases and unmaps are strictly rejected.
    Unmapped,
}

/// Shared control block for a memory-mapped file region.
pub struct MmapRegionControl {
    /// File identifier.
    pub file_number: u64,
    /// Active reader counter.
    active_readers: AtomicU64,
    /// Flag indicating unmap has been requested.
    unmap_requested: AtomicBool,
    /// Flag indicating physical unmapping has been executed.
    is_unmapped: AtomicBool,
}

impl MmapRegionControl {
    /// Attempts to create a new control block for a file region, validating that `file_number > 0`.
    pub fn try_new(file_number: u64) -> Result<Self, MmapQuiescenceViolation> {
        if file_number == 0 {
            return Err(MmapQuiescenceViolation::InvalidFileNumber);
        }
        Ok(Self {
            file_number,
            active_readers: AtomicU64::new(0),
            unmap_requested: AtomicBool::new(false),
            is_unmapped: AtomicBool::new(false),
        })
    }

    /// Creates a new control block for a file region (backward-compatible).
    pub fn new(file_number: u64) -> Self {
        Self::try_new(file_number).unwrap_or_else(|_| Self {
            file_number,
            active_readers: AtomicU64::new(0),
            unmap_requested: AtomicBool::new(false),
            is_unmapped: AtomicBool::new(false),
        })
    }

    /// Current reader count.
    pub fn reader_count(&self) -> u64 {
        self.active_readers.load(Ordering::Acquire)
    }

    /// Returns true if unmap has been requested.
    pub fn is_unmap_requested(&self) -> bool {
        self.unmap_requested.load(Ordering::Acquire)
    }

    /// Returns true if physically unmapped.
    pub fn is_unmapped(&self) -> bool {
        self.is_unmapped.load(Ordering::Acquire)
    }

    /// Current lifecycle state.
    pub fn state(&self) -> RegionState {
        if self.is_unmapped.load(Ordering::Acquire) {
            RegionState::Unmapped
        } else if self.unmap_requested.load(Ordering::Acquire) {
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
        let _ = self.control.active_readers.fetch_update(
            Ordering::Release,
            Ordering::Relaxed,
            |readers| Some(readers.saturating_sub(1)),
        );
    }
}

/// Coordinator managing mmap lifecycles and safe quiescence barriers.
pub struct MmapQuiescenceCoordinator;

impl MmapQuiescenceCoordinator {
    /// Attempts to acquire a reader lease on an active region, returning detailed violation on failure.
    pub fn try_acquire_lease(
        control: &Arc<MmapRegionControl>,
    ) -> Result<MmapReaderLease, MmapQuiescenceViolation> {
        loop {
            if control.is_unmapped.load(Ordering::Acquire) {
                return Err(MmapQuiescenceViolation::RegionUnmapped);
            }
            if control.unmap_requested.load(Ordering::Acquire) {
                return Err(MmapQuiescenceViolation::RegionPendingUnmap);
            }
            let current = control.active_readers.load(Ordering::Acquire);
            if current >= MAX_ACTIVE_READERS {
                return Err(MmapQuiescenceViolation::ReaderLimitExceeded {
                    current,
                    max: MAX_ACTIVE_READERS,
                });
            }
            match control.active_readers.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    // Post-CAS recheck to prevent race with request_unmap or mark_unmapped
                    if control.unmap_requested.load(Ordering::Acquire)
                        || control.is_unmapped.load(Ordering::Acquire)
                    {
                        let _ = control.active_readers.fetch_update(
                            Ordering::Release,
                            Ordering::Relaxed,
                            |readers| Some(readers.saturating_sub(1)),
                        );
                        if control.is_unmapped.load(Ordering::Acquire) {
                            return Err(MmapQuiescenceViolation::RegionUnmapped);
                        } else {
                            return Err(MmapQuiescenceViolation::RegionPendingUnmap);
                        }
                    }
                    return Ok(MmapReaderLease {
                        control: Arc::clone(control),
                    });
                }
                Err(_) => {
                    std::hint::spin_loop();
                }
            }
        }
    }

    /// Attempts to acquire a reader lease on an active region.
    ///
    /// Fails with `None` if the region is already pending unmap or unmapped, or if capacity is reached.
    pub fn acquire_lease(control: &Arc<MmapRegionControl>) -> Option<MmapReaderLease> {
        Self::try_acquire_lease(control).ok()
    }

    /// Background compaction requests that this region be unmapped and reclaimed.
    pub fn request_unmap(control: &MmapRegionControl) -> RegionState {
        if control.is_unmapped.load(Ordering::Acquire) {
            return RegionState::Unmapped;
        }
        control.unmap_requested.store(true, Ordering::Release);
        control.state()
    }

    /// Verifies that physical unmapping is safe to proceed without risking SIGBUS.
    pub fn verify_unmap_safety(control: &MmapRegionControl) -> Result<(), MmapQuiescenceViolation> {
        if control.is_unmapped.load(Ordering::Acquire) {
            return Err(MmapQuiescenceViolation::DoubleUnmapHazard);
        }
        let active = control.active_readers.load(Ordering::Acquire);
        if active > 0 {
            return Err(MmapQuiescenceViolation::ActiveReadersSigbusHazard {
                active_readers: active,
            });
        }
        if !control.unmap_requested.load(Ordering::Acquire) {
            return Err(MmapQuiescenceViolation::ProtocolNotInitiated);
        }
        Ok(())
    }

    /// Atomically transitions the region to Unmapped state upon physical reclamation.
    pub fn mark_unmapped(control: &MmapRegionControl) -> Result<(), MmapQuiescenceViolation> {
        Self::verify_unmap_safety(control)?;
        control
            .is_unmapped
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| MmapQuiescenceViolation::DoubleUnmapHazard)?;
        Ok(())
    }
}
