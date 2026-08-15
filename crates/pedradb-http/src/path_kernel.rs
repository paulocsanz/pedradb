//! Origin-form path for routing (RFC-0002 P34 / F91 / F92).
//!
//! Production `path_only` / `handle_kv` / `handle_dcs` call
//! [`origin_form_path`]. Query strip after that is the same for FIXED and AS-IS.

#![forbid(unsafe_code)]

/// First `/…` after an authority, or `"/"` if the authority has no path.
#[must_use]
pub fn path_after_authority(rest: &str) -> &str {
    rest.find('/').map(|i| &rest[i..]).unwrap_or("/")
}

/// Strip `http(s)://authority` (scheme case-insensitive — RFC 9110 / F145).
#[must_use]
pub fn strip_http_authority(target: &str) -> Option<&str> {
    let b = target.as_bytes();
    let rest = if b.len() >= 7 && b[..7].eq_ignore_ascii_case(b"http://") {
        Some(&target[7..])
    } else if b.len() >= 8 && b[..8].eq_ignore_ascii_case(b"https://") {
        Some(&target[8..])
    } else {
        None
    };
    rest.map(path_after_authority)
}

/// F91/F92: strip absolute-form / network-path, then the query string.
#[must_use]
pub fn origin_form_path(target: &str) -> &str {
    let p = if let Some(p) = strip_http_authority(target) {
        p
    } else if let Some(rest) = target.strip_prefix("//") {
        path_after_authority(rest)
    } else {
        target
    };
    p.split_once('?').map(|(a, _)| a).unwrap_or(p)
}

/// AS-IS F91/F92: only strip `?query` — `http://host/kv/x` never matches `/kv/`.
#[must_use]
pub fn origin_form_path_as_is(target: &str) -> &str {
    target.split_once('?').map(|(a, _)| a).unwrap_or(target)
}

/// Whether an authority-form target must be stripped before routing.
#[must_use]
pub fn strip_authority_for_routing(is_authority_form: bool) -> bool {
    is_authority_form
}

/// AS-IS: never strip authority (route sees `http://` / `//`).
#[must_use]
pub fn strip_authority_for_routing_as_is(_is_authority_form: bool) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_strips_absolute_and_network() {
        assert_eq!(origin_form_path("/kv/x"), "/kv/x");
        assert_eq!(origin_form_path("/kv/x?y=1"), "/kv/x");
        assert_eq!(origin_form_path("http://127.0.0.1:9/kv/x"), "/kv/x");
        assert_eq!(origin_form_path("https://h/kv/a%2Fb?q=1"), "/kv/a%2Fb");
        assert_eq!(origin_form_path("HTTP://H/dcs/kv/k"), "/dcs/kv/k");
        assert_eq!(origin_form_path("http://only-host"), "/");
        assert_eq!(origin_form_path("//127.0.0.1:9/kv/x"), "/kv/x");
        assert_eq!(origin_form_path("//h/dcs/kv/k?rev=1"), "/dcs/kv/k");
        assert_eq!(origin_form_path("//only-host"), "/");
        // F145: scheme is case-insensitive (RFC 9110).
        assert_eq!(origin_form_path("Http://h/kv/x"), "/kv/x");
        assert_eq!(origin_form_path("HtTpS://h/kv/y?z=1"), "/kv/y");
    }

    #[test]
    fn as_is_keeps_authority() {
        let abs = "http://127.0.0.1:9/kv/x";
        assert_eq!(origin_form_path_as_is(abs), abs);
        assert!(!origin_form_path_as_is(abs).starts_with("/kv/"));
        assert!(origin_form_path(abs).starts_with("/kv/"));
        let np = "//h/kv/x";
        assert_eq!(origin_form_path_as_is(np), np);
        assert_eq!(origin_form_path(np), "/kv/x");
    }

    #[test]
    fn strip_flag() {
        assert!(strip_authority_for_routing(true));
        assert!(!strip_authority_for_routing(false));
        assert!(!strip_authority_for_routing_as_is(true));
    }
}
