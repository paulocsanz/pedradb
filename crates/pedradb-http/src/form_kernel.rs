//! Form-urlencoded query values (RFC-0002 P35 / F101).
//!
//! **Single artifact (Aeneas-paid):** this file is what `rustc` links and
//! what the Lean defs run over — Charon+Aeneas extract of these exact
//! bodies. No Verus twin stands in for them.
//!
//!   ./scripts/aeneas_form.sh
//!
//! Production [`crate::query_param`] calls [`form_decode`]. Path segments still
//! use `%HH` only (`+` stays literal — RFC 3986).
//!
//! AS-IS: percent-decode only, so `hello+world` ≠ `hello world`.

#![forbid(unsafe_code)]

macro_rules! form_plus_byte_body {
    ($b:expr) => {
        if $b == b'+' {
            b' '
        } else {
            $b
        }
    };
}

macro_rules! form_plus_byte_as_is_body {
    ($b:expr) => {
        $b
    };
}

macro_rules! plus_before_percent_body {
    () => {
        true
    };
}

macro_rules! plus_before_percent_as_is_body {
    () => {
        false
    };
}

macro_rules! from_hex_as_is_body {
    ($c:expr) => {
        match $c {
            b'0'..=b'9' => Some($c - b'0'),
            _ => None,
        }
    };
}

macro_rules! query_u64_conflict_body {
    ($a:expr, $b:expr) => {
        $a != $b
    };
}

macro_rules! query_u64_conflict_as_is_body {
    ($a:expr, $b:expr) => {{
        let _ = ($a, $b);
        false
    }};
}

/// F101: a raw `+` in a query value is a space.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn form_plus_byte(b: u8) -> u8 {
    form_plus_byte_body!(b)
}

/// AS-IS F101: `+` stays `+` (path-style / F76 only).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn form_plus_byte_as_is(b: u8) -> u8 {
    form_plus_byte_as_is_body!(b)
}

/// `+` is mapped **before** `%HH`, so `%2B` remains a literal plus.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn plus_before_percent() -> bool {
    plus_before_percent_body!()
}

/// AS-IS F101 order: `%HH` happens without the `+`-first rule — the literal
/// `%2B` escape hatch is gone and ordering no longer discriminates.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn plus_before_percent_as_is() -> bool {
    plus_before_percent_as_is_body!()
}

/// Hex nibble for `%HH`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn from_hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// AS-IS: hex letters never decode — `%41` never becomes `A`, `%2F` never
/// becomes `/`, so `%HH` with a letter nibble stays literal.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn from_hex_as_is(c: u8) -> Option<u8> {
    from_hex_as_is_body!(c)
}

/// Query-value decode: `+` → space, then `%HH`.
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn query_values_conflict(values: &[&str]) -> bool {
    match values {
        [] | [_] => false,
        [first, rest @ ..] => rest.iter().any(|v| *v != *first),
    }
}

/// AS-IS F155: first value always wins; never a conflict.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn query_values_conflict_as_is(_values: &[&str]) -> bool {
    false
}

/// F155: parsed query ints disagree (`rev=1` then `rev=0`).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn query_u64_conflict(a: u64, b: u64) -> bool {
    query_u64_conflict_body!(a, b)
}

/// AS-IS F88-class: last/first wins, never reject.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn query_u64_conflict_as_is(a: u64, b: u64) -> bool {
    query_u64_conflict_as_is_body!(a, b)
}

/// F162: a query part with no `=` whose decoded name is `key` (`?rev`).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn query_part_is_bare_name(part: &str, key: &str) -> bool {
    !part.is_empty() && !part.contains('=') && form_decode(part) == key.as_bytes()
}

/// AS-IS F162: skip parts without `=`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn query_part_is_bare_name_as_is(_part: &str, _key: &str) -> bool {
    false
}

/// AS-IS F101: `%HH` only (no `+` → space).
#[cfg(not(verus_keep_ghost))]
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

    /// Three teeth: hex letters decode in the FIXED nibble fn and never in
    /// AS-IS — `%41`/`%2F` only become `A`/`/` when the fold is real.
    #[test]
    fn hex_letters_decode_as_is_does_not() {
        assert_eq!(from_hex(b'0'), Some(0));
        assert_eq!(from_hex(b'9'), Some(9));
        assert_eq!(from_hex(b'a'), Some(10));
        assert_eq!(from_hex(b'f'), Some(15));
        assert_eq!(from_hex(b'A'), Some(10));
        assert_eq!(from_hex(b'F'), Some(15));
        assert_eq!(from_hex(b'g'), None);
        // AS-IS: digits still decode, every letter nibble does not.
        assert_eq!(from_hex_as_is(b'0'), Some(0));
        assert_eq!(from_hex_as_is(b'9'), Some(9));
        assert_eq!(from_hex_as_is(b'a'), None);
        assert_eq!(from_hex_as_is(b'A'), None);
        // Downstream: `%41` becomes `A` only under the FIXED decoder.
        assert_eq!(form_decode("%41"), b"A");
    }

    /// Three teeth: the `+`-before-`%HH` ordering flag discriminates —
    /// FIXED keeps `%2B` a literal plus because `+` was already mapped.
    #[test]
    fn plus_order_flag_discriminates_as_is() {
        assert!(plus_before_percent());
        assert!(!plus_before_percent_as_is());
        // The flag is what makes `%2B` a literal plus in the FIXED decoder.
        assert_eq!(form_decode("plus%2Bsign"), b"plus+sign");
        assert_eq!(form_plus_byte(b'+'), b' ');
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
        assert!(query_part_is_bare_name("rev", "rev"));
        assert!(query_part_is_bare_name("%72ev", "rev"));
        assert!(!query_part_is_bare_name("rev=0", "rev"));
        assert!(!query_part_is_bare_name("ttl_ms", "rev"));
        assert!(!query_part_is_bare_name_as_is("rev", "rev"));
    }
}
