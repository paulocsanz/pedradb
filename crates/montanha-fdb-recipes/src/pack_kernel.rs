//! Pack cut tag (RFC-0002 P25 / F62).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). Vec concat is caller. No twin-cópia.
//!
//!   ./scripts/verus_pack_inject.sh
//!
//! Production [`crate::Subspace::pack`] / [`crate::Subspace::sub`] call
//! [`pack_cut_tag`]. Raw `0x00 || part` collides when a component embeds NUL.

#![forbid(unsafe_code)]

/// Separator before a length-prefixed component.
pub const PACK_CUT_SEP: u8 = 0x00;

macro_rules! pack_cut_tag_body {
    ($len:expr) => {
        $len
    };
}

macro_rules! pack_cut_tag_as_is_body {
    ($len:expr) => {{
        let _ = $len;
        0u32
    }};
}

/// Length recorded at the cut (FIXED: `|part|`).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn pack_cut_tag(len: u32) -> u32 {
    pack_cut_tag_body!(len)
}

/// AS-IS F62: no length at the cut — `pack([a\\0b, c])` equals `pack([a, b\\0c])`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn pack_cut_tag_as_is(len: u32) -> u32 {
    pack_cut_tag_as_is_body!(len)
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
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
    pack_cut_tag_body!(len)
}

pub open spec fn pack_cut_tag_as_is_spec(_len: u32) -> u32 {
    0
}

pub fn pack_cut_tag_as_is(len: u32) -> (n: u32)
    ensures
        n == 0,
        n == pack_cut_tag_as_is_spec(len),
{
    pack_cut_tag_as_is_body!(len)
}

proof fn lemma_as_is_collides(a: u32, b: u32)
    ensures
        pack_cut_tag_as_is_spec(a) == pack_cut_tag_as_is_spec(b),
        pack_cut_tag_as_is_spec(a) == 0,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_is_injective() {
        assert_eq!(pack_cut_tag(3), 3);
        assert_eq!(pack_cut_tag_as_is(3), 0);
        assert_ne!(pack_cut_tag(3), pack_cut_tag(7));
        assert_eq!(pack_cut_tag_as_is(3), pack_cut_tag_as_is(7));
    }

    /// Catalog three-teeth plant. Direct `pack_is_injective_when_components_contain_nul` is **not** this tooth.
    #[test]
    fn pack_cut_tag_on_live_subspace_is_not_ok() {
        assert_eq!(pack_cut_tag(3), 3);
        assert_eq!(
            pack_cut_tag_as_is(3),
            0,
            "AS-IS dente: no length at the cut"
        );
        let s = crate::Subspace::new(b"t");
        let a = s.pack(&[b"a\x00b", b"c"]);
        let b = s.pack(&[b"a", b"b\x00c"]);
        assert_ne!(
            a, b,
            "live Subspace::pack must not collide [a\\0b,c] vs [a,b\\0c]; AS-IS cut tag 0 would"
        );
    }
}
