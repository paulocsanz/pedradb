//! Multi-Tenant Prefix Isolation and Non-Interference Kernel (RFC-0285 Pilar 5).
//!
//! Guarantees that multi-tenant composite keys have strict prefix-free, injective,
//! and non-interfering encodings such that range/prefix scans for tenant A can never
//! leak or observe keys belonging to tenant B, even in the presence of binary zeroes (0x00).
//!
//! Axiom:
//! For all tenants T_A != T_B and all user keys K_1, K_2:
//! PrefixFreeKey(T_A, K_1) is NOT a prefix of PrefixFreeKey(T_B, K_2).

#![forbid(unsafe_code)]

/// Error occurring during multi-tenant prefix encoding or decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TenantPrefixError {
    /// Tenant identifier exceeds maximum allowable length (255 bytes).
    TenantIdTooLong { len: usize, max: usize },
    /// Encoded payload is malformed or truncated.
    MalformedEncoding { expected: usize, actual: usize },
    /// Tenant identifier mismatch during decoding.
    TenantMismatch { expected: String, actual: String },
}

/// Encoder and validator for isolated multi-tenant prefix namespaces.
pub struct TenantPrefixCodec;

impl TenantPrefixCodec {
    /// Maximum allowable length for a tenant ID in bytes.
    pub const MAX_TENANT_ID_LEN: usize = 255;

    /// Encodes a tenant-scoped key into a canonical length-prefixed format:
    /// `[len(tenant_id): u8] || [tenant_id: bytes] || [user_key: bytes]`
    ///
    /// This encoding is strictly injective and prefix-free across distinct tenants.
    pub fn encode_key(tenant_id: &str, user_key: &[u8]) -> Result<Vec<u8>, TenantPrefixError> {
        let t_bytes = tenant_id.as_bytes();
        if t_bytes.len() > Self::MAX_TENANT_ID_LEN {
            return Err(TenantPrefixError::TenantIdTooLong {
                len: t_bytes.len(),
                max: Self::MAX_TENANT_ID_LEN,
            });
        }

        let mut out = Vec::with_capacity(1 + t_bytes.len() + user_key.len());
        out.push(t_bytes.len() as u8);
        out.extend_from_slice(t_bytes);
        out.extend_from_slice(user_key);
        Ok(out)
    }

    /// Derives the strict lower bound (inclusive) and upper bound (exclusive)
    /// for range scanning all keys belonging to `tenant_id`.
    ///
    /// Upper bound increments the tenant prefix lexicographically without leaking
    /// into subsequent tenants.
    pub fn tenant_scan_bounds(tenant_id: &str) -> Result<(Vec<u8>, Vec<u8>), TenantPrefixError> {
        let lower = Self::encode_key(tenant_id, &[])?;
        let mut upper = lower.clone();

        // Increment the last byte of the tenant prefix
        let mut idx = upper.len();
        let mut carry = true;
        while idx > 0 && carry {
            idx -= 1;
            if upper[idx] == 0xFF {
                upper[idx] = 0x00;
            } else {
                upper[idx] += 1;
                carry = false;
            }
        }

        if carry {
            // Edge case: overflow across all bytes
            upper.push(0x00);
        }

        Ok((lower, upper))
    }

    /// Decodes a tenant-scoped key, verifying tenant identity and returning the raw user key.
    pub fn decode_key<'a>(
        expected_tenant: &str,
        encoded: &'a [u8],
    ) -> Result<&'a [u8], TenantPrefixError> {
        if encoded.is_empty() {
            return Err(TenantPrefixError::MalformedEncoding {
                expected: 1,
                actual: 0,
            });
        }
        let t_len = encoded[0] as usize;
        if encoded.len() < 1 + t_len {
            return Err(TenantPrefixError::MalformedEncoding {
                expected: 1 + t_len,
                actual: encoded.len(),
            });
        }

        let tenant_part = &encoded[1..1 + t_len];
        if tenant_part != expected_tenant.as_bytes() {
            let actual_str = String::from_utf8_lossy(tenant_part).to_string();
            return Err(TenantPrefixError::TenantMismatch {
                expected: expected_tenant.to_string(),
                actual: actual_str,
            });
        }

        Ok(&encoded[1 + t_len..])
    }

    /// Verifies non-interference between two tenant namespaces.
    /// Proves that no key belonging to tenant A can be a prefix of any key belonging to tenant B.
    pub fn verify_non_interference(
        tenant_a: &str,
        key_a: &[u8],
        tenant_b: &str,
        key_b: &[u8],
    ) -> bool {
        if tenant_a == tenant_b {
            return true; // Self-consistency
        }
        let enc_a = Self::encode_key(tenant_a, key_a).expect("valid tenant_a");
        let enc_b = Self::encode_key(tenant_b, key_b).expect("valid tenant_b");

        // Non-interference: neither is a prefix of the other unless tenant IDs match
        !enc_a.starts_with(&enc_b) && !enc_b.starts_with(&enc_a)
    }
}
