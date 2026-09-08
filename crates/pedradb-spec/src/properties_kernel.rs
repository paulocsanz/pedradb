//! RFC-0166 property kernel — the four product properties as pure
//! predicates over abstract models, with AS-IS weak versions and named
//! anti-vacuity teeth (a concrete witness where the weak version holds
//! and the property does not — the spec is neither trivial nor implied
//! by the weaker check).
//!
//! **Single artifact (pair `d1_durability`):** this file is what `rustc`
//! links *and* what Verus proves (`cfg(verus_keep_ghost)`). `d1_holds` is
//! the term. R1/T1/C1 stay rustc until their turn.
//!
//!   ./scripts/verus_spec_properties.sh
//!
//! - **D1** `d1_holds`: every acked write survives the crash prefix
//!   (put→Ok is durable — the G1 product promise). AS-IS `d1_holds_as_is`
//!   only promises barrier semantics for synced entries — the
//!   `sync=false` peer durability class: an implementation may ack
//!   before the barrier and lose exactly those writes.
//! - **R1** `r1_answer_ok`: a read answer is the entry of the NEWEST
//!   covering source — a newer tombstone dominates an older value (no
//!   resurrection), a newer value dominates an older one (no
//!   regression). AS-IS accepts any covering hit — the historical
//!   `.rev()` probe order of findings/2026-09-04-reopen-delete-resurrected.
//! - **T1** `t1_holds`: a transaction is all-or-nothing — every staged
//!   effect visible iff committed, none if aborted/in-flight, never
//!   both flags. AS-IS only checks byte-level integrity (visible
//!   indices are staged writes) — partial effects of an aborted tx pass.
//! - **C1** `c1_holds`: a served value is committed by a majority of
//!   every active config (joint: old AND new). AS-IS accepts the old
//!   majority alone — the joint_election as-is hole (RFC-0064).

#![forbid(unsafe_code)]

/// Majority of a config size (Raft §5 / `membership_kernel::majority_of`).
#[cfg(not(verus_keep_ghost))]
fn majority(n: u64) -> u64 {
    if n == 0 {
        1
    } else {
        n / 2 + 1
    }
}

/// D1 — durability of the ack: entry `i` acked (`Ok` returned to the
/// client) implies `i` is inside the crash-surviving prefix `survives`.
/// "Value or later value" is positional: a surviving entry is replayed,
/// and later surviving entries of the same key supersede it in order.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn d1_holds(acked: &[bool], survives: usize) -> bool {
    let mut i = 0;
    while i < acked.len() {
        if acked[i] && i >= survives {
            return false;
        }
        i += 1;
    }
    true
}

/// D1 AS-IS — the weaker barrier-only class: synced entries survive.
/// An implementation that acks before the barrier (the `sync=false`
/// peer) loses exactly the acked-but-not-synced writes and still passes.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn d1_holds_as_is(synced: &[bool], survives: usize) -> bool {
    let mut i = 0;
    while i < synced.len() {
        if synced[i] && i >= survives {
            return false;
        }
        i += 1;
    }
    true
}

/// One source's entry for a key: a value, a tombstone, or no coverage.
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// The source holds a value for the key.
    Value(u64),
    /// The source holds a tombstone (delete) for the key.
    Tombstone,
}

/// The newest covering entry in probe order (index 0 = newest source).
#[cfg(not(verus_keep_ghost))]
fn r1_first_hit(probes: &[Option<Slot>]) -> Option<Slot> {
    let mut i = 0;
    while i < probes.len() {
        if probes[i].is_some() {
            return probes[i];
        }
        i += 1;
    }
    None
}

/// R1 — the read answer is exactly the newest covering entry: a newer
/// tombstone answers "not present", a newer value answers itself.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn r1_answer_ok(probes: &[Option<Slot>], answer: Option<Slot>) -> bool {
    answer == r1_first_hit(probes)
}

/// R1 AS-IS — any covering hit is acceptable (probe order does not
/// matter): a resurrected older value under a newer tombstone passes.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn r1_answer_ok_as_is(probes: &[Option<Slot>], answer: Option<Slot>) -> bool {
    match answer {
        None => r1_first_hit(probes).is_none(),
        Some(_) => r1_first_hit(probes).is_some(),
    }
}

/// T1 — transaction all-or-nothing over the abstract tx state: never
/// both committed and aborted; committed ⇒ every staged write visible;
/// otherwise ⇒ nothing visible; and visible indices always name staged
/// writes.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn t1_holds(committed: bool, aborted: bool, staged_n: usize, visible: &[usize]) -> bool {
    if committed && aborted {
        return false;
    }
    let mut j = 0;
    while j < visible.len() {
        if visible[j] >= staged_n {
            return false;
        }
        j += 1;
    }
    if committed {
        visible.len() == staged_n
    } else {
        visible.is_empty()
    }
}

/// T1 AS-IS — byte-level integrity only: visible indices name staged
/// writes, but any subset may be visible whatever the status — an
/// aborted tx with partial effects committed passes.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn t1_holds_as_is(staged_n: usize, visible: &[usize]) -> bool {
    let mut j = 0;
    while j < visible.len() {
        if visible[j] >= staged_n {
            return false;
        }
        j += 1;
    }
    true
}

/// C1 — a served value is committed: replicated to a majority of every
/// active config (joint consensus: old AND new; single: old).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn c1_holds(
    old_n: u64,
    old_yes: u64,
    joint: bool,
    new_n: u64,
    new_yes: u64,
    served: bool,
) -> bool {
    if !served {
        return true;
    }
    if old_yes < majority(old_n) {
        return false;
    }
    if joint && new_yes < majority(new_n) {
        return false;
    }
    true
}

/// C1 AS-IS — the old majority alone suffices (the joint-election hole).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn c1_holds_as_is(old_n: u64, old_yes: u64, served: bool) -> bool {
    !served || old_yes >= majority(old_n)
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn d1_holds_spec(acked: Seq<bool>, survives: int) -> bool {
    forall|i: int| 0 <= i < acked.len() ==> (acked[i] ==> i < survives)
}

pub fn d1_holds(acked: &[bool], survives: usize) -> (b: bool)
    ensures
        b == d1_holds_spec(acked@, survives as int),
{
    let mut i = 0;
    while i < acked.len()
        invariant
            0 <= i <= acked.len(),
            forall|j: int| 0 <= j < i ==> (acked@[j] ==> j < survives as int),
        decreases acked.len() - i,
    {
        if acked[i] && i >= survives {
            assert(acked@[i as int] && i as int >= survives as int);
            return false;
        }
        i += 1;
    }
    true
}

pub open spec fn d1_holds_as_is_spec(synced: Seq<bool>, survives: int) -> bool {
    forall|i: int| 0 <= i < synced.len() ==> (synced[i] ==> i < survives)
}

pub fn d1_holds_as_is(synced: &[bool], survives: usize) -> (b: bool)
    ensures
        b == d1_holds_as_is_spec(synced@, survives as int),
{
    let mut i = 0;
    while i < synced.len()
        invariant
            0 <= i <= synced.len(),
            forall|j: int| 0 <= j < i ==> (synced@[j] ==> j < survives as int),
        decreases synced.len() - i,
    {
        if synced[i] && i >= survives {
            assert(synced@[i as int] && i as int >= survives as int);
            return false;
        }
        i += 1;
    }
    true
}

proof fn lemma_d1_as_is_does_not_imply_d1()
    ensures
        d1_holds_as_is_spec(seq![false], 0),
        !d1_holds_spec(seq![true], 0),
{
    assert(!d1_holds_spec(seq![true], 0)) by {
        assert(seq![true][0] == true);
        assert(!(0 < 0));
    };
}

} // verus!

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d1_as_is_does_not_imply_d1() {
        // acked-but-not-synced entry, crash keeps nothing: the sync=false
        // peer class. AS-IS (barrier-only) holds; D1 does not.
        let acked = [true];
        let synced = [false];
        assert!(d1_holds_as_is(&synced, 0));
        assert!(!d1_holds(&acked, 0));
    }

    #[test]
    fn r1_as_is_does_not_imply_r1() {
        // The resurrected-delete shape: newest source holds the
        // tombstone, an older one the value. AS-IS accepts the stale
        // value; R1 demands the tombstone.
        let probes = [Some(Slot::Tombstone), Some(Slot::Value(7))];
        assert!(r1_answer_ok_as_is(&probes, Some(Slot::Value(7))));
        assert!(!r1_answer_ok(&probes, Some(Slot::Value(7))));
        assert!(r1_answer_ok(&probes, Some(Slot::Tombstone)));
    }

    #[test]
    fn t1_as_is_does_not_imply_t1() {
        // Aborted tx with the first staged write visible: byte-level
        // integrity holds (AS-IS), atomicity does not (T1).
        assert!(t1_holds_as_is(2, &[0]));
        assert!(!t1_holds(false, true, 2, &[0]));
        assert!(t1_holds(true, false, 2, &[0, 1]));
    }

    #[test]
    fn c1_as_is_does_not_imply_c1() {
        // Joint config, old majority replicated, new majority not: the
        // entry is not committed, yet AS-IS lets a replica serve it.
        assert!(c1_holds_as_is(3, 2, true));
        assert!(!c1_holds(3, 2, true, 4, 1, true));
        assert!(c1_holds(3, 2, true, 4, 3, true));
    }
}
