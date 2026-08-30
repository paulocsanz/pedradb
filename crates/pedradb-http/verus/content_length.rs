// Verus proof of F86/F87/F88 Content-Length framing (RFC-0002 P32).
// Twin of `cl_kernel` (parse/socket are caller).
//
//   ./scripts/verus_content_length.sh

use vstd::prelude::*;

verus! {

pub open spec fn keep_body_without_cl_spec() -> bool {
    true
}

pub open spec fn keep_body_without_cl_as_is_spec() -> bool {
    false
}

/// F86: missing CL keeps the already-read body.
pub fn keep_body_without_cl() -> (d: bool)
    ensures
        d == keep_body_without_cl_spec(),
        d,
{
    true
}

pub open spec fn invalid_cl_as_zero_spec() -> bool {
    false
}

pub open spec fn invalid_cl_as_zero_as_is_spec() -> bool {
    true
}

/// F87: unparseable CL is not treated as 0.
pub fn invalid_cl_as_zero() -> (d: bool)
    ensures
        d == invalid_cl_as_zero_spec(),
        !d,
{
    false
}

pub open spec fn content_length_repeat_ok_spec(first: u64, next: u64) -> bool {
    first == next
}

pub open spec fn content_length_repeat_ok_as_is_spec(_first: u64, _next: u64) -> bool {
    true
}

/// F88: a second CL must equal the first.
pub fn content_length_repeat_ok(first: u64, next: u64) -> (d: bool)
    ensures
        d == content_length_repeat_ok_spec(first, next),
{
    first == next
}

proof fn lemma_f86_as_is_truncates()
    ensures
        keep_body_without_cl_spec(),
        !keep_body_without_cl_as_is_spec(),
{
}

proof fn lemma_f87_as_is_zero()
    ensures
        !invalid_cl_as_zero_spec(),
        invalid_cl_as_zero_as_is_spec(),
{
}

proof fn lemma_f88_as_is_last_wins(a: u64, b: u64)
    requires
        a != b,
    ensures
        !content_length_repeat_ok_spec(a, b),
        content_length_repeat_ok_as_is_spec(a, b),
{
}

} // verus!
