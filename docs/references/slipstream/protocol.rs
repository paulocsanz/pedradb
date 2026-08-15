//! Protocol decision kernels — the load-bearing guards of the snapshot
//! export/import protocol, extracted as pure functions so the PRODUCTION
//! code and the exhaustive model checker (`tests/model.rs`) execute the
//! **same logic**, not two hand-synchronized copies of it.
//!
//! Every function here is a guard whose correctness the machine-checked
//! theorems depend on. The call sites:
//!
//! | kernel                     | production                          | model              |
//! |----------------------------|-------------------------------------|--------------------|
//! | [`pointer_publish_allowed`]| `transport::swap_pointer`           | `Act::Publish`     |
//! | [`payload_prunable`]       | `transport::ObjectStoreTransport::prune` | `Act::Prune`  |
//! | [`resume_window_ok`]       | `nats` resume paths (`check_resume_window`) | `Act::Resume` |
//!
//! Because the model transitions call these very functions, a change to any
//! guard is re-verified against the full bounded state space on the next
//! `cargo test --test model` — the guards cannot drift from the proof. The
//! mutation tests in `tests/model.rs` additionally prove each guard is
//! load-bearing: substituting a broken variant makes the checker produce a
//! counterexample.
//!
//! Kernels operate on plain `u64` ranks (a [`WatchCursor`](crate::WatchCursor)'s
//! revision, with revisionless cursors ranked 0 by the callers) so they stay
//! free of I/O types and usable from the checker's `u8`-bounded state space.

/// What the publisher observed at the pointer key before deciding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerState {
    /// No pointer object exists — the slot is open (create-only publish).
    Absent,
    /// A pointer object exists. `rank` is its cursor's rank, or `None` when
    /// the object is unparseable — a corrupt pointer MUST be replaceable, or
    /// one bad write wedges publishing forever (the same rule as the export
    /// lease's corrupt-steal).
    Present {
        /// Rank of the existing pointer's cursor; `None` if unparseable.
        rank: Option<u64>,
    },
}

/// THE monotonic pointer guard: may `candidate_rank` be published over the
/// observed `current` pointer?
///
/// `true` for an open slot, a corrupt pointer, or a candidate at or above
/// the existing cursor; `false` exactly when the existing pointer is
/// parseable and STRICTLY newer — the refusal that makes a slow exporter's
/// stale publish a no-op instead of a regression.
///
/// Soundness of deciding on a read (before the conditional put): every
/// writer uses this guard with a compare-and-swap, so the pointer's rank is
/// monotone non-decreasing — once "strictly newer" is observed, it can never
/// become false, so refusal needs no CAS. Machine-checked as `published
/// cursor never regresses` in `tests/model.rs`.
pub fn pointer_publish_allowed(current: &PointerState, candidate_rank: u64) -> bool {
    match current {
        PointerState::Absent => true,
        PointerState::Present { rank: None } => true,
        PointerState::Present {
            rank: Some(existing),
        } => candidate_rank >= *existing,
    }
}

/// THE prune guard: may this payload object be deleted, given the current
/// pointer's rank?
///
/// A payload is prunable only when ALL hold:
/// - it is not the pointer's own target;
/// - its rank is parseable AND **strictly below** the pointer's (an
///   unparseable rank is never deleted — unknown objects are not ours);
/// - its age has cleared the grace period (`aged_out`; the model passes
///   `true`, checking the harshest zero-grace timing).
///
/// Strictly-below is what makes a dangling pointer impossible regardless of
/// timing: [`pointer_publish_allowed`] refuses any candidate below the
/// pointer, and the pointer is monotone — so anything this guard deletes
/// (rank < pointer-at-prune ≤ pointer-at-any-later-swap) can never be
/// successfully published afterward. The model checker FOUND the dangling
/// counterexample under the earlier age-only rule (a same-cursor payload
/// collected mid-publish, then published by the `>=` swap guard); this rule
/// is the structural fix, machine-checked as `pointer target always
/// fetchable` under zero-grace pruning.
pub fn payload_prunable(
    payload_rank: Option<u64>,
    pointer_rank: u64,
    is_pointer_target: bool,
    aged_out: bool,
) -> bool {
    !is_pointer_target && aged_out && payload_rank.is_some_and(|rank| rank < pointer_rank)
}

/// THE cursor-expiry guard: is resuming from `revision` sound, given the
/// stream's first retained sequence?
///
/// A resume reads `revision + 1` onward; it is sound iff nothing at or below
/// `revision + 1`'s predecessor gap has been head-evicted — i.e.
/// `first_sequence <= revision + 1`. Interior (per-subject) eviction inside
/// the gap is safe for a last-write-wins fold (an overwrite-evicted revision
/// implies a later revision of the same subject exists and will be
/// delivered); lost DELETES come from head eviction, which is exactly what
/// advances `first_sequence`.
///
/// This check must be performed by US: NATS does not error on a below-head
/// start sequence — it silently clamps to the first retained message
/// (pinned live by `tests/resync.rs::nats_silently_clamps_resume_below_first_seq`),
/// which would skip the gap's evicted delete markers with no fallback and no
/// resync. Machine-checked as `bootstrap never silently diverges` in
/// `tests/model.rs` (where the model's retention floor is
/// `first_sequence - 1`).
pub fn resume_window_ok(revision: u64, first_sequence: u64) -> bool {
    first_sequence <= revision.saturating_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_guard_boundaries() {
        let absent = PointerState::Absent;
        let corrupt = PointerState::Present { rank: None };
        let at = |r| PointerState::Present { rank: Some(r) };
        assert!(pointer_publish_allowed(&absent, 0));
        assert!(pointer_publish_allowed(&corrupt, 0));
        assert!(pointer_publish_allowed(&at(5), 5), "equal republishes");
        assert!(pointer_publish_allowed(&at(5), 6));
        assert!(!pointer_publish_allowed(&at(5), 4), "stale is refused");
    }

    #[test]
    fn prune_guard_boundaries() {
        // Strictly below, aged, not the target: prunable.
        assert!(payload_prunable(Some(4), 5, false, true));
        // Equal rank is NOT prunable — it is still publishable (>= guard).
        assert!(!payload_prunable(Some(5), 5, false, true));
        assert!(!payload_prunable(Some(6), 5, false, true));
        // The pointer's own target and unparseable ranks are never prunable.
        assert!(!payload_prunable(Some(4), 5, true, true));
        assert!(!payload_prunable(None, 5, false, true));
        // Grace window holds everything.
        assert!(!payload_prunable(Some(4), 5, false, false));
    }

    #[test]
    fn resume_guard_boundaries() {
        assert!(resume_window_ok(3, 4), "first retained == next read: sound");
        assert!(resume_window_ok(3, 1), "history intact");
        assert!(!resume_window_ok(3, 5), "gap head-evicted: expired");
        assert!(resume_window_ok(u64::MAX, u64::MAX), "saturating boundary");
    }
}
