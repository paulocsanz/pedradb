//! Decompression Scratch Arena Strict Isolation and Purge Kernel (RFC-0286 Fronteira 4).
//!
//! Provides mutually exclusive, zero-cross-talk scratch buffers for block decompressors,
//! guaranteeing that uninitialized or stale plaintext bytes never leak across concurrent reads.
//!
//! Guarantees:
//! 1. Disjoint lease partitioning: `Slot(T_1) != Slot(T_2)` under concurrent execution.
//! 2. Zero residual entropy: Released scratch buffers are zeroed before returning to the pool.
//! 3. Bound checking: Decoded payloads exceeding slot capacity are rejected fail-closed.

#![forbid(unsafe_code)]

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

/// Default capacity for decompression scratch slot (64 KiB).
pub const DECOMPRESSION_SLOT_SIZE: usize = 65536;

/// Errors resulting from scratch buffer isolation violations or memory limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScratchIsolationError {
    /// Mutex guarding slot was poisoned.
    PoisonedMutex,
    /// Requested creation of a scratch pool with zero slots.
    ZeroSlotsRequested,
    /// Output buffer of caller is smaller than the decompressed data.
    DestinationSliceTooSmall {
        /// Bytes required to hold decompressed payload.
        required: usize,
        /// Bytes provided in caller buffer.
        provided: usize,
    },
    /// Decompressed data exceeded fixed scratch buffer capacity.
    ExceededCapacity {
        /// Bytes produced by decompressor.
        attempted: usize,
        /// Maximum capacity of the slot.
        capacity: usize,
    },
    /// Decompression operation aborted or returned an error.
    DecompressionFailed,
}

impl fmt::Display for ScratchIsolationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PoisonedMutex => write!(f, "Scratch slot mutex was poisoned"),
            Self::ZeroSlotsRequested => write!(f, "Cannot create decompression pool with zero slots"),
            Self::DestinationSliceTooSmall { required, provided } => {
                write!(f, "Destination slice too small: provided {} bytes, required {}", provided, required)
            }
            Self::ExceededCapacity { attempted, capacity } => {
                write!(f, "Decompressed output {} bytes exceeded capacity {}", attempted, capacity)
            }
            Self::DecompressionFailed => write!(f, "Decompression operation aborted or failed"),
        }
    }
}

impl std::error::Error for ScratchIsolationError {}

/// Storage for a single decompression scratch buffer.
pub struct ScratchSlotStorage {
    /// In-memory byte buffer.
    pub buffer: [u8; DECOMPRESSION_SLOT_SIZE],
    /// Current written length.
    pub written_len: usize,
}

impl ScratchSlotStorage {
    const fn new() -> Self {
        Self {
            buffer: [0u8; DECOMPRESSION_SLOT_SIZE],
            written_len: 0,
        }
    }
}

/// Managed scratch buffer slot.
pub struct DecompressionScratchSlot {
    in_use: AtomicBool,
    generation: AtomicU64,
    storage: Mutex<ScratchSlotStorage>,
}

impl DecompressionScratchSlot {
    fn new() -> Self {
        Self {
            in_use: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            storage: Mutex::new(ScratchSlotStorage::new()),
        }
    }
}

/// Pool of reusable decompression scratch buffers.
pub struct DecompressionScratchPool {
    slots: Vec<DecompressionScratchSlot>,
    active_count: AtomicUsize,
}

/// RAII lease granting exclusive access to a single decompression scratch buffer.
pub struct DecompressionScratchLease<'a> {
    pool: &'a DecompressionScratchPool,
    slot_idx: usize,
    generation: u64,
}

impl<'a> DecompressionScratchLease<'a> {
    /// Slot index in the pool.
    pub fn slot_index(&self) -> usize {
        self.slot_idx
    }

    /// Slot generation epoch.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Decompresses data into the leased scratch slot using a closure.
    pub fn execute_decompression<F>(&mut self, decompress_fn: F) -> Result<usize, ScratchIsolationError>
    where
        F: FnOnce(&mut [u8]) -> Result<usize, &'static str>,
    {
        let mut guard = self.pool.slots[self.slot_idx]
            .storage
            .lock()
            .map_err(|_| ScratchIsolationError::PoisonedMutex)?;

        // Reset previous written length before starting
        guard.written_len = 0;

        let bytes_written = match decompress_fn(&mut guard.buffer) {
            Ok(n) => n,
            Err(_e) => {
                // Aborted decompression: purge buffer immediately so partial plaintext never lingers
                guard.buffer.fill(0);
                std::hint::black_box(&mut guard.buffer);
                return Err(ScratchIsolationError::DecompressionFailed);
            }
        };

        if bytes_written > DECOMPRESSION_SLOT_SIZE {
            guard.buffer.fill(0);
            std::hint::black_box(&mut guard.buffer);
            return Err(ScratchIsolationError::ExceededCapacity {
                attempted: bytes_written,
                capacity: DECOMPRESSION_SLOT_SIZE,
            });
        }
        guard.written_len = bytes_written;
        Ok(bytes_written)
    }

    /// Provides zero-copy read access to the decompressed payload slice.
    pub fn with_decompressed_slice<R, F>(&self, f: F) -> Result<R, ScratchIsolationError>
    where
        F: FnOnce(&[u8]) -> R,
    {
        let guard = self.pool.slots[self.slot_idx]
            .storage
            .lock()
            .map_err(|_| ScratchIsolationError::PoisonedMutex)?;

        Ok(f(&guard.buffer[..guard.written_len]))
    }

    /// Copies the decompressed data out to a caller-provided destination slice.
    pub fn copy_out(&self, dest: &mut [u8]) -> Result<usize, ScratchIsolationError> {
        let guard = self.pool.slots[self.slot_idx]
            .storage
            .lock()
            .map_err(|_| ScratchIsolationError::PoisonedMutex)?;

        if dest.len() < guard.written_len {
            return Err(ScratchIsolationError::DestinationSliceTooSmall {
                required: guard.written_len,
                provided: dest.len(),
            });
        }
        dest[..guard.written_len].copy_from_slice(&guard.buffer[..guard.written_len]);
        Ok(guard.written_len)
    }
}

impl<'a> Drop for DecompressionScratchLease<'a> {
    fn drop(&mut self) {
        let slot = &self.pool.slots[self.slot_idx];
        // Anti-forensic purge: complete zeroization even if mutex was poisoned during panic
        let guard_opt = match slot.storage.lock() {
            Ok(g) => Some(g),
            Err(poisoned) => Some(poisoned.into_inner()),
        };
        if let Some(mut guard) = guard_opt {
            guard.buffer.fill(0);
            std::hint::black_box(&mut guard.buffer);
            guard.written_len = 0;
        }
        slot.generation.fetch_add(1, Ordering::Release);
        slot.in_use.store(false, Ordering::Release);
        self.pool.active_count.fetch_sub(1, Ordering::Relaxed);
    }
}

impl DecompressionScratchPool {
    /// Creates a pool with `num_slots` pre-allocated scratch buffers.
    pub fn with_slots(num_slots: usize) -> Self {
        Self::try_with_slots(num_slots).unwrap_or_else(|_| Self {
            slots: Vec::new(),
            active_count: AtomicUsize::new(0),
        })
    }

    /// Safely creates a pool with `num_slots` pre-allocated scratch buffers,
    /// rejecting zero slots.
    pub fn try_with_slots(num_slots: usize) -> Result<Self, ScratchIsolationError> {
        if num_slots == 0 {
            return Err(ScratchIsolationError::ZeroSlotsRequested);
        }
        let mut slots = Vec::with_capacity(num_slots);
        for _ in 0..num_slots {
            slots.push(DecompressionScratchSlot::new());
        }
        Ok(Self {
            slots,
            active_count: AtomicUsize::new(0),
        })
    }

    /// Acquires an exclusive lease on a scratch slot without cross-talk.
    pub fn acquire_lease(&self) -> Option<DecompressionScratchLease<'_>> {
        for (idx, slot) in self.slots.iter().enumerate() {
            if !slot.in_use.swap(true, Ordering::AcqRel) {
                let gen = slot.generation.load(Ordering::Acquire);
                self.active_count.fetch_add(1, Ordering::Relaxed);
                return Some(DecompressionScratchLease {
                    pool: self,
                    slot_idx: idx,
                    generation: gen,
                });
            }
        }
        None // Pool exhausted
    }

    /// Number of active leases currently checked out.
    pub fn active_leases(&self) -> usize {
        self.active_count.load(Ordering::Relaxed)
    }

    /// Total capacity of the pool.
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decompression_scratch_hardening_red_to_green() {
        // 1. Rejeita 0 slots
        assert_eq!(
            DecompressionScratchPool::try_with_slots(0).err(),
            Some(ScratchIsolationError::ZeroSlotsRequested)
        );

        // 2. Destino insuficiente
        let pool = DecompressionScratchPool::try_with_slots(1).unwrap();
        let mut lease = pool.acquire_lease().unwrap();
        lease
            .execute_decompression(|buf| {
                buf[..10].copy_from_slice(b"1234567890");
                Ok::<usize, &'static str>(10)
            })
            .unwrap();

        let mut small_dest = [0u8; 5];
        assert_eq!(
            lease.copy_out(&mut small_dest),
            Err(ScratchIsolationError::DestinationSliceTooSmall {
                required: 10,
                provided: 5
            })
        );
        drop(lease);

        // 3. Purge completo pós-drop
        let lease2 = pool.acquire_lease().unwrap();
        lease2
            .with_decompressed_slice(|slice| {
                assert_eq!(slice.len(), 0);
            })
            .unwrap();
    }
}

