//! Pure AE persist-before-success and F16 log-entry decisions (RFC-0002 P5.2 / F48 / RFC-0152 F16).
//!
//! Same rules as `pedradb-raft::ae_kernel` (`ae_ack_success`, `ae_entry_action`,
//! `ae_prev_log_ok`). Store does not depend on `pedradb-raft`; keep the bodies
//! identical (drift-trap: harness grid).

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

/// Whether the follower's log matches the leader's `prev_log_*` (Raft §5.3).
///
/// `log_term_at_prev` is `None` if the follower has no entry at `prev_log_index`
/// (including when `prev_log_index == 0`, which always matches — pass `Some(0)`
/// or use [`ae_prev_log_ok`] with `prev_log_index == 0` short-circuit).
#[must_use]
pub fn ae_prev_log_ok(
    prev_log_index: u64,
    prev_log_term: u64,
    last_log_index: u64,
    log_term_at_prev: u64,
) -> bool {
    if prev_log_index == 0 {
        return true;
    }
    if last_log_index < prev_log_index {
        return false;
    }
    log_term_at_prev == prev_log_term
}

/// What to do with one leader log entry relative to the follower log (F16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AeEntryAction {
    /// Same index+term already present — leave suffix alone for now.
    Keep,
    /// No local entry; index is exactly `last_log_index + 1` — append.
    Append,
    /// Term conflict at an **uncommitted** index — truncate from here and install.
    TruncateAndInstall,
    /// Refuse the whole AE (committed rewrite, or non-contiguous hole).
    Refuse,
}

/// Pure rule for one AE log entry against follower state.
#[must_use]
pub fn ae_entry_action(
    entry_index: u64,
    entry_term: u64,
    // Term of existing follower entry at `entry_index`, if any.
    existing_term: Option<u64>,
    commit_index: u64,
    last_log_index: u64,
) -> AeEntryAction {
    match existing_term {
        Some(t) if t == entry_term => AeEntryAction::Keep,
        Some(_) => {
            // Term conflict.
            if entry_index <= commit_index {
                AeEntryAction::Refuse
            } else {
                AeEntryAction::TruncateAndInstall
            }
        }
        None => {
            let expect = last_log_index.saturating_add(1);
            if entry_index != expect {
                AeEntryAction::Refuse
            } else {
                AeEntryAction::Append
            }
        }
    }
}

/// AS-IS mutant: always truncate-and-install on term conflict, **even at/before
/// commit** (the F16 bug). Used only to prove the fixed rule has teeth.
#[must_use]
pub fn ae_entry_action_as_is_rewrite_committed(
    entry_index: u64,
    entry_term: u64,
    existing_term: Option<u64>,
    _commit_index: u64,
    last_log_index: u64,
) -> AeEntryAction {
    match existing_term {
        Some(t) if t == entry_term => AeEntryAction::Keep,
        Some(_) => AeEntryAction::TruncateAndInstall, // BUG: ignores commit
        None => {
            let expect = last_log_index.saturating_add(1);
            if entry_index != expect {
                AeEntryAction::Refuse
            } else {
                AeEntryAction::Append
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_persist_fail_is_not_success() {
        assert!(!ae_ack_success(true, false));
        assert!(ae_ack_success_as_is(true, false));
    }

    #[test]
    fn as_is_mutant_rewrites_committed() {
        let fixed = ae_entry_action(1, 9, Some(3), 1, 5);
        let mutant = ae_entry_action_as_is_rewrite_committed(1, 9, Some(3), 1, 5);
        assert_eq!(fixed, AeEntryAction::Refuse);
        assert_eq!(mutant, AeEntryAction::TruncateAndInstall);
    }
}
