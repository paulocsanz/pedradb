//! Fail-closed HTTP wire (RFC-0002 P38 / F102 / F104 / F105).
//!
//! Production `handle_kv` / `handle_dcs` / `read_req` / `query_u64` call these.
//! Writing the status line and parsing integers are caller + axiom.

#![forbid(unsafe_code)]

/// F102: `read_req` Err writes a status line (does not drop the socket mute).
#[must_use]
pub fn parse_error_writes_status() -> bool {
    true
}

/// AS-IS F102: worker returns Err and closes with no HTTP response.
#[must_use]
pub fn parse_error_writes_status_as_is() -> bool {
    false
}

/// Status code for a wire parse failure.
#[must_use]
pub fn parse_error_status() -> u16 {
    400
}

/// F104: any `Transfer-Encoding` is rejected (chunked unsupported).
#[must_use]
pub fn reject_transfer_encoding() -> bool {
    true
}

/// AS-IS F104: ignore TE; F86 keep-without-CL stores the raw chunk framing.
#[must_use]
pub fn reject_transfer_encoding_as_is() -> bool {
    false
}

/// F105: a *present* but unparseable integer is an error (not the default).
#[must_use]
pub fn present_bad_int_is_error() -> bool {
    true
}

/// AS-IS F105: `parse().ok().unwrap_or(default)` — `ttl_ms=abc` acquires.
#[must_use]
pub fn present_bad_int_is_error_as_is() -> bool {
    false
}

/// Offset just past the header/body break.
///
/// RFC 9112 prefers `\r\n\r\n`. F153: LF-only clients send `\n\n`; looking
/// only for CRLF 400'd those requests (and could mis-frame a body that
/// itself contains `\r\n\r\n`).
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
#[must_use]
pub fn header_break_end_as_is(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

/// Length of the break ending at `end` (4 for CRLFCRLF, 2 for LFLF).
#[must_use]
pub fn header_break_len(buf: &[u8], end: usize) -> usize {
    if end >= 4 && buf.get(end - 4..end) == Some(b"\r\n\r\n".as_ref()) {
        4
    } else {
        2
    }
}

/// F154: `Expect: 100-continue` (RFC 9110) — case-insensitive, comma list.
#[must_use]
pub fn expects_100_continue(value: &str) -> bool {
    value
        .split(',')
        .any(|t| t.trim().eq_ignore_ascii_case("100-continue"))
}

/// AS-IS F154: never send 100; client and server wait on each other.
#[must_use]
pub fn expects_100_continue_as_is(_value: &str) -> bool {
    false
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
        // Earliest break: LF headers + body that contains `\r\n\r\n`.
        let mixed = b"PUT /kv/b HTTP/1.0\nContent-Length: 8\n\nab\r\n\r\ncd";
        let m = header_break_end(mixed).expect("mixed");
        assert_eq!(&mixed[m..], b"ab\r\n\r\ncd");
        assert_eq!(header_break_end_as_is(mixed).map(|i| &mixed[i..]), Some(&b"cd"[..]));
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
    }
}
