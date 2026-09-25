//! Prefix-Free Composite Key Injective Encoding Kernel (RFC-0284 Pilar 1).
//!
//! Enforces injective framing and strict order preservation for composite keys:
//! `(user_key, sequence_number, value_type)`.
//!
//! Guarantees:
//! 1. Injective mapping: `Encode(a) == Encode(b) <=> a == b`.
//! 2. Zero-byte immunity: User keys containing `0x00` do not collide or truncate.
//! 3. MVCC order preservation: For identical user keys, higher sequence numbers sort earlier.

#![forbid(unsafe_code)]

/// Value type tag in internal keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum InternalValueType {
    /// Tombstone marker (Delete).
    Deletion = 0,
    /// Standard key-value put.
    Value = 1,
}

impl InternalValueType {
    /// Convert byte to value type.
    pub fn from_u8(b: u8) -> Option<Self> {
        match b {
            0 => Some(Self::Deletion),
            1 => Some(Self::Value),
            _ => None,
        }
    }
}

/// Parsed internal key tuple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedInternalKey<'a> {
    /// Raw user key bytes.
    pub user_key: &'a [u8],
    /// MVCC commit sequence number.
    pub sequence_number: u64,
    /// Value type tag.
    pub value_type: InternalValueType,
}

/// Error returned on invalid encoded key buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCodecError {
    /// Buffer is shorter than minimum header (4 bytes len + 8 bytes seq + 1 byte type = 13 bytes).
    BufferTooShort,
    /// User key length exceeds buffer bounds.
    LengthMismatch,
    /// Value type tag is invalid.
    InvalidValueType,
    /// Trailing bytes detected beyond valid encoding.
    TrailingGarbage,
}

/// Prefix-free length-prefixed composite key encoder and decoder.
pub struct PrefixFreeKeyCodec;

impl PrefixFreeKeyCodec {
    /// Fixed trailer size: 8 bytes sequence number + 1 byte value type.
    pub const TRAILER_SIZE: usize = 8 + 1;
    /// Length prefix size: 4 bytes big-endian length.
    pub const PREFIX_SIZE: usize = 4;
    /// Total minimum encoded key length (empty user key).
    pub const MIN_ENCODED_SIZE: usize = Self::PREFIX_SIZE + Self::TRAILER_SIZE;

    /// Encodes `(user_key, seq, value_type)` into an injective byte vector.
    ///
    /// Framing:
    /// `[len: u32_be][user_key bytes][!seq: u64_be][value_type: u8]`
    ///
    /// `!seq` ensures that within the same user key, higher sequence numbers
    /// produce lexicographically smaller bytes, ensuring newest versions sort first.
    pub fn encode(user_key: &[u8], sequence_number: u64, value_type: InternalValueType) -> Vec<u8> {
        let key_len = user_key.len() as u32;
        let mut buf = Vec::with_capacity(Self::MIN_ENCODED_SIZE + user_key.len());
        buf.extend_from_slice(&key_len.to_be_bytes());
        buf.extend_from_slice(user_key);
        // Bitwise NOT so descending seq is ascending in byte sort
        let inv_seq = !sequence_number;
        buf.extend_from_slice(&inv_seq.to_be_bytes());
        buf.push(value_type as u8);
        buf
    }

    /// Decodes an internal key buffer back into `ParsedInternalKey`.
    pub fn decode(buf: &[u8]) -> Result<ParsedInternalKey<'_>, KeyCodecError> {
        if buf.len() < Self::MIN_ENCODED_SIZE {
            return Err(KeyCodecError::BufferTooShort);
        }

        let key_len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        let expected_total = Self::PREFIX_SIZE + key_len + Self::TRAILER_SIZE;

        if buf.len() < expected_total {
            return Err(KeyCodecError::LengthMismatch);
        }
        if buf.len() > expected_total {
            return Err(KeyCodecError::TrailingGarbage);
        }

        let user_key_end = Self::PREFIX_SIZE + key_len;
        let user_key = &buf[Self::PREFIX_SIZE..user_key_end];

        let seq_bytes: [u8; 8] = buf[user_key_end..user_key_end + 8]
            .try_into()
            .map_err(|_| KeyCodecError::BufferTooShort)?;
        let inv_seq = u64::from_be_bytes(seq_bytes);
        let sequence_number = !inv_seq;

        let type_byte = buf[user_key_end + 8];
        let value_type =
            InternalValueType::from_u8(type_byte).ok_or(KeyCodecError::InvalidValueType)?;

        Ok(ParsedInternalKey {
            user_key,
            sequence_number,
            value_type,
        })
    }

    /// Compares two encoded keys according to LSM internal key ordering.
    pub fn compare(a: &[u8], b: &[u8]) -> Result<std::cmp::Ordering, KeyCodecError> {
        let da = Self::decode(a)?;
        let db = Self::decode(b)?;

        match da.user_key.cmp(db.user_key) {
            std::cmp::Ordering::Equal => {
                // Higher sequence number comes FIRST (greater visibility)
                match db.sequence_number.cmp(&da.sequence_number) {
                    std::cmp::Ordering::Equal => {
                        // Deletion before Value if same seq (impossible under strict monotonic seq)
                        Ok((da.value_type as u8).cmp(&(db.value_type as u8)))
                    }
                    ord => Ok(ord),
                }
            }
            ord => Ok(ord),
        }
    }
}
