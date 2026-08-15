// Verus proof of F62 pack injectivity (RFC-0002 P25).
// Twin of `src/pack_kernel.rs` (Vec concat is caller).
//
//   ./scripts/verus_pack_inject.sh

use vstd::prelude::*;

verus! {

pub open spec fn pack_cut_sep() -> u8 {
    0x00
}

pub open spec fn pack_cut_tag_spec(len: u32) -> u32 {
    len
}

pub fn pack_cut_tag(len: u32) -> (n: u32)
    ensures
        n == pack_cut_tag_spec(len),
        n == len,
{
    len
}

pub open spec fn pack_cut_tag_as_is(_len: u32) -> u32 {
    0
}

proof fn lemma_as_is_collides(a: u32, b: u32)
    ensures
        pack_cut_tag_as_is(a) == pack_cut_tag_as_is(b),
        pack_cut_tag_as_is(a) == 0,
        pack_cut_sep() == 0x00,
{
}

proof fn lemma_fixed_injective(a: u32, b: u32)
    requires
        a != b,
    ensures
        pack_cut_tag_spec(a) != pack_cut_tag_spec(b),
{
}

} // verus!
