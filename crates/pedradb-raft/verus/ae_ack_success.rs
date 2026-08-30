// Verus proof of AE persist-before-success (RFC-0002 P5.2 / F48;
// RFC-0053 Y2 caller refinement — mirrors vote-side `grant_after_persist`).
//
// Same rule as `src/ae_kernel.rs::ae_ack_success` and
// `pedradb-store::ae_ack_success`. Production `handle_append_entries`
// calls the kernel before sending `success: true`.
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

#[verifier::when_used_as_spec(ae_ack_success_spec)]
pub fn ae_ack_success(log_dirty: bool, persist_ok: bool) -> (d: bool)
    ensures
        d == ae_ack_success_spec(log_dirty, persist_ok),
        d ==> !log_dirty || persist_ok,
        (log_dirty && !persist_ok) ==> !d,
{
    !log_dirty || persist_ok
}

/// Y2 caller refinement (named theorem): a `success: true` reply for a dirty
/// log implies the persist returned Ok — the AE twin of vote-side
/// `grant_after_persist`'s `g ==> persist == Ok`.
proof fn lemma_success_reply_only_after_persist(log_dirty: bool, persist_ok: bool)
    ensures
        ae_ack_success(log_dirty, persist_ok) && log_dirty ==> persist_ok,
        !ae_ack_success(log_dirty, false) || !log_dirty,
{
}

proof fn lemma_mutant_swallows_persist_fail()
    ensures
        ae_ack_success_as_is(true, false),
        !ae_ack_success_spec(true, false),
{
}

} // verus!
