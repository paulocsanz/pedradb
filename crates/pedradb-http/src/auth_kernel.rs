//! Bearer scheme match (RFC-0002 P31 / F85).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). String `eq_ignore_ascii_case` is
//! caller; the byte fold is the term. No twin-cópia.
//!
//!   ./scripts/verus_bearer_scheme.sh
//!
//! Production `header_token` calls [`is_bearer_scheme`].
//! RFC 9110 §11.1: auth-scheme is case-insensitive.
//!
//! AS-IS: only exact `Bearer` / `bearer` — `BEARER tok` compared the whole
//! header to the configured token → 401.

#![forbid(unsafe_code)]

/// ASCII lowercase (`A`–`Z` → `a`–`z`).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn ascii_lower(b: u8) -> u8 {
    if b.is_ascii_uppercase() {
        b.to_ascii_lowercase()
    } else {
        b
    }
}

/// ASCII uppercase (`a`–`z` → `A`–`Z`). F79 methods.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn ascii_upper(b: u8) -> u8 {
    if b.is_ascii_lowercase() {
        b.to_ascii_uppercase()
    } else {
        b
    }
}

/// RFC 9110: method token compared in ASCII uppercase.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn normalize_http_method(m: &str) -> String {
    m.to_ascii_uppercase()
}

/// AS-IS F79: raw request token (`put` ≠ `PUT`).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn normalize_http_method_as_is(m: &str) -> String {
    m.to_string()
}

/// RFC 9110: scheme token equals `bearer` ignoring ASCII case.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn is_bearer_scheme(scheme: &str) -> bool {
    scheme.eq_ignore_ascii_case("bearer")
}

/// Other common auth-schemes. Scheme-only (`Authorization: Basic`) is not a
/// shared-secret token (F151).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn is_non_bearer_auth_scheme(scheme: &str) -> bool {
    scheme.eq_ignore_ascii_case("basic")
        || scheme.eq_ignore_ascii_case("digest")
        || scheme.eq_ignore_ascii_case("negotiate")
        || scheme.eq_ignore_ascii_case("ntlm")
}

/// AS-IS F85: only the two literal prefixes that were stripped.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn is_bearer_scheme_as_is(scheme: &str) -> bool {
    scheme == "Bearer" || scheme == "bearer"
}

/// Token after a Bearer scheme.
///
/// - `Bearer <tok>` / `BEARER <tok>` → `Some(tok)` (F85)
/// - Other auth-scheme (`Basic …`, `Digest …`) → `None` so the caller can keep
///   scanning (F150 — first Basic must not lock out a later Bearer)
/// - Scheme-only `Bearer` / `BEARER` (no credentials) → `None` (F151 — used to
///   be the raw token `"Bearer"` and stop the scan)
/// - Empty value → `None`
/// - Bare value with no scheme → `Some(value)` (legacy)
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn bearer_token_from_value(value: &str) -> Option<&str> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    if let Some((scheme, rest)) = v.split_once(char::is_whitespace) {
        if is_bearer_scheme(scheme) {
            let tok = rest.trim();
            if tok.is_empty() {
                return None;
            }
            return Some(tok);
        }
        return None;
    }
    // F151: `Authorization: Bearer` / `Basic` with no credentials is the
    // scheme, not a shared-secret token named "Bearer" / "Basic".
    if is_bearer_scheme(v) || is_non_bearer_auth_scheme(v) {
        return None;
    }
    Some(v)
}

/// AS-IS F85: only exact `Bearer ` / `bearer ` prefixes; `BEARER tok` is the
/// whole header compared to the configured token (401).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn bearer_token_from_value_as_is(value: &str) -> Option<&str> {
    let v = value.trim();
    if let Some(rest) = v.strip_prefix("Bearer ") {
        let tok = rest.trim();
        return if tok.is_empty() { Some(v) } else { Some(tok) };
    }
    if let Some(rest) = v.strip_prefix("bearer ") {
        let tok = rest.trim();
        return if tok.is_empty() { Some(v) } else { Some(tok) };
    }
    if v.is_empty() {
        return None;
    }
    Some(v)
}

/// F152: a later valid Bearer must win over an earlier dummy Bearer.
///
/// X-Pedra-Token is fallback only when no Bearer token was extracted (F149).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn authorization_matches<K: AsRef<str>, V: AsRef<str>>(
    headers: &[(K, V)],
    expected: &str,
) -> bool {
    let mut saw_bearer = false;
    let mut x_pedra: Option<&str> = None;
    for (k, v) in headers {
        let k = k.as_ref();
        let v = v.as_ref();
        if k.eq_ignore_ascii_case("authorization") {
            if let Some(t) = bearer_token_from_value(v) {
                saw_bearer = true;
                if t == expected {
                    return true;
                }
            }
            continue;
        }
        if k.eq_ignore_ascii_case("x-pedra-token") && x_pedra.is_none() {
            x_pedra = Some(v);
        }
    }
    if saw_bearer {
        return false;
    }
    x_pedra == Some(expected)
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn ascii_lower_spec(b: u8) -> u8 {
    if b >= 65u8 && b <= 90u8 {
        (b + 32) as u8
    } else {
        b
    }
}

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

fn ascii_eq_ignore_case(a: u8, b: u8) -> (d: bool)
    ensures
        d == ascii_eq_ignore_case_spec(a, b),
{
    ascii_lower(a) == ascii_lower(b)
}

pub open spec fn ascii_eq_as_is_spec(a: u8, b: u8) -> bool {
    a == b
}

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

proof fn lemma_as_is_misses_lower_p()
    ensures
        ascii_eq_ignore_case_spec(80u8, 112u8),
        !ascii_eq_as_is_spec(80u8, 112u8),
        ascii_upper_spec(112u8) == 80u8,
{
}

} // verus!

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_case_insensitive() {
        assert!(is_bearer_scheme("Bearer"));
        assert!(is_bearer_scheme("bearer"));
        assert!(is_bearer_scheme("BEARER"));
        assert!(is_bearer_scheme("BeArEr"));
        assert!(!is_bearer_scheme("Basic"));
        assert!(!is_bearer_scheme_as_is("BEARER"));
        assert!(is_bearer_scheme_as_is("Bearer"));
    }

    #[test]
    fn extracts_token_from_upper_scheme() {
        assert_eq!(bearer_token_from_value("BEARER sekrit"), Some("sekrit"));
        assert_eq!(bearer_token_from_value("Bearer sekrit"), Some("sekrit"));
        assert_ne!(is_bearer_scheme("BEARER"), is_bearer_scheme_as_is("BEARER"));
        // F150: non-Bearer schemes are not bearer tokens.
        assert_eq!(bearer_token_from_value("Basic YWJj"), None);
        assert_eq!(bearer_token_from_value("Digest abc"), None);
        assert_eq!(bearer_token_from_value("bare"), Some("bare"));
        // F151: scheme-only / empty Bearer is not a token.
        assert_eq!(bearer_token_from_value("Bearer"), None);
        assert_eq!(bearer_token_from_value("BEARER"), None);
        assert_eq!(bearer_token_from_value("Bearer   "), None);
        assert_eq!(bearer_token_from_value("Basic"), None);
        assert_eq!(bearer_token_from_value("DIGEST"), None);
        assert_eq!(bearer_token_from_value(""), None);
        assert_eq!(bearer_token_from_value("   "), None);
    }

    #[test]
    fn any_bearer_matches() {
        let dual = [
            ("authorization".to_string(), "Bearer nope".to_string()),
            ("authorization".to_string(), "Bearer sekrit".to_string()),
        ];
        assert!(authorization_matches(&dual, "sekrit"));
        assert!(!authorization_matches(&dual, "nope-other"));
        let x_only = [("x-pedra-token".to_string(), "sekrit".to_string())];
        assert!(authorization_matches(&x_only, "sekrit"));
        let bearer_then_x = [
            ("authorization".to_string(), "Bearer nope".to_string()),
            ("x-pedra-token".to_string(), "sekrit".to_string()),
        ];
        assert!(
            !authorization_matches(&bearer_then_x, "sekrit"),
            "F149: X-Pedra must not override a present (wrong) Bearer"
        );
    }

    #[test]
    fn theorem_ascii_lower_letters() {
        assert_eq!(ascii_lower(b'B'), b'b');
        assert_eq!(ascii_lower(b'b'), b'b');
        assert_eq!(ascii_lower(b'0'), b'0');
        for c in b'A'..=b'Z' {
            assert_eq!(ascii_lower(c), c + 32);
            assert_eq!(ascii_lower(c + 32), c + 32);
        }
    }

    #[test]
    fn method_uppercases() {
        assert_eq!(normalize_http_method("put"), "PUT");
        assert_eq!(normalize_http_method("GET"), "GET");
        assert_eq!(normalize_http_method_as_is("put"), "put");
        assert_ne!(
            normalize_http_method("put"),
            normalize_http_method_as_is("put")
        );
        assert_eq!(ascii_upper(b'p'), b'P');
    }
}
