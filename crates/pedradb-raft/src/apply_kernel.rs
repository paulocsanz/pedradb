//! Pure apply-loop decisions (Beyond-style kernel, RFC-0053 Y2.2).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). No twin-cópia.
//!
//!   ./scripts/verus_apply_advance.sh
//!
//! # Contract
//!
//! - **No I/O, no clock, no RNG.**
//! - Production [`crate::RaftNode::apply_committed`] calls this kernel once
//!   per loop iteration to decide Done / Stop / Apply; only the caller mutates
//!   `last_applied` and the store.
//! - Stateright / Verus must call **this same module**, not a paraphrase.
//!
//! The rustc bodies stay token-identical with the clone in
//! `pedradb-store::apply_kernel` (`catalog` `apply_raft_store`). Verus proofs
//! sit in the `cfg(verus_keep_ghost)` block above them (last-wins for
//! the clone lint is the rustc body).
//!
//! # Decision vs protocol
//!
//! | Piece | Where |
//! |-------|--------|
//! | stop when caught up, stop on hole, advance one entry | this kernel |
//! | store write, DCS marker decode, `last_applied = next` | caller |
//! | store durability / apply idempotence across restart | **axiom** — DST / FailingEnv |
//!
//! Spec page: `docs/rfc/0053-ironfleet-years.md` (Y2.2), F10-apply.

#![forbid(unsafe_code)]

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

/// Mirrors the rustc `ApplyAction` below (cfg-split so Verus does not see Debug).
pub enum ApplyAction {
    Done,
    Stop,
    Apply,
}

/// Closed-form spec — same arms as production `apply_advance`.
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

/// Executable decision — must match the rustc `apply_advance` bit-for-bit.
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

/// What the apply loop does for `next = last_applied + 1`.
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyAction {
    /// `last_applied == commit_index` — caught up, exit the loop.
    Done,
    /// Entry at `next` is missing (log hole) — stop **without** advancing:
    /// the committed prefix must stay contiguous.
    Stop,
    /// Entry at `next` exists and `last_applied < commit_index` — apply it
    /// and advance `last_applied` by exactly one.
    Apply,
}

/// Pure rule for one apply-loop step (F10-apply: applied ⊆ committed prefix,
/// contiguous, one at a time).
///
/// # Post-condition (theorem-ready)
///
/// ```text
/// ensures
///   (action == Apply) ==> (last_applied < commit_index && entry_present)
///   (action == Done)  ==> (last_applied >= commit_index)
///   (action == Stop)  ==> (last_applied < commit_index && !entry_present)
/// ```
///
/// Finite-domain check: [`tests::theorem_apply_step_on_finite_domain`].
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn apply_advance(last_applied: u64, commit_index: u64, entry_present: bool) -> ApplyAction {
    if last_applied >= commit_index {
        ApplyAction::Done
    } else if entry_present {
        ApplyAction::Apply
    } else {
        ApplyAction::Stop
    }
}

/// AS-IS F10-apply: skip holes — advance even when the entry is missing.
/// The state machine silently diverges from the committed log (teeth for
/// Inv-apply-contiguous).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn apply_advance_as_is_skip_holes(
    last_applied: u64,
    commit_index: u64,
    _entry_present: bool,
) -> ApplyAction {
    if last_applied >= commit_index {
        ApplyAction::Done
    } else {
        ApplyAction::Apply
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caught_up_is_done() {
        assert_eq!(apply_advance(3, 3, true), ApplyAction::Done);
        assert_eq!(apply_advance(4, 3, true), ApplyAction::Done);
        assert_eq!(apply_advance(3, 3, false), ApplyAction::Done);
    }

    #[test]
    fn present_and_behind_applies() {
        assert_eq!(apply_advance(2, 5, true), ApplyAction::Apply);
        assert_eq!(apply_advance(0, 1, true), ApplyAction::Apply);
    }

    #[test]
    fn hole_stops_not_skips() {
        assert_eq!(apply_advance(2, 5, false), ApplyAction::Stop);
    }

    #[test]
    fn as_is_mutant_skips_holes() {
        let fixed = apply_advance(2, 5, false);
        let mutant = apply_advance_as_is_skip_holes(2, 5, false);
        assert_eq!(fixed, ApplyAction::Stop);
        assert_eq!(mutant, ApplyAction::Apply);
        assert_ne!(fixed, mutant, "mutant must differ exactly on holes");
    }

    /// Finite-domain theorem: Apply ⇒ behind commit ∧ entry present; the
    /// mutant violates it on every hole.
    #[test]
    fn theorem_apply_step_on_finite_domain() {
        const B: u64 = 4;
        for last_applied in 0..B {
            for commit_index in 0..B {
                for present in [false, true] {
                    let a = apply_advance(last_applied, commit_index, present);
                    if matches!(a, ApplyAction::Apply) {
                        assert!(last_applied < commit_index, "never apply caught-up");
                        assert!(present, "never apply a hole");
                    }
                    if matches!(a, ApplyAction::Done) {
                        assert!(last_applied >= commit_index);
                    }
                    if matches!(a, ApplyAction::Stop) {
                        assert!(last_applied < commit_index && !present);
                    }
                    if last_applied < commit_index && !present {
                        let m = apply_advance_as_is_skip_holes(last_applied, commit_index, present);
                        assert_eq!(
                            m,
                            ApplyAction::Apply,
                            "mutant must apply the hole here"
                        );
                    }
                }
            }
        }
    }
}
