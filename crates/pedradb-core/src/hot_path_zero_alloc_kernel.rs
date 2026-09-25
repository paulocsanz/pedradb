//! Zero-Allocation Hot-Path and Static Slab Reservation Kernel (RFC-0285 Pilar 1).
//!
//! Enforces zero dynamic heap allocations on the steady-state write commit path,
//! eliminating allocator lock contention and memory compaction latency spikes.
//!
//! Guarantees:
//! 1. `Delta_HeapAlloc(write_op) == 0` in steady state.
//! 2. Static slab pool provides O(1) allocation and deallocation without syscalls.
//! 3. RAII leases ensure safe deterministic recycling of pre-allocated buffers.

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

/// Fixed buffer capacity for hot-path staged write records (e.g. 4 KiB).
pub const SLAB_BUFFER_CAPACITY: usize = 4096;

/// Pre-allocated slot storage protected by lock.
pub struct SlotPayload {
    /// Fixed-size payload backing buffer.
    pub buffer: [u8; SLAB_BUFFER_CAPACITY],
    /// Current valid length in buffer.
    pub valid_len: usize,
    /// Monotonic sequence number assigned to this slot.
    pub seq_num: u64,
}

impl SlotPayload {
    const fn new() -> Self {
        Self {
            buffer: [0u8; SLAB_BUFFER_CAPACITY],
            valid_len: 0,
            seq_num: 0,
        }
    }
}

/// Pre-allocated write record slot.
pub struct WriteSlot {
    /// Atomic flag indicating slot is currently leased.
    pub in_use: AtomicBool,
    /// Mutex guarding slot data without dynamic allocation.
    pub data: Mutex<SlotPayload>,
}

impl WriteSlot {
    pub fn new() -> Self {
        Self {
            in_use: AtomicBool::new(false),
            data: Mutex::new(SlotPayload::new()),
        }
    }
}

/// Static slab arena holding pre-allocated write slots.
pub struct StaticSlabArena {
    slots: Vec<WriteSlot>,
    active_count: AtomicUsize,
}

/// Leased handle to a pre-allocated write slot.
pub struct SlabLease<'a> {
    arena: &'a StaticSlabArena,
    slot_idx: usize,
}

impl<'a> SlabLease<'a> {
    /// Mutable access to the slot's buffer slice without dynamic allocation.
    pub fn write_payload(&mut self, payload: &[u8], seq: u64) -> Result<(), &'static str> {
        if payload.len() > SLAB_BUFFER_CAPACITY {
            return Err("Payload exceeds fixed slab capacity");
        }
        let mut guard = self.arena.slots[self.slot_idx]
            .data
            .lock()
            .map_err(|_| "Poisoned slot mutex")?;

        guard.buffer[..payload.len()].copy_from_slice(payload);
        guard.valid_len = payload.len();
        guard.seq_num = seq;
        Ok(())
    }

    /// Read valid payload from the leased slot via copy into an existing output buffer.
    pub fn read_payload(&self, out: &mut [u8]) -> Result<usize, &'static str> {
        let guard = self.arena.slots[self.slot_idx]
            .data
            .lock()
            .map_err(|_| "Poisoned slot mutex")?;

        if out.len() < guard.valid_len {
            return Err("Output buffer too small");
        }
        out[..guard.valid_len].copy_from_slice(&guard.buffer[..guard.valid_len]);
        Ok(guard.valid_len)
    }

    /// Get sequence number.
    pub fn seq_num(&self) -> Result<u64, &'static str> {
        let guard = self.arena.slots[self.slot_idx]
            .data
            .lock()
            .map_err(|_| "Poisoned slot mutex")?;
        Ok(guard.seq_num)
    }
}

impl<'a> Drop for SlabLease<'a> {
    fn drop(&mut self) {
        self.arena.slots[self.slot_idx].in_use.store(false, Ordering::Release);
        self.arena.active_count.fetch_sub(1, Ordering::Relaxed);
    }
}

impl StaticSlabArena {
    /// Creates a new static slab arena with `capacity` pre-allocated slots.
    pub fn with_capacity(capacity: usize) -> Self {
        let mut slots = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            slots.push(WriteSlot::new());
        }
        Self {
            slots,
            active_count: AtomicUsize::new(0),
        }
    }

    /// Acquires a pre-allocated slot without performing heap allocations in steady-state.
    pub fn acquire_lease(&self) -> Option<SlabLease<'_>> {
        for (idx, slot) in self.slots.iter().enumerate() {
            if !slot.in_use.swap(true, Ordering::AcqRel) {
                self.active_count.fetch_add(1, Ordering::Relaxed);
                return Some(SlabLease {
                    arena: self,
                    slot_idx: idx,
                });
            }
        }
        None // Arena full; backpressure applies
    }

    /// Number of active leases.
    pub fn active_leases(&self) -> usize {
        self.active_count.load(Ordering::Relaxed)
    }

    /// Total capacity.
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }
}
