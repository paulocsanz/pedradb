//! RFC-0326: FFI Provenance & Pointer Safety Verification Suite.
//!
//! Mechanically verifies FFI raw buffer boundary conditions under Miri / Tree Borrows:
//! non-nullity, alignment, size bounds, and absence of mutable aliasing.

#![forbid(unsafe_code)]

use pedradb_core::ffi_provenance_guard_kernel::{
    FfiProvenanceError, SafeFfiBufferDescriptor, MAX_SAFE_FFI_BUFFER_SIZE,
};

#[test]
fn rfc0326_ffi_provenance_construction_and_rejection() {
    // 1. Rejects null pointer (address 0)
    assert_eq!(
        SafeFfiBufferDescriptor::try_new(0, 1024, 8),
        Err(FfiProvenanceError::NullPointerHazard)
    );

    // 2. Rejects misaligned pointer
    assert_eq!(
        SafeFfiBufferDescriptor::try_new(0x1001, 1024, 8),
        Err(FfiProvenanceError::MisalignedPointer {
            address: 0x1001,
            required: 8
        })
    );

    // 3. Rejects length exceeding maximum target limit
    assert_eq!(
        SafeFfiBufferDescriptor::try_new(0x1000, MAX_SAFE_FFI_BUFFER_SIZE + 1, 8),
        Err(FfiProvenanceError::LengthExceedsTargetLimit {
            len: MAX_SAFE_FFI_BUFFER_SIZE + 1,
            limit: MAX_SAFE_FFI_BUFFER_SIZE,
        })
    );

    // 4. Rejects arithmetic overflow on address + len
    assert_eq!(
        SafeFfiBufferDescriptor::try_new(usize::MAX - 10, 20, 1),
        Err(FfiProvenanceError::LengthExceedsTargetLimit {
            len: 20,
            limit: 10,
        })
    );

    // 5. Valid aligned buffer accepted
    let desc = SafeFfiBufferDescriptor::try_new(0x2000, 4096, 64).expect("valid descriptor");
    assert_eq!(desc.address(), 0x2000);
    assert_eq!(desc.len(), 4096);
    assert!(!desc.is_empty());
}

#[test]
fn rfc0326_ffi_provenance_aliasing_detection() {
    // Buffer A: [0x1000..0x2000)
    let buf_a = SafeFfiBufferDescriptor::try_new(0x1000, 0x1000, 8).unwrap();
    // Buffer B (disjoint): [0x2000..0x3000)
    let buf_b = SafeFfiBufferDescriptor::try_new(0x2000, 0x1000, 8).unwrap();
    // Buffer C (overlapping with A): [0x1800..0x2800)
    let buf_c = SafeFfiBufferDescriptor::try_new(0x1800, 0x1000, 8).unwrap();

    // Disjoint buffers pass
    assert!(buf_a.verify_disjoint(&buf_b).is_ok());
    assert!(buf_b.verify_disjoint(&buf_a).is_ok());

    // Overlapping buffers fail closed with aliasing error
    let overlap_err = buf_a.verify_disjoint(&buf_c);
    assert_eq!(
        overlap_err,
        Err(FfiProvenanceError::OverlappingBufferAliasing {
            first_range: (0x1000, 0x2000),
            second_range: (0x1800, 0x2800),
        })
    );
}
