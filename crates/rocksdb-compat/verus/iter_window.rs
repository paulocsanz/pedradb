// Verus twin of `src/iter_kernel.rs::iter_window_keep` (RFC-0151 P1).
//
//   ./scripts/verus_iter_window.sh

use vstd::prelude::*;

verus! {

pub open spec fn iter_window_keep_spec(snapshot_live: bool) -> bool {
    snapshot_live
}

pub open spec fn iter_window_keep_as_is_spec(_snapshot_live: bool) -> bool {
    true
}

pub fn iter_window_keep(snapshot_live: bool) -> (d: bool)
    ensures
        d == iter_window_keep_spec(snapshot_live),
{
    snapshot_live
}

pub fn iter_window_keep_as_is(_snapshot_live: bool) -> (d: bool)
    ensures
        d == true,
{
    true
}

proof fn lemma_as_is_emits_hidden()
    ensures
        !iter_window_keep_spec(false),
        iter_window_keep_as_is_spec(false),
{
}

} // verus!
