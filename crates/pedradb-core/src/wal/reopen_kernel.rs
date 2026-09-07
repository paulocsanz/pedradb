//! Pure reopen decisions (RFC-0053 Y3.3 — crash dictionary on the `Db`
//! reopen path).
//!
//! **Single artifact (pair `reopen_outcome`):** this file is what `rustc`
//! links *and* what Verus proves (`cfg(verus_keep_ghost)`). Pair
//! `dictionary_link` still has a twin-cópia until its turn.
//!
//!   ./scripts/verus_reopen_outcome.sh
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
//! The rustc bodies stay byte-stable so non-`single_artifact` twins still
//! token-match. Verus proofs sit in the `cfg(verus_keep_ghost)` block
//! above them (last-wins for lint is the rustc body).
//!
//! Spec page: `docs/formal/crash-dictionary.md` (reopen section).

#![forbid(unsafe_code)]

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

/// Mirrors the rustc `ReopenDamage` below (cfg-split so Verus does not see Debug).
pub enum ReopenDamage {
    None,
    TruncatedHead,
    Crc,
    ZeroHeader,
    Resync,
}

/// Mirrors the rustc `ReopenOutcome` below.
pub enum ReopenOutcome {
    ServeAll,
    ServePrefixReport,
    RefuseOpen,
}

/// Closed-form spec — same arms as production `reopen_outcome`.
pub open spec fn reopen_outcome_spec(
    damage: ReopenDamage,
    point_in_time: bool,
    escalated: bool,
) -> ReopenOutcome {
    match damage {
        ReopenDamage::None => ReopenOutcome::ServeAll,
        _ => {
            if point_in_time && !escalated {
                ReopenOutcome::ServePrefixReport
            } else {
                ReopenOutcome::RefuseOpen
            }
        },
    }
}

/// AS-IS G8: swallow the damage — always serve everything, silently.
pub open spec fn reopen_outcome_as_is(
    _damage: ReopenDamage,
    _point_in_time: bool,
    _escalated: bool,
) -> ReopenOutcome {
    ReopenOutcome::ServeAll
}

/// Executable decision — must match the rustc `reopen_outcome` bit-for-bit.
#[verifier::when_used_as_spec(reopen_outcome_spec)]
pub fn reopen_outcome(
    damage: ReopenDamage,
    point_in_time: bool,
    escalated: bool,
) -> (o: ReopenOutcome)
    ensures
        o == reopen_outcome_spec(damage, point_in_time, escalated),
        (o == ReopenOutcome::ServeAll) ==> damage == ReopenDamage::None,
        (o == ReopenOutcome::ServePrefixReport)
            ==> (damage != ReopenDamage::None && point_in_time && !escalated),
        damage != ReopenDamage::None && !point_in_time ==> o == ReopenOutcome::RefuseOpen,
{
    match damage {
        ReopenDamage::None => ReopenOutcome::ServeAll,
        _ => {
            if point_in_time && !escalated {
                ReopenOutcome::ServePrefixReport
            } else {
                ReopenOutcome::RefuseOpen
            }
        },
    }
}

pub fn reopen_outcome_as_is_silent(
    _damage: ReopenDamage,
    _point_in_time: bool,
    _escalated: bool,
) -> (o: ReopenOutcome)
    ensures
        o == ReopenOutcome::ServeAll,
{
    ReopenOutcome::ServeAll
}

/// Mirrors the fail-stop recover kinds (`wal/recover_kernel.rs::RecoverKind`)
/// that route a reopen into a damage arm.
pub enum RecoverKind {
    Record,
    CleanEof,
    Truncated,
    LengthCorrupt,
    UnknownType,
    OrphanFragment,
    Crc,
    ZeroHeaderTail,
    Other,
}

/// Which reopen damage a fail-stop recover kind produces (None = the kind
/// does not route into a damage arm on its own).
pub open spec fn damage_from_recover_kind(kind: RecoverKind) -> ReopenDamage {
    match kind {
        RecoverKind::Crc => ReopenDamage::Crc,
        RecoverKind::ZeroHeaderTail => ReopenDamage::ZeroHeader,
        RecoverKind::Truncated => ReopenDamage::TruncatedHead,
        _ => ReopenDamage::None,
    }
}

/// Whether the recover kernel fail-stops on this kind at a fresh alignment
/// (i.e. `recover_collect_act` cannot keep collecting records silently).
pub open spec fn recover_failstops(kind: RecoverKind) -> bool {
    match kind {
        RecoverKind::Crc => true,
        RecoverKind::ZeroHeaderTail => true,
        RecoverKind::Truncated => true,
        RecoverKind::OrphanFragment => true,
        RecoverKind::Other => true,
        _ => false,
    }
}

/// Y3.3 named lemma (link): the fail-stop recover kinds that route a reopen
/// into a damage arm (CRC, ZeroHeaderTail at fresh alignment, head-Truncated)
/// all produce a damage — a reopen that refuses (or reports) is exactly a
/// reopen whose recover kernel could not proceed silently.
proof fn lemma_recover_failstop_routes_to_damage(kind: RecoverKind)
    requires
        kind is Crc || kind is ZeroHeaderTail || kind is Truncated,
    ensures
        damage_from_recover_kind(kind) != ReopenDamage::None,
{
}

/// Y3.3 named lemma (crash spec): a damaged reopen is NEVER ServeAll —
/// the discard is either reported or the open is refused. Never silent.
proof fn lemma_damaged_reopen_never_silent(
    damage: ReopenDamage,
    point_in_time: bool,
    escalated: bool,
)
    requires
        damage != ReopenDamage::None,
    ensures
        reopen_outcome(damage, point_in_time, escalated) != ReopenOutcome::ServeAll,
{
}

/// Y3.3 named lemma (crash spec): FailClosed refuses every damage — the
/// visible map never silently drops part of an acked suffix.
proof fn lemma_fail_closed_refuses_damage(damage: ReopenDamage, escalated: bool)
    requires
        damage != ReopenDamage::None,
    ensures
        reopen_outcome(damage, false, escalated) == ReopenOutcome::RefuseOpen,
{
}

/// Y3.3 named lemma (no false refusal): a clean reopen serves everything.
proof fn lemma_clean_reopen_serves_all(point_in_time: bool, escalated: bool)
    ensures
        reopen_outcome(ReopenDamage::None, point_in_time, escalated)
            == ReopenOutcome::ServeAll,
{
}

/// Teeth: the AS-IS swallow-damage mutant serves a damaged WAL silently —
/// exactly the G8 silent-wrong the fixed kernel refuses.
/// CrashMonkey (FAST'19) found a crash-consistency bug in verified FSCQ:
/// a proof of a model does not save a reopen that serves damaged bytes.
proof fn lemma_mutant_swallows_damage(damage: ReopenDamage, escalated: bool)
    requires
        damage != ReopenDamage::None,
    ensures
        reopen_outcome(damage, false, escalated) == ReopenOutcome::RefuseOpen,
        reopen_outcome_as_is(damage, false, escalated) == ReopenOutcome::ServeAll,
{
}

} // verus!

/// Which WAL damage the reopen observed (maps 1:1 to the recover kinds that
/// fail-stop at a fresh alignment — see the Verus `RecoverKind` above).
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
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
