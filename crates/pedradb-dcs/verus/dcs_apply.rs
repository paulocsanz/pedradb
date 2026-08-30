// Verus proof of DCS apply-advance (RFC-0002 P12 / F12 / F22).
// Twin of `src/apply_kernel.rs`. Not linked into production.
//
//   ./scripts/verus_dcs_apply.sh

use vstd::prelude::*;

verus! {

pub open spec fn dcs_apply_should_advance_spec(ok: bool, cas_failed: bool) -> bool {
    ok || cas_failed
}

pub fn dcs_apply_should_advance(ok: bool, cas_failed: bool) -> (d: bool)
    ensures
        d == dcs_apply_should_advance_spec(ok, cas_failed),
        d == (ok || cas_failed),
{
    ok || cas_failed
}

pub open spec fn dcs_apply_should_advance_as_is(ok: bool, _cas_failed: bool) -> bool {
    ok
}

proof fn lemma_as_is_freezes_cas()
    ensures
        dcs_apply_should_advance_spec(false, true),
        !dcs_apply_should_advance_as_is(false, true),
{
}

proof fn lemma_hard_fail_does_not_advance()
    ensures
        !dcs_apply_should_advance_spec(false, false),
{
}

} // verus!
