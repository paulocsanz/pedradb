//! Pure AppendEntries log decisions (Beyond-style kernel, F16).
//!
//! # Contract
//!
//! - **No I/O, no clock, no RNG.**
//! - Production [`crate::rpc_append_entries`] / `handle_append_entries` calls these
//!   helpers for prev-log and conflict/append rules, then applies persist/commit.
//! - Stateright / Verus must call **this same module**, not a paraphrase.
//!
//! # Decision vs protocol
//!
//! | Piece | Where |
//! |-------|--------|
//! | prev-log match, conflict ≤ commit → refuse, contiguous append | this kernel |
//! | truncate/push entries, `persist_log`, advance commit | caller |
//! | log bytes durable / AE network | **axiom** — World / FailingEnv |
//!
//! Spec page: `determinismo/pedradb-dst/formal/F16-ae-conflict.md`.

#![forbid(unsafe_code)]

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
///
/// # Post-condition (F16 / theorem-ready)
///
/// ```text
/// ensures
///   (action == TruncateAndInstall) ==> (entry_index > commit_index)
///   (action == Append) ==> (existing_term.is_none()
///                           && entry_index == last_log_index + 1)
///   (existing conflict && entry_index <= commit_index) ==> action == Refuse
/// ```
///
/// Finite-domain check: [`tests::theorem_ae_f16_on_finite_domain`].
/// ∀u64 Verus twin: `crates/pedradb-raft/verus/ae_entry_action.rs`.
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

/// F48 protocol: AE `success: true` only if a dirty log was persisted.
///
/// `success ⇒ !log_dirty ∨ persist_ok`. Persist is an axiom (FailingEnv / det_io).
#[must_use]
pub fn ae_ack_success(log_dirty: bool, persist_ok: bool) -> bool {
    !log_dirty || persist_ok
}

/// AS-IS F48: always ack success (swallow persist). Mutant must fail the theorem.
#[must_use]
pub fn ae_ack_success_as_is(_log_dirty: bool, _persist_ok: bool) -> bool {
    true
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
    fn prev_log_empty_ok() {
        assert!(ae_prev_log_ok(0, 0, 0, 0));
    }

    #[test]
    fn prev_log_mismatch_term() {
        assert!(!ae_prev_log_ok(3, 5, 10, 4));
    }

    #[test]
    fn prev_log_too_short() {
        assert!(!ae_prev_log_ok(5, 1, 3, 1));
    }

    #[test]
    fn keep_same_index_term() {
        assert_eq!(
            ae_entry_action(2, 7, Some(7), 1, 5),
            AeEntryAction::Keep
        );
    }

    #[test]
    fn refuse_conflict_at_or_before_commit() {
        assert_eq!(
            ae_entry_action(1, 9, Some(3), 1, 5),
            AeEntryAction::Refuse
        );
        assert_eq!(
            ae_entry_action(1, 9, Some(3), 2, 5),
            AeEntryAction::Refuse
        );
    }

    #[test]
    fn truncate_conflict_after_commit() {
        assert_eq!(
            ae_entry_action(3, 9, Some(3), 1, 5),
            AeEntryAction::TruncateAndInstall
        );
    }

    #[test]
    fn append_contiguous() {
        assert_eq!(ae_entry_action(6, 2, None, 1, 5), AeEntryAction::Append);
    }

    #[test]
    fn refuse_hole() {
        assert_eq!(ae_entry_action(8, 2, None, 1, 5), AeEntryAction::Refuse);
    }

    #[test]
    fn as_is_mutant_rewrites_committed() {
        let fixed = ae_entry_action(1, 9, Some(3), 1, 5);
        let mutant = ae_entry_action_as_is_rewrite_committed(1, 9, Some(3), 1, 5);
        assert_eq!(fixed, AeEntryAction::Refuse);
        assert_eq!(mutant, AeEntryAction::TruncateAndInstall);
    }

    #[test]
    fn ack_clean_log_without_persist() {
        assert!(ae_ack_success(false, false));
        assert!(ae_ack_success(false, true));
    }

    #[test]
    fn ack_dirty_requires_persist() {
        assert!(ae_ack_success(true, true));
        assert!(!ae_ack_success(true, false));
    }

    #[test]
    fn as_is_swallows_persist_fail() {
        assert!(ae_ack_success_as_is(true, false));
        assert_ne!(ae_ack_success(true, false), ae_ack_success_as_is(true, false));
    }

    #[test]
    fn theorem_ae_ack_on_bool_domain() {
        for dirty in [false, true] {
            for ok in [false, true] {
                let d = ae_ack_success(dirty, ok);
                assert_eq!(d, !dirty || ok);
                assert!(!(d && dirty && !ok), "F48: success ∧ dirty ⇒ persist");
                if dirty && !ok {
                    assert!(ae_ack_success_as_is(dirty, ok));
                    assert!(!d);
                }
            }
        }
    }

    /// P1.4-style finite theorem for F16 safety predicates.
    #[test]
    fn theorem_ae_f16_on_finite_domain() {
        const B: u64 = 5;
        let mut n = 0u64;
        for entry_index in 0..B {
            for entry_term in 0..B {
                for commit_index in 0..B {
                    for last_log_index in 0..B {
                        for ex in 0..=B {
                            let existing = if ex == B { None } else { Some(ex) };
                            let act = ae_entry_action(
                                entry_index,
                                entry_term,
                                existing,
                                commit_index,
                                last_log_index,
                            );
                            assert!(
                                ae_f16_safe(
                                    entry_index,
                                    entry_term,
                                    existing,
                                    commit_index,
                                    last_log_index,
                                    act
                                ),
                                "F16 unsafe at idx={entry_index} term={entry_term} ex={existing:?} commit={commit_index} last={last_log_index} → {act:?}"
                            );
                            // Mutant must violate F16 on committed conflict.
                            if let Some(t) = existing {
                                if t != entry_term && entry_index <= commit_index {
                                    let m = ae_entry_action_as_is_rewrite_committed(
                                        entry_index,
                                        entry_term,
                                        existing,
                                        commit_index,
                                        last_log_index,
                                    );
                                    assert!(
                                        !ae_f16_safe(
                                            entry_index,
                                            entry_term,
                                            existing,
                                            commit_index,
                                            last_log_index,
                                            m
                                        ),
                                        "mutant must violate F16 here"
                                    );
                                }
                            }
                            n += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(n, B.pow(4) * (B + 1));
    }
}
