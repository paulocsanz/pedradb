// Verus proof of F80 exact index-value range (RFC-0002 P26).
// Twin of `src/index_val_kernel.rs` (Vec encode is caller).
//
//   ./scripts/verus_index_val.sh

use vstd::prelude::*;

verus! {

pub open spec fn value_len_tag_spec(len: u32) -> u32 {
    len
}

pub fn value_len_tag(len: u32) -> (n: u32)
    ensures
        n == value_len_tag_spec(len),
        n == len,
{
    len
}

pub open spec fn value_len_tag_as_is(_len: u32) -> u32 {
    0
}

/// Children of an exact-value prefix start at `p || 0x00`.
pub open spec fn exact_value_child_start_byte() -> u8 {
    0x00
}

/// Exclusive end is `p || 0x01` (not `p || 0xff`).
pub open spec fn exact_value_child_end_byte() -> u8 {
    0x01
}

proof fn lemma_as_is_collides(a: u32, b: u32)
    ensures
        value_len_tag_as_is(a) == value_len_tag_as_is(b),
        value_len_tag_as_is(a) == 0,
{
}

proof fn lemma_fixed_injective(a: u32, b: u32)
    requires
        a != b,
    ensures
        value_len_tag_spec(a) != value_len_tag_spec(b),
{
}

proof fn lemma_child_range_bytes()
    ensures
        exact_value_child_start_byte() == 0x00,
        exact_value_child_end_byte() == 0x01,
        exact_value_child_start_byte() < exact_value_child_end_byte(),
{
}

} // verus!
