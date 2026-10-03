//! RFC-0327: Multi-Tenant Namespace Fence Verification Suite.
//!
//! Mechanically verifies strict homomorphic prefix isolation, range bounding,
//! and complete elimination of cross-tenant key leakage.

#![forbid(unsafe_code)]

use pedradb_core::multitenant_namespace_fence_kernel::{
    TenantId, TenantNamespaceError, TenantNamespaceFence,
};

#[test]
fn rfc0327_tenant_id_validation() {
    // 1. Rejects empty string
    assert_eq!(
        TenantId::try_new(""),
        Err(TenantNamespaceError::EmptyTenantIdentifier)
    );
    assert_eq!(
        TenantId::try_new("   "),
        Err(TenantNamespaceError::EmptyTenantIdentifier)
    );

    // 2. Rejects overly long tenant ID (> 256 bytes)
    let long_id = "a".repeat(257);
    assert_eq!(
        TenantId::try_new(long_id),
        Err(TenantNamespaceError::TenantIdTooLong { len: 257, max: 256 })
    );

    // 3. Valid tenant ID
    let t1 = TenantId::try_new("tenant_acme_corp").expect("valid tenant");
    assert_eq!(t1.as_str(), "tenant_acme_corp");
}

#[test]
fn rfc0327_tenant_key_encoding_and_cross_tenant_isolation() {
    let tenant_a = TenantId::try_new("tenant_alpha").unwrap();
    let tenant_b = TenantId::try_new("tenant_beta").unwrap();

    // 1. Rejects empty user key
    assert_eq!(
        TenantNamespaceFence::encode_key(&tenant_a, b""),
        Err(TenantNamespaceError::EmptyUserKey)
    );

    // 2. Encode keys for both tenants
    let key_a = TenantNamespaceFence::encode_key(&tenant_a, b"user:1001:profile").unwrap();
    let key_b = TenantNamespaceFence::encode_key(&tenant_b, b"user:1001:profile").unwrap();

    // Homomorphic separation: physical keys are distinct despite identical user keys
    assert_ne!(key_a, key_b);

    // 3. Decode valid key for tenant A
    let decoded_a = TenantNamespaceFence::decode_key(&tenant_a, &key_a).unwrap();
    assert_eq!(decoded_a, b"user:1001:profile");

    // 4. Cross-tenant access attempt: Tenant A attempts to decode Tenant B's physical key!
    let cross_err = TenantNamespaceFence::decode_key(&tenant_a, &key_b);
    assert_eq!(
        cross_err,
        Err(TenantNamespaceError::CrossTenantAccessViolation {
            expected_tenant: "tenant_alpha".to_owned(),
            observed_tenant: "tenant_beta".to_owned(),
        })
    );

    // 5. Scan bounds containment
    assert!(TenantNamespaceFence::is_in_bounds(&tenant_a, &key_a));
    assert!(!TenantNamespaceFence::is_in_bounds(&tenant_a, &key_b));

    let (start, end) = TenantNamespaceFence::scan_bounds(&tenant_a);
    assert!(key_a.as_slice() >= start.as_slice());
    assert!(key_a.as_slice() < end.as_slice());
}
