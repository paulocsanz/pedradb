// Verus proof of the pure apply-loop decision (RFC-0053 Y2.2 / F10-apply).
//
// Source of truth for production remains `src/apply_kernel.rs` (same rules).
// This file is the machine-checked theorem: exec == spec, spec ⇒ applied
// stays a contiguous committed prefix, and the AS-IS skip-holes mutant
// violates it.
//
// Build/verify (requires verus on PATH, e.g. ~/.local/verus/verus-arm64-macos):
//   verus apply_advance.rs --crate-type=lib --multiple-errors 10
//
// Or: ../../../scripts/verus_apply_advance.sh
//
// Do not link this into the production crate — it is a twin of the pure kernel.

use vstd::prelude::*;

verus! {

/// Mirrors `ApplyAction` in apply_kernel.rs.
pub enum ApplyAction {
    Done,
    Stop,
    Apply,
}

/// Closed-form spec — same arms as `apply_kernel::apply_advance`.
pub open spec fn apply_advance_spec(
    last_applied: u64,
    commit_index: u64,
    entry_present: bool,
) -> ApplyAction {
    if last_applied >= commit_index {
        ApplyAction::Done
    } else if entry_present {
        ApplyAction::Apply
    } else {
        ApplyAction::Stop
    }
}

/// AS-IS mutant: skip holes — advance even when the entry is missing.
pub open spec fn apply_advance_as_is(
    last_applied: u64,
    commit_index: u64,
) -> ApplyAction {
    if last_applied >= commit_index {
        ApplyAction::Done
    } else {
        ApplyAction::Apply
    }
}

/// Executable decision — must match `pedradb_raft::apply_advance` bit-for-bit.
#[verifier::when_used_as_spec(apply_advance_spec)]
pub fn apply_advance(
    last_applied: u64,
    commit_index: u64,
    entry_present: bool,
) -> (a: ApplyAction)
    ensures
        a == apply_advance_spec(last_applied, commit_index, entry_present),
        (a == ApplyAction::Apply) ==> (last_applied < commit_index && entry_present),
        (a == ApplyAction::Done) ==> (last_applied >= commit_index),
        (a == ApplyAction::Stop) ==> (last_applied < commit_index && !entry_present),
{
    if last_applied >= commit_index {
        ApplyAction::Done
    } else if entry_present {
        ApplyAction::Apply
    } else {
        ApplyAction::Stop
    }
}

/// Named caller refinement (Y2.2): the apply loop only ever advances
/// `last_applied` while it is strictly behind `commit_index` **and** the
/// entry exists — applied ⊆ contiguous committed prefix (F10-apply).
proof fn lemma_apply_only_contiguous_committed_prefix(
    last_applied: u64,
    commit_index: u64,
    entry_present: bool,
)
    ensures
        apply_advance(last_applied, commit_index, entry_present) == ApplyAction::Apply
            ==> last_applied < commit_index && entry_present,
        apply_advance(last_applied, commit_index, entry_present) != ApplyAction::Apply
            || entry_present,
{
}

/// Mutant applies a hole (teeth): behind commit with no entry ⇒ fixed stops,
/// mutant applies — the state machine diverges from the committed log.
proof fn lemma_mutant_applies_holes(last_applied: u64, commit_index: u64)
    requires
        last_applied < commit_index,
    ensures
        apply_advance(last_applied, commit_index, false) == ApplyAction::Stop,
        apply_advance_as_is(last_applied, commit_index) == ApplyAction::Apply,
{
}

} // verus!
