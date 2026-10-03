//! RFC-0289: VLog Hole-Punching Alignment and Block Geometry Invariant Kernel.
//!
//! Enforces physical filesystem block alignment (e.g. 4096B) when reclaiming VLog space
//! via sparse hole-punching, mathematically guaranteeing that adjacent live payloads are never zeroed.

/// Default POSIX filesystem physical block size in bytes (4 KiB).
pub const DEFAULT_FS_BLOCK_SIZE: u64 = 4096;

/// Errors that can occur during VLog hole alignment and planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VLogHoleAlignmentError {
    /// Filesystem block size must be greater than zero and a power of two.
    InvalidBlockSize(u64),
    /// Span start offset must be less than or equal to end offset.
    InvalidSpan {
        /// Start offset.
        start: u64,
        /// End offset.
        end: u64,
    },
    /// VLog file size cannot be zero.
    ZeroFileSize,
    /// Span offset exceeds current VLog file bounds.
    SpanExceedsFileSize {
        /// Exceeding offset.
        offset: u64,
        /// Configured file size.
        file_size: u64,
    },
    /// Proposed hole punch boundaries are not aligned to filesystem physical block size.
    UnalignedHolePunch {
        /// Offending unaligned offset.
        offset: u64,
        /// Block size.
        block_size: u64,
    },
    /// A planned punch collides with and destroys an active live payload.
    LiveRecordOverlap {
        /// Live record offset.
        live_offset: u64,
        /// Live record length.
        live_len: u64,
        /// Colliding punch.
        punch: SafeHolePunch,
    },
}

impl std::fmt::Display for VLogHoleAlignmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBlockSize(b) => write!(f, "Invalid block size {b}; must be > 0 and a power of two"),
            Self::InvalidSpan { start, end } => write!(f, "Invalid dead span: start={start} > end={end}"),
            Self::ZeroFileSize => write!(f, "VLog file size cannot be zero"),
            Self::SpanExceedsFileSize { offset, file_size } => {
                write!(f, "Span offset {offset} exceeds VLog file size {file_size}")
            }
            Self::UnalignedHolePunch { offset, block_size } => {
                write!(f, "Hole punch offset {offset} is not aligned to block size {block_size}")
            }
            Self::LiveRecordOverlap { live_offset, live_len, punch } => {
                write!(
                    f,
                    "Live record [{live_offset}, {}) collides with hole punch [{}, {})",
                    live_offset + live_len,
                    punch.aligned_start,
                    punch.aligned_end
                )
            }
        }
    }
}

impl std::error::Error for VLogHoleAlignmentError {}

/// A contiguous byte span of dead (obsolete) values in the VLog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeadValueSpan {
    /// Starting byte offset in VLog file (inclusive).
    pub start_offset: u64,
    /// Ending byte offset in VLog file (exclusive).
    pub end_offset: u64,
}

impl DeadValueSpan {
    /// Creates a new dead value span with strict boundary validation.
    pub fn try_new(start_offset: u64, end_offset: u64) -> Result<Self, VLogHoleAlignmentError> {
        if start_offset > end_offset {
            return Err(VLogHoleAlignmentError::InvalidSpan {
                start: start_offset,
                end: end_offset,
            });
        }
        Ok(Self { start_offset, end_offset })
    }

    /// Creates a new dead value span.
    ///
    /// # Panics
    /// Panics if `start_offset > end_offset`.
    #[must_use]
    pub fn new(start_offset: u64, end_offset: u64) -> Self {
        Self::try_new(start_offset, end_offset).expect("start must be <= end")
    }

    /// Returns the raw length of dead bytes, safe against inverted offsets.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.end_offset.saturating_sub(self.start_offset)
    }

    /// Checks if the span is empty or inverted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.start_offset >= self.end_offset
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

impl SafeHolePunch {
    /// Validates and constructs a safe hole punch, ensuring strict block alignment and non-empty range.
    pub fn try_new(
        aligned_start: u64,
        aligned_end: u64,
        block_size: u64,
    ) -> Result<Self, VLogHoleAlignmentError> {
        if block_size == 0 || !block_size.is_power_of_two() {
            return Err(VLogHoleAlignmentError::InvalidBlockSize(block_size));
        }
        if aligned_start >= aligned_end {
            return Err(VLogHoleAlignmentError::InvalidSpan {
                start: aligned_start,
                end: aligned_end,
            });
        }
        if aligned_start % block_size != 0 {
            return Err(VLogHoleAlignmentError::UnalignedHolePunch {
                offset: aligned_start,
                block_size,
            });
        }
        if aligned_end % block_size != 0 {
            return Err(VLogHoleAlignmentError::UnalignedHolePunch {
                offset: aligned_end,
                block_size,
            });
        }
        Ok(Self {
            aligned_start,
            aligned_end,
            punched_bytes: aligned_end - aligned_start,
        })
    }
}

/// Planner computing safe hole-punch regions.
pub struct VLogHolePunchPlanner {
    fs_block_size: u64,
}

impl VLogHolePunchPlanner {
    /// Attempts to create a new planner, validating that `fs_block_size` is a non-zero power of two.
    pub fn try_new(fs_block_size: u64) -> Result<Self, VLogHoleAlignmentError> {
        if fs_block_size == 0 || !fs_block_size.is_power_of_two() {
            return Err(VLogHoleAlignmentError::InvalidBlockSize(fs_block_size));
        }
        Ok(Self { fs_block_size })
    }

    /// Creates a new planner with specific filesystem block size.
    ///
    /// # Panics
    /// Panics if `fs_block_size == 0` or is not a power of two.
    #[must_use]
    pub fn new(fs_block_size: u64) -> Self {
        Self::try_new(fs_block_size).expect("block size must be a power of two")
    }

    /// Returns the configured filesystem block size.
    #[must_use]
    pub fn fs_block_size(&self) -> u64 {
        self.fs_block_size
    }

    /// Sums the total bytes reclaimed across a slice of planned punches.
    #[must_use]
    pub fn total_reclaimed_bytes(punches: &[SafeHolePunch]) -> u64 {
        punches.iter().map(|p| p.punched_bytes).sum()
    }

    /// Plans the safe, block-aligned hole punch for a single dead value span.
    ///
    /// Mathematical Invariant:
    /// `aligned_start >= span.start_offset` and `aligned_end <= span.end_offset`.
    /// Zero live bytes can ever fall into `[aligned_start, aligned_end)`.
    #[must_use]
    pub fn plan_span_punch(&self, span: &DeadValueSpan) -> Option<SafeHolePunch> {
        if span.start_offset >= span.end_offset {
            return None;
        }

        let b = self.fs_block_size;

        // Ceiling of start: round UP to next block boundary without overflow
        let aligned_start = if span.start_offset % b == 0 {
            Some(span.start_offset)
        } else {
            (span.start_offset / b)
                .checked_add(1)
                .and_then(|blocks| blocks.checked_mul(b))
        }?;

        // Floor of end: round DOWN to previous block boundary
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

    /// Plans safe hole punches across multiple dead value spans, coalescing contiguous spans in byte space before alignment.
    #[must_use]
    pub fn plan_multi_punch(&self, spans: &[DeadValueSpan]) -> Vec<SafeHolePunch> {
        if spans.is_empty() {
            return Vec::new();
        }

        // 1. Sort spans by start offset and end offset
        let mut sorted_spans: Vec<DeadValueSpan> = spans.to_vec();
        sorted_spans.sort_by_key(|s| (s.start_offset, s.end_offset));

        // 2. Coalesce adjacent and overlapping dead spans in raw byte space
        let mut merged_spans: Vec<DeadValueSpan> = Vec::new();
        for span in sorted_spans {
            if span.is_empty() {
                continue;
            }
            if let Some(last) = merged_spans.last_mut() {
                if span.start_offset <= last.end_offset {
                    last.end_offset = last.end_offset.max(span.end_offset);
                    continue;
                }
            }
            merged_spans.push(span);
        }

        // 3. Plan safe, block-aligned hole punches on canonical merged spans
        let mut punches = Vec::new();
        for span in merged_spans {
            if let Some(punch) = self.plan_span_punch(&span) {
                punches.push(punch);
            }
        }
        punches
    }

    /// Verifies that a live record spanning `[live_offset, live_offset + live_len)` never intersects any planned punch.
    #[must_use]
    pub fn verify_live_span_isolated(
        &self,
        live_offset: u64,
        live_len: u64,
        punches: &[SafeHolePunch],
    ) -> bool {
        if live_len == 0 {
            return true;
        }
        let live_end = live_offset.saturating_add(live_len);
        for punch in punches {
            // Half-open interval intersection: [live_offset, live_end) overlaps [punch.aligned_start, punch.aligned_end)
            if live_offset < punch.aligned_end && live_end > punch.aligned_start {
                return false; // Live record would be destroyed by hole punch!
            }
        }
        true
    }

    /// Verifies that a given point offset of a live value is NEVER inside any planned punch.
    #[must_use]
    pub fn verify_live_offset_isolated(&self, live_offset: u64, punches: &[SafeHolePunch]) -> bool {
        self.verify_live_span_isolated(live_offset, 1, punches)
    }

    /// Plans safe hole punches bounded by the current VLog file size.
    /// Rejects spans that extend past `file_size` or if `file_size == 0`.
    pub fn plan_bounded_punches(
        &self,
        spans: &[DeadValueSpan],
        file_size: u64,
    ) -> Result<Vec<SafeHolePunch>, VLogHoleAlignmentError> {
        if file_size == 0 {
            return Err(VLogHoleAlignmentError::ZeroFileSize);
        }
        for span in spans {
            if span.end_offset > file_size {
                return Err(VLogHoleAlignmentError::SpanExceedsFileSize {
                    offset: span.end_offset,
                    file_size,
                });
            }
        }
        Ok(self.plan_multi_punch(spans))
    }

    /// Verifies the entire punch plan against a set of live records.
    /// Returns `Ok(())` if zero live bytes will be destroyed, or `Err(LiveRecordOverlap)` on first collision.
    pub fn verify_punch_plan(
        &self,
        punches: &[SafeHolePunch],
        live_records: &[(u64, u64)],
    ) -> Result<(), VLogHoleAlignmentError> {
        for &(live_offset, live_len) in live_records {
            if live_len == 0 {
                continue;
            }
            let live_end = live_offset.saturating_add(live_len);
            for punch in punches {
                if live_offset < punch.aligned_end && live_end > punch.aligned_start {
                    return Err(VLogHoleAlignmentError::LiveRecordOverlap {
                        live_offset,
                        live_len,
                        punch: *punch,
                    });
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_hole_punch_unaligned_rejected() {
        let err = SafeHolePunch::try_new(100, 4096, 4096).unwrap_err();
        assert_eq!(
            err,
            VLogHoleAlignmentError::UnalignedHolePunch {
                offset: 100,
                block_size: 4096,
            }
        );
    }

    #[test]
    fn test_bounded_punches_zero_file_size_rejected() {
        let planner = VLogHolePunchPlanner::new(4096);
        let spans = [DeadValueSpan::new(0, 4096)];
        let err = planner.plan_bounded_punches(&spans, 0).unwrap_err();
        assert_eq!(err, VLogHoleAlignmentError::ZeroFileSize);
    }

    #[test]
    fn test_bounded_punches_exceeding_file_size_rejected() {
        let planner = VLogHolePunchPlanner::new(4096);
        let spans = [DeadValueSpan::new(0, 8192)];
        let err = planner.plan_bounded_punches(&spans, 4096).unwrap_err();
        assert_eq!(
            err,
            VLogHoleAlignmentError::SpanExceedsFileSize {
                offset: 8192,
                file_size: 4096,
            }
        );
    }

    #[test]
    fn test_verify_punch_plan_collision_detected() {
        let planner = VLogHolePunchPlanner::new(4096);
        let punch = SafeHolePunch {
            aligned_start: 4096,
            aligned_end: 8192,
            punched_bytes: 4096,
        };
        let live = [(4000, 200)]; // overlaps 4096..8192
        let err = planner.verify_punch_plan(&[punch], &live).unwrap_err();
        assert_eq!(
            err,
            VLogHoleAlignmentError::LiveRecordOverlap {
                live_offset: 4000,
                live_len: 200,
                punch,
            }
        );
    }
}

