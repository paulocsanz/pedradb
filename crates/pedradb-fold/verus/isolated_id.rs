// Verus proof of F83 isolated-id match (RFC-0002 P29).
// Twin of `src/isolated_kernel.rs` (slice starts_with is caller).
//
//   ./scripts/verus_isolated_id.sh

use vstd::prelude::*;

verus! {

pub open spec fn isolated_child_sep() -> u8 {
    0x2f
}

pub open spec fn isolated_child_byte_spec(next: u8) -> bool {
    next == isolated_child_sep()
}

pub fn isolated_child_byte(next: u8) -> (d: bool)
    ensures
        d == isolated_child_byte_spec(next),
        d == (next == 0x2f),
{
    next == 0x2f
}

pub open spec fn isolated_child_byte_as_is(_next: u8) -> bool {
    true
}

proof fn lemma_slash_is_child()
    ensures
        isolated_child_byte_spec(0x2f),
{
}

proof fn lemma_as_is_leaks_sibling_b()
    ensures
        !isolated_child_byte_spec(0x62),
        isolated_child_byte_as_is(0x62),
{
}

} // verus!
