//! RFC-0289: VLog Hole-Punching Alignment and Block Geometry Invariant Kernel.
//!
//! Enforces physical filesystem block alignment (e.g. 4096B) when reclaiming VLog space
//! via sparse hole-punching, mathematically guaranteeing that adjacent live payloads are never zeroed.

/// Default POSIX filesystem physical block size in bytes (4 KiB).
pub const DEFAULT_FS_BLOCK_SIZE: u64 = 4096;

/// A contiguous byte span of dead (obsolete) values in the VLog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeadValueSpan {
    /// Starting byte offset in VLog file (inclusive).
    pub start_offset: u64,
    /// Ending byte offset in VLog file (exclusive).
    pub end_offset: u64,
}

impl DeadValueSpan {
    /// Creates a new dead value span.
    #[must_use]
    pub fn new(start_offset: u64, end_offset: u64) -> Self {
        assert!(start_offset <= end_offset, "start must be <= end");
        Self { start_offset, end_offset }
    }

    /// Returns the raw length of dead bytes.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.end_offset - self.start_offset
    }

    /// Checks if the span is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.start_offset == self.end_offset
    }
}

/// A verified, block-aligned hole-punch command safe for execution via POSIX fallocate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafeHolePunch {
    /// Aligned starting byte offset (multiple of fs_block_size).
    pub aligned_start: u64,
    /// Aligned ending byte offset (multiple of fs_block_size).
    pub aligned_end: u64,
    /// Bytes that will be reclaimed from disk.
    pub punched_bytes: u64,
}

/// Planner computing safe hole-punch regions.
pub struct VLogHolePunchPlanner {
    fs_block_size: u64,
}

impl VLogHolePunchPlanner {
    /// Creates a new planner with specific filesystem block size.
    #[must_use]
    pub fn new(fs_block_size: u64) -> Self {
        assert!(fs_block_size > 0 && fs_block_size.is_power_of_two(), "block size must be a power of two");
        Self { fs_block_size }
    }

    /// Plans the safe, block-aligned hole punch for a single dead value span.
    ///
    /// Mathematical Invariant:
    /// `aligned_start >= span.start_offset` and `aligned_end <= span.end_offset`.
    /// Zero live bytes can ever fall into `[aligned_start, aligned_end)`.
    #[must_use]
    pub fn plan_span_punch(&self, span: &DeadValueSpan) -> Option<SafeHolePunch> {
        let b = self.fs_block_size;

        // Ceiling of start: round UP to next block boundary to prevent zeroing preceding live data
        let aligned_start = (span.start_offset.saturating_add(b - 1) / b) * b;

        // Floor of end: round DOWN to previous block boundary to prevent zeroing succeeding live data
        let aligned_end = (span.end_offset / b) * b;

        if aligned_start < aligned_end {
            Some(SafeHolePunch {
                aligned_start,
                aligned_end,
                punched_bytes: aligned_end - aligned_start,
            })
        } else {
            // Span is smaller than a single aligned block; punching would risk neighbor corruption
            None
        }
    }

    /// Plans safe hole punches across multiple dead value spans, coalescing contiguous punches.
    #[must_use]
    pub fn plan_multi_punch(&self, spans: &[DeadValueSpan]) -> Vec<SafeHolePunch> {
        let mut punches = Vec::new();
        for span in spans {
            if let Some(punch) = self.plan_span_punch(span) {
                // Try to coalesce with previous punch if contiguous
                if let Some(last) = punches.last_mut() {
                    let last_ref: &mut SafeHolePunch = last;
                    if last_ref.aligned_end == punch.aligned_start {
                        last_ref.aligned_end = punch.aligned_end;
                        last_ref.punched_bytes += punch.punched_bytes;
                        continue;
                    }
                }
                punches.push(punch);
            }
        }
        punches
    }

    /// Verifies that a given offset of a live value is NEVER inside any planned punch.
    #[must_use]
    pub fn verify_live_offset_isolated(&self, live_offset: u64, punches: &[SafeHolePunch]) -> bool {
        for punch in punches {
            if live_offset >= punch.aligned_start && live_offset < punch.aligned_end {
                return false; // Live offset would be destroyed by hole punch!
            }
        }
        true
    }
}
