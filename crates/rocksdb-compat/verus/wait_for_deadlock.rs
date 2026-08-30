// Verus twin of `src/locktab.rs::wait_for_deadlock` (RFC-0150 P2c).
// Wait-for cycle ⇒ Deadlock. AS-IS misses the cycle.
// Model: owner_waits_for_waiter is the 2-cycle; longer chains use the
// production walk (tests).
//
//   ./scripts/verus_wait_for_deadlock.sh
//
// Do not link this into the production crate.

use vstd::prelude::*;

verus! {

pub open spec fn wait_for_deadlock_spec(waiter: u64, owner: u64, owner_next: Option<u64>) -> bool {
    match owner_next {
        Some(next) => next == waiter,
        None => false,
    }
}

/// Production walk returns `true` on a cycle (two-cycle or longer).
pub open spec fn wait_for_deadlock_found() -> bool {
    true
}

pub open spec fn wait_for_deadlock_as_is_spec(
    _waiter: u64,
    _owner: u64,
    _owner_next: Option<u64>,
) -> bool {
    false
}

pub fn wait_for_deadlock(waiter: u64, owner: u64, owner_next: Option<u64>) -> (d: bool)
    ensures
        d == wait_for_deadlock_spec(waiter, owner, owner_next),
        d ==> owner_next == Some(waiter),
{
    match owner_next {
        Some(next) => next == waiter,
        None => false,
    }
}

pub fn wait_for_deadlock_as_is(_waiter: u64, _owner: u64, _owner_next: Option<u64>) -> (d: bool)
    ensures
        d == false,
{
    false
}

proof fn lemma_two_cycle_is_deadlock(a: u64, b: u64)
    requires
        a != b,
    ensures
        wait_for_deadlock_spec(a, b, Some(a)),
        !wait_for_deadlock_as_is_spec(a, b, Some(a)),
        !wait_for_deadlock_spec(a, b, None),
{
}

} // verus!
