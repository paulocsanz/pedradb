//! Content-Length framing (RFC-0002 P32 / F86 / F87 / F88).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). No twin-cópia.
//!
//!   ./scripts/verus_content_length.sh
//!
//! Production `read_req` calls these. Socket read / `parse::<usize>` are
//! caller + axiom.

#![forbid(unsafe_code)]

macro_rules! keep_body_without_cl_body {
    () => {
        true
    };
}

macro_rules! keep_body_without_cl_as_is_body {
    () => {
        false
    };
}

macro_rules! invalid_cl_as_zero_body {
    () => {
        false
    };
}

macro_rules! invalid_cl_as_zero_as_is_body {
    () => {
        true
    };
}

macro_rules! content_length_repeat_ok_body {
    ($first:expr, $next:expr) => {
        $first == $next
    };
}

macro_rules! content_length_repeat_ok_as_is_body {
    ($first:expr, $next:expr) => {{
        let _ = ($first, $next);
        true
    }};
}

macro_rules! short_body_vs_cl_is_error_body {
    ($got:expr, $declared:expr) => {
        $got < $declared
    };
}

macro_rules! short_body_vs_cl_is_error_as_is_body {
    ($got:expr, $declared:expr) => {{
        let _ = ($got, $declared);
        false
    }};
}

/// F86: no `Content-Length` → keep bytes already past `\r\n\r\n`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn keep_body_without_cl() -> bool {
    keep_body_without_cl_body!()
}

/// AS-IS F86: always `truncate(content_len)` with default 0.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn keep_body_without_cl_as_is() -> bool {
    keep_body_without_cl_as_is_body!()
}

/// F87: unparseable `Content-Length` becomes length 0? No — fail closed.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn invalid_cl_as_zero() -> bool {
    invalid_cl_as_zero_body!()
}

/// AS-IS F87: `parse().unwrap_or(0)` then truncate.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn invalid_cl_as_zero_as_is() -> bool {
    invalid_cl_as_zero_as_is_body!()
}

/// F88: a repeated CL must equal the first (RFC 9112).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn content_length_repeat_ok(first: u64, next: u64) -> bool {
    content_length_repeat_ok_body!(first, next)
}

/// AS-IS F88: last header wins.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn content_length_repeat_ok_as_is(first: u64, next: u64) -> bool {
    content_length_repeat_ok_as_is_body!(first, next)
}

/// F146: EOF before `Content-Length` bytes is a framing error (not a short store).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn short_body_vs_cl_is_error(got: u64, declared: u64) -> bool {
    short_body_vs_cl_is_error_body!(got, declared)
}

/// AS-IS F146: accept whatever arrived and truncate.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn short_body_vs_cl_is_error_as_is(got: u64, declared: u64) -> bool {
    short_body_vs_cl_is_error_as_is_body!(got, declared)
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn keep_body_without_cl_spec() -> bool {
    true
}

pub fn keep_body_without_cl() -> (d: bool)
    ensures
        d == keep_body_without_cl_spec(),
        d,
{
    keep_body_without_cl_body!()
}

pub open spec fn keep_body_without_cl_as_is_spec() -> bool {
    false
}

pub fn keep_body_without_cl_as_is() -> (d: bool)
    ensures
        d == false,
        d == keep_body_without_cl_as_is_spec(),
{
    keep_body_without_cl_as_is_body!()
}

pub open spec fn invalid_cl_as_zero_spec() -> bool {
    false
}

pub fn invalid_cl_as_zero() -> (d: bool)
    ensures
        d == invalid_cl_as_zero_spec(),
        !d,
{
    invalid_cl_as_zero_body!()
}

pub open spec fn invalid_cl_as_zero_as_is_spec() -> bool {
    true
}

pub fn invalid_cl_as_zero_as_is() -> (d: bool)
    ensures
        d == true,
        d == invalid_cl_as_zero_as_is_spec(),
{
    invalid_cl_as_zero_as_is_body!()
}

pub open spec fn content_length_repeat_ok_spec(first: u64, next: u64) -> bool {
    first == next
}

pub fn content_length_repeat_ok(first: u64, next: u64) -> (d: bool)
    ensures
        d == content_length_repeat_ok_spec(first, next),
{
    content_length_repeat_ok_body!(first, next)
}

pub open spec fn content_length_repeat_ok_as_is_spec(_first: u64, _next: u64) -> bool {
    true
}

pub fn content_length_repeat_ok_as_is(first: u64, next: u64) -> (d: bool)
    ensures
        d == true,
        d == content_length_repeat_ok_as_is_spec(first, next),
{
    content_length_repeat_ok_as_is_body!(first, next)
}

pub fn short_body_vs_cl_is_error(got: u64, declared: u64) -> (d: bool)
    ensures
        d == (got < declared),
{
    short_body_vs_cl_is_error_body!(got, declared)
}

pub fn short_body_vs_cl_is_error_as_is(got: u64, declared: u64) -> (d: bool)
    ensures
        d == false,
{
    short_body_vs_cl_is_error_as_is_body!(got, declared)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f86_keeps_body_without_cl() {
        assert!(keep_body_without_cl());
        assert!(!keep_body_without_cl_as_is());
    }

    #[test]
    fn f87_invalid_is_not_zero() {
        assert!(!invalid_cl_as_zero());
        assert!(invalid_cl_as_zero_as_is());
    }

    #[test]
    fn f88_conflict_rejected() {
        assert!(content_length_repeat_ok(5, 5));
        assert!(!content_length_repeat_ok(5, 0));
        assert!(content_length_repeat_ok_as_is(5, 0));
    }

    #[test]
    fn theorem_on_small_domain() {
        let mut n = 0u32;
        for a in 0u64..6 {
            for b in 0u64..6 {
                let d = content_length_repeat_ok(a, b);
                assert_eq!(d, a == b);
                assert!(content_length_repeat_ok_as_is(a, b));
                if a != b {
                    assert!(!d);
                }
                n += 1;
            }
        }
        assert_eq!(n, 36);
        assert_ne!(keep_body_without_cl(), keep_body_without_cl_as_is());
        assert_ne!(invalid_cl_as_zero(), invalid_cl_as_zero_as_is());
        assert!(short_body_vs_cl_is_error(2, 5));
        assert!(!short_body_vs_cl_is_error(5, 5));
        assert!(!short_body_vs_cl_is_error_as_is(2, 5));
    }
}
