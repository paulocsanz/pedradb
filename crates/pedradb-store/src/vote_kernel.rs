//! Pure RequestVote decision (RFC-0152 P0 / F15).
//!
//! Same rules as `pedradb-raft::vote_kernel`. Production code must not
//! depend on `pedradb-raft` (it is a dev-dependency only); token identity
//! is frozen by `pedra_formal.py --ci` (check_clones) and same-function
//! agreement is pinned by the cross-crate twin test below, so a drift on
//! either side breaks `cargo test`, not just the lint.

#![forbid(unsafe_code)]

/// Inputs for a RequestVote decision after the follower has already adopted
/// `args.term` into hard state when `args.term > current_term` (caller-side).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoteInputs {
    /// Follower current term (post step-down if candidate term was higher).
    pub current_term: u64,
    /// Follower `voted_for` in this term (`None` if free or just stepped down).
    pub voted_for: Option<u64>,
    /// Follower last log term (0 if empty).
    pub last_log_term: u64,
    /// Follower last log index (0 if empty).
    pub last_log_index: u64,
    /// Candidate term.
    pub candidate_term: u64,
    /// Candidate id.
    pub candidate_id: u64,
    /// Candidate last log term.
    pub candidate_last_log_term: u64,
    /// Candidate last log index.
    pub candidate_last_log_index: u64,
}

/// Outcome of the pure vote rule (Raft §5.2 / §5.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoteDecision {
    /// Caller may persist `voted_for = candidate` and, **only if persist Ok**, set `vote_granted`.
    WouldGrant,
    /// Do not grant (stale term, already voted elsewhere, or candidate log not up-to-date).
    Deny,
}

/// Whether the follower may still vote for `candidate_id` in this term.
#[must_use]
pub fn can_vote(voted_for: Option<u64>, candidate_id: u64) -> bool {
    // Match, not `Option ==`: Aeneas has no model of `PartialEq<Option<u64>>`
    // (extract would axiom it). `u64 == u64` is in the Lean std.
    match voted_for {
        None => true,
        Some(v) => v == candidate_id,
    }
}

/// Raft §5.4.1 log up-to-date (candidate at least as new as local).
#[must_use]
pub fn log_up_to_date(
    my_last_term: u64,
    my_last_index: u64,
    cand_last_term: u64,
    cand_last_index: u64,
) -> bool {
    cand_last_term > my_last_term
        || (cand_last_term == my_last_term && cand_last_index >= my_last_index)
}

/// Pure RequestVote decision.
#[must_use]
pub fn vote_decision(i: VoteInputs) -> VoteDecision {
    if i.candidate_term != i.current_term {
        return VoteDecision::Deny;
    }
    if can_vote(i.voted_for, i.candidate_id)
        && log_up_to_date(
            i.last_log_term,
            i.last_log_index,
            i.candidate_last_log_term,
            i.candidate_last_log_index,
        )
    {
        VoteDecision::WouldGrant
    } else {
        VoteDecision::Deny
    }
}

/// AS-IS / mutant: grant whenever the term matches, **ignoring** log up-to-date
/// and existing vote. Used only to prove the model/kernel invariant has teeth.
#[must_use]
pub fn vote_decision_as_is_ignore_log_and_vote(i: VoteInputs) -> VoteDecision {
    if i.candidate_term == i.current_term {
        VoteDecision::WouldGrant
    } else {
        VoteDecision::Deny
    }
}

/// Persist result the handler sees (axiom of the environment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistOutcome {
    /// `persist_hard` returned Ok.
    Ok,
    /// `persist_hard` returned Err.
    Err,
}

/// F15 protocol: Grant on the wire only if the kernel would grant **and** persist Ok.
///
/// This is the refinement of `handle_request_vote_with_persist` minus I/O.
/// `sent_grant ⇒ persist == Ok`.
#[must_use]
pub fn grant_after_persist(decision: VoteDecision, persist: PersistOutcome) -> bool {
    // Match, not `==` on enums: derived PartialEq extracts to discriminant
    // `Result` wrappers that Lean cannot `cases` through.
    matches!(
        (decision, persist),
        (VoteDecision::WouldGrant, PersistOutcome::Ok)
    )
}

/// AS-IS F15: grant as soon as the kernel says so, persist is ignored.
#[must_use]
pub fn grant_after_persist_as_is(decision: VoteDecision, persist: PersistOutcome) -> bool {
    let _ = persist;
    decision == VoteDecision::WouldGrant
}

/// F125/F127 outcome of stepping to a newer term under a persist result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableTerm {
    /// Incoming term is not newer — no step.
    Keep,
    /// Newer term and hard state durable: follow at the new term.
    Raised,
    /// Newer term but persist failed: restore term/vote, force Follower,
    /// clear `leader_id` — never act at a term that is not on disk.
    Restored,
}

/// F125/F127: the term rises only when hard state is durable.
/// `Raised ⇒ persist == Ok`; `Restored ⇒ previous term/vote survive`.
#[must_use]
pub fn durable_term_if_newer(
    current_term: u64,
    incoming_term: u64,
    persist: PersistOutcome,
) -> DurableTerm {
    // Match, not `==` on enums: derived PartialEq extracts to discriminant
    // `Result` wrappers that Lean cannot `cases` through.
    match (incoming_term > current_term, persist) {
        (false, _) => DurableTerm::Keep,
        (true, PersistOutcome::Ok) => DurableTerm::Raised,
        (true, PersistOutcome::Err) => DurableTerm::Restored,
    }
}

/// AS-IS F125/F127 mutant: keep the raised term even when persist failed —
/// the process acts at a term that never hit disk. Used to prove teeth.
#[must_use]
pub fn durable_term_if_newer_as_is(
    current_term: u64,
    incoming_term: u64,
    persist: PersistOutcome,
) -> DurableTerm {
    let _ = persist;
    if incoming_term > current_term {
        DurableTerm::Raised
    } else {
        DurableTerm::Keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stale_log() -> VoteInputs {
        VoteInputs {
            current_term: 5,
            voted_for: None,
            last_log_term: 3,
            last_log_index: 10,
            candidate_term: 5,
            candidate_id: 1,
            candidate_last_log_term: 1,
            candidate_last_log_index: 1,
        }
    }

    #[test]
    fn as_is_mutant_differs_on_stale_log() {
        assert_eq!(vote_decision(stale_log()), VoteDecision::Deny);
        assert_eq!(
            vote_decision_as_is_ignore_log_and_vote(stale_log()),
            VoteDecision::WouldGrant
        );
    }

    #[test]
    fn grant_after_persist_implies_ok() {
        assert!(grant_after_persist(
            VoteDecision::WouldGrant,
            PersistOutcome::Ok
        ));
        assert!(!grant_after_persist(
            VoteDecision::WouldGrant,
            PersistOutcome::Err
        ));
        assert!(!grant_after_persist(VoteDecision::Deny, PersistOutcome::Ok));
        assert!(grant_after_persist_as_is(
            VoteDecision::WouldGrant,
            PersistOutcome::Err
        ));
    }

    #[test]
    fn durable_term_keeps_on_stale_or_equal_term() {
        assert_eq!(
            durable_term_if_newer(5, 4, PersistOutcome::Ok),
            DurableTerm::Keep
        );
        assert_eq!(
            durable_term_if_newer(5, 5, PersistOutcome::Err),
            DurableTerm::Keep
        );
    }

    #[test]
    fn durable_term_raises_only_on_newer_and_ok() {
        assert_eq!(
            durable_term_if_newer(5, 6, PersistOutcome::Ok),
            DurableTerm::Raised
        );
    }

    #[test]
    fn durable_term_restores_on_newer_and_err() {
        assert_eq!(
            durable_term_if_newer(5, 6, PersistOutcome::Err),
            DurableTerm::Restored
        );
    }

    /// Mutation: the undurable raise must differ from the fixed rule (teeth).
    #[test]
    fn durable_term_as_is_mutant_keeps_undurable_raise() {
        assert_eq!(
            durable_term_if_newer_as_is(5, 6, PersistOutcome::Err),
            DurableTerm::Raised,
            "mutant keeps a raised term that never hit disk (teeth)"
        );
        assert_eq!(
            durable_term_if_newer(5, 6, PersistOutcome::Err),
            DurableTerm::Restored
        );
    }

    /// Clone twin (catalog `vote_raft_store`): both copies must implement
    /// the same function. The shared types (`VoteInputs`, `VoteDecision`,
    /// `PersistOutcome`, `DurableTerm`) are duplicated per crate, so inputs
    /// are built on both sides from the same scalars and results compared
    /// via a common discriminant.
    #[test]
    fn twin_agrees_with_raft_vote_kernel_on_full_domain() {
        use pedradb_raft::vote_kernel as raft;
        let disc_vote = |d: VoteDecision| -> u8 {
            match d {
                VoteDecision::WouldGrant => 0,
                VoteDecision::Deny => 1,
            }
        };
        let disc_vote_r = |d: raft::VoteDecision| -> u8 {
            match d {
                raft::VoteDecision::WouldGrant => 0,
                raft::VoteDecision::Deny => 1,
            }
        };
        let disc_dur = |d: DurableTerm| -> u8 {
            match d {
                DurableTerm::Keep => 0,
                DurableTerm::Raised => 1,
                DurableTerm::Restored => 2,
            }
        };
        let disc_dur_r = |d: raft::DurableTerm| -> u8 {
            match d {
                raft::DurableTerm::Keep => 0,
                raft::DurableTerm::Raised => 1,
                raft::DurableTerm::Restored => 2,
            }
        };
        let mut checked = 0usize;

        // (current_term, voted_for, last_log_term, last_log_index,
        //  candidate_term, candidate_id, cand_last_term, cand_last_index)
        let cases: &[(u64, Option<u64>, u64, u64, u64, u64, u64, u64)] = &[
            (5, None, 3, 10, 5, 1, 1, 1),
            (5, None, 3, 10, 5, 1, 3, 10),
            (5, None, 3, 10, 5, 1, 4, 0),
            (5, None, 3, 10, 5, 1, 3, 9),
            (5, None, 3, 10, 5, 1, 3, 11),
            (5, Some(1), 3, 10, 5, 1, 9, 9),
            (5, Some(2), 3, 10, 5, 1, 9, 9),
            (5, None, 3, 10, 4, 1, 9, 9),
            (5, None, 3, 10, 6, 1, 9, 9),
            (0, None, 0, 0, 0, 0, 0, 0),
            (u64::MAX, None, u64::MAX, u64::MAX, u64::MAX, 7, u64::MAX, u64::MAX),
            (u64::MAX, Some(u64::MAX), 0, u64::MAX, u64::MAX, u64::MAX, 0, 0),
            (u64::MAX, None, u64::MAX, u64::MAX, u64::MAX, 1, u64::MAX, u64::MAX - 1),
            (1, None, u64::MAX, u64::MAX, 1, 2, u64::MAX, u64::MAX),
            (1, Some(0), 0, 0, 1, 0, 0, 0),
            (1, Some(0), 0, 0, 1, 1, u64::MAX, u64::MAX),
        ];
        for &(ct, vf, mlt, mli, kt, kid, clt, cli) in cases {
            let mine = VoteInputs {
                current_term: ct,
                voted_for: vf,
                last_log_term: mlt,
                last_log_index: mli,
                candidate_term: kt,
                candidate_id: kid,
                candidate_last_log_term: clt,
                candidate_last_log_index: cli,
            };
            let theirs = raft::VoteInputs {
                current_term: ct,
                voted_for: vf,
                last_log_term: mlt,
                last_log_index: mli,
                candidate_term: kt,
                candidate_id: kid,
                candidate_last_log_term: clt,
                candidate_last_log_index: cli,
            };
            assert_eq!(
                disc_vote(vote_decision(mine)),
                disc_vote_r(raft::vote_decision(theirs)),
                "vote_decision({ct},{vf:?},{mlt},{mli},{kt},{kid},{clt},{cli})"
            );
            assert_eq!(
                disc_vote(vote_decision_as_is_ignore_log_and_vote(mine)),
                disc_vote_r(raft::vote_decision_as_is_ignore_log_and_vote(theirs)),
                "vote_decision_as_is({ct},{vf:?},{mlt},{mli},{kt},{kid},{clt},{cli})"
            );
            checked += 2;
        }

        for vf in [None, Some(0u64), Some(1), Some(5)] {
            for &id in &[0u64, 1, 3, 5, u64::MAX] {
                assert_eq!(can_vote(vf, id), raft::can_vote(vf, id), "can_vote({vf:?},{id})");
                checked += 1;
            }
        }

        let q = [0u64, 1, 3, u64::MAX];
        for &mt in &q {
            for &mi in &q {
                for &ct in &q {
                    for &ci in &q {
                        assert_eq!(
                            log_up_to_date(mt, mi, ct, ci),
                            raft::log_up_to_date(mt, mi, ct, ci),
                            "log_up_to_date({mt},{mi},{ct},{ci})"
                        );
                        checked += 1;
                    }
                }
            }
        }

        let dv = [
            (VoteDecision::WouldGrant, raft::VoteDecision::WouldGrant, "WouldGrant"),
            (VoteDecision::Deny, raft::VoteDecision::Deny, "Deny"),
        ];
        let pv = [
            (PersistOutcome::Ok, raft::PersistOutcome::Ok, "Ok"),
            (PersistOutcome::Err, raft::PersistOutcome::Err, "Err"),
        ];
        for (d, dr, dn) in dv {
            for (p, pr, pn) in pv {
                assert_eq!(
                    grant_after_persist(d, p),
                    raft::grant_after_persist(dr, pr),
                    "grant_after_persist({dn},{pn})"
                );
                assert_eq!(
                    grant_after_persist_as_is(d, p),
                    raft::grant_after_persist_as_is(dr, pr),
                    "grant_after_persist_as_is({dn},{pn})"
                );
                checked += 2;
            }
        }

        for &cur in &q {
            for &inc in &q {
                for (p, pr, pn) in pv {
                    assert_eq!(
                        disc_dur(durable_term_if_newer(cur, inc, p)),
                        disc_dur_r(raft::durable_term_if_newer(cur, inc, pr)),
                        "durable_term_if_newer({cur},{inc},{pn})"
                    );
                    assert_eq!(
                        disc_dur(durable_term_if_newer_as_is(cur, inc, p)),
                        disc_dur_r(raft::durable_term_if_newer_as_is(cur, inc, pr)),
                        "durable_term_if_newer_as_is({cur},{inc},{pn})"
                    );
                    checked += 2;
                }
            }
        }
        // 16 cases ×2 + 4·5 can_vote + 4⁴ log_up_to_date + 2·2·2 grant + 4·4·2 durable.
        assert_eq!(checked, 32 + 20 + 256 + 8 + 64);
    }

    /// Mirror of the raft-side F15/F125 invariants on THIS copy (the store
    /// side had no domain theorem): grant implies durable persist, a raise
    /// implies a newer term that hit disk, and log up-to-date is exactly
    /// the lexicographic (term, index) order.
    #[test]
    fn theorem_vote_invariants_on_finite_domain() {
        let q = [0u64, 1, 3, u64::MAX];
        for &cur in &q {
            for &inc in &q {
                for p in [PersistOutcome::Ok, PersistOutcome::Err] {
                    let d = durable_term_if_newer(cur, inc, p);
                    match d {
                        DurableTerm::Keep => assert!(inc <= cur, "Keep needs non-newer term"),
                        DurableTerm::Raised => {
                            assert!(inc > cur, "F125: Raised needs newer term");
                            assert_eq!(p, PersistOutcome::Ok, "F125: Raised needs durable persist");
                        }
                        DurableTerm::Restored => {
                            assert!(inc > cur, "F127: Restored needs newer term");
                            assert_eq!(p, PersistOutcome::Err, "F127: Restored needs failed persist");
                        }
                    }
                    if matches!(
                        durable_term_if_newer_as_is(cur, inc, p),
                        DurableTerm::Raised
                    ) && p == PersistOutcome::Err
                    {
                        assert_eq!(d, DurableTerm::Restored, "mutant keeps an undurable raise");
                    }
                }
            }
        }
        for &mt in &q {
            for &mi in &q {
                for &ct in &q {
                    for &ci in &q {
                        assert_eq!(
                            log_up_to_date(mt, mi, ct, ci),
                            (ct, ci) >= (mt, mi),
                            "up-to-date is lexicographic (term, index)"
                        );
                    }
                }
            }
        }
        for d in [VoteDecision::WouldGrant, VoteDecision::Deny] {
            for p in [PersistOutcome::Ok, PersistOutcome::Err] {
                let g = grant_after_persist(d, p);
                assert_eq!(g, d == VoteDecision::WouldGrant && p == PersistOutcome::Ok);
                if g {
                    assert!(matches!((d, p), (VoteDecision::WouldGrant, PersistOutcome::Ok)));
                }
            }
        }
    }
}
