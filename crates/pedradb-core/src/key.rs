//! Internal keys: user key + sequence number + value type.
//!
//! RocksDB/Pebble-style versioning for MVCC. Hot paths use the struct form;
//! [`InternalKey::encode`] / [`InternalKey::decode`] exist for WAL/SST payloads.
//!
//! # Ordering
//! 1. User key ascending (bytewise)
//! 2. Sequence number **descending** (newest first)
//! 3. [`ValueType`] descending
//!
//! A point lookup at snapshot `S` takes the first entry with the same user key
//! and `sequence <= S` under this order.

use std::cmp::Ordering;

use bytes::Bytes;

use crate::error::{CoreError, Result};

/// Monotonic version stamp assigned to each write.
///
/// Packed into 56 bits when encoded with a [`ValueType`] (RocksDB-compatible).
pub type SequenceNumber = u64;

/// Largest sequence that fits in the on-disk 56-bit field.
pub const MAX_SEQUENCE_NUMBER: SequenceNumber = (1u64 << 56) - 1;

/// Kind of an internal key entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ValueType {
    /// Tombstone: hides older values for the same user key.
    Deletion = 0,
    /// Ordinary put.
    Value = 1,
}

impl ValueType {
    /// Decode from the low byte of a packed trailer.
    #[must_use]
    pub fn from_u8(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Deletion),
            1 => Some(Self::Value),
            _ => None,
        }
    }

    /// Encode as the type nibble of a packed trailer.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Pack sequence and type into the 8-byte internal-key trailer (RocksDB layout).
#[must_use]
pub fn pack_sequence_and_type(sequence: SequenceNumber, kind: ValueType) -> u64 {
    debug_assert!(sequence <= MAX_SEQUENCE_NUMBER);
    (sequence << 8) | u64::from(kind.as_u8())
}

/// Unpack an 8-byte trailer into sequence and type.
///
/// # Errors
/// Returns [`CoreError::Internal`] if the type byte is unknown.
pub fn unpack_sequence_and_type(packed: u64) -> Result<(SequenceNumber, ValueType)> {
    let type_byte = (packed & 0xff) as u8;
    let kind = ValueType::from_u8(type_byte).ok_or_else(|| {
        CoreError::Internal(format!("unknown internal key value type: {type_byte:#04x}"))
    })?;
    let sequence = packed >> 8;
    Ok((sequence, kind))
}

/// Versioned key used inside MemTable, SST, and the write path.
#[derive(Debug, Clone)]
pub struct InternalKey {
    /// Application key bytes.
    pub user_key: Bytes,
    /// Write version; higher is newer.
    pub sequence: SequenceNumber,
    /// Put vs deletion (and future kinds).
    pub kind: ValueType,
}

impl InternalKey {
    /// Build a key from owned/shared user-key bytes.
    #[must_use]
    pub fn new(user_key: impl Into<Bytes>, sequence: SequenceNumber, kind: ValueType) -> Self {
        Self {
            user_key: user_key.into(),
            sequence,
            kind,
        }
    }

    /// Probe key for a point lookup at `snapshot` (RocksDB `kValueTypeForSeek`).
    #[must_use]
    pub fn for_lookup(user_key: impl Into<Bytes>, snapshot: SequenceNumber) -> Self {
        Self::new(user_key, snapshot, ValueType::Value)
    }

    /// Encode as `user_key || trailer_be`, suitable for WAL/SST.
    #[must_use]
    pub fn encode(&self) -> Bytes {
        let mut buf = Vec::with_capacity(self.user_key.len() + 8);
        buf.extend_from_slice(&self.user_key);
        let packed = pack_sequence_and_type(self.sequence, self.kind);
        buf.extend_from_slice(&packed.to_be_bytes());
        Bytes::from(buf)
    }

    /// Decode `user_key || trailer_be`.
    ///
    /// # Errors
    /// Returns [`CoreError::Internal`] if the buffer is shorter than 8 bytes or
    /// the type is unknown.
    pub fn decode(encoded: &[u8]) -> Result<Self> {
        if encoded.len() < 8 {
            return Err(CoreError::Internal(format!(
                "internal key too short: {} bytes",
                encoded.len()
            )));
        }
        let split = encoded.len() - 8;
        let user_key = Bytes::copy_from_slice(&encoded[..split]);
        let mut trailer = [0u8; 8];
        trailer.copy_from_slice(&encoded[split..]);
        let packed = u64::from_be_bytes(trailer);
        let (sequence, kind) = unpack_sequence_and_type(packed)?;
        Ok(Self {
            user_key,
            sequence,
            kind,
        })
    }
}

impl PartialEq for InternalKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for InternalKey {}

impl PartialOrd for InternalKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for InternalKey {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.user_key.as_ref().cmp(other.user_key.as_ref()) {
            Ordering::Equal => {}
            ord => return ord,
        }
        // Sequence descending: higher sequence is "smaller".
        match other.sequence.cmp(&self.sequence) {
            Ordering::Equal => {}
            ord => return ord,
        }
        // Kind descending.
        other.kind.cmp(&self.kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_type_round_trip() {
        assert_eq!(ValueType::from_u8(0), Some(ValueType::Deletion));
        assert_eq!(ValueType::from_u8(1), Some(ValueType::Value));
        assert_eq!(ValueType::from_u8(2), None);
    }

    #[test]
    fn pack_unpack_round_trip() {
        let packed = pack_sequence_and_type(42, ValueType::Value);
        let (seq, kind) = unpack_sequence_and_type(packed).unwrap();
        assert_eq!(seq, 42);
        assert_eq!(kind, ValueType::Value);
    }

    #[test]
    fn encode_decode_round_trip() {
        let key = InternalKey::new(Bytes::from_static(b"hello"), 99, ValueType::Deletion);
        let encoded = key.encode();
        let decoded = InternalKey::decode(&encoded).unwrap();
        assert_eq!(decoded.user_key, key.user_key);
        assert_eq!(decoded.sequence, key.sequence);
        assert_eq!(decoded.kind, key.kind);
    }

    #[test]
    fn ordering_same_user_key_newest_first() {
        let a = InternalKey::new(Bytes::from_static(b"k"), 10, ValueType::Value);
        let b = InternalKey::new(Bytes::from_static(b"k"), 5, ValueType::Value);
        let c = InternalKey::new(Bytes::from_static(b"k"), 1, ValueType::Value);
        assert!(a < b);
        assert!(b < c);
        assert!(a < c);
    }

    #[test]
    fn ordering_user_key_bytewise() {
        let a = InternalKey::new(Bytes::from_static(b"a"), 1, ValueType::Value);
        let b = InternalKey::new(Bytes::from_static(b"b"), 100, ValueType::Value);
        assert!(a < b);
    }

    #[test]
    fn lookup_probe_lands_on_visible_version() {
        // MemTable-style: map ordered by InternalKey; lower_bound(probe) should
        // hit the newest seq <= snapshot.
        use std::collections::BTreeMap;

        let mut map = BTreeMap::new();
        map.insert(
            InternalKey::new(Bytes::from_static(b"k"), 10, ValueType::Value),
            Bytes::from_static(b"v10"),
        );
        map.insert(
            InternalKey::new(Bytes::from_static(b"k"), 5, ValueType::Value),
            Bytes::from_static(b"v5"),
        );
        map.insert(
            InternalKey::new(Bytes::from_static(b"k"), 1, ValueType::Value),
            Bytes::from_static(b"v1"),
        );

        let probe = InternalKey::for_lookup(Bytes::from_static(b"k"), 7);
        let (key, val) = map.range(probe..).next().unwrap();
        assert_eq!(key.sequence, 5);
        assert_eq!(val.as_ref(), b"v5");
    }

    #[test]
    fn decode_rejects_short_buffer() {
        assert!(InternalKey::decode(&[0u8; 7]).is_err());
    }
}
