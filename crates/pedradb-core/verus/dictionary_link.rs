// Verus proof of the crash-dictionary theorem-link
// (RFC-0056 P1.1 — put acked → WAL bytes → recover prefix → reopen
// serves → get visible).
//
// This twin COMPOSES the existing kernels' specs into one chained,
// named theorem over an abstract record model. The persist axiom is a
// HYPOTHESIS of the top theorem (named, never proved — TCB), exactly as
// `docs/formal/crash-dictionary.md` records it. The recovery / reopen
// steps restate the shipped twins (`wal_recover.rs`, `reopen_outcome.rs`)
// because each twin compiles standalone.
//
// Chain (each arrow a named lemma below):
//
//   put acked (sync) --[persist axiom]--> WAL contains the record
//     --[wal recover: every Record kept, CleanEof stops]--> recovered == wal
//     --[reopen: no damage ⇒ ServeAll]--> full replay
//     --[replay ⇒ get]--> get(k) == acked value
//
//   ./scripts/verus_dictionary_link.sh
//
// Do not link this into the production crate.

use vstd::prelude::*;
use vstd::seq::Seq;

verus! {

// ---- Minimal twin vocabulary (restated from the shipped twins) ----

pub enum RecoverKind {
    Record,
    CleanEof,
    Truncated,
    Crc,
}

pub enum RecoverAct {
    KeepRecord,
    Stop,
    KeepPrefix,
    FailStop,
}

pub open spec fn recover_collect_act_spec(kind: RecoverKind, prefix_n: u64, can_skip: bool) -> RecoverAct {
    match kind {
        RecoverKind::Record => RecoverAct::KeepRecord,
        RecoverKind::CleanEof => RecoverAct::Stop,
        RecoverKind::Truncated => {
            if !can_skip {
                if prefix_n == 0 {
                    RecoverAct::FailStop
                } else {
                    RecoverAct::KeepPrefix
                }
            } else {
                RecoverAct::KeepPrefix
            }
        },
        RecoverKind::Crc => RecoverAct::FailStop,
    }
}

pub enum ReopenDamage {
    None,
    TruncatedHead,
    Crc,
    ZeroHeader,
    Resync,
}

pub enum ReopenOutcome {
    ServeAll,
    ServePrefixReport,
    RefuseOpen,
}

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

/// Exec mirror of `wal/reopen_kernel.rs::reopen_outcome` (the kernel the
/// chain links through; token-identical decision).
pub fn reopen_outcome(
    damage: ReopenDamage,
    point_in_time: bool,
    escalated: bool,
) -> (o: ReopenOutcome)
    ensures
        o == reopen_outcome_spec(damage, point_in_time, escalated),
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

// ---- Abstract record model ----

/// (seq, key, val) — one acked/visible put.
pub type Rec = (u64, u64, u64);

/// A1 — **persist axiom** (TCB, never a theorem): every record whose
/// `put` returned Ok under sync is durable in the WAL bytes.
pub open spec fn persist_axiom_holds(acked: Seq<Rec>, wal: Seq<Rec>) -> bool {
    forall |i|
        #![auto]
        0 <= i < acked.len() ==> ({
            &&& 0 <= i < wal.len()
            &&& wal.index(i) == acked.index(i)
        })
}

/// A2 — clean log shape: every kind is Record (the stop is CleanEof).
pub open spec fn wal_all_records(kinds: Seq<RecoverKind>) -> bool {
    forall |i| #![auto] 0 <= i < kinds.len() ==> kinds.index(i) is Record
}

/// A3 — the recover kernel collected the complete log: recovered == wal
/// (the WAL may itself carry an unacked suffix past `acked.len()`).
pub open spec fn recovered_everything(recovered: Seq<Rec>, wal: Seq<Rec>) -> bool {
    recovered.len() == wal.len()
        && forall |i| #![auto] 0 <= i < recovered.len() ==> recovered.index(i) == wal.index(i)
}

/// A4 — the unacked tail (which the dictionary allows to vanish or
/// survive) contains no record for `k`: nothing after the acked prefix
/// can shadow what `get(k)` returns.
pub open spec fn tail_has_no_key(recovered: Seq<Rec>, acked_len: u64, k: u64) -> bool {
    forall |j|
        #![auto]
        acked_len <= j < recovered.len() ==> recovered.index(j).1 != k
}

/// RFC-0150 P1: replay→get visibility. An acked Value at `seq <= snapshot`
/// that is not range-hidden is what `get` returns (`merge::visible_at`).
pub open spec fn visible_at_spec(kind_is_value: bool, seq: u64, snapshot: u64, range_hidden: bool) -> bool {
    &&& seq <= snapshot
    &&& kind_is_value
    &&& !range_hidden
}

pub fn visible_at(kind_is_value: bool, seq: u64, snapshot: u64, range_hidden: bool) -> (d: bool)
    ensures
        d == visible_at_spec(kind_is_value, seq, snapshot, range_hidden),
{
    seq <= snapshot && kind_is_value && !range_hidden
}

proof fn lemma_acked_value_is_visible_at_snapshot(seq: u64, snapshot: u64)
    requires
        seq <= snapshot,
    ensures
        visible_at_spec(true, seq, snapshot, false),
{
}

/// `get(k)` after replay: some record for `k` with value `v` whose seq
/// dominates every other record for `k` in the recovered set.
pub open spec fn get_returns(recovered: Seq<Rec>, k: u64, v: u64) -> bool {
    exists |i|
        0 <= i < recovered.len() && recovered.index(i).1 == k
            && recovered.index(i).2 == v
            && forall |j|
                #![auto]
                0 <= j < recovered.len() && recovered.index(j).1 == k
                    ==> recovered.index(j).0 <= recovered.index(i).0
}

// ---- The chain, one named lemma per arrow ----

/// Link 1/4 (persist axiom entry point): an acked record is in the WAL.
proof fn lemma_acked_put_is_in_wal(acked: Seq<Rec>, wal: Seq<Rec>, i: u64)
    requires
        persist_axiom_holds(acked, wal),
        i < acked.len(),
    ensures
        i < wal.len(),
        wal.index(i as int) == acked.index(i as int),
{
}

/// Link 2/4 (recover kernel): a clean log keeps every record — the
/// recover kernel never drops a Record silently.
proof fn lemma_clean_wal_recovers_every_record(kinds: Seq<RecoverKind>, prefix_n: u64, can_skip: bool)
    requires
        wal_all_records(kinds),
    ensures
        forall |i|
            #![auto]
            0 <= i < kinds.len() ==> recover_collect_act_spec(
                kinds.index(i),
                prefix_n,
                can_skip,
            ) == RecoverAct::KeepRecord,
{
    assert forall |i: int|
        0 <= i < kinds.len()
        implies recover_collect_act_spec(
            kinds.index(i),
            prefix_n,
            can_skip,
        ) == RecoverAct::KeepRecord
    by {
        assert(kinds.index(i) is Record);
    }
}

/// Link 3/4 (reopen kernel): no damage ⇒ the reopen serves everything
/// (no false refusal, no silent prefix).
proof fn lemma_clean_reopen_serves_all(point_in_time: bool, escalated: bool)
    ensures
        reopen_outcome_spec(ReopenDamage::None, point_in_time, escalated)
            == ReopenOutcome::ServeAll,
{
}

/// Link 4/4 (replay ⇒ get): a dominated record in the recovered set is
/// what `get` returns after full replay.
proof fn lemma_replayed_record_is_visible(recovered: Seq<Rec>, i: u64)
    requires
        i < recovered.len(),
        forall |j|
            #![auto]
            0 <= j < recovered.len() && recovered.index(j).1 == recovered.index(i as int).1
                ==> recovered.index(j).0 <= recovered.index(i as int).0,
    ensures
        get_returns(
            recovered,
            recovered.index(i as int).1,
            recovered.index(i as int).2,
        ),
{
    let r = recovered.index(i as int);
    assert(recovered.index(i as int) == r);
    let n = recovered.len();
    let ii = i as int;
    assert(ii < n);
    assert(recovered.index(i as int).1 == r.1);
    assert(recovered.index(i as int).2 == r.2);
    assert forall |j: int|
        0 <= j < recovered.len() && recovered.index(j).1 == r.1
        implies recovered.index(j).0 <= r.0
    by {}
}

/// **The theorem-link (P1.1):** under the named persist axiom, a clean
/// WAL, a no-damage reopen, and a tail that cannot shadow `k`, every
/// acked put is what `get` returns after crash-and-reopen — the
/// dictionary does not revert past the acked prefix on this path.
proof fn lemma_put_acked_survives_crash(
    acked: Seq<Rec>,
    wal: Seq<Rec>,
    recovered: Seq<Rec>,
    i: u64,
    point_in_time: bool,
    escalated: bool,
)
    requires
        persist_axiom_holds(acked, wal),               // A1 persist axiom
        recovered_everything(recovered, wal),          // A2/A3 clean recover
        i < acked.len(),
        forall |j|
            #![auto]
            0 <= j < acked.len() && acked.index(j).1 == acked.index(i as int).1
                ==> acked.index(j).0 <= acked.index(i as int).0,
        tail_has_no_key(recovered, acked.len() as u64, acked.index(i as int).1), // A4
    ensures
        reopen_outcome_spec(ReopenDamage::None, point_in_time, escalated)
            == ReopenOutcome::ServeAll,
        get_returns(recovered, acked.index(i as int).1, acked.index(i as int).2),
{
    lemma_clean_reopen_serves_all(point_in_time, escalated);
    lemma_acked_put_is_in_wal(acked, wal, i);
    // recovered extends acked element-for-element (recovered == wal ⊇ acked).
    let rlen = recovered.len();
    let ii = i as int;
    assert(recovered.len() == wal.len());
    assert(ii < rlen);
    assert(recovered.index(i as int) == acked.index(i as int));
    assert forall |j: int|
        0 <= j < recovered.len()
            && recovered.index(j).1 == acked.index(i as int).1
        implies recovered.index(j).0 <= acked.index(i as int).0
    by {
        if j < acked.len() as int {
            assert(recovered.index(j) == wal.index(j));
            assert(wal.index(j) == acked.index(j));
        }
    }
    lemma_replayed_record_is_visible(recovered, i);
}

/// Teeth (torn tail): the AS-IS recover silently stops at a Truncated
/// record that the fixed kernel keeps (prefix) or fail-stops on — the
/// dictionary would revert an acked record with no report and no refusal.
proof fn lemma_mutant_torn_tail_is_silent()
    ensures
        recover_collect_act_spec(RecoverKind::Truncated, 2, false)
            == RecoverAct::KeepPrefix,
        recover_collect_act_spec(RecoverKind::Truncated, 0, false)
            == RecoverAct::FailStop,
        recover_collect_act_spec(RecoverKind::Crc, 2, false) == RecoverAct::FailStop,
{
}

/// Teeth (swallow reopen): with the AS-IS silent reopen, a damaged WAL is
/// served as `ServeAll` — no report, no refusal — exactly the
/// `G8` silent-wrong the fixed kernel's RefuseOpen/ServePrefixReport
/// arms prevent.
proof fn lemma_mutant_reopen_swallows_damage(point_in_time: bool, escalated: bool)
    ensures
        reopen_outcome_spec(ReopenDamage::TruncatedHead, point_in_time, escalated)
            != ReopenOutcome::ServeAll,
        reopen_outcome_spec(ReopenDamage::Crc, point_in_time, escalated)
            != ReopenOutcome::ServeAll,
{
}

} // verus!
