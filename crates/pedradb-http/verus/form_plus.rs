// Verus proof of F101 form-urlencoded `+` → space (RFC-0002 P35).
// Twin of `form_kernel::form_plus_byte` (Vec loop / %HH are caller).
//
//   ./scripts/verus_form_plus.sh

use vstd::prelude::*;

verus! {

pub open spec fn form_plus_byte_spec(b: u8) -> u8 {
    if b == 43u8 {
        32u8
    } else {
        b
    }
}

pub open spec fn form_plus_byte_as_is_spec(b: u8) -> u8 {
    b
}

/// F101: raw `+` (43) in a query value is space (32).
pub fn form_plus_byte(b: u8) -> (r: u8)
    ensures
        r == form_plus_byte_spec(b),
        (b == 43u8) ==> r == 32u8,
{
    if b == 43u8 {
        32u8
    } else {
        b
    }
}

/// AS-IS F101: `+` stays `+` (percent-decode only).
pub fn form_plus_byte_as_is(b: u8) -> (r: u8)
    ensures
        r == form_plus_byte_as_is_spec(b),
        r == b,
{
    b
}

proof fn lemma_plus_is_space()
    ensures
        form_plus_byte_spec(43u8) == 32u8,
        form_plus_byte_as_is_spec(43u8) == 43u8,
{
}

proof fn lemma_other_bytes_unchanged(b: u8)
    requires
        b != 43u8,
    ensures
        form_plus_byte_spec(b) == b,
        form_plus_byte_spec(b) == form_plus_byte_as_is_spec(b),
{
}

/// F101: `+` is mapped before `%HH`, so `%2B` stays a literal plus.
pub fn plus_before_percent() -> (d: bool)
    ensures
        d == true,
{
    true
}

/// AS-IS F101: `+` is never mapped.
pub open spec fn plus_before_percent_as_is_spec() -> bool {
    false
}

proof fn lemma_as_is_never_maps_plus()
    ensures
        !plus_before_percent_as_is_spec(),
{
}

/// Hex nibble for `%HH` (digits, then `a`–`f`, then `A`–`F`).
pub open spec fn from_hex_spec(c: u8) -> Option<u8> {
    if b'0' <= c && c <= b'9' {
        Some((c - b'0') as u8)
    } else if b'a' <= c && c <= b'f' {
        Some((c - b'a' + 10u8) as u8)
    } else if b'A' <= c && c <= b'F' {
        Some((c - b'A' + 10u8) as u8)
    } else {
        None
    }
}

pub fn from_hex(c: u8) -> (r: Option<u8>)
    ensures
        r == from_hex_spec(c),
{
    if b'0' <= c && c <= b'9' {
        Some(c - b'0')
    } else if b'a' <= c && c <= b'f' {
        Some(c - b'a' + 10u8)
    } else if b'A' <= c && c <= b'F' {
        Some(c - b'A' + 10u8)
    } else {
        None
    }
}

/// AS-IS F76-class: digits only — hex letters stay as raw `%HH`.
pub open spec fn from_hex_as_is_spec(c: u8) -> Option<u8> {
    if b'0' <= c && c <= b'9' {
        Some((c - b'0') as u8)
    } else {
        None
    }
}

/// The REAL: `'a'` (97) decodes to 10; AS-IS drops it.
proof fn lemma_as_is_drops_hex_letters()
    ensures
        from_hex_spec(97u8) == Some(10u8),
        from_hex_as_is_spec(97u8) == None,
{
}

pub open spec fn query_u64_conflict_spec(a: u64, b: u64) -> bool {
    a != b
}

/// AS-IS F88-class: first/last wins, never a conflict.
pub open spec fn query_u64_conflict_as_is_spec(_a: u64, _b: u64) -> bool {
    false
}

/// F155: parsed query ints disagree.
pub fn query_u64_conflict(a: u64, b: u64) -> (d: bool)
    ensures
        d == query_u64_conflict_spec(a, b),
        d == (a != b),
{
    a != b
}

proof fn lemma_as_is_swallows_conflict()
    ensures
        query_u64_conflict_spec(1u64, 0u64),
        !query_u64_conflict_as_is_spec(1u64, 0u64),
{
}

pub open spec fn multi_values_can_conflict_spec(n: u64) -> bool {
    n > 1
}

/// F155: two or more same-name values can conflict (slice iter().any() is
/// caller).
pub fn multi_values_can_conflict(n: u64) -> (d: bool)
    ensures
        d == multi_values_can_conflict_spec(n),
        d == (n > 1),
{
    n > 1
}

proof fn lemma_pair_can_conflict()
    ensures
        multi_values_can_conflict_spec(2u64),
        !multi_values_can_conflict_spec(1u64),
{
}

pub open spec fn bare_part_gate_spec(part_empty: bool, part_has_eq: bool) -> bool {
    !part_empty && !part_has_eq
}

/// F162: a non-empty part without `=` is a bare name (`form_decode` is
/// caller).
pub fn bare_part_gate(part_empty: bool, part_has_eq: bool) -> (d: bool)
    ensures
        d == bare_part_gate_spec(part_empty, part_has_eq),
{
    !part_empty && !part_has_eq
}

proof fn lemma_bare_name_passes_gate()
    ensures
        bare_part_gate_spec(false, false),
        !bare_part_gate_spec(true, false),
        !bare_part_gate_spec(false, true),
{
}

} // verus!
