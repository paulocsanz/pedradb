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

} // verus!
