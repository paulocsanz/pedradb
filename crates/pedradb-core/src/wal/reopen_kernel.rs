//! Pure reopen decisions (RFC-0053 Y3.3 — crash dictionary on the `Db`
//! reopen path).
//!
//! Production [`crate::Db::open_with_env`] calls this kernel in every WAL
//! damage arm: the outcome of a reopen under damage is decided **here**
//! (refuse / serve decoded prefix + report), never ad hoc at the call site.
//! Bytes on disk, journaling, escalation counters, and fsync are
//! **caller + axiom** (DST / FailingEnv drive those).
//!
//! Named decisions (the ones that were `SilentWrong` when inverted):
//! - **FailClosed + damage ⇒ refuse the open** — never serve a subset
//!   silently (G8).
//! - **PointInTime + damage ⇒ report + serve prefix** — the discard is
//!   observable (`RecoveryReport`), never silent.
//! - **PointInTime + escalated ⇒ refuse** — RFC-0038 D wins over the
//!   permissive profile.
//! - **No damage ⇒ serve everything** — no false refusal.
//!
//! Spec page: `docs/formal/crash-dictionary.md` (reopen section).

#![forbid(unsafe_code)]

/// Which WAL damage the reopen observed (maps 1:1 to the recover kinds that
/// fail-stop at a fresh alignment — see `verus/reopen_outcome.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReopenDamage {
    /// Clean log — nothing to decide.
    None,
    /// `Truncated(0)` on a non-tiny WAL (bitrot of the first record, F4).
    TruncatedHead,
    /// Mid-WAL CRC mismatch (G8 fail-stop, journaled).
    Crc,
    /// Zero type+len at fresh alignment with junk after (F170).
    ZeroHeader,
    /// A resync walk skipped damaged bytes and re-anchored later (F171).
    Resync,
}

/// What the reopen does with the damage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReopenOutcome {
    /// No damage: replay everything recovered, no report.
    ServeAll,
    /// Damage, PointInTime, not escalated: serve the decoded prefix and
    /// publish a [`crate::RecoveryReport`] (observable discard).
    ServePrefixReport,
    /// Refuse the open (FailClosed mode, or RFC-0038 D escalation).
    RefuseOpen,
}

/// Pure rule for one damaged reopen (RFC-0053 Y3.3).
///
/// # Post-condition (theorem-ready)
///
/// ```text
/// ensures
///   (outcome == ServeAll)         ==> damage == None
///   (outcome == ServePrefixReport) ==> (damage != None && point_in_time && !escalated)
///   (outcome == RefuseOpen)        ==> (damage != None && (!point_in_time || escalated))
///   damage != None && !point_in_time ==> outcome == RefuseOpen   // never silent
/// ```
///
/// Finite-domain check: [`tests::theorem_reopen_on_finite_domain`].
/// ∀ Verus twin: `crates/pedradb-core/verus/reopen_outcome.rs`.
#[must_use]
pub fn reopen_outcome(damage: ReopenDamage, point_in_time: bool, escalated: bool) -> ReopenOutcome {
    match damage {
        ReopenDamage::None => ReopenOutcome::ServeAll,
        _ => {
            if point_in_time && !escalated {
                ReopenOutcome::ServePrefixReport
            } else {
                ReopenOutcome::RefuseOpen
            }
        }
    }
}

/// AS-IS G8: swallow the damage — always serve everything, no report, no
/// refusal (silent-wrong). Mutant must fail every theorem above.
#[must_use]
pub fn reopen_outcome_as_is_silent(
    _damage: ReopenDamage,
    _point_in_time: bool,
    _escalated: bool,
) -> ReopenOutcome {
    ReopenOutcome::ServeAll
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_log_serves_all() {
        for pt in [false, true] {
            for esc in [false, true] {
                assert_eq!(
                    reopen_outcome(ReopenDamage::None, pt, esc),
                    ReopenOutcome::ServeAll
                );
            }
        }
    }

    #[test]
    fn fail_closed_refuses_every_damage() {
        for d in [
            ReopenDamage::TruncatedHead,
            ReopenDamage::Crc,
            ReopenDamage::ZeroHeader,
            ReopenDamage::Resync,
        ] {
            assert_eq!(reopen_outcome(d, false, false), ReopenOutcome::RefuseOpen);
            assert_eq!(reopen_outcome(d, false, true), ReopenOutcome::RefuseOpen);
        }
    }

    #[test]
    fn point_in_time_reports_unless_escalated() {
        for d in [
            ReopenDamage::TruncatedHead,
            ReopenDamage::Crc,
            ReopenDamage::ZeroHeader,
            ReopenDamage::Resync,
        ] {
            assert_eq!(
                reopen_outcome(d, true, false),
                ReopenOutcome::ServePrefixReport
            );
            assert_eq!(reopen_outcome(d, true, true), ReopenOutcome::RefuseOpen);
        }
    }

    /// Finite-domain theorem: damaged reopens are never ServeAll (never
    /// silent); FailClosed never serves a damaged prefix; the AS-IS mutant
    /// is silent on every damaged input.
    #[test]
    fn theorem_reopen_on_finite_domain() {
        let damages = [
            ReopenDamage::None,
            ReopenDamage::TruncatedHead,
            ReopenDamage::Crc,
            ReopenDamage::ZeroHeader,
            ReopenDamage::Resync,
        ];
        for d in damages {
            for pt in [false, true] {
                for esc in [false, true] {
                    let o = reopen_outcome(d, pt, esc);
                    if o == ReopenOutcome::ServeAll {
                        assert_eq!(d, ReopenDamage::None, "never serve-all on damage");
                    }
                    if d != ReopenDamage::None && !pt {
                        assert_eq!(o, ReopenOutcome::RefuseOpen, "fail-closed refuses");
                    }
                    if o == ReopenOutcome::ServePrefixReport {
                        assert!(d != ReopenDamage::None && pt && !esc);
                    }
                    if d != ReopenDamage::None {
                        let m = reopen_outcome_as_is_silent(d, pt, esc);
                        assert_eq!(
                            m,
                            ReopenOutcome::ServeAll,
                            "AS-IS must be silent-wrong on damage"
                        );
                        assert_ne!(m, o, "mutant must differ from fixed on damage");
                    }
                }
            }
        }
    }

    #[test]
    fn reopen_outcome_on_live_crc_is_not_ok() {
        assert_eq!(
            reopen_outcome(ReopenDamage::Crc, false, false),
            ReopenOutcome::RefuseOpen
        );
        assert_eq!(
            reopen_outcome_as_is_silent(ReopenDamage::Crc, false, false),
            ReopenOutcome::ServeAll,
            "AS-IS dente: damaged WAL served"
        );
    }
}
