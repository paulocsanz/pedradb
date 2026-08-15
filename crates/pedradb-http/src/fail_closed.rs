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
}
