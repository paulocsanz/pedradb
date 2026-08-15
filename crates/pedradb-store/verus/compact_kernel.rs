// Verus proof of compact membership + may_compact (RFC-0002 P13 / F27 / F28).
// Twin of `src/compact_kernel.rs`. Not linked into production.
//
//   ./scripts/verus_compact_kernel.sh

use vstd::prelude::*;

verus! {

pub fn peer_counts_for_compact(_is_participating: bool) -> (d: bool)
    ensures
        d,
{
    true
}

pub open spec fn peer_counts_for_compact_as_is(is_participating: bool) -> bool {
    is_participating
}

proof fn lemma_as_is_skips_offline()
    ensures
        peer_counts_for_compact_as_is(false) == false,
{
}

pub fn compact_ready(min_applied: u64) -> (d: bool)
    ensures
        d == (min_applied > 0),
{
    min_applied > 0
}

pub open spec fn may_compact_through_spec(snap: u64, through: u64, term: u64) -> bool {
    through > 0 && through > snap && term != 0
}

pub fn may_compact_through(snapshot_index: u64, through: u64, term_at_through: u64) -> (d: bool)
    ensures
        d == may_compact_through_spec(snapshot_index, through, term_at_through),
        d ==> term_at_through != 0 && through > snapshot_index,
{
    if through == 0 || through <= snapshot_index {
        false
    } else if term_at_through == 0 {
        false
    } else {
        true
    }
}

pub open spec fn may_compact_through_as_is(snap: u64, through: u64, _term: u64) -> bool {
    through > 0 && through > snap
}

proof fn lemma_as_is_compacts_missing_term(through: u64)
    requires
        through > 0,
    ensures
        !may_compact_through_spec(0, through, 0),
        may_compact_through_as_is(0, through, 0),
{
}

pub fn compact_index_floor(through: u64) -> (f: u64)
    ensures
        through < u64::MAX ==> f == through + 1,
        f >= 1 || through == 0,
{
    if through == u64::MAX {
        through
    } else {
        through + 1
    }
}

} // verus!
