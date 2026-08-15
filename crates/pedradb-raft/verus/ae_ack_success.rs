// Verus proof of AE persist-before-success (RFC-0002 P5.2 / F48).
//
// Same rule as `src/ae_kernel.rs::ae_ack_success` and
// `pedradb-store::ae_ack_success`. Not linked into production.
//
//   ./scripts/verus_ae_ack_success.sh

use vstd::prelude::*;

verus! {

pub open spec fn ae_ack_success_spec(log_dirty: bool, persist_ok: bool) -> bool {
    !log_dirty || persist_ok
}

pub open spec fn ae_ack_success_as_is(_log_dirty: bool, _persist_ok: bool) -> bool {
    true
}

pub fn ae_ack_success(log_dirty: bool, persist_ok: bool) -> (d: bool)
    ensures
        d == ae_ack_success_spec(log_dirty, persist_ok),
        d ==> !log_dirty || persist_ok,
        (log_dirty && !persist_ok) ==> !d,
{
    !log_dirty || persist_ok
}

proof fn lemma_mutant_swallows_persist_fail()
    ensures
        ae_ack_success_as_is(true, false),
        !ae_ack_success_spec(true, false),
{
}

} // verus!
