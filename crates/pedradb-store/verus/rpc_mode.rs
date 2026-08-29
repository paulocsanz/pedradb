// Verus twin of `src/rpc_mode_kernel.rs::allow_direct_rpc` (RFC-0067 P2.1).
// Not linked into production.
//
//   ./scripts/verus_rpc_mode.sh

use vstd::prelude::*;

verus! {

pub open spec fn allow_direct_rpc_spec(dst_pin: bool, want_direct: bool) -> bool {
    !(want_direct && dst_pin)
}

/// AS-IS: pin does not stick — Direct is always admitted.
pub open spec fn allow_direct_rpc_as_is_spec(_dst_pin: bool, _want_direct: bool) -> bool {
    true
}

pub fn allow_direct_rpc(dst_pin: bool, want_direct: bool) -> (ok: bool)
    ensures
        ok == allow_direct_rpc_spec(dst_pin, want_direct),
{
    if want_direct && dst_pin {
        false
    } else {
        true
    }
}

pub fn allow_direct_rpc_as_is(_dst_pin: bool, _want_direct: bool) -> (ok: bool)
    ensures
        ok == allow_direct_rpc_as_is_spec(_dst_pin, _want_direct),
{
    true
}

proof fn lemma_pin_refuses_direct()
    ensures
        !allow_direct_rpc_spec(true, true),
        allow_direct_rpc_as_is_spec(true, true),
{
}

fn main() {}

} // verus!
