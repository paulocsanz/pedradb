//! RFC-0317: Vectorized Scatter-Gather Sector Aligner test suite.
//!
//! Verifies:
//! - Exact alignment vs sub-sector padding calculation.
//! - Sector size power-of-two enforcement.
//! - Arithmetic overflow guard on u64 bounds.
//! - Scatter-gather memory vector integrity and length validation.

use pedradb_core::sector_aligned_io_kernel::{
    ScatterGatherValidator, SectorAlignError, SectorAlignedIoPlan,
};

#[test]
fn test_exact_sector_aligned_plan() {
    let plan = SectorAlignedIoPlan::calculate(4096, 8192, 4096).expect("valid plan");
    assert!(plan.is_exact_sector_aligned());
    assert_eq!(plan.head_padding, 0);
    assert_eq!(plan.tail_padding, 0);
    assert_eq!(plan.aligned_offset, 4096);
    assert_eq!(plan.aligned_len, 8192);
    assert_eq!(plan.total_sectors(), 2);
    assert!(plan.verify_internal_invariants());
}

#[test]
fn test_sub_sector_padding_calculation() {
    // Logical offset: 1000 (starts inside sector 0)
    // Payload len: 5000 (ends at 6000, which is inside sector 1 [4096..8192))
    // Aligned offset: 0, Aligned end: 8192, Aligned len: 8192
    // Head padding: 1000 - 0 = 1000
    // Tail padding: 8192 - 6000 = 2192
    let plan = SectorAlignedIoPlan::calculate(1000, 5000, 4096).expect("valid plan");
    assert!(!plan.is_exact_sector_aligned());
    assert_eq!(plan.aligned_offset, 0);
    assert_eq!(plan.head_padding, 1000);
    assert_eq!(plan.tail_padding, 2192);
    assert_eq!(plan.aligned_len, 8192);
    assert_eq!(plan.total_sectors(), 2);
    assert!(plan.verify_internal_invariants());

    // Verify 512-byte sector geometry
    let plan512 = SectorAlignedIoPlan::calculate(100, 50, 512).expect("valid plan");
    assert_eq!(plan512.aligned_offset, 0);
    assert_eq!(plan512.head_padding, 100);
    assert_eq!(plan512.tail_padding, 512 - 150);
    assert_eq!(plan512.aligned_len, 512);
    assert_eq!(plan512.total_sectors(), 1);
    assert!(plan512.verify_internal_invariants());
}

#[test]
fn test_invalid_sector_size_and_overflow_rejection() {
    assert_eq!(
        SectorAlignedIoPlan::calculate(0, 100, 0).err(),
        Some(SectorAlignError::InvalidSectorSize(0))
    );
    assert_eq!(
        SectorAlignedIoPlan::calculate(0, 100, 1000).err(),
        Some(SectorAlignError::InvalidSectorSize(1000))
    );
    assert_eq!(
        SectorAlignedIoPlan::calculate(0, 100, 4095).err(),
        Some(SectorAlignError::InvalidSectorSize(4095))
    );

    // Overflow check
    assert_eq!(
        SectorAlignedIoPlan::calculate(u64::MAX - 10, 50, 4096).err(),
        Some(SectorAlignError::OffsetOverflow)
    );
}

#[test]
fn test_scatter_gather_vector_validation() {
    let slices = vec![(0usize, 1024usize), (1024, 2048), (3072, 1024)];
    let total = ScatterGatherValidator::validate_iov_slices(&slices, 4096).expect("valid iov");
    assert_eq!(total, 4096);

    // Mismatched total expected
    assert_eq!(
        ScatterGatherValidator::validate_iov_slices(&slices, 4095).err(),
        Some(SectorAlignError::LengthMismatch {
            expected: 4095,
            actual: 4096,
        })
    );

    // Empty chunk rejection
    let bad_slices = vec![(0, 1024), (1024, 0)];
    assert_eq!(
        ScatterGatherValidator::validate_iov_slices(&bad_slices, 1024).err(),
        Some(SectorAlignError::EmptySliceChunk)
    );
}

#[test]
fn test_sector_aligned_io_red_invariants() {
    // 1. Error implements std::error::Error
    let err: Box<dyn std::error::Error> = Box::new(SectorAlignError::EmptySliceChunk);
    assert!(!err.to_string().is_empty());

    // 2. Reject zero payload length
    assert_eq!(
        SectorAlignedIoPlan::calculate(100, 0, 4096).err(),
        Some(SectorAlignError::ZeroPayloadLength)
    );

    // 3. Reject unphysically large sector size (> 65536)
    assert_eq!(
        SectorAlignedIoPlan::calculate(0, 4096, 131072).err(),
        Some(SectorAlignError::InvalidSectorSize(131072))
    );

    // 4. Critical: Reject overlapping memory slices in scatter-gather vectors
    let overlapping = vec![(0usize, 1024usize), (512usize, 1024usize)];
    let res = ScatterGatherValidator::validate_iov_slices(&overlapping, 2048);
    assert!(matches!(res, Err(SectorAlignError::OverlappingSliceRange { .. })));
}

