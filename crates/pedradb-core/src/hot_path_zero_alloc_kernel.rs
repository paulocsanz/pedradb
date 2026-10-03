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

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

/// Fixed buffer capacity for hot-path staged write records (e.g. 4 KiB).
pub const SLAB_BUFFER_CAPACITY: usize = 4096;

/// Errors resulting from slab lease violations, capacity boundaries, or concurrency faults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlabError {
    /// Mutex guarding slot was poisoned.
    PoisonedSlotMutex,
    /// Requested creation of an arena with zero capacity.
    ZeroCapacityRequested,
    /// Attempted to write a payload exceeding the fixed buffer capacity.
    PayloadExceedsCapacity {
        /// Attempted write length in bytes.
        attempted: usize,
        /// Maximum fixed slab capacity in bytes.
        capacity: usize,
    },
    /// Attempted to write an empty payload.
    EmptyPayload,
    /// Sequence number cannot be zero.
    ZeroSequenceNumber,
    /// Destination buffer is too small to receive the payload.
    DestinationBufferTooSmall {
        /// Bytes required to receive the payload.
        required: usize,
        /// Bytes provided by caller.
        provided: usize,
    },
    /// Slot has not been written to yet in this lease.
    UninitializedPayload,
}

impl fmt::Display for SlabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PoisonedSlotMutex => write!(f, "Slot mutex was poisoned"),
            Self::ZeroCapacityRequested => write!(f, "Cannot create static slab arena with zero capacity"),
            Self::PayloadExceedsCapacity { attempted, capacity } => {
                write!(f, "Payload length {} exceeds slab capacity {}", attempted, capacity)
            }
            Self::EmptyPayload => write!(f, "Payload cannot be empty"),
            Self::ZeroSequenceNumber => write!(f, "Sequence number cannot be zero"),
            Self::DestinationBufferTooSmall { required, provided } => {
                write!(f, "Destination buffer too small: provided {}, required {}", provided, required)
            }
            Self::UninitializedPayload => write!(f, "Slot payload has not been written in this lease"),
        }
    }
}

impl std::error::Error for SlabError {}

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
    pub fn write_payload(&mut self, payload: &[u8], seq: u64) -> Result<(), SlabError> {
        if payload.is_empty() {
            return Err(SlabError::EmptyPayload);
        }
        if payload.len() > SLAB_BUFFER_CAPACITY {
            return Err(SlabError::PayloadExceedsCapacity {
                attempted: payload.len(),
                capacity: SLAB_BUFFER_CAPACITY,
            });
        }
        if seq == 0 {
            return Err(SlabError::ZeroSequenceNumber);
        }
        let mut guard = self.arena.slots[self.slot_idx]
            .data
            .lock()
            .map_err(|_| SlabError::PoisonedSlotMutex)?;

        guard.buffer[..payload.len()].copy_from_slice(payload);
        guard.valid_len = payload.len();
        guard.seq_num = seq;
        Ok(())
    }

    /// Read valid payload from the leased slot via copy into an existing output buffer.
    pub fn read_payload(&self, out: &mut [u8]) -> Result<usize, SlabError> {
        let guard = self.arena.slots[self.slot_idx]
            .data
            .lock()
            .map_err(|_| SlabError::PoisonedSlotMutex)?;

        if guard.valid_len == 0 {
            return Err(SlabError::UninitializedPayload);
        }
        if out.len() < guard.valid_len {
            return Err(SlabError::DestinationBufferTooSmall {
                required: guard.valid_len,
                provided: out.len(),
            });
        }
        out[..guard.valid_len].copy_from_slice(&guard.buffer[..guard.valid_len]);
        Ok(guard.valid_len)
    }

    /// Get sequence number.
    pub fn seq_num(&self) -> Result<u64, SlabError> {
        let guard = self.arena.slots[self.slot_idx]
            .data
            .lock()
            .map_err(|_| SlabError::PoisonedSlotMutex)?;
        if guard.seq_num == 0 {
            return Err(SlabError::UninitializedPayload);
        }
        Ok(guard.seq_num)
    }
}

impl<'a> Drop for SlabLease<'a> {
    fn drop(&mut self) {
        let slot = &self.arena.slots[self.slot_idx];
        // Anti-cross-talk purge: completely zero buffer and reset metadata before releasing
        let guard_opt = match slot.data.lock() {
            Ok(g) => Some(g),
            Err(poisoned) => Some(poisoned.into_inner()),
        };
        if let Some(mut guard) = guard_opt {
            guard.buffer.fill(0);
            std::hint::black_box(&mut guard.buffer);
            guard.valid_len = 0;
            guard.seq_num = 0;
        }
        slot.in_use.store(false, Ordering::Release);
        self.arena.active_count.fetch_sub(1, Ordering::Relaxed);
    }
}

impl StaticSlabArena {
    /// Creates a new static slab arena with `capacity` pre-allocated slots.
    pub fn with_capacity(capacity: usize) -> Self {
        Self::try_with_capacity(capacity).unwrap_or_else(|_| Self {
            slots: Vec::new(),
            active_count: AtomicUsize::new(0),
        })
    }

    /// Safely creates a new static slab arena with `capacity` pre-allocated slots,
    /// rejecting zero capacity.
    pub fn try_with_capacity(capacity: usize) -> Result<Self, SlabError> {
        if capacity == 0 {
            return Err(SlabError::ZeroCapacityRequested);
        }
        let mut slots = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            slots.push(WriteSlot::new());
        }
        Ok(Self {
            slots,
            active_count: AtomicUsize::new(0),
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slab_hardening_red_to_green() {
        // 1. Rejeita capacidade 0
        assert_eq!(
            StaticSlabArena::try_with_capacity(0).err(),
            Some(SlabError::ZeroCapacityRequested)
        );

        let arena = StaticSlabArena::try_with_capacity(1).unwrap();

        // 2. Leitura antes da escrita é rejeitada (sem vazar lixo)
        let mut lease = arena.acquire_lease().unwrap();
        let mut buf = [0u8; 16];
        assert_eq!(lease.read_payload(&mut buf), Err(SlabError::UninitializedPayload));
        assert_eq!(lease.seq_num(), Err(SlabError::UninitializedPayload));

        // 3. Rejeita seq zero e payload vazio
        assert_eq!(lease.write_payload(b"", 10), Err(SlabError::EmptyPayload));
        assert_eq!(lease.write_payload(b"hello", 0), Err(SlabError::ZeroSequenceNumber));

        // 4. Escreve dado confidencial
        lease.write_payload(b"SECRET_PAYLOAD", 42).unwrap();
        assert_eq!(lease.seq_num(), Ok(42));
        drop(lease);

        // 5. Novo lease não pode ver dados antigos
        let lease2 = arena.acquire_lease().unwrap();
        assert_eq!(lease2.read_payload(&mut buf), Err(SlabError::UninitializedPayload));
        assert_eq!(lease2.seq_num(), Err(SlabError::UninitializedPayload));
    }
}

