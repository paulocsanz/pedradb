//! Fail-closed HTTP wire (RFC-0002 P38 / F102 / F104 / F105).
//!
//! **Single artifact (Aeneas-paid):** this file is what `rustc` links and
//! what the Lean defs run over — Charon+Aeneas extract of these exact
//! bodies. No Verus twin stands in for them.
//!
//!   ./scripts/aeneas_fail_closed.sh
//!
//! Production `handle_kv` / `handle_dcs` / `read_req` / `query_u64` call these.
//! Writing the status line and parsing integers are caller + axiom.

#![forbid(unsafe_code)]

macro_rules! parse_error_writes_status_body {
    () => {
        true
    };
}
macro_rules! parse_error_writes_status_as_is_body {
    () => {
        false
    };
}
macro_rules! parse_error_status_body {
    () => {
        400u16
    };
}
macro_rules! reject_transfer_encoding_body {
    () => {
        true
    };
}
macro_rules! reject_transfer_encoding_as_is_body {
    () => {
        false
    };
}
macro_rules! present_bad_int_is_error_body {
    () => {
        true
    };
}
macro_rules! present_bad_int_is_error_as_is_body {
    () => {
        false
    };
}

/// F102: `read_req` Err writes a status line (does not drop the socket mute).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn parse_error_writes_status() -> bool {
    parse_error_writes_status_body!()
}

/// AS-IS F102: worker returns Err and closes with no HTTP response.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn parse_error_writes_status_as_is() -> bool {
    parse_error_writes_status_as_is_body!()
}

/// Status code for a wire parse failure.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn parse_error_status() -> u16 {
    parse_error_status_body!()
}

/// F104: any `Transfer-Encoding` is rejected (chunked unsupported).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn reject_transfer_encoding() -> bool {
    reject_transfer_encoding_body!()
}

/// AS-IS F104: ignore TE; F86 keep-without-CL stores the raw chunk framing.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn reject_transfer_encoding_as_is() -> bool {
    reject_transfer_encoding_as_is_body!()
}

/// F105: a *present* but unparseable integer is an error (not the default).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn present_bad_int_is_error() -> bool {
    present_bad_int_is_error_body!()
}

/// AS-IS F105: `parse().ok().unwrap_or(default)` — `ttl_ms=abc` acquires.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn present_bad_int_is_error_as_is() -> bool {
    present_bad_int_is_error_as_is_body!()
}

/// Offset just past the header/body break.
///
/// RFC 9112 prefers `\r\n\r\n`. F153: LF-only clients send `\n\n`; looking
/// only for CRLF 400'd those requests (and could mis-frame a body that
/// itself contains `\r\n\r\n`).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn header_break_end(buf: &[u8]) -> Option<usize> {
    let crlf = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4);
    let lf = buf.windows(2).position(|w| w == b"\n\n").map(|i| i + 2);
    match (crlf, lf) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// AS-IS F153: only the four-byte CRLF break.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn header_break_end_as_is(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

/// Length of the break ending at `end` (4 for CRLFCRLF, 2 for LFLF).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn header_break_len(buf: &[u8], end: usize) -> usize {
    if end >= 4 && buf.get(end - 4..end) == Some(b"\r\n\r\n".as_ref()) {
        4
    } else {
        2
    }
}

/// F154: `Expect: 100-continue` (RFC 9110) — case-insensitive, comma list.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn expects_100_continue(value: &str) -> bool {
    value
        .split(',')
        .any(|t| t.trim().eq_ignore_ascii_case("100-continue"))
}

/// AS-IS F154: never send 100; client and server wait on each other.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn expects_100_continue_as_is(_value: &str) -> bool {
    false
}

/// F159: every Expect token is empty or `100-continue` (RFC 9110).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn expect_field_ok(value: &str) -> bool {
    value.split(',').all(|t| {
        let t = t.trim();
        t.is_empty() || t.eq_ignore_ascii_case("100-continue")
    })
}

/// AS-IS F159: unknown Expect is ignored.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn expect_field_ok_as_is(_value: &str) -> bool {
    true
}

/// Status for an unrecognized expectation.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn expectation_failed_status() -> u16 {
    417
}

/// F157: RFC 9112 — HTTP/1.1 (and later) request-line requires `Host`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn http_version_requires_host(version: &str) -> bool {
    let v = version.trim();
    let rest = if v.len() >= 5 && v[..5].eq_ignore_ascii_case("HTTP/") {
        &v[5..]
    } else {
        return false;
    };
    rest != "1.0" && !rest.eq_ignore_ascii_case("1.0")
}

/// AS-IS F157: never require Host.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn http_version_requires_host_as_is(_version: &str) -> bool {
    false
}

/// F157: two `Host` field-values disagree.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn host_values_conflict(a: &str, b: &str) -> bool {
    a != b
}

/// AS-IS: last Host wins.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn host_values_conflict_as_is(_a: &str, _b: &str) -> bool {
    false
}

/// F158: RFC 9112 invalid `Host` field-value (empty after OWS trim).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn host_value_ok(value: &str) -> bool {
    !value.is_empty()
}

/// AS-IS F157 residual: empty Host counted as present.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn host_value_ok_as_is(_value: &str) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f102_writes_400() {
        assert!(parse_error_writes_status());
        assert_eq!(parse_error_status(), 400);
        assert!(!parse_error_writes_status_as_is());
    }

    #[test]
    fn f104_rejects_te() {
        assert!(reject_transfer_encoding());
        assert!(!reject_transfer_encoding_as_is());
    }

    #[test]
    fn f105_present_bad_int() {
        assert!(present_bad_int_is_error());
        assert!(!present_bad_int_is_error_as_is());
    }

    #[test]
    fn f153_lf_header_break() {
        let crlf = b"PUT /kv/x HTTP/1.0\r\nContent-Length: 2\r\n\r\nok";
        let lf = b"PUT /kv/x HTTP/1.0\nContent-Length: 2\n\nok";
        let crlf_end = header_break_end(crlf).expect("crlf break");
        let lf_end = header_break_end(lf).expect("lf break");
        assert_eq!(&crlf[crlf_end..], b"ok");
        assert_eq!(&lf[lf_end..], b"ok");
        assert_eq!(header_break_end_as_is(crlf), Some(crlf_end));
        assert_eq!(header_break_end_as_is(lf), None);
        assert_ne!(header_break_end(lf), header_break_end_as_is(lf));
        assert_eq!(header_break_len(crlf, crlf_end), 4);
        assert_eq!(header_break_len(lf, lf_end), 2);
        let mixed = b"PUT /kv/b HTTP/1.0\nContent-Length: 8\n\nab\r\n\r\ncd";
        let m = header_break_end(mixed).expect("mixed");
        assert_eq!(&mixed[m..], b"ab\r\n\r\ncd");
        assert_eq!(
            header_break_end_as_is(mixed).map(|i| &mixed[i..]),
            Some(&b"cd"[..])
        );
    }

    #[test]
    fn f154_expect_100() {
        assert!(expects_100_continue("100-continue"));
        assert!(expects_100_continue("100-Continue"));
        assert!(expects_100_continue("100-CONTINUE"));
        assert!(expects_100_continue(" 100-continue "));
        assert!(expects_100_continue("100-continue, foo"));
        assert!(!expects_100_continue(""));
        assert!(!expects_100_continue("102-processing"));
        assert!(!expects_100_continue_as_is("100-continue"));
        assert!(expect_field_ok("100-continue"));
        assert!(expect_field_ok("100-Continue"));
        assert!(expect_field_ok(""));
        assert!(!expect_field_ok("blah"));
        assert!(!expect_field_ok("100-continue, foo"));
        assert!(expect_field_ok_as_is("blah"));
        assert_eq!(expectation_failed_status(), 417);
    }

    #[test]
    fn f157_http11_host() {
        assert!(http_version_requires_host("HTTP/1.1"));
        assert!(http_version_requires_host("http/1.1"));
        assert!(http_version_requires_host("HTTP/2.0"));
        assert!(!http_version_requires_host("HTTP/1.0"));
        assert!(!http_version_requires_host(""));
        assert!(!http_version_requires_host_as_is("HTTP/1.1"));
        assert!(host_values_conflict("a", "b"));
        assert!(!host_values_conflict("a", "a"));
        assert!(!host_values_conflict_as_is("a", "b"));
        assert!(host_value_ok("localhost"));
        assert!(!host_value_ok(""));
        assert!(host_value_ok_as_is(""));
    }
}
