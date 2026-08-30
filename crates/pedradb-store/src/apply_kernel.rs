//! Pure apply-loop decisions (RFC-0152 P1.36 / F10-apply).
//!
//! Same rules as `pedradb-raft::apply_kernel`. Store does not depend on
//! `pedradb-raft`; keep the two bodies identical (drift-trap: harness grid).

#![forbid(unsafe_code)]

/// What the apply loop does for `next = last_applied + 1`.
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
    fn hole_stops_not_skips() {
        assert_eq!(apply_advance(2, 5, false), ApplyAction::Stop);
        assert_eq!(
            apply_advance_as_is_skip_holes(2, 5, false),
            ApplyAction::Apply
        );
    }
}
