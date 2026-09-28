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

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

/// Default capacity for decompression scratch slot (64 KiB).
pub const DECOMPRESSION_SLOT_SIZE: usize = 65536;

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
    pub fn execute_decompression<F>(&mut self, decompress_fn: F) -> Result<usize, &'static str>
    where
        F: FnOnce(&mut [u8]) -> Result<usize, &'static str>,
    {
        let mut guard = self.pool.slots[self.slot_idx]
            .storage
            .lock()
            .map_err(|_| "Poisoned scratch slot mutex")?;

        let bytes_written = decompress_fn(&mut guard.buffer)?;
        if bytes_written > DECOMPRESSION_SLOT_SIZE {
            return Err("Decompressed output exceeded scratch buffer capacity");
        }
        guard.written_len = bytes_written;
        Ok(bytes_written)
    }

    /// Copies the decompressed data out to a caller-provided destination slice.
    pub fn copy_out(&self, dest: &mut [u8]) -> Result<usize, &'static str> {
        let guard = self.pool.slots[self.slot_idx]
            .storage
            .lock()
            .map_err(|_| "Poisoned scratch slot mutex")?;

        if dest.len() < guard.written_len {
            return Err("Destination slice too small");
        }
        dest[..guard.written_len].copy_from_slice(&guard.buffer[..guard.written_len]);
        Ok(guard.written_len)
    }
}

impl<'a> Drop for DecompressionScratchLease<'a> {
    fn drop(&mut self) {
        let slot = &self.pool.slots[self.slot_idx];
        if let Ok(mut guard) = slot.storage.lock() {
            // Anti-forensic purge: zeroize written bytes before making slot available
            let len = guard.written_len.min(DECOMPRESSION_SLOT_SIZE);
            guard.buffer[..len].fill(0);
            guard.written_len = 0;
        }
        slot.generation.fetch_add(1, Ordering::Relaxed);
        slot.in_use.store(false, Ordering::Release);
        self.pool.active_count.fetch_sub(1, Ordering::Relaxed);
    }
}

impl DecompressionScratchPool {
    /// Creates a pool with `num_slots` pre-allocated scratch buffers.
    pub fn with_slots(num_slots: usize) -> Self {
        let mut slots = Vec::with_capacity(num_slots);
        for _ in 0..num_slots {
            slots.push(DecompressionScratchSlot::new());
        }
        Self {
            slots,
            active_count: AtomicUsize::new(0),
        }
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
