//! Content-Length framing (RFC-0002 P32 / F86 / F87 / F88).
//!
//! Production `read_req` calls these. Socket read / `parse::<usize>` are
//! caller + axiom.

#![forbid(unsafe_code)]

/// F86: no `Content-Length` → keep bytes already past `\r\n\r\n`.
#[must_use]
pub fn keep_body_without_cl() -> bool {
    true
}

/// AS-IS F86: always `truncate(content_len)` with default 0.
#[must_use]
pub fn keep_body_without_cl_as_is() -> bool {
    false
}

/// F87: unparseable `Content-Length` becomes length 0? No — fail closed.
#[must_use]
pub fn invalid_cl_as_zero() -> bool {
    false
}

/// AS-IS F87: `parse().unwrap_or(0)` then truncate.
#[must_use]
pub fn invalid_cl_as_zero_as_is() -> bool {
    true
}

/// F88: a repeated CL must equal the first (RFC 9112).
#[must_use]
pub fn content_length_repeat_ok(first: u64, next: u64) -> bool {
    first == next
}

/// AS-IS F88: last header wins.
#[must_use]
pub fn content_length_repeat_ok_as_is(_first: u64, _next: u64) -> bool {
    true
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
    }
}
