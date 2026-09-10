//! Bearer scheme match (RFC-0002 P31 / F85).
//!
//! **Term:** this file is what `rustc` links. Aeneas extracts that body
//! (`scripts/aeneas_auth.sh`). A u8-fold view of the rustc `&str` bearer
//! match is a model twin — not last-wins (deleted).
//!
//!   ./scripts/aeneas_auth.sh --required
//!
//! Production `header_token` calls [`is_bearer_scheme`].
//! RFC 9110 §11.1: auth-scheme is case-insensitive.
//!
//! AS-IS: only exact `Bearer` / `bearer` — `BEARER tok` compared the whole
//! header to the configured token → 401.

#![forbid(unsafe_code)]

/// ASCII lowercase (`A`–`Z` → `a`–`z`).
#[must_use]
pub fn ascii_lower(b: u8) -> u8 {
    if b.is_ascii_uppercase() {
        b.to_ascii_lowercase()
    } else {
        b
    }
}

/// ASCII uppercase (`a`–`z` → `A`–`Z`). F79 methods.
#[must_use]
pub fn ascii_upper(b: u8) -> u8 {
    if b.is_ascii_lowercase() {
        b.to_ascii_uppercase()
    } else {
        b
    }
}

/// AS-IS F85: no fold — `A` stays `A`, so `BEARER` never matches `bearer`.
#[must_use]
pub fn ascii_lower_as_is(b: u8) -> u8 {
    b
}

/// AS-IS F79: no fold — `a` stays `a`, so `put` never matches `PUT`.
#[must_use]
pub fn ascii_upper_as_is(b: u8) -> u8 {
    b
}

/// RFC 9110: method token compared in ASCII uppercase.
#[must_use]
pub fn normalize_http_method(m: &str) -> String {
    m.to_ascii_uppercase()
}

/// AS-IS F79: raw request token (`put` ≠ `PUT`).
#[must_use]
pub fn normalize_http_method_as_is(m: &str) -> String {
    m.to_string()
}

/// RFC 9110: scheme token equals `bearer` ignoring ASCII case.
#[must_use]
pub fn is_bearer_scheme(scheme: &str) -> bool {
    scheme.eq_ignore_ascii_case("bearer")
}

/// Other common auth-schemes. Scheme-only (`Authorization: Basic`) is not a
/// shared-secret token (F151).
#[must_use]
pub fn is_non_bearer_auth_scheme(scheme: &str) -> bool {
    scheme.eq_ignore_ascii_case("basic")
        || scheme.eq_ignore_ascii_case("digest")
        || scheme.eq_ignore_ascii_case("negotiate")
        || scheme.eq_ignore_ascii_case("ntlm")
}

/// AS-IS F150/F151: scheme-blind — every header value is a candidate token, so
/// a scheme-only `Basic` becomes the token and the first non-bearer scheme
/// locks out a later Bearer.
#[must_use]
pub fn is_non_bearer_auth_scheme_as_is(_scheme: &str) -> bool {
    false
}

/// AS-IS F85: only the two literal prefixes that were stripped.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_kernel_has_no_verus_cartoon() {
        let src = include_str!("auth_kernel.rs");
        let block = concat!("verus", "!", " {");
        let cfg = concat!("cfg(", "verus", "_keep", "_ghost)");
        assert!(
            !src.contains(block),
            "u8-fold stand-in is not last-wins of rustc &str bearer"
        );
        assert!(
            !src.contains(cfg),
            "cfg split hides rustc types from the prover"
        );
    }

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

    /// Three teeth (F79/F85): the fold is what makes case-insensitive tokens
    /// match — AS-IS leaves every byte untouched.
    #[test]
    fn ascii_fold_discriminates_as_is() {
        assert_eq!(ascii_lower(b'B'), b'b');
        assert_eq!(ascii_lower_as_is(b'B'), b'B');
        assert_eq!(ascii_upper(b'b'), b'B');
        assert_eq!(ascii_upper_as_is(b'b'), b'b');
        // The fold atoms compose into the scheme/method decisions:
        // "BEARER" folds onto "bearer", "put" folds onto "PUT".
        let folded_scheme: Vec<u8> = "BEARER".bytes().map(ascii_lower).collect();
        assert_eq!(folded_scheme, b"bearer");
        let unfolded: Vec<u8> = "BEARER".bytes().map(ascii_lower_as_is).collect();
        assert_eq!(unfolded, b"BEARER");
        let folded_method: Vec<u8> = "put".bytes().map(ascii_upper).collect();
        assert_eq!(folded_method, b"PUT");
        let unfolded_method: Vec<u8> = "put".bytes().map(ascii_upper_as_is).collect();
        assert_eq!(unfolded_method, b"put");
    }

    /// Three teeth (F150/F151): the non-bearer gate is what keeps the scan
    /// alive past `Basic` — AS-IS is scheme-blind.
    #[test]
    fn non_bearer_scheme_gate() {
        assert!(is_non_bearer_auth_scheme("Basic"));
        assert!(is_non_bearer_auth_scheme("DIGEST"));
        assert!(is_non_bearer_auth_scheme("Negotiate"));
        assert!(is_non_bearer_auth_scheme("ntlm"));
        assert!(!is_non_bearer_auth_scheme("Bearer"));
        assert!(!is_non_bearer_auth_scheme("bearer"));
        // AS-IS: scheme-blind — `Basic` is indistinguishable from a token and
        // the scan never learns to keep going.
        assert!(!is_non_bearer_auth_scheme_as_is("Basic"));
        assert_ne!(
            is_non_bearer_auth_scheme("Basic"),
            is_non_bearer_auth_scheme_as_is("Basic")
        );
    }
}
