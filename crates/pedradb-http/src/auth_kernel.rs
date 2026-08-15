//! Bearer scheme match (RFC-0002 P31 / F85).
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

/// RFC 9110: scheme token equals `bearer` ignoring ASCII case.
#[must_use]
pub fn is_bearer_scheme(scheme: &str) -> bool {
    scheme.eq_ignore_ascii_case("bearer")
}

/// AS-IS F85: only the two literal prefixes that were stripped.
#[must_use]
pub fn is_bearer_scheme_as_is(scheme: &str) -> bool {
    scheme == "Bearer" || scheme == "bearer"
}

/// Token after a Bearer scheme, or the whole value if the scheme is not Bearer.
#[must_use]
pub fn bearer_token_from_value(value: &str) -> Option<&str> {
    let v = value.trim();
    if let Some((scheme, rest)) = v.split_once(char::is_whitespace) {
        if is_bearer_scheme(scheme) {
            return Some(rest.trim());
        }
    }
    Some(v)
}

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
}
