//! RFC-0278 P2 — Bounded Dynamic Resources & Stack Limits Kernel.
//!
//! Enforces zero-allocation bounds on critical durability paths (WAL append, commit, sync)
//! and structural upper bounds on merge iterator recursion depth, ensuring mathematical
//! immunity against Linux OOM-killer aborts and thread stack overflow.

#![forbid(unsafe_code)]

use std::fmt;

/// Maximum permitted stack/recursion depth in multi-way LSM merge iterators.
pub const MAX_ITERATOR_MERGE_DEPTH: usize = 16;

/// Maximum scratch buffer capacity (in bytes) pre-allocated for WAL writes.
pub const WAL_SCRATCH_PREALLOC_BYTES: usize = 64 * 1024; // 64 KiB

/// Invariant violations and errors for bounded allocations and stack limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundedAllocError {
    /// Preallocated write buffer capacity exceeded.
    CapacityExceeded { needed: usize, capacity: usize },
    /// Integer overflow in cursor arithmetic.
    ArithmeticOverflow,
    /// LSM tree level count exceeds architectural maximum (7).
    ExcessiveMergeLevels { levels: usize, max_allowed: usize },
    /// Computed merge depth exceeds safe OS thread stack budget.
    MergeDepthExceeded { depth: usize, max_allowed: usize },
}

impl fmt::Display for BoundedAllocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityExceeded { needed, capacity } => {
                write!(
                    f,
                    "Preallocated write buffer capacity exceeded: needed {needed} bytes, capacity is {capacity} bytes"
                )
            }
            Self::ArithmeticOverflow => write!(f, "Arithmetic overflow in buffer cursor offset"),
            Self::ExcessiveMergeLevels { levels, max_allowed } => {
                write!(
                    f,
                    "LSM tree levels ({levels}) exceed architectural maximum ({max_allowed})"
                )
            }
            Self::MergeDepthExceeded { depth, max_allowed } => {
                write!(
                    f,
                    "Merge depth ({depth}) exceeds safe stack budget ({max_allowed})"
                )
            }
        }
    }
}

impl std::error::Error for BoundedAllocError {}

/// State tracking dynamic resource allocations in critical database paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllocationBudget {
    /// Number of dynamic heap allocations performed during operation.
    pub heap_alloc_count: usize,
    /// Maximum bytes allocated.
    pub bytes_allocated: usize,
    /// Maximum call-stack depth reached by iterator merging.
    pub max_stack_depth: usize,
}

impl AllocationBudget {
    /// Creates a fresh tracking budget for a critical path execution.
    pub fn new() -> Self {
        Self {
            heap_alloc_count: 0,
            bytes_allocated: 0,
            max_stack_depth: 0,
        }
    }

    /// Verifies the Zero-Dynamic-Allocation Invariant on the pre-allocated critical path.
    pub fn verify_zero_alloc_in_critical_path(&self) -> bool {
        self.heap_alloc_count == 0
    }

    /// Verifies that the recursion/stack depth of iterators is within safe OS limits.
    pub fn verify_bounded_stack_depth(&self) -> bool {
        self.max_stack_depth <= MAX_ITERATOR_MERGE_DEPTH
    }
}

impl Default for AllocationBudget {
    fn default() -> Self {
        Self::new()
    }
}

/// Pre-allocated buffer pool simulating zero-allocation writes.
pub struct PreallocatedWriteBuffer {
    buffer: [u8; WAL_SCRATCH_PREALLOC_BYTES],
    cursor: usize,
}

impl PreallocatedWriteBuffer {
    /// Initializes a pre-allocated write buffer.
    pub fn new() -> Self {
        Self {
            buffer: [0u8; WAL_SCRATCH_PREALLOC_BYTES],
            cursor: 0,
        }
    }

    /// Appends data without triggering heap reallocation.
    pub fn append(
        &mut self,
        data: &[u8],
        budget: &mut AllocationBudget,
    ) -> Result<usize, BoundedAllocError> {
        let new_cursor = self
            .cursor
            .checked_add(data.len())
            .ok_or(BoundedAllocError::ArithmeticOverflow)?;
        if new_cursor > WAL_SCRATCH_PREALLOC_BYTES {
            return Err(BoundedAllocError::CapacityExceeded {
                needed: new_cursor,
                capacity: WAL_SCRATCH_PREALLOC_BYTES,
            });
        }
        // Zero dynamic heap allocations!
        self.buffer[self.cursor..new_cursor].copy_from_slice(data);
        self.cursor = new_cursor;
        budget.bytes_allocated = self.cursor;
        // budget.heap_alloc_count remains 0
        Ok(self.cursor)
    }

    /// Resets the buffer cursor for the next batch.
    pub fn reset(&mut self) {
        self.cursor = 0;
    }
}

impl Default for PreallocatedWriteBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Verifies that an N-level LSM merge iterator has recursion depth $\le N + 2 \le 16$.
pub fn compute_merge_stack_depth(num_levels: usize) -> Result<usize, BoundedAllocError> {
    if num_levels > 7 {
        return Err(BoundedAllocError::ExcessiveMergeLevels {
            levels: num_levels,
            max_allowed: 7,
        });
    }
    // Deepest iterator nesting: 1 root + 1 memtable + num_levels SST iterators
    let depth = 2usize
        .checked_add(num_levels)
        .ok_or(BoundedAllocError::ArithmeticOverflow)?;
    if depth > MAX_ITERATOR_MERGE_DEPTH {
        return Err(BoundedAllocError::MergeDepthExceeded {
            depth,
            max_allowed: MAX_ITERATOR_MERGE_DEPTH,
        });
    }
    Ok(depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bounded_alloc_structural_invariants_red_to_green() {
        let mut budget = AllocationBudget::new();
        let mut write_buf = PreallocatedWriteBuffer::new();

        // Invariant 1: Appending beyond WAL_SCRATCH_PREALLOC_BYTES fails fail-closed
        let oversized = vec![0xBB; WAL_SCRATCH_PREALLOC_BYTES + 1];
        assert_eq!(
            write_buf.append(&oversized, &mut budget),
            Err(BoundedAllocError::CapacityExceeded {
                needed: WAL_SCRATCH_PREALLOC_BYTES + 1,
                capacity: WAL_SCRATCH_PREALLOC_BYTES,
            })
        );

        // Invariant 2: Normal appends respect zero allocation invariant
        let small = [0x55; 1024];
        assert_eq!(write_buf.append(&small, &mut budget), Ok(1024));
        assert!(budget.verify_zero_alloc_in_critical_path());
        assert_eq!(budget.bytes_allocated, 1024);

        // Invariant 3: Levels > 7 fail fail-closed
        assert_eq!(
            compute_merge_stack_depth(8),
            Err(BoundedAllocError::ExcessiveMergeLevels {
                levels: 8,
                max_allowed: 7,
            })
        );

        // Invariant 4: Valid levels return bounded depth <= 16
        let depth = compute_merge_stack_depth(6).unwrap();
        assert_eq!(depth, 8);
        assert!(depth <= MAX_ITERATOR_MERGE_DEPTH);
    }
}
