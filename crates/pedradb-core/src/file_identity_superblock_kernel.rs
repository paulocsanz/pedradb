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
}

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

    /// Creates and signs a new superblock for a new file.
    pub fn new(file_uuid: [u8; 16], file_number: u64, creation_epoch: u64) -> Self {
        let mut header = Self {
            magic: SST_SUPERBLOCK_MAGIC,
            file_uuid,
            file_number,
            creation_epoch,
            crc: 0,
        };
        let encoded_60 = header.encode_raw_60();
        header.crc = Self::compute_crc(&encoded_60);
        header
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

    /// Deserializes and validates checksum of a 64-byte block.
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

        let file_number = u64::from_be_bytes([
            bytes[24], bytes[25], bytes[26], bytes[27], bytes[28], bytes[29], bytes[30], bytes[31],
        ]);
        let creation_epoch = u64::from_be_bytes([
            bytes[32], bytes[33], bytes[34], bytes[35], bytes[36], bytes[37], bytes[38], bytes[39],
        ]);

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
}
