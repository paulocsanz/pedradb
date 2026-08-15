// Verus proof of install-snapshot key filters (RFC-0002 P14 / F38 / F40 / F41).
// Twin of `src/snapshot_kernel.rs`. Not linked into production.
//
//   ./scripts/verus_snapshot_kernel.sh

use vstd::prelude::*;

verus! {

pub fn snapshot_touches_user_key(is_reserved: bool) -> (d: bool)
    ensures
        d == !is_reserved,
{
    !is_reserved
}

pub open spec fn snapshot_touches_user_key_as_is(_is_reserved: bool) -> bool {
    true
}

proof fn lemma_as_is_exports_reserved()
    ensures
        snapshot_touches_user_key_as_is(true),
        !{ let reserved = true; !reserved },
{
}

pub fn snapshot_needs_txn_meta_clear() -> (d: bool)
    ensures
        d,
{
    true
}

pub open spec fn snapshot_needs_txn_meta_clear_as_is() -> bool {
    false
}

proof fn lemma_as_is_skips_txn_clear()
    ensures
        snapshot_needs_txn_meta_clear_as_is() == false,
{
}

} // verus!
