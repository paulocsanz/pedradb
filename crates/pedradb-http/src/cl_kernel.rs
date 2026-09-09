//! Content-Length framing (RFC-0002 P32 / F86 / F87 / F88).
//!
//! **Single artifact (Aeneas-paid):** this file is what `rustc` links and
//! what the Lean defs run over — Charon+Aeneas extract of these exact
//! bodies. No Verus twin stands in for them.
//!
//!   ./scripts/aeneas_cl.sh
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
