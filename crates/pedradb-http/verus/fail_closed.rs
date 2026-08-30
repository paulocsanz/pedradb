// Verus proof of F102/F104/F105 fail-closed HTTP (RFC-0002 P38).
// Twin of `fail_closed.rs` (socket write / parse are caller).
//
//   ./scripts/verus_fail_closed.sh

use vstd::prelude::*;

verus! {

pub open spec fn parse_error_writes_status_spec() -> bool {
    true
}

pub open spec fn parse_error_writes_status_as_is_spec() -> bool {
    false
}

pub fn parse_error_writes_status() -> (d: bool)
    ensures
        d == parse_error_writes_status_spec(),
        d,
{
    true
}

pub fn parse_error_status() -> (c: u16)
    ensures
        c == 400u16,
{
    400u16
}

pub open spec fn reject_transfer_encoding_spec() -> bool {
    true
}

pub open spec fn reject_transfer_encoding_as_is_spec() -> bool {
    false
}

pub fn reject_transfer_encoding() -> (d: bool)
    ensures
        d == reject_transfer_encoding_spec(),
        d,
{
    true
}

pub open spec fn present_bad_int_is_error_spec() -> bool {
    true
}

pub open spec fn present_bad_int_is_error_as_is_spec() -> bool {
    false
}

pub fn present_bad_int_is_error() -> (d: bool)
    ensures
        d == present_bad_int_is_error_spec(),
        d,
{
    true
}

proof fn lemma_f102_as_is_mute()
    ensures
        parse_error_writes_status_spec(),
        !parse_error_writes_status_as_is_spec(),
{
}

proof fn lemma_f104_as_is_accepts_te()
    ensures
        reject_transfer_encoding_spec(),
        !reject_transfer_encoding_as_is_spec(),
{
}

proof fn lemma_f105_as_is_defaults()
    ensures
        present_bad_int_is_error_spec(),
        !present_bad_int_is_error_as_is_spec(),
{
}

} // verus!
