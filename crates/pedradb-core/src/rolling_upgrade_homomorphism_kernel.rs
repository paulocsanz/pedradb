//! Bidirectional Schema Homomorphism and Unknown-Field Preservation Kernel (RFC-0285 Pilar 10).
//!
//! Guarantees bidirectional compatibility and zero data-loss during federated rolling upgrades,
//! where older cluster nodes (V1) process and re-transmit messages originated by newer nodes (V2).
//!
//! Axiom:
//! E_V2(D_V1(payload_V2)) == payload_V2.
//! Unknown extension fields are held in a faithful opaque envelope and round-tripped without truncation.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

/// Error during schema homomorphism decode or verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaHomomorphismError {
    /// Incompatible wire format magic header.
    InvalidWireMagic,
    /// Truncated payload or unexpected EOF.
    UnexpectedEof { expected: usize, actual: usize },
    /// Unknown required field that cannot be safely skipped.
    UnsupportedCriticalField { field_id: u16 },
    /// Wire version is zero (invalid uninitialized version).
    ZeroWireVersion,
    /// Wire payload contains unparsed trailing garbage bytes.
    TrailingGarbage { consumed: usize, total: usize },
    /// Wire payload contains duplicate field IDs.
    DuplicateFieldId { field_id: u16 },
}

impl std::fmt::Display for SchemaHomomorphismError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidWireMagic => write!(f, "invalid wire magic header"),
            Self::UnexpectedEof { expected, actual } => {
                write!(f, "unexpected EOF: expected {expected} bytes, found {actual}")
            }
            Self::UnsupportedCriticalField { field_id } => {
                write!(f, "unsupported critical field ID: {field_id}")
            }
            Self::ZeroWireVersion => write!(f, "wire version cannot be zero"),
            Self::TrailingGarbage { consumed, total } => {
                write!(
                    f,
                    "payload contains {total} bytes but only {consumed} were consumed by fields"
                )
            }
            Self::DuplicateFieldId { field_id } => {
                write!(f, "duplicate field ID encountered in wire payload: {field_id}")
            }
        }
    }
}

impl std::error::Error for SchemaHomomorphismError {}

/// Error during extensible message construction or encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensibleMessageError {
    /// Wire version is zero.
    ZeroWireVersion,
    /// Total count of fields exceeds u16::MAX.
    TotalFieldsOverflow { count: usize },
    /// Field payload exceeds u32::MAX.
    PayloadTooLarge { field_id: u16, len: usize },
    /// A field ID appears in both base fields and unknown extensions.
    OverlappingFieldId { field_id: u16 },
}

impl std::fmt::Display for ExtensibleMessageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroWireVersion => write!(f, "wire version cannot be zero"),
            Self::TotalFieldsOverflow { count } => {
                write!(f, "total field count {count} exceeds wire limit of {}", u16::MAX)
            }
            Self::PayloadTooLarge { field_id, len } => {
                write!(f, "payload for field {field_id} has length {len} exceeding u32::MAX")
            }
            Self::OverlappingFieldId { field_id } => {
                write!(f, "field ID {field_id} is present in both base and extension sets")
            }
        }
    }
}

impl std::error::Error for ExtensibleMessageError {}

/// A wire-format message with known base fields and opaque future extensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensibleMessage {
    /// Schema version declared by producer.
    pub wire_version: u16,
    /// Known base key/value map.
    pub base_fields: BTreeMap<u16, Vec<u8>>,
    /// Unknown opaque extension fields preserved intact for round-tripping.
    pub unknown_extensions: BTreeMap<u16, Vec<u8>>,
}

impl ExtensibleMessage {
    /// Magic header identifier: `[0x50, 0x45, 0x44, 0x52]` ("PEDR").
    pub const MAGIC: [u8; 4] = [0x50, 0x45, 0x44, 0x52];

    /// Construct a verified extensible message.
    pub fn try_new(
        wire_version: u16,
        base_fields: BTreeMap<u16, Vec<u8>>,
        unknown_extensions: BTreeMap<u16, Vec<u8>>,
    ) -> Result<Self, ExtensibleMessageError> {
        if wire_version == 0 {
            return Err(ExtensibleMessageError::ZeroWireVersion);
        }
        let total = base_fields
            .len()
            .checked_add(unknown_extensions.len())
            .ok_or(ExtensibleMessageError::TotalFieldsOverflow { count: usize::MAX })?;
        if total > u16::MAX as usize {
            return Err(ExtensibleMessageError::TotalFieldsOverflow { count: total });
        }
        for (&field_id, val) in &base_fields {
            if unknown_extensions.contains_key(&field_id) {
                return Err(ExtensibleMessageError::OverlappingFieldId { field_id });
            }
            if val.len() > u32::MAX as usize {
                return Err(ExtensibleMessageError::PayloadTooLarge {
                    field_id,
                    len: val.len(),
                });
            }
        }
        for (&field_id, val) in &unknown_extensions {
            if val.len() > u32::MAX as usize {
                return Err(ExtensibleMessageError::PayloadTooLarge {
                    field_id,
                    len: val.len(),
                });
            }
        }
        Ok(Self {
            wire_version,
            base_fields,
            unknown_extensions,
        })
    }

    /// Encodes this message into a canonical wire format with strict validation.
    pub fn try_encode(&self) -> Result<Vec<u8>, ExtensibleMessageError> {
        if self.wire_version == 0 {
            return Err(ExtensibleMessageError::ZeroWireVersion);
        }
        let total_fields = self
            .base_fields
            .len()
            .checked_add(self.unknown_extensions.len())
            .ok_or(ExtensibleMessageError::TotalFieldsOverflow { count: usize::MAX })?;
        if total_fields > u16::MAX as usize {
            return Err(ExtensibleMessageError::TotalFieldsOverflow { count: total_fields });
        }

        for (&field_id, val) in &self.base_fields {
            if self.unknown_extensions.contains_key(&field_id) {
                return Err(ExtensibleMessageError::OverlappingFieldId { field_id });
            }
            if val.len() > u32::MAX as usize {
                return Err(ExtensibleMessageError::PayloadTooLarge {
                    field_id,
                    len: val.len(),
                });
            }
        }
        for (&field_id, val) in &self.unknown_extensions {
            if val.len() > u32::MAX as usize {
                return Err(ExtensibleMessageError::PayloadTooLarge {
                    field_id,
                    len: val.len(),
                });
            }
        }

        let mut out = Vec::new();
        out.extend_from_slice(&Self::MAGIC);
        out.extend_from_slice(&self.wire_version.to_le_bytes());
        out.extend_from_slice(&(total_fields as u16).to_le_bytes());

        // Emit base fields
        for (&field_id, val) in &self.base_fields {
            out.extend_from_slice(&field_id.to_le_bytes());
            let len = val.len() as u32;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(val);
        }

        // Emit preserved unknown extensions
        for (&field_id, val) in &self.unknown_extensions {
            out.extend_from_slice(&field_id.to_le_bytes());
            let len = val.len() as u32;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(val);
        }

        Ok(out)
    }

    /// Encodes this message into a canonical wire format.
    pub fn encode(&self) -> Vec<u8> {
        self.try_encode()
            .expect("message must be valid for canonical encoding")
    }

    /// Decodes a message using local knowledge bounded by `max_known_field_id`.
    ///
    /// Any field with `field_id > max_known_field_id` is preserved intact
    /// in `unknown_extensions` without data loss.
    pub fn decode(bytes: &[u8], max_known_field_id: u16) -> Result<Self, SchemaHomomorphismError> {
        if bytes.len() < 8 {
            return Err(SchemaHomomorphismError::UnexpectedEof {
                expected: 8,
                actual: bytes.len(),
            });
        }

        if bytes[0..4] != Self::MAGIC {
            return Err(SchemaHomomorphismError::InvalidWireMagic);
        }

        let wire_version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if wire_version == 0 {
            return Err(SchemaHomomorphismError::ZeroWireVersion);
        }

        let num_fields = u16::from_le_bytes([bytes[6], bytes[7]]) as usize;

        let mut offset = 8;
        let mut base_fields = BTreeMap::new();
        let mut unknown_extensions = BTreeMap::new();

        for _ in 0..num_fields {
            if bytes.len() < offset + 6 {
                return Err(SchemaHomomorphismError::UnexpectedEof {
                    expected: offset + 6,
                    actual: bytes.len(),
                });
            }

            let field_id = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
            let field_len = u32::from_le_bytes([
                bytes[offset + 2],
                bytes[offset + 3],
                bytes[offset + 4],
                bytes[offset + 5],
            ]) as usize;
            offset += 6;

            if bytes.len() < offset + field_len {
                return Err(SchemaHomomorphismError::UnexpectedEof {
                    expected: offset + field_len,
                    actual: bytes.len(),
                });
            }

            let payload = bytes[offset..offset + field_len].to_vec();
            offset += field_len;

            if field_id <= max_known_field_id {
                if base_fields.contains_key(&field_id) {
                    return Err(SchemaHomomorphismError::DuplicateFieldId { field_id });
                }
                base_fields.insert(field_id, payload);
            } else {
                if unknown_extensions.contains_key(&field_id) {
                    return Err(SchemaHomomorphismError::DuplicateFieldId { field_id });
                }
                unknown_extensions.insert(field_id, payload);
            }
        }

        if offset < bytes.len() {
            return Err(SchemaHomomorphismError::TrailingGarbage {
                consumed: offset,
                total: bytes.len(),
            });
        }

        Ok(Self {
            wire_version,
            base_fields,
            unknown_extensions,
        })
    }

    /// Verifies the homomorphic round-trip property:
    /// `encode(decode(raw)) == raw`.
    pub fn verify_homomorphic_roundtrip(raw: &[u8], local_max_field: u16) -> bool {
        match Self::decode(raw, local_max_field) {
            Ok(msg) => msg.encode() == raw,
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rolling_upgrade_homomorphism_structural_invariants_red_to_green() {
        // 1. Valid roundtrip with unknown extensions preservation
        let mut base = BTreeMap::new();
        base.insert(1u16, b"alice".to_vec());
        let mut ext = BTreeMap::new();
        ext.insert(100u16, b"future_payload".to_vec());

        let msg = ExtensibleMessage::try_new(1, base.clone(), ext.clone())
            .expect("valid extensible message");
        let wire = msg.encode();

        assert!(ExtensibleMessage::verify_homomorphic_roundtrip(&wire, 10));

        // Node with local max 10 decodes msg: field 1 is base, field 100 is extension
        let decoded = ExtensibleMessage::decode(&wire, 10).expect("decode should succeed");
        assert_eq!(decoded.wire_version, 1);
        assert_eq!(decoded.base_fields.get(&1), Some(&b"alice".to_vec()));
        assert_eq!(decoded.unknown_extensions.get(&100), Some(&b"future_payload".to_vec()));
        assert_eq!(decoded.encode(), wire);

        // 2. Reject zero wire version
        let err_new = ExtensibleMessage::try_new(0, base.clone(), ext.clone());
        assert_eq!(err_new, Err(ExtensibleMessageError::ZeroWireVersion));

        let mut zero_wire = wire.clone();
        zero_wire[4] = 0;
        zero_wire[5] = 0;
        let err_decode_zero = ExtensibleMessage::decode(&zero_wire, 10);
        assert_eq!(err_decode_zero, Err(SchemaHomomorphismError::ZeroWireVersion));

        // 3. Reject trailing garbage bytes
        let mut garbage_wire = wire.clone();
        garbage_wire.extend_from_slice(b"TRASH_BYTES");
        let err_garbage = ExtensibleMessage::decode(&garbage_wire, 10);
        assert_eq!(
            err_garbage,
            Err(SchemaHomomorphismError::TrailingGarbage {
                consumed: wire.len(),
                total: garbage_wire.len(),
            })
        );

        // 4. Reject duplicate field IDs in decode
        let mut dup_wire = Vec::new();
        dup_wire.extend_from_slice(&ExtensibleMessage::MAGIC);
        dup_wire.extend_from_slice(&1u16.to_le_bytes()); // version 1
        dup_wire.extend_from_slice(&2u16.to_le_bytes()); // 2 fields
        // field 1, len 3, "foo"
        dup_wire.extend_from_slice(&1u16.to_le_bytes());
        dup_wire.extend_from_slice(&3u32.to_le_bytes());
        dup_wire.extend_from_slice(b"foo");
        // duplicate field 1, len 3, "bar"
        dup_wire.extend_from_slice(&1u16.to_le_bytes());
        dup_wire.extend_from_slice(&3u32.to_le_bytes());
        dup_wire.extend_from_slice(b"bar");

        let err_dup = ExtensibleMessage::decode(&dup_wire, 10);
        assert_eq!(
            err_dup,
            Err(SchemaHomomorphismError::DuplicateFieldId { field_id: 1 })
        );

        // 5. Reject overlapping field ID in try_new and try_encode
        let mut overlapping_ext = BTreeMap::new();
        overlapping_ext.insert(1u16, b"overlap".to_vec());
        let err_overlap = ExtensibleMessage::try_new(1, base, overlapping_ext);
        assert_eq!(
            err_overlap,
            Err(ExtensibleMessageError::OverlappingFieldId { field_id: 1 })
        );

        // 6. Verify Display & Error implementations
        let err_display = format!("{}", SchemaHomomorphismError::ZeroWireVersion);
        assert!(err_display.contains("zero"));
        let err_display2 = format!(
            "{}",
            SchemaHomomorphismError::TrailingGarbage {
                consumed: 10,
                total: 15
            }
        );
        assert!(err_display2.contains("10 were consumed"));
    }
}
