//! Pure install-snapshot key decisions (RFC-0002 P14 / F38 / F40 / F41).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). No twin-cópia.
//!
//!   ./scripts/verus_snapshot_kernel.sh
//!
//! Production [`crate::StoreCluster::export_range_kv`] /
//! [`crate::StoreCluster::on_install_snapshot`] call these helpers.
//! Persist and range scan are **axioms**.

#![forbid(unsafe_code)]

macro_rules! snapshot_touches_user_key_body {
    ($is_reserved:expr) => {
        !$is_reserved
    };
}

macro_rules! snapshot_touches_user_key_as_is_body {
    ($is_reserved:expr) => {{
        let _ = $is_reserved;
        true
    }};
}

macro_rules! snapshot_needs_txn_meta_clear_body {
    () => {
        true
    };
}

macro_rules! snapshot_needs_txn_meta_clear_as_is_body {
    () => {
        false
    };
}

/// F38/F41: snapshot export / wipe / payload apply only **user** keys.
///
/// Reserved `\0store/*` (raft meta, intents, SI) is per-node or out-of-range.
/// Shipping or wiping it is F41 (leader meta) / F38-overshoot.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn snapshot_touches_user_key(is_reserved: bool) -> bool {
    snapshot_touches_user_key_body!(is_reserved)
}

/// AS-IS F41: treat reserved keys like user keys (merge/export `\0store/*`).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn snapshot_touches_user_key_as_is(is_reserved: bool) -> bool {
    snapshot_touches_user_key_as_is_body!(is_reserved)
}

/// F40: after replacing user keys, always clear intent/txn meta for the range.
///
/// Intents live under `\0store/intent/…`, outside the user keyspace, so the
/// F38 wipe does not see them.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn snapshot_needs_txn_meta_clear() -> bool {
    snapshot_needs_txn_meta_clear_body!()
}

/// AS-IS F40: skip txn-meta clear (orphan intents survive catch-up).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn snapshot_needs_txn_meta_clear_as_is() -> bool {
    snapshot_needs_txn_meta_clear_as_is_body!()
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn snapshot_touches_user_key_spec(is_reserved: bool) -> bool {
    !is_reserved
}

pub fn snapshot_touches_user_key(is_reserved: bool) -> (d: bool)
    ensures
        d == !is_reserved,
        d == snapshot_touches_user_key_spec(is_reserved),
{
    snapshot_touches_user_key_body!(is_reserved)
}

pub open spec fn snapshot_touches_user_key_as_is_spec(_is_reserved: bool) -> bool {
    true
}

pub fn snapshot_touches_user_key_as_is(is_reserved: bool) -> (d: bool)
    ensures
        d == true,
        d == snapshot_touches_user_key_as_is_spec(is_reserved),
{
    snapshot_touches_user_key_as_is_body!(is_reserved)
}

proof fn lemma_as_is_exports_reserved()
    ensures
        snapshot_touches_user_key_as_is_spec(true),
        !snapshot_touches_user_key_spec(true),
{
}

pub fn snapshot_needs_txn_meta_clear() -> (d: bool)
    ensures
        d,
{
    snapshot_needs_txn_meta_clear_body!()
}

pub open spec fn snapshot_needs_txn_meta_clear_as_is_spec() -> bool {
    false
}

pub fn snapshot_needs_txn_meta_clear_as_is() -> (d: bool)
    ensures
        d == false,
        d == snapshot_needs_txn_meta_clear_as_is_spec(),
{
    snapshot_needs_txn_meta_clear_as_is_body!()
}

proof fn lemma_as_is_skips_txn_clear()
    ensures
        snapshot_needs_txn_meta_clear_as_is_spec() == false,
{
}

} // verus!

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
