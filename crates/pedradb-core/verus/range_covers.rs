// Verus proof of range-tombstone cover (RFC-0002 P18 / F30).
// Twin of `merge::range_tombstone_covers` (bytewise order modeled as u64).
//
//   ./scripts/verus_range_covers.sh

use vstd::prelude::*;

verus! {

pub open spec fn range_covers_spec(start: u64, end: u64, key: u64) -> bool {
    key >= start && key < end
}

pub fn range_tombstone_covers(start: u64, end: u64, key: u64) -> (d: bool)
    ensures
        d == range_covers_spec(start, end, key),
        d ==> key >= start && key < end,
{
    key >= start && key < end
}

pub open spec fn range_tombstone_covers_as_is(start: u64, _end: u64, key: u64) -> bool {
    key == start
}

proof fn lemma_as_is_misses_interior(start: u64, end: u64, key: u64)
    requires
        start < key,
        key < end,
    ensures
        range_covers_spec(start, end, key),
        !range_tombstone_covers_as_is(start, end, key),
{
}

proof fn lemma_end_exclusive(start: u64, end: u64)
    requires
        start < end,
    ensures
        !range_covers_spec(start, end, end),
{
}

} // verus!
