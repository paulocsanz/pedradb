//! Pure AE persist-before-success decision (RFC-0002 P5.2 / F48).
//!
//! Same rule as `pedradb-raft::ae_ack_success`. Store does not depend on
//! `pedradb-raft`; keep the two bodies identical (drift-trap: harness grid).

#![forbid(unsafe_code)]

/// F48: AE `success: true` only if a dirty log was persisted.
#[must_use]
pub fn ae_ack_success(log_dirty: bool, persist_ok: bool) -> bool {
    !log_dirty || persist_ok
}

/// AS-IS F48: always ack success (swallow persist).
#[must_use]
pub fn ae_ack_success_as_is(_log_dirty: bool, _persist_ok: bool) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_persist_fail_is_not_success() {
        assert!(!ae_ack_success(true, false));
        assert!(ae_ack_success_as_is(true, false));
    }
}
