//! Form-urlencoded query values (RFC-0002 P35 / F101).
//!
//! Production [`crate::query_param`] calls [`form_decode`]. Path segments still
//! use `%HH` only (`+` stays literal — RFC 3986).
//!
//! AS-IS: percent-decode only, so `hello+world` ≠ `hello world`.

#![forbid(unsafe_code)]

/// F101: a raw `+` in a query value is a space.
#[must_use]
pub fn form_plus_byte(b: u8) -> u8 {
    if b == b'+' {
        b' '
    } else {
        b
    }
}

/// AS-IS F101: `+` stays `+` (path-style / F76 only).
#[must_use]
pub fn form_plus_byte_as_is(b: u8) -> u8 {
    b
}

/// `+` is mapped **before** `%HH`, so `%2B` remains a literal plus.
#[must_use]
pub fn plus_before_percent() -> bool {
    true
}

/// Hex nibble for `%HH`.
#[must_use]
pub fn from_hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Query-value decode: `+` → space, then `%HH`.
#[must_use]
pub fn form_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if plus_before_percent() && b[i] == b'+' {
            out.push(form_plus_byte(b[i]));
            i += 1;
            continue;
        }
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (from_hex(b[i + 1]), from_hex(b[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// F155: two decoded query values for the same name disagree.
#[must_use]
pub fn query_values_conflict(values: &[&str]) -> bool {
    match values {
        [] | [_] => false,
        [first, rest @ ..] => rest.iter().any(|v| *v != *first),
    }
}

/// AS-IS F155: first value always wins; never a conflict.
#[must_use]
pub fn query_values_conflict_as_is(_values: &[&str]) -> bool {
    false
}

/// F155: parsed query ints disagree (`rev=1` then `rev=0`).
#[must_use]
pub fn query_u64_conflict(a: u64, b: u64) -> bool {
    a != b
}

/// AS-IS F88-class: last/first wins, never reject.
#[must_use]
pub fn query_u64_conflict_as_is(_a: u64, _b: u64) -> bool {
    false
}

/// AS-IS F101: `%HH` only (no `+` → space).
#[must_use]
pub fn form_decode_as_is(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (from_hex(b[i + 1]), from_hex(b[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plus_is_space_before_percent() {
        assert_eq!(form_decode("hello+world"), b"hello world");
        assert_eq!(form_decode("plus%2Bsign"), b"plus+sign");
        assert_eq!(form_decode_as_is("hello+world"), b"hello+world");
        assert_eq!(form_plus_byte(b'+'), b' ');
        assert_eq!(form_plus_byte_as_is(b'+'), b'+');
        assert!(plus_before_percent());
    }

    #[test]
    fn theorem_plus_byte() {
        assert_eq!(form_plus_byte(b'+'), b' ');
        assert_eq!(form_plus_byte(b'a'), b'a');
        assert_eq!(form_plus_byte(b'%'), b'%');
        for c in 0u8..=255 {
            if c == b'+' {
                assert_ne!(form_plus_byte(c), form_plus_byte_as_is(c));
            } else {
                assert_eq!(form_plus_byte(c), form_plus_byte_as_is(c));
            }
        }
    }

    #[test]
    fn f155_query_conflict() {
        assert!(!query_values_conflict(&[]));
        assert!(!query_values_conflict(&["1"]));
        assert!(!query_values_conflict(&["1", "1"]));
        assert!(query_values_conflict(&["1", "0"]));
        assert!(!query_values_conflict_as_is(&["1", "0"]));
        assert!(query_u64_conflict(1, 0));
        assert!(!query_u64_conflict(1, 1));
        assert!(!query_u64_conflict_as_is(1, 0));
    }
}
