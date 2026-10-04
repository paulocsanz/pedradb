//! Superblock Cryptographic Identity and Anti-Inode-Reuse Kernel (RFC-0285 Pilar 3).
//!
//! Enforces an immutable 64-byte self-identifying superblock on every SST and WAL file,
//! immunizing PedraDB against POSIX inode reuse, stale file descriptors, and directory bitrot.
//!
//! Guarantees:
//! 1. Open handshake: every file open verifies `(UUID, FileNumber, CRC)` against MANIFEST.
//! 2. Fail-closed: any 1-bit discrepancy in identity immediately aborts file access.
//! 3. Zero zombie writes: background threads cannot overwrite recycled inodes.

#![forbid(unsafe_code)]

use std::fmt;

/// Magic identifier for PedraDB SST file superblocks.
pub const SST_SUPERBLOCK_MAGIC: [u8; 8] = *b"PEDRASST";
/// Total superblock size in bytes.
pub const SUPERBLOCK_SIZE: usize = 64;

/// 64-byte immutable file header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileSuperblock {
    /// Magic constant bytes ("PEDRASST").
    pub magic: [u8; 8],
    /// Cryptographically secure 128-bit file UUID.
    pub file_uuid: [u8; 16],
    /// Monotonic file number assigned by MANIFEST.
    pub file_number: u64,
    /// Epoch / timestamp of file creation.
    pub creation_epoch: u64,
    /// CRC32C over the first 60 bytes.
    pub crc: u32,
}

/// Verification failure during superblock identity handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuperblockError {
    /// Magic bytes mismatch (corrupted header or foreign file).
    InvalidMagic,
    /// Checksum verification failed (torn write or physical bitrot).
    CrcMismatch { computed: u32, stored: u32 },
    /// File number does not match expected MANIFEST record.
    FileNumberMismatch { expected: u64, actual: u64 },
    /// UUID does not match expected MANIFEST record (inode recycled).
    UuidMismatch,
    /// File number cannot be zero.
    ZeroFileNumber,
    /// File UUID cannot be all zeroes (nil UUID).
    NilUuid,
    /// Creation epoch cannot be zero.
    ZeroEpoch,
    /// Reserved superblock bytes (40..60) must be zero.
    ReservedBytesNonZero,
    /// Byte slice does not match required 64-byte superblock size.
    SliceLengthMismatch { expected: usize, actual: usize },
}

impl fmt::Display for SuperblockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "Invalid superblock magic bytes"),
            Self::CrcMismatch { computed, stored } => {
                write!(f, "Superblock CRC mismatch: computed={computed:#010x}, stored={stored:#010x}")
            }
            Self::FileNumberMismatch { expected, actual } => {
                write!(f, "File number mismatch: expected {expected}, actual {actual}")
            }
            Self::UuidMismatch => write!(f, "File UUID mismatch (inode recycled)"),
            Self::ZeroFileNumber => write!(f, "File number cannot be zero"),
            Self::NilUuid => write!(f, "File UUID cannot be all zeroes"),
            Self::ZeroEpoch => write!(f, "File creation epoch cannot be zero"),
            Self::ReservedBytesNonZero => write!(f, "Reserved superblock bytes must be zero"),
            Self::SliceLengthMismatch { expected, actual } => {
                write!(f, "Superblock slice length mismatch: expected {expected} bytes, got {actual}")
            }
        }
    }
}

impl std::error::Error for SuperblockError {}

impl FileSuperblock {
    /// Simple CRC32C computation for 60-byte payload.
    pub fn compute_crc(data: &[u8]) -> u32 {
        let mut crc: u32 = 0xFFFF_FFFF;
        for &byte in data {
            crc ^= byte as u32;
            for _ in 0..8 {
                if (crc & 1) != 0 {
                    crc = (crc >> 1) ^ 0x82F6_3B78;
                } else {
                    crc >>= 1;
                }
            }
        }
        !crc
    }

    /// Safely validates parameters and creates a new superblock.
    pub fn try_new(file_uuid: [u8; 16], file_number: u64, creation_epoch: u64) -> Result<Self, SuperblockError> {
        if file_number == 0 {
            return Err(SuperblockError::ZeroFileNumber);
        }
        if file_uuid == [0u8; 16] {
            return Err(SuperblockError::NilUuid);
        }
        if creation_epoch == 0 {
            return Err(SuperblockError::ZeroEpoch);
        }
        let mut header = Self {
            magic: SST_SUPERBLOCK_MAGIC,
            file_uuid,
            file_number,
            creation_epoch,
            crc: 0,
        };
        let encoded_60 = header.encode_raw_60();
        header.crc = Self::compute_crc(&encoded_60);
        Ok(header)
    }

    /// Creates and signs a new superblock for a new file (panics if invalid, for backwards compatibility).
    pub fn new(file_uuid: [u8; 16], file_number: u64, creation_epoch: u64) -> Self {
        Self::try_new(file_uuid, file_number, creation_epoch).expect("Invalid superblock parameters")
    }

    fn encode_raw_60(&self) -> [u8; 60] {
        let mut buf = [0u8; 60];
        buf[0..8].copy_from_slice(&self.magic);
        buf[8..24].copy_from_slice(&self.file_uuid);
        buf[24..32].copy_from_slice(&self.file_number.to_be_bytes());
        buf[32..40].copy_from_slice(&self.creation_epoch.to_be_bytes());
        // Remaining 20 bytes reserved as zeros
        buf
    }

    /// Serializes superblock into 64-byte array.
    pub fn encode(&self) -> [u8; SUPERBLOCK_SIZE] {
        let mut buf = [0u8; SUPERBLOCK_SIZE];
        buf[0..60].copy_from_slice(&self.encode_raw_60());
        buf[60..64].copy_from_slice(&self.crc.to_be_bytes());
        buf
    }

    /// Deserializes and validates checksum and fields of a 64-byte block.
    pub fn decode(bytes: &[u8; SUPERBLOCK_SIZE]) -> Result<Self, SuperblockError> {
        let mut magic = [0u8; 8];
        magic.copy_from_slice(&bytes[0..8]);
        if magic != SST_SUPERBLOCK_MAGIC {
            return Err(SuperblockError::InvalidMagic);
        }

        let computed_crc = Self::compute_crc(&bytes[0..60]);
        let stored_crc = u32::from_be_bytes([bytes[60], bytes[61], bytes[62], bytes[63]]);

        if computed_crc != stored_crc {
            return Err(SuperblockError::CrcMismatch {
                computed: computed_crc,
                stored: stored_crc,
            });
        }

        let mut file_uuid = [0u8; 16];
        file_uuid.copy_from_slice(&bytes[8..24]);
        if file_uuid == [0u8; 16] {
            return Err(SuperblockError::NilUuid);
        }

        let file_number = u64::from_be_bytes([
            bytes[24], bytes[25], bytes[26], bytes[27], bytes[28], bytes[29], bytes[30], bytes[31],
        ]);
        if file_number == 0 {
            return Err(SuperblockError::ZeroFileNumber);
        }

        let creation_epoch = u64::from_be_bytes([
            bytes[32], bytes[33], bytes[34], bytes[35], bytes[36], bytes[37], bytes[38], bytes[39],
        ]);
        if creation_epoch == 0 {
            return Err(SuperblockError::ZeroEpoch);
        }

        // Verify reserved 20 bytes (40..60) are zero
        if bytes[40..60].iter().any(|&b| b != 0) {
            return Err(SuperblockError::ReservedBytesNonZero);
        }

        Ok(Self {
            magic,
            file_uuid,
            file_number,
            creation_epoch,
            crc: stored_crc,
        })
    }

    /// Performs cross-validation handshake against expected MANIFEST metadata.
    pub fn verify_handshake(
        &self,
        expected_file_number: u64,
        expected_file_uuid: [u8; 16],
    ) -> Result<(), SuperblockError> {
        if self.file_number != expected_file_number {
            return Err(SuperblockError::FileNumberMismatch {
                expected: expected_file_number,
                actual: self.file_number,
            });
        }
        if self.file_uuid != expected_file_uuid {
            return Err(SuperblockError::UuidMismatch);
        }
        Ok(())
    }

    /// Deserializes and validates checksum and fields of a byte slice.
    pub fn try_decode_slice(bytes: &[u8]) -> Result<Self, SuperblockError> {
        if bytes.len() != SUPERBLOCK_SIZE {
            return Err(SuperblockError::SliceLengthMismatch {
                expected: SUPERBLOCK_SIZE,
                actual: bytes.len(),
            });
        }
        let mut arr = [0u8; SUPERBLOCK_SIZE];
        arr.copy_from_slice(bytes);
        Self::decode(&arr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_identity_superblock_structural_invariants_red_to_green() {
        let uuid = [7u8; 16];
        let sb = FileSuperblock::try_new(uuid, 100, 1).unwrap();
        let encoded = sb.encode();

        // 1. Slice length mismatch cleanly rejected
        assert_eq!(
            FileSuperblock::try_decode_slice(&encoded[..63]),
            Err(SuperblockError::SliceLengthMismatch {
                expected: 64,
                actual: 63,
            })
        );
        assert_eq!(
            FileSuperblock::try_decode_slice(&[0u8; 100]),
            Err(SuperblockError::SliceLengthMismatch {
                expected: 64,
                actual: 100,
            })
        );

        // 2. Valid decode matches original
        let decoded = FileSuperblock::try_decode_slice(&encoded).unwrap();
        assert_eq!(decoded.file_number, 100);
        assert_eq!(decoded.file_uuid, uuid);
        assert_eq!(decoded.verify_handshake(100, uuid), Ok(()));

        // 3. Handshake mismatches
        assert_eq!(
            decoded.verify_handshake(101, uuid),
            Err(SuperblockError::FileNumberMismatch {
                expected: 101,
                actual: 100,
            })
        );
    }
}

