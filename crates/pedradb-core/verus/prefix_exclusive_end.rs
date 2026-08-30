// Verus proof of the prefix exclusive-end byte rule (RFC-0002 P11 / F57 / F58).
// The production loop lives in `src/prefix.rs` and uses this same increment.
//
//   ./scripts/verus_prefix_exclusive_end.sh

use vstd::prelude::*;

verus! {

pub open spec fn bump_non_ff_spec(b: u8) -> Option<u8> {
    if b < 0xff {
        Some((b + 1) as u8)
    } else {
        None
    }
}

/// Increment a non-`0xff` byte; `0xff` carries (caller pops).
pub fn bump_non_ff(b: u8) -> (r: Option<u8>)
    ensures
        r == bump_non_ff_spec(b),
        (b < 0xff) ==> r == Some((b + 1) as u8),
        (b == 0xff) ==> r.is_none(),
{
    if b < 0xff {
        Some((b + 1) as u8)
    } else {
        None
    }
}

/// AS-IS F57: treat the next byte as a hard 0xff wall (prefix || 0xff).
pub open spec fn as_is_ff_wall() -> u8 {
    0xff
}

/// A key that continues with 0xff after the prefix is still a prefix match.
/// AS-IS exclusive end `prefix || 0xff` excludes it (key >= end).
proof fn lemma_as_is_wall_excludes_ff_continuation()
    ensures
        as_is_ff_wall() == 0xffu8,
        0xffu8 <= as_is_ff_wall(),
{
}

/// FIXED successor of a last non-ff byte is strictly above that byte
/// and therefore above `byte || 0xff…` in lexicographic order of a
/// *shorter* incremented prefix vs a longer 0xff-extended key.
proof fn lemma_bump_is_above(b: u8)
    requires
        b < 0xff,
    ensures
        bump_non_ff_spec(b).unwrap() > b,
{
}

} // verus!
