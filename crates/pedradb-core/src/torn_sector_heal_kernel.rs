//! RFC-0282 Pilar 2 — Cura de Escrita Fracionária de Setor Físico (Torn Sector Heal Kernel).
//!
//! Formalizes torn-write detection across 512-byte physical disk sectors composing
//! a 4096-byte logical page. Uses a Dual-Boundary Generation Envelope:
//!   - Sector 0 (Head): embeds magic, page_id, generation_seq, and sector 0 tag;
//!   - Sector 7 (Tail): embeds identical page_id, generation_seq (full 64-bit), and overall page CRC32C.
//!
//! Mathematically guarantees that any partial write (1 to 7 sectors updated before power loss)
//! is immediately detected as a torn write and rejected fail-closed or rolled back.

#![forbid(unsafe_code)]

use std::fmt;

/// Physical sector size in standard NVMe/hard disk controllers (512 bytes).
pub const PHYSICAL_SECTOR_SIZE: usize = 512;

/// Logical page size (4096 bytes = 8 sectors).
pub const LOGICAL_PAGE_SIZE: usize = 4096;

/// Number of 512B physical sectors in one 4KB page.
pub const SECTORS_PER_PAGE: usize = LOGICAL_PAGE_SIZE / PHYSICAL_SECTOR_SIZE;

/// Maximum usable payload capacity in bytes.
pub const MAX_PAYLOAD_SIZE: usize = 4032;

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
    /// Generation number cannot be 0.
    ZeroGeneration,
    /// Page ID cannot be 0.
    ZeroPageId,
    /// Both target and backup copies are corrupt; page cannot be healed.
    UnrecoverableDualCorruption,
    /// Payload exceeds maximum capacity.
    PayloadTooLarge {
        /// Provided length.
        length: usize,
        /// Maximum capacity.
        max_capacity: usize,
    },
}

impl fmt::Display for TornSectorViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BoundaryGenerationMismatch { head_generation, tail_generation } => {
                write!(
                    f,
                    "Torn write generation mismatch: head={head_generation}, tail={tail_generation}"
                )
            }
            Self::BoundaryPageIdMismatch { head_page_id, tail_page_id } => {
                write!(
                    f,
                    "Misdirected write page ID mismatch: head={head_page_id}, tail={tail_page_id}"
                )
            }
            Self::InvalidEnvelopeMagic { found_magic } => {
                write!(f, "Invalid envelope magic: {found_magic:#010x}")
            }
            Self::PagePayloadChecksumCorrupted { computed_crc, stored_crc } => {
                write!(
                    f,
                    "Page payload CRC mismatch: computed={computed_crc:#010x}, stored={stored_crc:#010x}"
                )
            }
            Self::ZeroGeneration => {
                write!(f, "Page generation cannot be 0")
            }
            Self::ZeroPageId => {
                write!(f, "Page ID cannot be 0")
            }
            Self::UnrecoverableDualCorruption => {
                write!(f, "Dual unrecoverable corruption: target and backup pages are both damaged")
            }
            Self::PayloadTooLarge { length, max_capacity } => {
                write!(f, "Payload length {length} exceeds maximum capacity {max_capacity}")
            }
        }
    }
}

impl std::error::Error for TornSectorViolation {}

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

        // Copy up to 4032 bytes of payload in the data section (bytes 32..4064)
        let usable_payload_len = payload_data.len().min(MAX_PAYLOAD_SIZE);
        page_out[32..32 + usable_payload_len].copy_from_slice(&payload_data[..usable_payload_len]);

        // Sector 0 (Head):
        // [0..4]: ENVELOPE_MAGIC
        page_out[0..4].copy_from_slice(&ENVELOPE_MAGIC.to_le_bytes());
        // [4..12]: page_id
        page_out[4..12].copy_from_slice(&page_id.to_le_bytes());
        // [12..20]: generation (full 64-bit)
        page_out[12..20].copy_from_slice(&generation.to_le_bytes());
        // [20..24]: payload length (u32)
        page_out[20..24].copy_from_slice(&(usable_payload_len as u32).to_le_bytes());

        // Sector 7 (Tail):
        // [4076..4080]: generation upper 32 bits
        let gen_hi = (generation >> 32) as u32;
        page_out[4076..4080].copy_from_slice(&gen_hi.to_le_bytes());

        // Compute payload CRC across bytes 0..4080
        let crc = crc32c::crc32c(&page_out[0..4080]);

        // [4080..4088]: page_id
        page_out[4080..4088].copy_from_slice(&page_id.to_le_bytes());
        // [4088..4092]: stored CRC
        page_out[4088..4092].copy_from_slice(&crc.to_le_bytes());
        // [4092..4096]: generation lower 32 bits
        let gen_tail = generation as u32;
        page_out[4092..4096].copy_from_slice(&gen_tail.to_le_bytes());
    }

    /// Safely encodes a page buffer, rejecting zero generation or oversized payloads without truncation.
    pub fn try_encode_page(
        page_id: u64,
        generation: u64,
        payload_data: &[u8],
        page_out: &mut [u8; LOGICAL_PAGE_SIZE],
    ) -> Result<(), TornSectorViolation> {
        if page_id == 0 {
            return Err(TornSectorViolation::ZeroPageId);
        }
        if generation == 0 {
            return Err(TornSectorViolation::ZeroGeneration);
        }
        if payload_data.len() > MAX_PAYLOAD_SIZE {
            return Err(TornSectorViolation::PayloadTooLarge {
                length: payload_data.len(),
                max_capacity: MAX_PAYLOAD_SIZE,
            });
        }
        Self::encode_page(page_id, generation, payload_data, page_out);
        Ok(())
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

        if head_page_id == 0 {
            return Err(TornSectorViolation::ZeroPageId);
        }

        let mut head_gen_bytes = [0u8; 8];
        head_gen_bytes.copy_from_slice(&page_in[12..20]);
        let head_generation = u64::from_le_bytes(head_gen_bytes);

        if head_generation == 0 {
            return Err(TornSectorViolation::ZeroGeneration);
        }

        // 3. Read Sector 7 parameters
        let mut tail_gen_hi_bytes = [0u8; 4];
        tail_gen_hi_bytes.copy_from_slice(&page_in[4076..4080]);
        let tail_gen_hi = u32::from_le_bytes(tail_gen_hi_bytes);

        let mut tail_page_bytes = [0u8; 8];
        tail_page_bytes.copy_from_slice(&page_in[4080..4088]);
        let tail_page_id = u64::from_le_bytes(tail_page_bytes);

        let mut tail_crc_bytes = [0u8; 4];
        tail_crc_bytes.copy_from_slice(&page_in[4088..4092]);
        let stored_crc = u32::from_le_bytes(tail_crc_bytes);

        let mut tail_gen_bytes = [0u8; 4];
        tail_gen_bytes.copy_from_slice(&page_in[4092..4096]);
        let tail_gen_low = u32::from_le_bytes(tail_gen_bytes);

        let tail_generation = ((tail_gen_hi as u64) << 32) | (tail_gen_low as u64);

        // Check page ID match
        if head_page_id != tail_page_id {
            return Err(TornSectorViolation::BoundaryPageIdMismatch {
                head_page_id,
                tail_page_id,
            });
        }

        // Check generation match across full 64 bits (head vs tail)
        if head_generation != tail_generation {
            return Err(TornSectorViolation::BoundaryGenerationMismatch {
                head_generation,
                tail_generation,
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

    /// Extracts the valid payload slice from a verified page envelope.
    pub fn extract_payload(page_in: &[u8; LOGICAL_PAGE_SIZE]) -> Result<&[u8], TornSectorViolation> {
        Self::verify_page(page_in)?;
        let mut len_bytes = [0u8; 4];
        len_bytes.copy_from_slice(&page_in[20..24]);
        let payload_len = (u32::from_le_bytes(len_bytes) as usize).min(MAX_PAYLOAD_SIZE);
        Ok(&page_in[32..32 + payload_len])
    }

    /// Attempts to heal a damaged or torn target page using a verified backup page.
    ///
    /// If the target page is fully valid and intact, returns `HealOutcome::TargetIntact`.
    /// If the target page exhibits a torn write or CRC failure, validates the backup page.
    /// If the backup is valid, copies it over the target and returns `HealOutcome::HealedFromBackup`.
    /// If both target and backup are damaged, returns `Err(TornSectorViolation::UnrecoverableDualCorruption)`.
    pub fn heal_page(
        target_page: &mut [u8; LOGICAL_PAGE_SIZE],
        backup_page: &[u8; LOGICAL_PAGE_SIZE],
    ) -> Result<HealOutcome, TornSectorViolation> {
        match Self::verify_page(target_page) {
            Ok((page_id, generation)) => Ok(HealOutcome::TargetIntact { page_id, generation }),
            Err(_) => match Self::verify_page(backup_page) {
                Ok((page_id, generation)) => {
                    target_page.copy_from_slice(backup_page);
                    Ok(HealOutcome::HealedFromBackup { page_id, generation })
                }
                Err(_) => Err(TornSectorViolation::UnrecoverableDualCorruption),
            },
        }
    }
}

/// Result of attempting to heal a torn page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HealOutcome {
    /// Target page was already intact and valid; no healing needed.
    TargetIntact {
        /// Intact page ID.
        page_id: u64,
        /// Intact generation.
        generation: u64,
    },
    /// Target page was damaged or torn and was successfully recovered from the backup page.
    HealedFromBackup {
        /// Restored page ID.
        page_id: u64,
        /// Restored generation.
        generation: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_page_id_rejected() {
        let mut buf = [0u8; LOGICAL_PAGE_SIZE];
        let err = PageEnvelope::try_encode_page(0, 10, b"data", &mut buf).unwrap_err();
        assert_eq!(err, TornSectorViolation::ZeroPageId);
    }

    #[test]
    fn test_extract_payload_exact() {
        let mut buf = [0u8; LOGICAL_PAGE_SIZE];
        let payload = b"pedradb_high_integrity_record";
        PageEnvelope::try_encode_page(101, 1, payload, &mut buf).unwrap();

        let extracted = PageEnvelope::extract_payload(&buf).unwrap();
        assert_eq!(extracted, payload);
    }

    #[test]
    fn test_heal_torn_page_from_backup_success() {
        let mut target_buf = [0u8; LOGICAL_PAGE_SIZE];
        let mut backup_buf = [0u8; LOGICAL_PAGE_SIZE];
        let payload = b"safe_backup_record";

        PageEnvelope::try_encode_page(50, 10, payload, &mut target_buf).unwrap();
        PageEnvelope::try_encode_page(50, 10, payload, &mut backup_buf).unwrap();

        // Corrupt target buffer (torn write at tail)
        target_buf[4092..4096].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        assert!(PageEnvelope::verify_page(&target_buf).is_err());

        // Heal from backup
        let outcome = PageEnvelope::heal_page(&mut target_buf, &backup_buf).unwrap();
        assert_eq!(
            outcome,
            HealOutcome::HealedFromBackup {
                page_id: 50,
                generation: 10
            }
        );
        // Verify target is now intact and matches extracted payload
        assert_eq!(PageEnvelope::extract_payload(&target_buf).unwrap(), payload);
    }

    #[test]
    fn test_heal_unrecoverable_when_both_corrupted() {
        let mut target_buf = [0u8; LOGICAL_PAGE_SIZE];
        let mut backup_buf = [0u8; LOGICAL_PAGE_SIZE];

        PageEnvelope::try_encode_page(50, 10, b"target", &mut target_buf).unwrap();
        PageEnvelope::try_encode_page(50, 10, b"backup", &mut backup_buf).unwrap();

        // Corrupt both
        target_buf[0..4].fill(0);
        backup_buf[0..4].fill(0);

        let err = PageEnvelope::heal_page(&mut target_buf, &backup_buf).unwrap_err();
        assert_eq!(err, TornSectorViolation::UnrecoverableDualCorruption);
    }
}
