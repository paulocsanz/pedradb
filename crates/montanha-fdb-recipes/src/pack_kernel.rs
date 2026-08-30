//! Pack cut tag (RFC-0002 P25 / F62).
//!
//! Production [`crate::Subspace::pack`] / [`crate::Subspace::sub`] call
//! [`pack_cut_tag`]. Raw `0x00 || part` collides when a component embeds NUL.

#![forbid(unsafe_code)]

/// Separator before a length-prefixed component.
pub const PACK_CUT_SEP: u8 = 0x00;

/// Length recorded at the cut (FIXED: `|part|`).
#[must_use]
pub fn pack_cut_tag(len: u32) -> u32 {
    len
}

/// AS-IS F62: no length at the cut — `pack([a\\0b, c])` equals `pack([a, b\\0c])`.
#[must_use]
pub fn pack_cut_tag_as_is(_len: u32) -> u32 {
    0
}

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
