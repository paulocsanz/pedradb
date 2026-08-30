// Verus proof of F85 Bearer scheme case-fold (RFC-0002 P31).
// Twin of `auth_kernel::ascii_lower` (string eq_ignore_ascii_case is caller).
//
//   ./scripts/verus_bearer_scheme.sh

use vstd::prelude::*;

verus! {

pub open spec fn ascii_lower_spec(b: u8) -> u8 {
    if b >= 65u8 && b <= 90u8 {
        (b + 32) as u8
    } else {
        b
    }
}

/// ASCII `A`–`Z` fold to `a`–`z`.
pub fn ascii_lower(b: u8) -> (r: u8)
    ensures
        r == ascii_lower_spec(b),
        (b >= 65u8 && b <= 90u8) ==> r == (b + 32) as u8,
{
    if b >= 65u8 && b <= 90u8 {
        (b + 32) as u8
    } else {
        b
    }
}

pub open spec fn ascii_eq_ignore_case_spec(a: u8, b: u8) -> bool {
    ascii_lower_spec(a) == ascii_lower_spec(b)
}

pub fn ascii_eq_ignore_case(a: u8, b: u8) -> (d: bool)
    ensures
        d == ascii_eq_ignore_case_spec(a, b),
{
    ascii_lower(a) == ascii_lower(b)
}

/// AS-IS F85: exact byte match (`Bearer`/`bearer` only).
pub open spec fn ascii_eq_as_is_spec(a: u8, b: u8) -> bool {
    a == b
}

/// The REAL: `'B'` (66) equals `'b'` (98) after fold, not as-is.
proof fn lemma_as_is_misses_upper_b()
    ensures
        ascii_eq_ignore_case_spec(66u8, 98u8),
        !ascii_eq_as_is_spec(66u8, 98u8),
{
}

pub open spec fn ascii_upper_spec(b: u8) -> u8 {
    if b >= 97u8 && b <= 122u8 {
        (b - 32) as u8
    } else {
        b
    }
}

/// ASCII `a`–`z` fold to `A`–`Z` (F79 methods).
pub fn ascii_upper(b: u8) -> (r: u8)
    ensures
        r == ascii_upper_spec(b),
        (b >= 97u8 && b <= 122u8) ==> r == (b - 32) as u8,
{
    if b >= 97u8 && b <= 122u8 {
        (b - 32) as u8
    } else {
        b
    }
}

/// The REAL F79: `'p'` (112) equals `'P'` (80) after fold, not as-is.
proof fn lemma_as_is_misses_lower_p()
    ensures
        ascii_eq_ignore_case_spec(80u8, 112u8),
        !ascii_eq_as_is_spec(80u8, 112u8),
        ascii_upper_spec(112u8) == 80u8,
{
}

} // verus!
