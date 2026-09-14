//! Packed-children exclusive end (RFC-0002 P23 / F59).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). Vec concat is caller. No twin-cópia.
//!
//!   ./scripts/verus_children_range.sh
//!
//! Production [`crate::Subspace::range_end`] / [`crate::Subspace::children_range`]
//! / [`crate::Subspace::range_start`] call these. Tuple `pack` is caller.
//!
//! Recipe children are the next field after `0x00`. Exclusive end is
//! `packed || 0x01`, **not** `prefix_exclusive_end` (F57/F58) and **not**
//! `packed || 0xff` (F59: zip `900` sorts inside `[pack(90), pack(90)||0xff)`).

#![forbid(unsafe_code)]

/// Separator before a packed child component.
pub const PACKED_CHILD_SEP: u8 = 0x00;
/// Exclusive end byte of packed children (`packed || 0x01`).
pub const PACKED_CHILD_END: u8 = 0x01;
/// AS-IS F59 exclusive end byte (`packed || 0xff`).
pub const PACKED_CHILD_END_AS_IS: u8 = 0xff;

/// Inclusive start of packed children: `packed || 0x00`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn packed_children_start(packed: &[u8]) -> Vec<u8> {
    let mut s = packed.to_vec();
    s.push(PACKED_CHILD_SEP);
    s
}

/// AS-IS: start is the packed prefix with no separator.
#[must_use]
pub fn packed_children_start_as_is(packed: &[u8]) -> Vec<u8> {
    packed.to_vec()
}

/// Exclusive end of packed children: `packed || 0x01`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn packed_children_end(packed: &[u8]) -> Vec<u8> {
    let mut e = packed.to_vec();
    e.push(PACKED_CHILD_END);
    e
}

/// AS-IS F59: `packed || 0xff`. Includes prefix-sibling components
/// (`pack("90")||0xff` contains `pack("900")` under the old join).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn packed_children_end_as_is(packed: &[u8]) -> Vec<u8> {
    let mut e = packed.to_vec();
    e.push(PACKED_CHILD_END_AS_IS);
    e
}

/// Half-open membership `[start, end)` (bytewise).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn key_in_half_open(key: &[u8], start: &[u8], end: &[u8]) -> bool {
    key >= start && key < end
}

/// AS-IS: missing end is treated as included.
#[must_use]
pub fn key_in_half_open_as_is(key: &[u8], start: &[u8], end: &[u8]) -> bool {
    key >= start
}

/// After `packed`, the next byte is a child iff it is the `0x00` separator.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn next_byte_in_packed_children(next: u8) -> bool {
    next == PACKED_CHILD_SEP
}

/// AS-IS: any next byte `< 0xff` is inside `[pack, pack||0xff)`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn next_byte_in_packed_children_as_is(next: u8) -> bool {
    next < PACKED_CHILD_END_AS_IS
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
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

pub open spec fn next_byte_in_packed_children_as_is_spec(next: u8) -> bool {
    next < 0xff
}

pub fn next_byte_in_packed_children_as_is(next: u8) -> (d: bool)
    ensures
        d == next_byte_in_packed_children_as_is_spec(next),
        d == (next < 0xff),
{
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
        next_byte_in_packed_children_as_is_spec(0x30),
        packed_child_end() == 0x01,
        packed_child_end_as_is() == 0xff,
{
}

} // verus!

#[cfg(test)]
mod tests {
    use super::*;

    /// Old-style `pack("90") = zip\\0 90` (F59 witness, independent of F62).
    fn pack90() -> &'static [u8] {
        b"zip\x0090"
    }

    fn child_short() -> Vec<u8> {
        let mut k = pack90().to_vec();
        k.push(PACKED_CHILD_SEP);
        k.extend_from_slice(b"short");
        k
    }

    fn sibling_900() -> Vec<u8> {
        let mut k = pack90().to_vec();
        k.push(b'0');
        k.extend_from_slice(b"\x00long");
        k
    }

    #[test]
    fn fixed_keeps_child_drops_sibling() {
        let p = pack90();
        let start = packed_children_start(p);
        let end = packed_children_end(p);
        assert!(key_in_half_open(&child_short(), &start, &end));
        assert!(
            !key_in_half_open(&sibling_900(), &start, &end),
            "FIXED must drop zip 900 end={end:?}"
        );
    }

    #[test]
    fn as_is_leaks_sibling_900() {
        let p = pack90();
        let start = p.to_vec();
        let end = packed_children_end_as_is(p);
        assert!(
            key_in_half_open(&sibling_900(), &start, &end),
            "AS-IS pack||0xff must include zip 900"
        );
        assert_ne!(
            key_in_half_open(
                &sibling_900(),
                &packed_children_start(p),
                &packed_children_end(p)
            ),
            key_in_half_open(&sibling_900(), &start, &end)
        );
    }

    #[test]
    fn ff_user_id_is_a_child() {
        let p = pack90();
        let mut child = packed_children_start(p);
        child.push(0xff);
        child.push(b'z');
        assert!(key_in_half_open(
            &child,
            &packed_children_start(p),
            &packed_children_end(p)
        ));
    }

    #[test]
    fn raw_prefix_sibling_subspace() {
        let zip = b"zip";
        let zip_x = b"zipX";
        assert!(!key_in_half_open(
            zip_x,
            &packed_children_start(zip),
            &packed_children_end(zip)
        ));
        assert!(key_in_half_open(
            zip_x,
            zip,
            &packed_children_end_as_is(zip)
        ));
    }

    #[test]
    fn theorem_next_byte_domain() {
        let mut n = 0u32;
        for next in 0u8..=255 {
            let d = next_byte_in_packed_children(next);
            assert_eq!(d, next == 0);
            let as_is = next_byte_in_packed_children_as_is(next);
            assert_eq!(as_is, next < 0xff);
            if next > 0 && next < 0xff {
                assert!(as_is);
                assert!(!d);
            }
            n += 1;
        }
        assert_eq!(n, 256);
    }

    /// Catalog three-teeth plant. Direct `as_is_leaks_sibling_900` is **not** this tooth.
    #[test]
    fn packed_children_start_on_raw_parent_is_not_ok() {
        let p = pack90();
        assert_ne!(
            packed_children_start(p),
            packed_children_start_as_is(p),
            "AS-IS dente: start without 0x00 sep"
        );
        assert_eq!(packed_children_start(p).last().copied(), Some(PACKED_CHILD_SEP));
    }

    #[test]
    fn key_in_half_open_on_missing_end_is_not_ok() {
        let start = b"a";
        let end = b"c";
        let key = b"c";
        assert!(!key_in_half_open(key, start, end));
        assert!(
            key_in_half_open_as_is(key, start, end),
            "AS-IS dente: end included"
        );
    }

    #[test]
    fn packed_children_end_on_live_subspace_is_not_ok() {
        let p = pack90();
        let ss = crate::Subspace::new(p);
        let start = ss.range_start();
        let end = ss.range_end();
        assert_eq!(end, packed_children_end(p));
        assert_ne!(
            end,
            packed_children_end_as_is(p),
            "AS-IS dente: packed||0xff leaks zip 900"
        );
        assert!(key_in_half_open(&child_short(), &start, &end));
        assert!(
            !key_in_half_open(&sibling_900(), &start, &end),
            "live Subspace::range_end must drop sibling 900"
        );
    }
}
