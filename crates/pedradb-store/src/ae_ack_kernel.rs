//! Pure AE persist-before-success and F16 log-entry decisions (RFC-0002 P5.2 / F48 / RFC-0152 F16).
//!
//! Same rules as `pedradb-raft::ae_kernel` (`ae_ack_success`, `ae_entry_action`,
//! `ae_prev_log_ok`). Production code must not depend on `pedradb-raft` (it is
//! a dev-dependency only); token identity is frozen by `pedra_formal.py --ci`
//! (check_clones) and same-function agreement is pinned by the cross-crate
//! twin test below, so a drift on either side breaks `cargo test`, not just
//! the lint.

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

/// Spec predicates for F16 safety (used by finite-domain theorem).
#[must_use]
pub fn ae_f16_safe(
    entry_index: u64,
    entry_term: u64,
    existing_term: Option<u64>,
    commit_index: u64,
    last_log_index: u64,
    action: AeEntryAction,
) -> bool {
    let _ = entry_term;
    // Never truncate/install at or before commit.
    if matches!(action, AeEntryAction::TruncateAndInstall) && entry_index <= commit_index {
        return false;
    }
    // Append only contiguous hole-free.
    if matches!(action, AeEntryAction::Append) {
        if existing_term.is_some() {
            return false;
        }
        if entry_index != last_log_index.saturating_add(1) {
            return false;
        }
    }
    // Conflict on committed index must Refuse (when existing differs).
    if let Some(t) = existing_term {
        if t != entry_term && entry_index <= commit_index {
            return matches!(action, AeEntryAction::Refuse);
        }
    }
    true
}

/// AS-IS F16: vacuous always-true safety gate (no check at all). Mutant must
/// fail the theorem — it passes a committed rewrite the production gate rejects.
#[must_use]
pub fn ae_f16_safe_as_is(
    _entry_index: u64,
    _entry_term: u64,
    _existing_term: Option<u64>,
    _commit_index: u64,
    _last_log_index: u64,
    _action: AeEntryAction,
) -> bool {
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

    #[test]
    fn as_is_mutant_rewrites_committed() {
        let fixed = ae_entry_action(1, 9, Some(3), 1, 5);
        let mutant = ae_entry_action_as_is_rewrite_committed(1, 9, Some(3), 1, 5);
        assert_eq!(fixed, AeEntryAction::Refuse);
        assert_eq!(mutant, AeEntryAction::TruncateAndInstall);
        assert!(!ae_f16_safe(1, 9, Some(3), 1, 5, mutant));
        assert!(ae_f16_safe_as_is(1, 9, Some(3), 1, 5, mutant));
        assert!(ae_f16_safe(1, 9, Some(3), 1, 5, fixed));
    }

    /// Clone twin (catalog `ae_ack_raft_store`): both copies must implement
    /// the same function. Token identity (lint) catches one-sided drift;
    /// this live cross-crate sweep also catches both-sides drift at
    /// `cargo test` time. `AeEntryAction` is a distinct enum per crate, so
    /// actions are compared via a common discriminant.
    #[test]
    fn twin_agrees_with_raft_ae_kernel_on_full_domain() {
        use pedradb_raft::ae_kernel as raft;
        let disc = |a: AeEntryAction| -> u8 {
            match a {
                AeEntryAction::Keep => 0,
                AeEntryAction::Append => 1,
                AeEntryAction::TruncateAndInstall => 2,
                AeEntryAction::Refuse => 3,
            }
        };
        let disc_raft = |a: raft::AeEntryAction| -> u8 {
            match a {
                raft::AeEntryAction::Keep => 0,
                raft::AeEntryAction::Append => 1,
                raft::AeEntryAction::TruncateAndInstall => 2,
                raft::AeEntryAction::Refuse => 3,
            }
        };
        let bb = [false, true];
        let u = [0u64, 1, 2, 3, 5, u64::MAX];
        let t = [0u64, 1, 3, 5];
        let o = [None, Some(0u64), Some(1), Some(3), Some(5)];
        let mut checked = 0usize;

        for (n, f, g) in [
            (
                "ae_ack_success",
                ae_ack_success as fn(bool, bool) -> bool,
                raft::ae_ack_success as fn(bool, bool) -> bool,
            ),
            (
                "ae_ack_success_as_is",
                ae_ack_success_as_is as fn(bool, bool) -> bool,
                raft::ae_ack_success_as_is as fn(bool, bool) -> bool,
            ),
        ] {
            for &d in &bb {
                for &p in &bb {
                    assert_eq!(f(d, p), g(d, p), "{n}({d},{p})");
                    checked += 1;
                }
            }
        }
        for &pi in &u {
            for &pt in &t {
                for &last in &u {
                    for &term in &t {
                        assert_eq!(
                            ae_prev_log_ok(pi, pt, last, term),
                            raft::ae_prev_log_ok(pi, pt, last, term),
                            "ae_prev_log_ok({pi},{pt},{last},{term})"
                        );
                        checked += 1;
                    }
                }
            }
        }
        for (n, f, g) in [
            (
                "ae_entry_action",
                ae_entry_action as fn(u64, u64, Option<u64>, u64, u64) -> AeEntryAction,
                raft::ae_entry_action
                    as fn(u64, u64, Option<u64>, u64, u64) -> raft::AeEntryAction,
            ),
            (
                "ae_entry_action_as_is_rewrite_committed",
                ae_entry_action_as_is_rewrite_committed
                    as fn(u64, u64, Option<u64>, u64, u64) -> AeEntryAction,
                raft::ae_entry_action_as_is_rewrite_committed
                    as fn(u64, u64, Option<u64>, u64, u64) -> raft::AeEntryAction,
            ),
        ] {
            for &idx in &u {
                for &term in &t {
                    for &ex in &o {
                        for &commit in &u {
                            for &last in &u {
                                let xs = match ex {
                                    None => "None".to_string(),
                                    Some(v) => format!("Some({v})"),
                                };
                                assert_eq!(
                                    disc(f(idx, term, ex, commit, last)),
                                    disc_raft(g(idx, term, ex, commit, last)),
                                    "{n}({idx},{term},{xs},{commit},{last})"
                                );
                                checked += 1;
                            }
                        }
                    }
                }
            }
        }
        let store_acts = [
            AeEntryAction::Keep,
            AeEntryAction::Append,
            AeEntryAction::TruncateAndInstall,
            AeEntryAction::Refuse,
        ];
        let raft_acts = [
            raft::AeEntryAction::Keep,
            raft::AeEntryAction::Append,
            raft::AeEntryAction::TruncateAndInstall,
            raft::AeEntryAction::Refuse,
        ];
        for &idx in &u {
            for &term in &t {
                for &ex in &o {
                    for &commit in &u {
                        for &last in &u {
                            for i in 0..4 {
                                assert_eq!(
                                    ae_f16_safe(idx, term, ex, commit, last, store_acts[i]),
                                    raft::ae_f16_safe(idx, term, ex, commit, last, raft_acts[i]),
                                    "ae_f16_safe({idx},{term},{ex:?},{commit},{last},{i})"
                                );
                                assert_eq!(
                                    ae_f16_safe_as_is(idx, term, ex, commit, last, store_acts[i]),
                                    raft::ae_f16_safe_as_is(idx, term, ex, commit, last, raft_acts[i]),
                                    "ae_f16_safe_as_is({idx},{term},{ex:?},{commit},{last},{i})"
                                );
                                checked += 2;
                            }
                        }
                    }
                }
            }
        }
        // 2 fns × 2×2 bools + 1 fn × 6·4·6·4 + 2 fns × 6·4·5·6·6 + 2 fns × 6·4·5·6·6 × 4 acts.
        assert_eq!(checked, 8 + 576 + 8640 + 34560);
    }

    /// Mirror of the raft-side F16 finite-domain theorem on THIS copy:
    /// post-conditions of `ae_entry_action` hold on the swept domain, and
    /// the as-is mutant violates the committed-conflict one.
    #[test]
    fn theorem_ae_f16_postconditions_on_finite_domain() {
        let u = [0u64, 1, 2, 3, 5, u64::MAX];
        let t = [0u64, 1, 3, 5];
        let o = [None, Some(0u64), Some(1), Some(3), Some(5)];
        let mut n = 0usize;
        for &idx in &u {
            for &term in &t {
                for &ex in &o {
                    for &commit in &u {
                        for &last in &u {
                            let act = ae_entry_action(idx, term, ex, commit, last);
                            if matches!(act, AeEntryAction::TruncateAndInstall) {
                                assert!(idx > commit, "F16: never truncate at/before commit");
                            }
                            if matches!(act, AeEntryAction::Append) {
                                assert!(ex.is_none(), "F16: append only into a hole");
                                assert_eq!(idx, last.saturating_add(1), "F16: append only contiguous");
                            }
                            if let Some(v) = ex {
                                if v != term && idx <= commit {
                                    assert_eq!(act, AeEntryAction::Refuse, "F16: committed rewrite refused");
                                    assert_eq!(
                                        ae_entry_action_as_is_rewrite_committed(idx, term, ex, commit, last),
                                        AeEntryAction::TruncateAndInstall,
                                        "mutant must have teeth here"
                                    );
                                }
                            }
                            n += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(n, u.len() * t.len() * o.len() * u.len() * u.len());
    }
}
