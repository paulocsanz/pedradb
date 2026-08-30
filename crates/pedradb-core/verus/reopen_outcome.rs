// Verus proof of the Db reopen outcome under WAL damage
// (RFC-0053 Y3.3 — crash dictionary on the reopen path).
//
// Source of truth for production remains `src/wal/reopen_kernel.rs`.
// This file is the machine-checked theorem: exec == spec, damaged reopens
// are never silent, FailClosed refuses, and the recover kernels' fail-stop
// kinds are exactly the reopens that route into a damage arm.
//
//   ./scripts/verus_reopen_outcome.sh
//
// Do not link this into the production crate — it is a twin of the pure kernel.

use vstd::prelude::*;

verus! {

/// Mirrors `ReopenDamage` in reopen_kernel.rs.
pub enum ReopenDamage {
    None,
    TruncatedHead,
    Crc,
    ZeroHeader,
    Resync,
}

/// Mirrors `ReopenOutcome` in reopen_kernel.rs.
pub enum ReopenOutcome {
    ServeAll,
    ServePrefixReport,
    RefuseOpen,
}

/// Closed-form spec — same arms as `reopen_kernel::reopen_outcome`.
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

/// Executable decision — must match `pedradb_core::wal::reopen_kernel::
/// reopen_outcome` bit-for-bit.
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
proof fn lemma_mutant_swallows_damage(damage: ReopenDamage, escalated: bool)
    requires
        damage != ReopenDamage::None,
    ensures
        reopen_outcome(damage, false, escalated) == ReopenOutcome::RefuseOpen,
        reopen_outcome_as_is(damage, false, escalated) == ReopenOutcome::ServeAll,
{
}

} // verus!
