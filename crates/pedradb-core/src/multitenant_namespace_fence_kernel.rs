//! RFC-0327: Multi-Tenant Namespace Fence Kernel.
//!
//! Provides strict homomorphic tenant prefix isolation, preventing cross-tenant
//! data leakage, enforcing bounded range iteration, and rejecting malformed or
//! cross-tenant key access attempts across cloud multi-tenant deployments.

#![forbid(unsafe_code)]

use std::fmt;

/// Maximum allowed length for a tenant identifier string (256 bytes).
pub const MAX_TENANT_ID_LEN: usize = 256;

/// Errors arising from multi-tenant namespace fencing and prefix boundary checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TenantNamespaceError {
    /// Tenant identifier is empty or whitespace-only.
    EmptyTenantIdentifier,
    /// Tenant identifier exceeds the maximum allowed length.
    TenantIdTooLong { len: usize, max: usize },
    /// User key is empty.
    EmptyUserKey,
    /// Cross-tenant access attempt: physical key belongs to a different tenant.
    CrossTenantAccessViolation {
        expected_tenant: String,
        observed_tenant: String,
    },
    /// Key payload is shorter than the minimum physical tenant envelope.
    PhysicalKeyUnderflow { len: usize },
}

impl fmt::Display for TenantNamespaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTenantIdentifier => write!(f, "Tenant ID cannot be empty or whitespace-only"),
            Self::TenantIdTooLong { len, max } => {
                write!(f, "Tenant ID length {len} exceeds maximum allowed {max}")
            }
            Self::EmptyUserKey => write!(f, "User key cannot be empty in tenant namespace"),
            Self::CrossTenantAccessViolation { expected_tenant, observed_tenant } => {
                write!(
                    f,
                    "Security violation: cross-tenant access rejected (expected '{expected_tenant}', found '{observed_tenant}')"
                )
            }
            Self::PhysicalKeyUnderflow { len } => {
                write!(f, "Physical key underflow: length {len} is too short for tenant envelope")
            }
        }
    }
}

impl std::error::Error for TenantNamespaceError {}

/// Validated strong tenant identifier guaranteed non-empty and bounded.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TenantId(String);

impl TenantId {
    /// Attempts to construct a `TenantId`, rejecting empty or overly long strings.
    pub fn try_new(raw: impl Into<String>) -> Result<Self, TenantNamespaceError> {
        let s = raw.into();
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(TenantNamespaceError::EmptyTenantIdentifier);
        }
        if s.len() > MAX_TENANT_ID_LEN {
            return Err(TenantNamespaceError::TenantIdTooLong {
                len: s.len(),
                max: MAX_TENANT_ID_LEN,
            });
        }
        Ok(Self(s))
    }

    /// Access the underlying string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Static namespace fence enforcing homomorphic key isolation.
#[derive(Debug, Clone, Default)]
pub struct TenantNamespaceFence;

impl TenantNamespaceFence {
    /// Formats the raw prefix separator for a tenant: `__tenant__<id>__\x00`.
    #[must_use]
    pub fn prefix_bytes(tenant: &TenantId) -> Vec<u8> {
        let mut prefix = Vec::with_capacity(tenant.as_str().len() + 12);
        prefix.extend_from_slice(b"__t__:");
        prefix.extend_from_slice(tenant.as_str().as_bytes());
        prefix.extend_from_slice(b":\x00");
        prefix
    }

    /// Encodes a user key into its physical tenant-isolated representation.
    pub fn encode_key(
        tenant: &TenantId,
        user_key: &[u8],
    ) -> Result<Vec<u8>, TenantNamespaceError> {
        if user_key.is_empty() {
            return Err(TenantNamespaceError::EmptyUserKey);
        }
        let mut phys = Self::prefix_bytes(tenant);
        phys.extend_from_slice(user_key);
        Ok(phys)
    }

    /// Decodes a physical key, verifying that it belongs to `expected_tenant`.
    pub fn decode_key<'a>(
        tenant: &TenantId,
        physical_key: &'a [u8],
    ) -> Result<&'a [u8], TenantNamespaceError> {
        let prefix = Self::prefix_bytes(tenant);
        if physical_key.len() <= prefix.len() {
            return Err(TenantNamespaceError::PhysicalKeyUnderflow {
                len: physical_key.len(),
            });
        }
        if !physical_key.starts_with(&prefix) {
            // Attempt to diagnose observed tenant
            let observed = if physical_key.starts_with(b"__t__:") {
                let rest = &physical_key[6..];
                if let Some(pos) = rest.iter().position(|&b| b == b':') {
                    String::from_utf8_lossy(&rest[..pos]).into_owned()
                } else {
                    "corrupted_tenant_tag".to_owned()
                }
            } else {
                "global_unscoped_namespace".to_owned()
            };

            return Err(TenantNamespaceError::CrossTenantAccessViolation {
                expected_tenant: tenant.as_str().to_owned(),
                observed_tenant: observed,
            });
        }

        Ok(&physical_key[prefix.len()..])
    }

    /// Computes the closed-open range bounds `[start, end)` for scanning a tenant's keyspace.
    #[must_use]
    pub fn scan_bounds(tenant: &TenantId) -> (Vec<u8>, Vec<u8>) {
        let start = Self::prefix_bytes(tenant);
        let mut end = start.clone();
        // Increment the last byte of prefix to form an exclusive upper bound
        if let Some(last) = end.last_mut() {
            *last = last.saturating_add(1);
        }
        (start, end)
    }

    /// Verifies whether a physical key sits strictly within the tenant's bounded range.
    #[must_use]
    pub fn is_in_bounds(tenant: &TenantId, physical_key: &[u8]) -> bool {
        let prefix = Self::prefix_bytes(tenant);
        physical_key.starts_with(&prefix)
    }
}
