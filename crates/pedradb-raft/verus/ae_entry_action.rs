// Verus proof of the pure AppendEntries entry decision (RFC-0002 P4.1 / F16).
//
// Source of truth for production remains `src/ae_kernel.rs` (same rules).
// This file is the machine-checked theorem: exec == spec, and spec ⇒ F16.
//
// Build/verify (requires verus on PATH, e.g. ~/.local/verus/verus-arm64-macos):
//   verus ae_entry_action.rs --crate-type=lib --multiple-errors 10
//
// Or: ../../../scripts/verus_ae_entry_action.sh
//
// Do not link this into the production crate — it is a twin of the pure kernel.

use vstd::prelude::*;

verus! {

/// Mirrors `AeEntryAction` in ae_kernel.rs.
pub enum AeEntryAction {
    Keep,
    Append,
    TruncateAndInstall,
    Refuse,
}

/// Faithful `u64::saturating_add(1)` (production uses saturating_add).
pub open spec fn sat_add1(x: u64) -> u64 {
    if x == u64::MAX {
        x
    } else {
        (x + 1) as u64
    }
}

/// Closed-form spec — same arms as `ae_kernel::ae_entry_action`.
pub open spec fn ae_entry_action_spec(
    entry_index: u64,
    entry_term: u64,
    existing_term: Option<u64>,
    commit_index: u64,
    last_log_index: u64,
) -> AeEntryAction {
    match existing_term {
        Some(t) => {
            if t == entry_term {
                AeEntryAction::Keep
            } else if entry_index <= commit_index {
                AeEntryAction::Refuse
            } else {
                AeEntryAction::TruncateAndInstall
            }
        },
        None => {
            if entry_index != sat_add1(last_log_index) {
                AeEntryAction::Refuse
            } else {
                AeEntryAction::Append
            }
        },
    }
}

/// F16 safety predicates (mirrors `ae_kernel::ae_f16_safe`).
pub open spec fn ae_f16_safe(
    entry_index: u64,
    entry_term: u64,
    existing_term: Option<u64>,
    commit_index: u64,
    last_log_index: u64,
    action: AeEntryAction,
) -> bool {
    &&& !(action == AeEntryAction::TruncateAndInstall && entry_index <= commit_index)
    &&& (action == AeEntryAction::Append ==>
            existing_term.is_none() && entry_index == sat_add1(last_log_index))
    &&& (match existing_term {
            Some(t) => {
                t != entry_term && entry_index <= commit_index ==>
                    action == AeEntryAction::Refuse
            },
            None => true,
        })
}

/// AS-IS mutant: truncate on any term conflict, even at/before commit (F16 bug).
pub open spec fn ae_entry_action_as_is(
    entry_index: u64,
    entry_term: u64,
    existing_term: Option<u64>,
    last_log_index: u64,
) -> AeEntryAction {
    match existing_term {
        Some(t) => {
            if t == entry_term {
                AeEntryAction::Keep
            } else {
                AeEntryAction::TruncateAndInstall
            }
        },
        None => {
            if entry_index != sat_add1(last_log_index) {
                AeEntryAction::Refuse
            } else {
                AeEntryAction::Append
            }
        },
    }
}

/// Executable decision — must match `pedradb_raft::ae_entry_action` bit-for-bit.
pub fn ae_entry_action(
    entry_index: u64,
    entry_term: u64,
    existing_term: Option<u64>,
    commit_index: u64,
    last_log_index: u64,
) -> (d: AeEntryAction)
    ensures
        d == ae_entry_action_spec(
            entry_index,
            entry_term,
            existing_term,
            commit_index,
            last_log_index,
        ),
        ae_f16_safe(
            entry_index,
            entry_term,
            existing_term,
            commit_index,
            last_log_index,
            d,
        ),
        (d == AeEntryAction::TruncateAndInstall) ==> (entry_index > commit_index),
        (d == AeEntryAction::Append) ==> (
            existing_term.is_none() && entry_index == sat_add1(last_log_index)
        ),
{
    if let Some(t) = existing_term {
        if t == entry_term {
            AeEntryAction::Keep
        } else if entry_index <= commit_index {
            AeEntryAction::Refuse
        } else {
            AeEntryAction::TruncateAndInstall
        }
    } else {
        let expect = if last_log_index == u64::MAX {
            last_log_index
        } else {
            last_log_index + 1
        };
        if entry_index != expect {
            AeEntryAction::Refuse
        } else {
            AeEntryAction::Append
        }
    }
}

/// Prev-log house (Raft §5.3) — same short-circuit as production.
pub fn ae_prev_log_ok(
    prev_log_index: u64,
    prev_log_term: u64,
    last_log_index: u64,
    log_term_at_prev: u64,
) -> (ok: bool)
    ensures
        ok == (prev_log_index == 0 || (last_log_index >= prev_log_index
            && log_term_at_prev == prev_log_term)),
{
    if prev_log_index == 0 {
        true
    } else if last_log_index < prev_log_index {
        false
    } else {
        log_term_at_prev == prev_log_term
    }
}

/// Spec always satisfies F16 (independent of exec).
proof fn lemma_spec_is_f16_safe(
    entry_index: u64,
    entry_term: u64,
    existing_term: Option<u64>,
    commit_index: u64,
    last_log_index: u64,
)
    ensures
        ae_f16_safe(
            entry_index,
            entry_term,
            existing_term,
            commit_index,
            last_log_index,
            ae_entry_action_spec(
                entry_index,
                entry_term,
                existing_term,
                commit_index,
                last_log_index,
            ),
        ),
{
}

/// Mutant violates F16 on committed term conflict (teeth).
proof fn lemma_mutant_violates_committed_conflict(
    entry_index: u64,
    entry_term: u64,
    existing: u64,
    commit_index: u64,
    last_log_index: u64,
)
    requires
        existing != entry_term,
        entry_index <= commit_index,
    ensures
        ae_entry_action_as_is(
            entry_index,
            entry_term,
            Some(existing),
            last_log_index,
        ) == AeEntryAction::TruncateAndInstall,
        !ae_f16_safe(
            entry_index,
            entry_term,
            Some(existing),
            commit_index,
            last_log_index,
            AeEntryAction::TruncateAndInstall,
        ),
{
}

} // verus!
