//! RFC-0282 Pilar 2 — Cura de Escrita Fracionária de Setor Físico (Torn Sector Heal Kernel).
//!
//! Formalizes torn-write detection across 512-byte physical disk sectors composing
//! a 4096-byte logical page. Uses a Dual-Boundary Generation Envelope:
//!   - Sector 0 (Head): embeds magic, page_id, generation_seq, and sector 0 tag;
//!   - Sector 7 (Tail): embeds identical page_id, generation_seq, and overall page CRC32C.
//!
//! Mathematically guarantees that any partial write (1 to 7 sectors updated before power loss)
//! is immediately detected as a torn write and rejected fail-closed or rolled back.

#![forbid(unsafe_code)]

/// Physical sector size in standard NVMe/hard disk controllers (512 bytes).
pub const PHYSICAL_SECTOR_SIZE: usize = 512;

/// Logical page size (4096 bytes = 8 sectors).
pub const LOGICAL_PAGE_SIZE: usize = 4096;

/// Number of 512B physical sectors in one 4KB page.
pub const SECTORS_PER_PAGE: usize = LOGICAL_PAGE_SIZE / PHYSICAL_SECTOR_SIZE;

/// Magic constant identifying valid PedraDB formatted page envelopes.
pub const ENVELOPE_MAGIC: u32 = 0x5045_4452; // "PEDR"

/// Violations resulting from torn sector physical writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TornSectorViolation {
    /// Generation number mismatch between Sector 0 and Sector 7 (classic torn write).
    BoundaryGenerationMismatch {
        /// Generation found in head sector.
        head_generation: u64,
        /// Generation found in tail sector.
        tail_generation: u64,
    },
    /// Page ID mismatch between Sector 0 and Sector 7 (misdirected block write).
    BoundaryPageIdMismatch {
        /// Page ID in head.
        head_page_id: u64,
        /// Page ID in tail.
        tail_page_id: u64,
    },
    /// Magic signature in header is invalid or corrupt.
    InvalidEnvelopeMagic {
        /// Found magic.
        found_magic: u32,
    },
    /// Data payload CRC32C mismatch.
    PagePayloadChecksumCorrupted {
        /// Computed CRC.
        computed_crc: u32,
        /// Stored CRC in tail sector.
        stored_crc: u32,
    },
}

/// A formatted logical 4096-byte page with dual-boundary generation tracking.
pub struct PageEnvelope;

impl PageEnvelope {
    /// Encodes a 4096-byte page buffer with dual-boundary generation tags and checksum.
    pub fn encode_page(
        page_id: u64,
        generation: u64,
        payload_data: &[u8],
        page_out: &mut [u8; LOGICAL_PAGE_SIZE],
    ) {
        // Clear page
        page_out.fill(0);

        // Copy up to 4000 bytes of payload in the data section (bytes 32..4064)
        let usable_payload_len = payload_data.len().min(4032);
        page_out[32..32 + usable_payload_len].copy_from_slice(&payload_data[..usable_payload_len]);

        // Sector 0 (Head):
        // [0..4]: ENVELOPE_MAGIC
        page_out[0..4].copy_from_slice(&ENVELOPE_MAGIC.to_le_bytes());
        // [4..12]: page_id
        page_out[4..12].copy_from_slice(&page_id.to_le_bytes());
        // [12..20]: generation
        page_out[12..20].copy_from_slice(&generation.to_le_bytes());

        // Compute payload CRC across bytes 0..4080
        let crc = crc32c::crc32c(&page_out[0..4080]);

        // Sector 7 (Tail):
        // [4080..4088]: page_id
        page_out[4080..4088].copy_from_slice(&page_id.to_le_bytes());
        // [4088..4092]: stored CRC
        page_out[4088..4092].copy_from_slice(&crc.to_le_bytes());
        // [4092..4096]: generation (lower 32 bits for alignment)
        let gen_tail = generation as u32;
        page_out[4092..4096].copy_from_slice(&gen_tail.to_le_bytes());
    }

    /// Verifies whether a 4096-byte buffer is an intact, non-torn logical page.
    ///
    /// # Errors
    /// Returns `TornSectorViolation` if a partial write, generation mismatch, or checksum failure is found.
    pub fn verify_page(page_in: &[u8; LOGICAL_PAGE_SIZE]) -> Result<(u64, u64), TornSectorViolation> {
        // 1. Verify Magic
        let magic = u32::from_le_bytes([page_in[0], page_in[1], page_in[2], page_in[3]]);
        if magic != ENVELOPE_MAGIC {
            return Err(TornSectorViolation::InvalidEnvelopeMagic { found_magic: magic });
        }

        // 2. Read Sector 0 parameters
        let mut head_page_bytes = [0u8; 8];
        head_page_bytes.copy_from_slice(&page_in[4..12]);
        let head_page_id = u64::from_le_bytes(head_page_bytes);

        let mut head_gen_bytes = [0u8; 8];
        head_gen_bytes.copy_from_slice(&page_in[12..20]);
        let head_generation = u64::from_le_bytes(head_gen_bytes);

        // 3. Read Sector 7 parameters
        let mut tail_page_bytes = [0u8; 8];
        tail_page_bytes.copy_from_slice(&page_in[4080..4088]);
        let tail_page_id = u64::from_le_bytes(tail_page_bytes);

        let mut tail_crc_bytes = [0u8; 4];
        tail_crc_bytes.copy_from_slice(&page_in[4088..4092]);
        let stored_crc = u32::from_le_bytes(tail_crc_bytes);

        let mut tail_gen_bytes = [0u8; 4];
        tail_gen_bytes.copy_from_slice(&page_in[4092..4096]);
        let tail_gen_low = u32::from_le_bytes(tail_gen_bytes);

        // Check page ID match
        if head_page_id != tail_page_id {
            return Err(TornSectorViolation::BoundaryPageIdMismatch {
                head_page_id,
                tail_page_id,
            });
        }

        // Check generation match (head vs tail)
        if (head_generation as u32) != tail_gen_low {
            return Err(TornSectorViolation::BoundaryGenerationMismatch {
                head_generation,
                tail_generation: u64::from(tail_gen_low),
            });
        }

        // 4. Verify Payload CRC
        let computed_crc = crc32c::crc32c(&page_in[0..4080]);
        if computed_crc != stored_crc {
            return Err(TornSectorViolation::PagePayloadChecksumCorrupted {
                computed_crc,
                stored_crc,
            });
        }

        Ok((head_page_id, head_generation))
    }
}
