// Verus twin of the RFC-0166 property kernel (crates/pedradb-spec/src/
// properties_kernel.rs). Not linked into production.
//
//   ./scripts/verus_spec_properties.sh
//
// Theorems:
// - exec == spec for D1/R1/T1/C1 and their AS-IS mutants;
// - one named teeth witness per property: the AS-IS weak version holds
//   and the property does not (anti-vacuity at the property level);
// - D1 bridge: AS-IS (barrier-only) + the WAL invariant acked=>synced
//   implies D1 — the exact seam the Inv-WAL kernel (RFC-0166 P1.2)
//   closes on the implementation side.

use vstd::prelude::*;

verus! {

// ---------------------------------------------------------------------------
// D1 — durability of the ack
// ---------------------------------------------------------------------------

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

/// Teeth (D1): the sync=false peer class — acked-but-not-synced entry,
/// crash keeps nothing. Barrier-only AS-IS holds; D1 does not.
proof fn d1_as_is_does_not_imply_d1()
    ensures
        d1_holds_as_is_spec(seq![false], 0),
        !d1_holds_spec(seq![true], 0),
{
    // Instantiate the negated forall at i = 0 (Z3 needs a ground term
    // matching the trigger acked[i]): seq![true][0] holds, 0 < 0 does not.
    assert(!d1_holds_spec(seq![true], 0)) by {
        assert(seq![true][0] == true);
    }
}

/// D1 bridge: barrier-only AS-IS + the WAL invariant (acked => synced)
/// implies D1. This is the seam Inv-WAL (RFC-0166 P1.2) closes: the
/// implementation never acks an unsynced entry, so the weaker barrier
/// class plus the invariant delivers the product promise.
proof fn d1_as_is_and_inv_implies_d1(acked: &[bool], synced: &[bool], survives: usize)
    requires
        acked@.len() == synced@.len(),
        forall|i: int| 0 <= i < acked@.len() ==> (acked@[i] ==> synced@[i]),
        d1_holds_as_is_spec(synced@, survives as int),
    ensures
        d1_holds_spec(acked@, survives as int),
{
    assert(forall|i: int| 0 <= i < acked@.len() ==> (acked@[i] ==> i < survives as int));
}

// ---------------------------------------------------------------------------
// R1 — the read answer is the newest covering entry
// ---------------------------------------------------------------------------

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Slot {
    Value(u64),
    Tombstone,
}

pub open spec fn r1_first_hit_rec(probes: Seq<Option<Slot>>, i: int) -> Option<Slot>
    decreases probes.len() - i,
{
    if i < 0 || i >= probes.len() {
        None
    } else if probes[i].is_some() {
        probes[i]
    } else {
        r1_first_hit_rec(probes, i + 1)
    }
}

pub open spec fn r1_first_hit_spec(probes: Seq<Option<Slot>>) -> Option<Slot> {
    r1_first_hit_rec(probes, 0)
}

proof fn r1_rec_skip_none(probes: Seq<Option<Slot>>, i: int, k: int)
    requires
        0 <= i <= k <= probes.len(),
        forall|j: int| i <= j < k ==> probes[j].is_none(),
    ensures r1_first_hit_rec(probes, i) == r1_first_hit_rec(probes, k)
    decreases k - i,
{
    if i < k {
        r1_rec_skip_none(probes, i + 1, k);
    }
}

pub fn r1_first_hit(probes: &[Option<Slot>]) -> (r: Option<Slot>)
    ensures
        r == r1_first_hit_spec(probes@),
{
    let mut i = 0;
    while i < probes.len()
        invariant
            0 <= i <= probes.len(),
            forall|j: int| 0 <= j < i ==> probes@[j].is_none(),
            r1_first_hit_rec(probes@, 0) == r1_first_hit_rec(probes@, i as int),
        decreases probes.len() - i,
    {
        if probes[i].is_some() {
            proof {
                r1_rec_skip_none(probes@, 0, i as int);
            }
            assert(r1_first_hit_rec(probes@, i as int) == probes@[i as int]);
            return probes[i];
        }
        proof {
            r1_rec_skip_none(probes@, 0, i as int + 1);
        }
        i += 1;
    }
    assert(r1_first_hit_rec(probes@, probes.len() as int) == None);
    None
}

pub open spec fn r1_answer_ok_spec(probes: Seq<Option<Slot>>, answer: Option<Slot>) -> bool {
    answer == r1_first_hit_spec(probes)
}

/// Exec-level structural equality with a spec-level ensures — exec `==`
/// on Option<Slot> has no known tie to spec equality, so the check is
/// by-constructor with only u64 equality in exec code.
fn slot_eq(a: Slot, b: Slot) -> (r: bool)
    ensures
        r == (a == b),
{
    match (a, b) {
        (Slot::Value(x), Slot::Value(y)) => x == y,
        (Slot::Tombstone, Slot::Tombstone) => true,
        _ => false,
    }
}

pub fn r1_answer_ok(probes: &[Option<Slot>], answer: Option<Slot>) -> (b: bool)
    ensures
        b == r1_answer_ok_spec(probes@, answer),
{
    let hit = r1_first_hit(probes);
    match (answer, hit) {
        (None, None) => true,
        (Some(a), Some(h)) => slot_eq(a, h),
        _ => false,
    }
}

pub open spec fn r1_answer_ok_as_is_spec(probes: Seq<Option<Slot>>, answer: Option<Slot>) -> bool {
    answer.is_some() == r1_first_hit_spec(probes).is_some()
}

pub fn r1_answer_ok_as_is(probes: &[Option<Slot>], answer: Option<Slot>) -> (b: bool)
    ensures
        b == r1_answer_ok_as_is_spec(probes@, answer),
{
    let hit = r1_first_hit(probes);
    let answer_some = match answer {
        Some(_) => true,
        None => false,
    };
    let hit_some = match hit {
        Some(_) => true,
        None => false,
    };
    answer_some == hit_some
}

/// Teeth (R1): the resurrected-delete shape — newest source holds the
/// tombstone, an older one the value. AS-IS accepts the stale value;
/// R1 rejects it and demands the tombstone.
proof fn r1_as_is_does_not_imply_r1()
    ensures
        r1_answer_ok_as_is_spec(
            seq![Some(Slot::Tombstone), Some(Slot::Value(7))],
            Some(Slot::Value(7)),
        ),
        !r1_answer_ok_spec(
            seq![Some(Slot::Tombstone), Some(Slot::Value(7))],
            Some(Slot::Value(7)),
        ),
        r1_answer_ok_spec(
            seq![Some(Slot::Tombstone), Some(Slot::Value(7))],
            Some(Slot::Tombstone),
        ),
{
}

// ---------------------------------------------------------------------------
// T1 — transaction all-or-nothing
// ---------------------------------------------------------------------------

pub open spec fn t1_holds_spec(
    committed: bool,
    aborted: bool,
    staged_n: usize,
    visible: Seq<usize>,
) -> bool {
    (committed && aborted) == false
    && (forall|j: int| 0 <= j < visible.len() ==> visible[j] < staged_n)
    && (if committed { visible.len() == staged_n as int } else { visible.len() == 0 })
}

pub fn t1_holds(committed: bool, aborted: bool, staged_n: usize, visible: &[usize]) -> (b: bool)
    ensures
        b == t1_holds_spec(committed, aborted, staged_n, visible@),
{
    if committed && aborted {
        return false;
    }
    let mut j = 0;
    while j < visible.len()
        invariant
            0 <= j <= visible.len(),
            forall|k: int| 0 <= k < j ==> visible@[k] < staged_n,
        decreases visible.len() - j,
    {
        if visible[j] >= staged_n {
            assert(visible@[j as int] >= staged_n);
            return false;
        }
        j += 1;
    }
    if committed {
        visible.len() == staged_n
    } else {
        visible.len() == 0
    }
}

pub open spec fn t1_holds_as_is_spec(staged_n: usize, visible: Seq<usize>) -> bool {
    forall|j: int| 0 <= j < visible.len() ==> visible[j] < staged_n
}

pub fn t1_holds_as_is(staged_n: usize, visible: &[usize]) -> (b: bool)
    ensures
        b == t1_holds_as_is_spec(staged_n, visible@),
{
    let mut j = 0;
    while j < visible.len()
        invariant
            0 <= j <= visible.len(),
            forall|k: int| 0 <= k < j ==> visible@[k] < staged_n,
        decreases visible.len() - j,
    {
        if visible[j] >= staged_n {
            assert(visible@[j as int] >= staged_n);
            return false;
        }
        j += 1;
    }
    true
}

/// Teeth (T1): aborted tx with the first staged write visible — byte
/// integrity holds (AS-IS), atomicity does not (T1).
proof fn t1_as_is_does_not_imply_t1()
    ensures
        t1_holds_as_is_spec(2, seq![0]),
        !t1_holds_spec(false, true, 2, seq![0]),
        t1_holds_spec(true, false, 2, seq![0, 1]),
{
}

// ---------------------------------------------------------------------------
// C1 — served implies committed in every active config
// ---------------------------------------------------------------------------

pub open spec fn c1_majority_spec(n: u64) -> int {
    if n == 0 {
        1
    } else {
        n / 2 + 1
    }
}

pub fn c1_majority(n: u64) -> (m: u64)
    ensures
        m == c1_majority_spec(n),
{
    if n == 0 {
        1
    } else {
        n / 2 + 1
    }
}

fn majority(n: u64) -> (m: u64)
    ensures
        m == c1_majority_spec(n),
{
    c1_majority(n)
}

pub open spec fn c1_holds_spec(
    old_n: u64,
    old_yes: u64,
    joint: bool,
    new_n: u64,
    new_yes: u64,
    served: bool,
) -> bool {
    !served || (old_yes >= c1_majority_spec(old_n) && (!joint || new_yes >= c1_majority_spec(new_n)))
}

pub fn c1_holds(
    old_n: u64,
    old_yes: u64,
    joint: bool,
    new_n: u64,
    new_yes: u64,
    served: bool,
) -> (b: bool)
    ensures
        b == c1_holds_spec(old_n, old_yes, joint, new_n, new_yes, served),
{
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

pub open spec fn c1_holds_as_is_spec(old_n: u64, old_yes: u64, served: bool) -> bool {
    !served || old_yes >= c1_majority_spec(old_n)
}

pub fn c1_holds_as_is(old_n: u64, old_yes: u64, served: bool) -> (b: bool)
    ensures
        b == c1_holds_as_is_spec(old_n, old_yes, served),
{
    !served || old_yes >= majority(old_n)
}

/// Teeth (C1): joint config, old majority replicated, new majority not
/// — the entry is not committed, yet AS-IS lets a replica serve it.
proof fn c1_as_is_does_not_imply_c1()
    ensures
        c1_holds_as_is_spec(3, 2, true),
        !c1_holds_spec(3, 2, true, 4, 1, true),
        c1_holds_spec(3, 2, true, 4, 3, true),
{
}

} // verus!
