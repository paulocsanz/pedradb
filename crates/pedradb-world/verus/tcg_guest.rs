// Verus twin of `src/tcg.rs::tcg_guest_admitted` (RFC-0079 P2.1).
// Not linked into production.
//
//   ./scripts/verus_tcg_guest.sh

use vstd::prelude::*;

verus! {

pub open spec fn tcg_guest_admitted_spec(guest_reachable: bool) -> bool {
    guest_reachable
}

pub open spec fn tcg_guest_admitted_as_is_spec(_guest_reachable: bool) -> bool {
    true
}

pub fn tcg_guest_admitted(guest_reachable: bool) -> (ok: bool)
    ensures
        ok == tcg_guest_admitted_spec(guest_reachable),
{
    guest_reachable
}

pub fn tcg_guest_admitted_as_is(_guest_reachable: bool) -> (ok: bool)
    ensures
        ok == tcg_guest_admitted_as_is_spec(_guest_reachable),
{
    true
}

proof fn lemma_native_world_is_not_tcg()
    ensures
        !tcg_guest_admitted_spec(false),
        tcg_guest_admitted_as_is_spec(false),
{
}

} // verus!
