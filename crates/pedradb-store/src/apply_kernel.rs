//! Pure apply-loop decisions (RFC-0152 P1.36 / F10-apply).
//!
//! Same rules as `pedradb-raft::apply_kernel`. Production code must not
//! depend on `pedradb-raft` (it is a dev-dependency only); token identity
//! is frozen by `pedra_formal.py --ci` (check_clones) and same-function
//! agreement is pinned by the cross-crate twin test below, so a drift on
//! either side breaks `cargo test`, not just the lint.

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

/// Where an applied `Put` record lands (RFC-0191 P2.3 cadence, second if:
/// the record-dispatch fate of the apply path). Reserved keys never
/// receive user data; the gen-0 floor never persists hist (F52 on the
/// apply side); any other gen persists hist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyPutFate {
    /// Reserved store key (TX/SI machinery) — the record is dropped.
    Skip,
    /// Gen-0 floor — user data only, no hist write.
    ApplyOnly,
    /// User data plus the SI-hist entry for this gen.
    ApplyAndHist,
}

/// Pure rule for one applied Put record: `Skip` iff reserved,
/// `ApplyAndHist` iff live and gen > 0, else `ApplyOnly`.
#[must_use]
pub fn apply_put_plan(is_reserved: bool, si_gen: u64) -> ApplyPutFate {
    if is_reserved {
        ApplyPutFate::Skip
    } else if si_gen == 0 {
        ApplyPutFate::ApplyOnly
    } else {
        ApplyPutFate::ApplyAndHist
    }
}

/// AS-IS: stomp reserved keys with applied user data (drops the TX/SI
/// key protection and the gen-0 floor).
#[must_use]
pub fn apply_put_plan_as_is(_is_reserved: bool, _si_gen: u64) -> ApplyPutFate {
    ApplyPutFate::ApplyAndHist
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

    /// RFC-0191 P2.3: the apply-path Put fate — reserved keys are dropped
    /// (any gen), the gen-0 floor never persists hist, a live key at
    /// gen > 0 persists hist; the AS-IS dente stomps reserved keys.
    #[test]
    fn apply_put_plan_on_live_reserved_and_floor() {
        assert_eq!(
            apply_put_plan(true, 5),
            ApplyPutFate::Skip,
            "reserved key never receives applied data"
        );
        assert_eq!(apply_put_plan(true, 0), ApplyPutFate::Skip);
        assert_eq!(
            apply_put_plan(false, 0),
            ApplyPutFate::ApplyOnly,
            "gen-0 floor never persists hist"
        );
        assert_eq!(apply_put_plan(false, 7), ApplyPutFate::ApplyAndHist);
        assert_eq!(apply_put_plan_as_is(true, 5), ApplyPutFate::ApplyAndHist);
    }

    /// Clone twin (catalog `apply_raft_store`): both copies must implement
    /// the same function. `ApplyAction` is a distinct enum per crate, so
    /// results are compared via a common discriminant.
    #[test]
    fn twin_agrees_with_raft_apply_kernel_on_full_domain() {
        use pedradb_raft::apply_kernel as raft;
        let disc = |a: ApplyAction| -> u8 {
            match a {
                ApplyAction::Done => 0,
                ApplyAction::Stop => 1,
                ApplyAction::Apply => 2,
            }
        };
        let disc_r = |a: raft::ApplyAction| -> u8 {
            match a {
                raft::ApplyAction::Done => 0,
                raft::ApplyAction::Stop => 1,
                raft::ApplyAction::Apply => 2,
            }
        };
        let u = [0u64, 1, 2, 3, 5, u64::MAX];
        let mut checked = 0usize;
        for (n, f, g) in [
            (
                "apply_advance",
                apply_advance as fn(u64, u64, bool) -> ApplyAction,
                raft::apply_advance as fn(u64, u64, bool) -> raft::ApplyAction,
            ),
            (
                "apply_advance_as_is_skip_holes",
                apply_advance_as_is_skip_holes as fn(u64, u64, bool) -> ApplyAction,
                raft::apply_advance_as_is_skip_holes as fn(u64, u64, bool) -> raft::ApplyAction,
            ),
        ] {
            for &la in &u {
                for &ci in &u {
                    for present in [false, true] {
                        assert_eq!(
                            disc(f(la, ci, present)),
                            disc_r(g(la, ci, present)),
                            "{n}({la},{ci},{present})"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 2 * 6 * 6 * 2);
    }

    /// Mirror of the raft-side finite-domain theorem on THIS copy:
    /// Apply ⇒ behind commit ∧ entry present; Stop ⇒ behind ∧ hole;
    /// Done ⇔ caught up; the as-is mutant applies into holes.
    #[test]
    fn theorem_apply_step_on_finite_domain() {
        let u = [0u64, 1, 2, 3, 5, u64::MAX];
        for &la in &u {
            for &ci in &u {
                for present in [false, true] {
                    let act = apply_advance(la, ci, present);
                    match act {
                        ApplyAction::Done => assert!(la >= ci, "Done needs caught-up"),
                        ApplyAction::Stop => {
                            assert!(la < ci, "Stop needs behind-commit");
                            assert!(!present, "F10: stop only on a hole");
                        }
                        ApplyAction::Apply => {
                            assert!(la < ci, "F10: apply only behind commit");
                            assert!(present, "F10: apply only when present");
                        }
                    }
                    if la < ci && !present {
                        assert_eq!(act, ApplyAction::Stop);
                        assert_eq!(
                            apply_advance_as_is_skip_holes(la, ci, present),
                            ApplyAction::Apply,
                            "mutant must skip the hole (teeth)"
                        );
                    }
                }
            }
        }
    }
}
