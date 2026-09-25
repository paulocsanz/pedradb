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

/// Error during schema homomorphism decode or serialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaHomomorphismError {
    /// Incompatible wire format magic header.
    InvalidWireMagic,
    /// Truncated payload or unexpected EOF.
    UnexpectedEof { expected: usize, actual: usize },
    /// Unknown required field that cannot be safely skipped.
    UnsupportedCriticalField { field_id: u16 },
}

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

    /// Encodes this message into a canonical wire format.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&Self::MAGIC);
        out.extend_from_slice(&self.wire_version.to_le_bytes());

        let total_fields = (self.base_fields.len() + self.unknown_extensions.len()) as u16;
        out.extend_from_slice(&total_fields.to_le_bytes());

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

        out
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
                base_fields.insert(field_id, payload);
            } else {
                unknown_extensions.insert(field_id, payload);
            }
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
