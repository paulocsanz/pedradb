//! Pure install-snapshot key decisions (RFC-0002 P14 / F38 / F40 / F41).
//!
//! Production [`crate::StoreCluster::export_range_kv`] /
//! [`crate::StoreCluster::on_install_snapshot`] call these helpers.
//! Persist and range scan are **axioms**.

#![forbid(unsafe_code)]

/// F38/F41: snapshot export / wipe / payload apply only **user** keys.
///
/// Reserved `\0store/*` (raft meta, intents, SI) is per-node or out-of-range.
/// Shipping or wiping it is F41 (leader meta) / F38-overshoot.
#[must_use]
pub fn snapshot_touches_user_key(is_reserved: bool) -> bool {
    !is_reserved
}

/// AS-IS F41: treat reserved keys like user keys (merge/export `\0store/*`).
#[must_use]
pub fn snapshot_touches_user_key_as_is(_is_reserved: bool) -> bool {
    true
}

/// F40: after replacing user keys, always clear intent/txn meta for the range.
///
/// Intents live under `\0store/intent/…`, outside the user keyspace, so the
/// F38 wipe does not see them.
#[must_use]
pub fn snapshot_needs_txn_meta_clear() -> bool {
    true
}

/// AS-IS F40: skip txn-meta clear (orphan intents survive catch-up).
#[must_use]
pub fn snapshot_needs_txn_meta_clear_as_is() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_not_touched() {
        assert!(!snapshot_touches_user_key(true));
        assert!(snapshot_touches_user_key(false));
    }

    #[test]
    fn as_is_exports_reserved() {
        assert!(snapshot_touches_user_key_as_is(true));
        assert_ne!(
            snapshot_touches_user_key(true),
            snapshot_touches_user_key_as_is(true)
        );
    }

    #[test]
    fn always_clear_txn_meta() {
        assert!(snapshot_needs_txn_meta_clear());
        assert!(!snapshot_needs_txn_meta_clear_as_is());
    }

    #[test]
    fn theorem_on_bool_domain() {
        for reserved in [false, true] {
            assert_eq!(snapshot_touches_user_key(reserved), !reserved);
            assert!(snapshot_touches_user_key_as_is(reserved));
        }
        assert!(snapshot_needs_txn_meta_clear());
        assert!(!snapshot_needs_txn_meta_clear_as_is());
    }
}
