//! Origin-form path for routing (RFC-0002 P34 / F91 / F92).
//!
//! **Single artifact (Aeneas-paid):** this file is what `rustc` links and
//! what the Lean defs run over — Charon+Aeneas extract of these exact
//! bodies. No Verus twin stands in for them.
//!
//!   ./scripts/aeneas_path.sh
//!
//! Production `path_only` / `handle_kv` / `handle_dcs` call
//! [`origin_form_path`]. Query strip after that is the same for FIXED and AS-IS.

#![forbid(unsafe_code)]

macro_rules! strip_authority_for_routing_body {
    ($is_authority_form:expr) => {
        $is_authority_form
    };
}

macro_rules! strip_authority_for_routing_as_is_body {
    ($is_authority_form:expr) => {{
        let _ = $is_authority_form;
        false
    }};
}

/// First `/…` after an authority, or `"/"` if the authority has no path.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn path_after_authority(rest: &str) -> &str {
    rest.find('/').map(|i| &rest[i..]).unwrap_or("/")
}

/// AS-IS F91: the authority text stays in the path (absolute-form never lands on `/kv/…`).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn path_after_authority_as_is(rest: &str) -> &str {
    rest
}

/// Strip `http(s)://authority` (scheme case-insensitive — RFC 9110 / F145).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn strip_http_authority(target: &str) -> Option<&str> {
    strip_http_authority_rest(target).map(path_after_authority)
}

/// AS-IS F91/F145: the scheme prefix is never recognized — `http://h/kv/x` routes nowhere.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn strip_http_authority_as_is(_target: &str) -> Option<&str> {
    None
}

/// Authority of an absolute-form or network-path target (`host[:port]`).
#[cfg(not(verus_keep_ghost))]
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

/// AS-IS F161: the authority is never extracted — absolute-form targets are
/// not inspected, so Host/target disagreement cannot be detected.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn request_target_authority_as_is(_target: &str) -> Option<&str> {
    None
}

#[cfg(not(verus_keep_ghost))]
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

/// F161/F162: Host and absolute-form / network-path authority disagree (RFC 9112).
/// F162: ignore `userinfo@` and default `:80` / `:443` (raw compare 400'd those).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn host_authority_mismatch(host: &str, authority: &str) -> bool {
    let (h1, p1) = split_host_port(host);
    let (h2, p2) = split_host_port(authority);
    if !h1.eq_ignore_ascii_case(h2) {
        return true;
    }
    !ports_equivalent(p1, p2)
}

/// Host / `[v6]` and optional numeric port. Strips a leading `userinfo@`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn split_host_port(raw: &str) -> (&str, Option<&str>) {
    let s = raw.rsplit_once('@').map_or(raw, |(_, h)| h);
    if let Some(rest) = s.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            let host = &s[..=end + 1];
            let port = rest[end + 1..].strip_prefix(':').filter(|p| !p.is_empty());
            return (host, port);
        }
    }
    if let Some((h, p)) = s.rsplit_once(':') {
        if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) {
            return (h, Some(p));
        }
    }
    (s, None)
}

/// AS-IS F162: the authority atoms (`userinfo@`, `[v6]`, numeric port) are never
/// separated — the raw string is the host and the port is invisible.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn split_host_port_as_is(raw: &str) -> (&str, Option<&str>) {
    (raw, None)
}

#[cfg(not(verus_keep_ghost))]
fn ports_equivalent(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x == y,
        (None, Some(p)) | (Some(p), None) => p == "80" || p == "443",
    }
}

/// AS-IS F161: never compare Host to the request-target authority.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn host_authority_mismatch_as_is(_host: &str, _authority: &str) -> bool {
    false
}

/// RFC 3986: `#fragment` is not part of the request-target path or query.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn strip_uri_fragment(target: &str) -> &str {
    target.split_once('#').map(|(a, _)| a).unwrap_or(target)
}

/// AS-IS F156: fragment stays in the path / last query value.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn strip_uri_fragment_as_is(target: &str) -> &str {
    target
}

/// F91/F92: strip absolute-form / network-path, then the query string.
/// F156: `#fragment` is not a path segment (same class as `?query` / F74).
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn origin_form_path_as_is(target: &str) -> &str {
    target.split_once('?').map(|(a, _)| a).unwrap_or(target)
}

/// Whether an authority-form target must be stripped before routing.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn strip_authority_for_routing(is_authority_form: bool) -> bool {
    strip_authority_for_routing_body!(is_authority_form)
}

/// AS-IS: never strip authority (route sees `http://` / `//`).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn strip_authority_for_routing_as_is(is_authority_form: bool) -> bool {
    strip_authority_for_routing_as_is_body!(is_authority_form)
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
        assert_eq!(origin_form_path("Http://h/kv/x"), "/kv/x");
        assert_eq!(origin_form_path("HtTpS://h/kv/y?z=1"), "/kv/y");
        assert_eq!(origin_form_path("/kv/x#frag"), "/kv/x");
        assert_eq!(origin_form_path("/kv/x?y=1#f"), "/kv/x");
        assert_eq!(origin_form_path("http://h/kv/x#f"), "/kv/x");
        assert_eq!(strip_uri_fragment("/dcs/kv/k?rev=0#x"), "/dcs/kv/k?rev=0");
        assert_eq!(strip_uri_fragment_as_is("/kv/x#f"), "/kv/x#f");
        assert_eq!(
            request_target_authority("http://evil.example/kv/x"),
            Some("evil.example")
        );
        assert_eq!(request_target_authority("//h:9/kv/x?q=1"), Some("h:9"));
        assert_eq!(request_target_authority("/kv/x"), None);
        assert!(host_authority_mismatch("localhost", "evil.example"));
        assert!(!host_authority_mismatch("LocalHost", "localhost"));
        assert!(!host_authority_mismatch_as_is("localhost", "evil.example"));
        assert!(!host_authority_mismatch("localhost", "localhost:80"));
        assert!(!host_authority_mismatch("localhost", "localhost:443"));
        assert!(!host_authority_mismatch("localhost", "user:pass@localhost"));
        assert!(host_authority_mismatch("localhost", "localhost:8080"));
        assert!(host_authority_mismatch("localhost", "evil.example:80"));
        assert_eq!(split_host_port("user:p@h:80"), ("h", Some("80")));
        assert_eq!(split_host_port("[::1]:80"), ("[::1]", Some("80")));
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

    /// Three teeth (F91/F145/F161/F162): FIXED and AS-IS part ways on every
    /// authority atom — scheme recognition, first-`/` after authority, and
    /// userinfo/`[v6]`/port splitting.
    #[test]
    fn authority_atoms_discriminate_as_is() {
        // path_after_authority (F91): FIXED drops the authority text before `/`.
        assert_eq!(path_after_authority("h/kv/x"), "/kv/x");
        assert_eq!(path_after_authority_as_is("h/kv/x"), "h/kv/x");
        assert_eq!(path_after_authority("only-host"), "/");
        assert_eq!(path_after_authority_as_is("only-host"), "only-host");

        // strip_http_authority (F91/F145): FIXED folds scheme case.
        assert_eq!(strip_http_authority("HTTP://H/kv/x"), Some("/kv/x"));
        assert_eq!(strip_http_authority("https://h/kv/x"), Some("/kv/x"));
        assert_eq!(strip_http_authority_as_is("HTTP://H/kv/x"), None);
        assert_eq!(strip_http_authority_as_is("http://h/kv/x"), None);

        // split_host_port (F162): FIXED splits userinfo@, `[v6]`, numeric port.
        assert_eq!(split_host_port("user:p@h:80"), ("h", Some("80")));
        assert_eq!(split_host_port_as_is("user:p@h:80"), ("user:p@h:80", None));
        assert_eq!(split_host_port("[::1]:80"), ("[::1]", Some("80")));
        assert_eq!(split_host_port_as_is("[::1]:80"), ("[::1]:80", None));

        // request_target_authority (F161): FIXED extracts, AS-IS is blind.
        assert_eq!(
            request_target_authority("http://evil.example/kv/x"),
            Some("evil.example")
        );
        assert_eq!(
            request_target_authority_as_is("http://evil.example/kv/x"),
            None
        );
    }
}
