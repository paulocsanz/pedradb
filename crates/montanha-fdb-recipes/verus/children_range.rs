// Verus proof of packed-children exclusive end (RFC-0002 P23 / F59).
// Twin of `src/children_kernel.rs` (Vec concat is caller).
//
//   ./scripts/verus_children_range.sh

use vstd::prelude::*;

verus! {

pub open spec fn packed_child_sep() -> u8 {
    0x00
}

pub open spec fn packed_child_end() -> u8 {
    0x01
}

pub open spec fn packed_child_end_as_is() -> u8 {
    0xff
}

pub open spec fn next_byte_in_packed_children_spec(next: u8) -> bool {
    next == packed_child_sep()
}

pub fn next_byte_in_packed_children(next: u8) -> (d: bool)
    ensures
        d == next_byte_in_packed_children_spec(next),
        d == (next == 0x00),
{
    next == 0x00
}

pub open spec fn next_byte_in_packed_children_as_is(next: u8) -> bool {
    next < 0xff
}

proof fn lemma_sep_is_child()
    ensures
        next_byte_in_packed_children_spec(0x00),
{
}

proof fn lemma_as_is_leaks_zero_char()
    ensures
        !next_byte_in_packed_children_spec(0x30),
        next_byte_in_packed_children_as_is(0x30),
        packed_child_end() == 0x01,
        packed_child_end_as_is() == 0xff,
{
}

} // verus!
