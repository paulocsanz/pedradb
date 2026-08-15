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
    strip_http_authority_rest(target).map(path_after_authority)
}

/// Authority of an absolute-form or network-path target (`host[:port]`).
#[must_use]
pub fn request_target_authority(target: &str) -> Option<&str> {
    let target = strip_uri_fragment(target);
    let rest = if let Some(r) = strip_http_authority_rest(target) {
        r
    } else {
        target.strip_prefix("//")?
    };
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let auth = &rest[..end];
    if auth.is_empty() {
        None
    } else {
        Some(auth)
    }
}

fn strip_http_authority_rest(target: &str) -> Option<&str> {
    let b = target.as_bytes();
    if b.len() >= 7 && b[..7].eq_ignore_ascii_case(b"http://") {
        Some(&target[7..])
    } else if b.len() >= 8 && b[..8].eq_ignore_ascii_case(b"https://") {
        Some(&target[8..])
    } else {
        None
    }
}

/// F161: Host and absolute-form / network-path authority disagree (RFC 9112).
#[must_use]
pub fn host_authority_mismatch(host: &str, authority: &str) -> bool {
    !host.eq_ignore_ascii_case(authority)
}

/// AS-IS F161: never compare Host to the request-target authority.
#[must_use]
pub fn host_authority_mismatch_as_is(_host: &str, _authority: &str) -> bool {
    false
}

/// RFC 3986: `#fragment` is not part of the request-target path or query.
#[must_use]
pub fn strip_uri_fragment(target: &str) -> &str {
    target.split_once('#').map(|(a, _)| a).unwrap_or(target)
}

/// AS-IS F156: fragment stays in the path / last query value.
#[must_use]
pub fn strip_uri_fragment_as_is(target: &str) -> &str {
    target
}

/// F91/F92: strip absolute-form / network-path, then the query string.
/// F156: `#fragment` is not a path segment (same class as `?query` / F74).
#[must_use]
pub fn origin_form_path(target: &str) -> &str {
    let target = strip_uri_fragment(target);
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
        // F156: fragment is not a path component.
        assert_eq!(origin_form_path("/kv/x#frag"), "/kv/x");
        assert_eq!(origin_form_path("/kv/x?y=1#f"), "/kv/x");
        assert_eq!(origin_form_path("http://h/kv/x#f"), "/kv/x");
        assert_eq!(strip_uri_fragment("/dcs/kv/k?rev=0#x"), "/dcs/kv/k?rev=0");
        assert_eq!(strip_uri_fragment_as_is("/kv/x#f"), "/kv/x#f");
        assert_eq!(
            request_target_authority("http://evil.example/kv/x"),
            Some("evil.example")
        );
        assert_eq!(
            request_target_authority("//h:9/kv/x?q=1"),
            Some("h:9")
        );
        assert_eq!(request_target_authority("/kv/x"), None);
        assert!(host_authority_mismatch("localhost", "evil.example"));
        assert!(!host_authority_mismatch("LocalHost", "localhost"));
        assert!(!host_authority_mismatch_as_is("localhost", "evil.example"));
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
