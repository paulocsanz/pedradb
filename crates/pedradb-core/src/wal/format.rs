//! On-disk format constants and record types for the WAL.
//!
//! The log is a sequence of fixed-size **blocks**. Each block holds zero or
//! more **physical records**, each with a 7-byte header followed by a payload
//! fragment. A logical application record that does not fit in a single
//! fragment is split across blocks using `First`/`Middle`/`Last`.
//!
//! This mirrors RocksDB's `db/log_format.h` so the two are directly comparable.

/// Block size in bytes. Records are laid out within blocks of this size.
/// Matches RocksDB's `kBlockSize`.
pub const BLOCK_SIZE: usize = 32_768;

/// Size of a physical record header: 4 bytes CRC + 2 bytes length + 1 byte type.
pub const HEADER_SIZE: usize = 7;

/// Physical record types. Values match RocksDB's `RecordType` enum so a WAL
/// produced by either engine can be parsed by the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RecordType {
    /// Zero record — reserved for preallocated / zeroed regions.
    Zero = 0,
    /// The full record fits in this single fragment.
    Full = 1,
    /// First fragment of a multi-fragment record.
    First = 2,
    /// Interior fragment of a multi-fragment record.
    Middle = 3,
    /// Final fragment of a multi-fragment record.
    Last = 4,
}

impl RecordType {
    /// Decode a raw type byte, returning `None` for an unknown value.
    #[must_use]
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Zero),
            1 => Some(Self::Full),
            2 => Some(Self::First),
            3 => Some(Self::Middle),
            4 => Some(Self::Last),
            _ => None,
        }
    }
}

/// Decode a little-endian `u16` length from a 2-byte slice (header bytes 4..6).
#[must_use]
pub fn decode_length(bytes: [u8; 2]) -> usize {
    u16::from_le_bytes(bytes) as usize
}

/// Decode a little-endian `u32` CRC from a 4-byte slice (header bytes 0..4).
#[must_use]
pub fn decode_crc(bytes: [u8; 4]) -> u32 {
    u32::from_le_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_type_round_trip() {
        for (byte, expected) in [
            (0u8, RecordType::Zero),
            (1, RecordType::Full),
            (2, RecordType::First),
            (3, RecordType::Middle),
            (4, RecordType::Last),
        ] {
            assert_eq!(RecordType::from_byte(byte), Some(expected));
        }
        assert_eq!(RecordType::from_byte(5), None);
        assert_eq!(RecordType::from_byte(255), None);
    }
}
