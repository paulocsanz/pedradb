//! RFC-0312: Deterministic Verification Suite for NVMe Direct-I/O Sector Alignment.
//!
//! Enforces zero-twin production verification of:
//! 1. Physical sector size validation and memory pointer alignment gates.
//! 2. Mathematical invariant: Prefix + Aligned + Suffix == Total Length across all offsets.
//! 3. Sector alignment of DMA operations (zero unaligned kernel EINVAL errors).
//! 4. Bit-for-bit data isomorphism under reassembly across arbitrary byte boundaries.

use pedradb_core::direct_io_sector_alignment_kernel::{
    AlignmentError, DirectIoAligner, LEGACY_SECTOR_SIZE, STANDARD_SECTOR_SIZE,
};

#[test]
fn test_sector_size_and_buffer_alignment_validation() {
    // Valid standard NVMe sector size (4096)
    let aligner_4k = DirectIoAligner::new(STANDARD_SECTOR_SIZE).expect("4096 is valid");
    assert_eq!(aligner_4k.sector_size(), 4096);

    // Valid legacy block sector size (512)
    let aligner_512 = DirectIoAligner::new(LEGACY_SECTOR_SIZE).expect("512 is valid");
    assert_eq!(aligner_512.sector_size(), 512);

    // Invalid: non power of two
    assert_eq!(
        DirectIoAligner::new(1000),
        Err(AlignmentError::InvalidSectorSize(1000))
    );

    // Invalid: under 512
    assert_eq!(
        DirectIoAligner::new(256),
        Err(AlignmentError::InvalidSectorSize(256))
    );

    // Memory buffer alignment check
    assert!(aligner_4k.validate_buffer_alignment(0x1000).is_ok());
    assert!(aligner_4k.validate_buffer_alignment(0x2000).is_ok());
    assert!(matches!(
        aligner_4k.validate_buffer_alignment(0x1001),
        Err(AlignmentError::UnalignedBufferAddress { .. })
    ));
}

#[test]
fn test_mathematical_decomposition_invariant() {
    let aligner = DirectIoAligner::new(STANDARD_SECTOR_SIZE).unwrap();

    // Test cases: (offset, len)
    let cases = [
        // 1. Fully aligned request
        (4096u64, 8192usize),
        // 2. Unaligned start, aligned end
        (4097u64, 8191usize),
        // 3. Aligned start, unaligned end
        (4096u64, 5000usize),
        // 4. Completely unaligned spanning multiple sectors
        (1234u64, 15000usize),
        // 5. Completely contained within a single sector
        (500u64, 300usize),
        // 6. Zero length
        (4096u64, 0usize),
        // 7. Large request
        (10_000_001u64, 2_000_003usize),
    ];

    for (offset, len) in cases {
        let plan = aligner.plan_io_slices(offset, len);

        // Invariant 1: Sum of decomposed slices matches total length exactly
        assert_eq!(
            plan.prefix_len + plan.aligned_len + plan.suffix_len,
            plan.total_len,
            "Decomposition must conserve length for offset={offset}, len={len}"
        );

        // Invariant 2: Aligned middle body is strictly aligned to sector boundaries
        assert_eq!(
            plan.aligned_offset % (aligner.sector_size() as u64),
            0,
            "Aligned body offset must be sector-aligned"
        );
        assert_eq!(
            plan.aligned_len % aligner.sector_size(),
            0,
            "Aligned body length must be multiple of sector size"
        );

        // Invariant 3: Prefix head does not exceed sector size
        assert!(
            plan.prefix_len < aligner.sector_size(),
            "Prefix head must be strictly smaller than one sector"
        );

        // Invariant 4: Suffix tail does not exceed sector size
        assert!(
            plan.suffix_len < aligner.sector_size(),
            "Suffix tail must be strictly smaller than one sector"
        );
    }
}

#[test]
fn test_data_isomorphism_reassembly_roundtrip() {
    let aligner = DirectIoAligner::new(512).unwrap();

    // Create a continuous data payload of 2000 bytes starting at offset 300
    let file_offset = 300u64;
    let payload: Vec<u8> = (0..2000).map(|i| (i % 251) as u8).collect();

    let plan = aligner.plan_io_slices(file_offset, payload.len());

    // Simulate physical storage medium (512-byte sectors)
    let total_file_size = 4096;
    let mut virtual_disk = vec![0u8; total_file_size];

    // 1. Write prefix head via bounce buffer
    if plan.prefix_len > 0 {
        let mut bounce = vec![0u8; plan.prefix_bounce_len];
        // Read existing sector into bounce
        let sec_off = plan.prefix_sector_offset as usize;
        bounce.copy_from_slice(&virtual_disk[sec_off..sec_off + plan.prefix_bounce_len]);
        // Overlay payload prefix
        let start_in_bounce = (plan.logical_offset - plan.prefix_sector_offset) as usize;
        bounce[start_in_bounce..start_in_bounce + plan.prefix_len]
            .copy_from_slice(&payload[..plan.prefix_len]);
        // Write back aligned sector
        virtual_disk[sec_off..sec_off + plan.prefix_bounce_len].copy_from_slice(&bounce);
    }

    // 2. Write aligned middle body via direct DMA
    if plan.aligned_len > 0 {
        let body_start = plan.prefix_len;
        let body_end = body_start + plan.aligned_len;
        let disk_off = plan.aligned_offset as usize;
        virtual_disk[disk_off..disk_off + plan.aligned_len]
            .copy_from_slice(&payload[body_start..body_end]);
    }

    // 3. Write suffix tail via bounce buffer
    if plan.suffix_len > 0 {
        let mut bounce = vec![0u8; plan.suffix_bounce_len];
        let sec_off = plan.suffix_offset as usize;
        bounce.copy_from_slice(&virtual_disk[sec_off..sec_off + plan.suffix_bounce_len]);
        let body_end = plan.prefix_len + plan.aligned_len;
        bounce[..plan.suffix_len].copy_from_slice(&payload[body_end..]);
        virtual_disk[sec_off..sec_off + plan.suffix_bounce_len].copy_from_slice(&bounce);
    }

    // Verification: Read back bytes from disk and verify bit-for-bit identity
    let read_back = &virtual_disk[file_offset as usize..file_offset as usize + payload.len()];
    assert_eq!(read_back, payload.as_slice(), "Reassembled Direct-I/O data must be isomorphic");
}
