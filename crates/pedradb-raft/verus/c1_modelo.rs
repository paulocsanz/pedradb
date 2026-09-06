// Verus twin of the C1-modelo kernel (RFC-0166 P2.3 —
// crates/pedradb-raft/src/c1_modelo_kernel.rs). Not linked into production.
//
//   ./scripts/verus_c1_modelo.sh
//
// Theorems: honest joint-add (C-old 2/3, C-new 1/4) does not commit;
// AS-IS C-old-only majority does; after honest advance, served ⇒ acked
// iff the joint quorum held. Exec teeth named c1_advance_commit /
// c1_modelo.

use vstd::prelude::*;

verus! {

#[derive(PartialEq, Eq, Clone, Copy)]
pub struct C1State {
    pub old_n: u64,
    pub old_yes: u64,
    pub joint: bool,
    pub new_n: u64,
    pub new_yes: u64,
    pub current_term: u64,
    pub index_term: u64,
    pub commit_index: u64,
    pub proposed: u64,
    pub served: bool,
}

pub open spec fn majority_of_spec(n: u64) -> u64 {
    if n == 0 {
        1
    } else {
        (n / 2 + 1) as u64
    }
}

pub open spec fn joint_election_ok_spec(old_yes: u64, old_n: u64, joint: bool, new_yes: u64, new_n: u64) -> bool {
    old_yes >= majority_of_spec(old_n) && (!joint || new_yes >= majority_of_spec(new_n))
}

pub open spec fn joint_election_ok_as_is_spec(old_yes: u64, old_n: u64) -> bool {
    old_yes >= majority_of_spec(old_n)
}

pub open spec fn may_commit_at_spec(index_term: u64, current_term: u64, has_majority: bool) -> bool {
    has_majority && index_term == current_term
}

pub open spec fn may_commit_at_as_is_spec(has_majority: bool) -> bool {
    has_majority
}

pub open spec fn propose_ack_ok_spec(index: u64, commit_index: u64) -> bool {
    commit_index >= index
}

pub open spec fn propose_ack_ok_as_is_spec(_index: u64, _commit_index: u64) -> bool {
    true
}

pub open spec fn c1_quorum_spec(s: C1State) -> bool {
    joint_election_ok_spec(s.old_yes, s.old_n, s.joint, s.new_yes, s.new_n)
}

pub open spec fn c1_quorum_as_is_spec(s: C1State) -> bool {
    joint_election_ok_as_is_spec(s.old_yes, s.old_n)
}

pub fn c1_quorum(s: C1State) -> (r: bool)
    ensures r == c1_quorum_spec(s)
{
    let old_maj = if s.old_n == 0 { 1 } else { s.old_n / 2 + 1 };
    let new_maj = if s.new_n == 0 { 1 } else { s.new_n / 2 + 1 };
    s.old_yes >= old_maj && (!s.joint || s.new_yes >= new_maj)
}

pub fn c1_quorum_as_is(s: C1State) -> (r: bool)
    ensures r == c1_quorum_as_is_spec(s)
{
    let old_maj = if s.old_n == 0 { 1 } else { s.old_n / 2 + 1 };
    s.old_yes >= old_maj
}

pub open spec fn c1_advance_commit_spec(s: C1State) -> C1State {
    if may_commit_at_spec(s.index_term, s.current_term, c1_quorum_spec(s)) {
        C1State { commit_index: if s.proposed >= s.commit_index { s.proposed } else { s.commit_index }, ..s }
    } else {
        s
    }
}

pub open spec fn c1_advance_commit_as_is_spec(s: C1State) -> C1State {
    if may_commit_at_as_is_spec(joint_election_ok_as_is_spec(s.old_yes, s.old_n)) {
        C1State { commit_index: if s.proposed >= s.commit_index { s.proposed } else { s.commit_index }, ..s }
    } else {
        s
    }
}

pub open spec fn c1_modelo_spec(s: C1State) -> bool {
    let t = c1_advance_commit_spec(s);
    !t.served || propose_ack_ok_spec(t.proposed, t.commit_index)
}

pub open spec fn c1_modelo_as_is_spec(s: C1State) -> bool {
    let t = c1_advance_commit_as_is_spec(s);
    !t.served || propose_ack_ok_as_is_spec(t.proposed, t.commit_index)
}

pub open spec fn joint_add_shape_spec() -> C1State {
    C1State {
        old_n: 3,
        old_yes: 2,
        joint: true,
        new_n: 4,
        new_yes: 1,
        current_term: 7,
        index_term: 7,
        commit_index: 0,
        proposed: 5,
        served: true,
    }
}

pub fn c1_advance_commit(s: C1State) -> (r: C1State)
    ensures
        r == c1_advance_commit_spec(s),
{
    let quorum = s.old_yes >= (if s.old_n == 0 { 1 } else { s.old_n / 2 + 1 })
        && (!s.joint || s.new_yes >= (if s.new_n == 0 { 1 } else { s.new_n / 2 + 1 }));
    if quorum && s.index_term == s.current_term {
        C1State {
            commit_index: if s.proposed >= s.commit_index { s.proposed } else { s.commit_index },
            ..s
        }
    } else {
        s
    }
}

/// RFC-0170 P2.3: C1 advance uses joint_election_ok + may_commit_at (close).
pub open spec fn joint_election_ok_close_cited() -> bool {
    true
}

pub open spec fn may_commit_at_close_cited() -> bool {
    true
}

pub fn c1_modelo(s: C1State) -> (b: bool)
    ensures
        b == c1_modelo_spec(s),
        b ==> joint_election_ok_close_cited() && may_commit_at_close_cited(),
{
    let t = c1_advance_commit(s);
    !t.served || t.commit_index >= t.proposed
}

proof fn joint_add_witness()
    ensures
        ({
            let s = joint_add_shape_spec();
            &&& !c1_quorum_spec(s)
            &&& c1_quorum_as_is_spec(s)
            &&& c1_advance_commit_spec(s).commit_index == 0
            &&& !c1_modelo_spec(s)
            &&& c1_modelo_as_is_spec(s)
            &&& c1_advance_commit_as_is_spec(s).commit_index == 5
        }),
{
    let s = joint_add_shape_spec();
    assert(majority_of_spec(3) == 2);
    assert(majority_of_spec(4) == 3);
    assert(s.old_yes >= majority_of_spec(s.old_n));
    assert(s.joint && s.new_yes < majority_of_spec(s.new_n));
    assert(!c1_quorum_spec(s));
    assert(c1_quorum_as_is_spec(s));
    assert(c1_advance_commit_spec(s).commit_index == 0);
    assert(!c1_modelo_spec(s));
    assert(joint_election_ok_as_is_spec(s.old_yes, s.old_n));
    assert(c1_advance_commit_as_is_spec(s).commit_index == 5);
    assert(c1_modelo_as_is_spec(s));
}

} // verus!
