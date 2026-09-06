// Verus twin of `cqe_kernel` decisions (F203/F208, RFC-0156 P0.3).
// The ring itself (SQE layout, submit_and_wait, CQ ring) stays TCB —
// RFC-0074 P2.2 refuses a ring model; `cqe_ring_model_admitted` pins that.
//
//   ./scripts/verus_cqe_kernel.sh

use vstd::prelude::*;

verus! {

// ---- F203: unique, never-zero tags ----

pub open spec fn next_tag_spec(c: u64) -> u64 {
    if c != 0 { c } else { 1 }
}

/// Next SQE tag is the counter itself; 0 is reserved (skipped to 1).
pub fn next_user_data(c: u64) -> (tag: u64)
    ensures
        tag == next_tag_spec(c),
        tag != 0,
{
    if c != 0 { c } else { 1 }
}

proof fn lemma_f203_tags_never_zero(c: u64)
    ensures
        next_tag_spec(c) != 0,
{
}

/// Two draws from distinct non-zero counters never collide — a leftover
/// from another op is always distinguishable.
proof fn lemma_f203_distinct_states(a: u64, b: u64)
    requires
        a != b,
        a != 0,
        b != 0,
    ensures
        next_tag_spec(a) != next_tag_spec(b),
{
}

// ---- F203: leftover discrimination ----

pub enum CqeAct {
    Take,
    Discard,
}

pub open spec fn cqe_act_spec(user_data: u64, want: u64) -> bool {
    user_data == want
}

/// Only the CQE tagged with our `user_data` belongs to this op.
pub fn cqe_act(user_data: u64, want: u64) -> (d: CqeAct)
    ensures
        d === CqeAct::Take <==> cqe_act_spec(user_data, want),
{
    if user_data == want {
        CqeAct::Take
    } else {
        CqeAct::Discard
    }
}

/// Someone else's CQE is never adopted (no wrong res / false Ok).
proof fn lemma_f203_leftover_discarded(leftover: u64, want: u64)
    requires
        leftover != want,
    ensures
        !cqe_act_spec(leftover, want),
{
}

// ---- F208: a pushed SQE is in flight until its CQE is harvested ----

pub enum SubmitCompleteAct {
    UseHarvested,
    WaitMore,
    ReturnSubmitErr,
}

pub open spec fn submit_complete_act_spec(_submit_ok: bool, harvested: bool) -> SubmitCompleteAct {
    if harvested { SubmitCompleteAct::UseHarvested } else { SubmitCompleteAct::WaitMore }
}

pub open spec fn submit_complete_act_as_is_spec(submit_ok: bool) -> SubmitCompleteAct {
    if submit_ok { SubmitCompleteAct::WaitMore } else { SubmitCompleteAct::ReturnSubmitErr }
}

/// Test/Linux-soak telemetry for the F208 arm (kernel:
/// `F208_WAITMORE_AFTER_SUBMIT_ERR`); opaque to the proof — it observes,
/// it never decides.
#[verifier::external_body]
pub fn f208_bump_waitmore_after_submit_err() {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Production never returns submit Err after a push: harvest if the CQE is
/// already there, wait otherwise (the caller's buffer stays borrowed).
pub fn submit_complete_act(_submit_ok: bool, harvested: bool) -> (d: SubmitCompleteAct)
    ensures
        d == submit_complete_act_spec(_submit_ok, harvested),
        d === SubmitCompleteAct::UseHarvested <==> harvested,
        d === SubmitCompleteAct::WaitMore <==> !harvested,
{
    if harvested {
        SubmitCompleteAct::UseHarvested
    } else {
        if !_submit_ok {
            f208_bump_waitmore_after_submit_err();
        }
        SubmitCompleteAct::WaitMore
    }
}

/// I/O completed despite EINTR: submit Err + CQE in the CQ still uses the
/// harvested res; the as-is returns Err and never harvests.
proof fn lemma_eintr_harvest_diverges()
    ensures
        submit_complete_act_spec(false, true) === SubmitCompleteAct::UseHarvested,
        submit_complete_act_as_is_spec(false) === SubmitCompleteAct::ReturnSubmitErr,
{
}

/// F208: submit Err with an empty CQ waits again (SQE already pushed, the
/// kernel may still DMA into the caller's buffer); the as-is returns Err
/// and drops the buffer — the UAF shape.
proof fn lemma_f208_empty_cq_diverges()
    ensures
        submit_complete_act_spec(false, false) === SubmitCompleteAct::WaitMore,
        submit_complete_act_as_is_spec(false) === SubmitCompleteAct::ReturnSubmitErr,
{
}

// ---- R-uring refusal pin (RFC-0074 P2.2) ----

pub open spec fn cqe_ring_model_admitted_spec() -> bool {
    false
}

pub open spec fn cqe_ring_model_admitted_as_is_spec() -> bool {
    true
}

/// The res-gate twin is not a ring model. Always false.
pub fn cqe_ring_model_admitted() -> (d: bool)
    ensures
        d == cqe_ring_model_admitted_spec(),
        !d,
{
    false
}

pub fn cqe_ring_model_admitted_as_is() -> (d: bool)
    ensures
        d == cqe_ring_model_admitted_as_is_spec(),
        d,
{
    true
}

proof fn lemma_ring_refusal_diverges()
    ensures
        cqe_ring_model_admitted_spec() != cqe_ring_model_admitted_as_is_spec(),
{
}

} // verus!
