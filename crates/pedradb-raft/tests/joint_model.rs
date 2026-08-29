//! RFC-0064 P1.1: Stateright over production [`joint_election_ok`].
//!
//! Joint add 3→4: C-old maj=2, C-new maj=3. Voters {1,2,3} are C-old;
//! {1,2,3,4} are C-new. The model calls the **same** kernel the store uses
//! to promote a candidate — not a paraphrase.
//!
//! - **Inv-joint:** a leader is never elected on C-old majority alone.
//! - AS-IS (`joint_election_ok_as_is`) must discover the invariant
//!   (the 0064 hole: elect during joint add with 2 of 3 old votes).

use pedradb_raft::{joint_election_ok, joint_election_ok_as_is};
use stateright::{Checker, Model, Property};

const OLD_N: u64 = 3;
const NEW_N: u64 = 4;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct St {
    /// Bit i set ⇒ voter (i+1) granted.
    granted: u8,
    elected: bool,
    /// Elected without C-new majority (the hole).
    elected_old_only: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Grant { voter: u8 },
    TryElect,
}

#[derive(Clone)]
struct JointModel {
    fixed: bool,
}

impl JointModel {
    fn ok(&self, old_yes: u64, new_yes: u64) -> bool {
        if self.fixed {
            joint_election_ok(old_yes, OLD_N, Some((new_yes, NEW_N)))
        } else {
            joint_election_ok_as_is(old_yes, OLD_N, Some((new_yes, NEW_N)))
        }
    }
}

fn yes(granted: u8, n: u64) -> u64 {
    (0..n).filter(|i| granted & (1 << i) != 0).count() as u64
}

impl Model for JointModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            granted: 0,
            elected: false,
            elected_old_only: false,
        }]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        if state.elected {
            return;
        }
        for v in 0..4u8 {
            if state.granted & (1 << v) == 0 {
                actions.push(Act::Grant { voter: v });
            }
        }
        if state.granted != 0 {
            actions.push(Act::TryElect);
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::Grant { voter } => {
                next.granted |= 1 << voter;
            }
            Act::TryElect => {
                let old_yes = yes(state.granted, OLD_N);
                let new_yes = yes(state.granted, NEW_N);
                if self.ok(old_yes, new_yes) {
                    next.elected = true;
                    if old_yes >= 2 && new_yes < 3 {
                        next.elected_old_only = true;
                    }
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-joint", inv_joint),
            Property::sometimes("non-vacuity-joint-elect", non_vacuity),
        ]
    }
}

fn inv_joint(_: &JointModel, s: &St) -> bool {
    !s.elected_old_only
}

fn non_vacuity(_: &JointModel, s: &St) -> bool {
    s.elected && !s.elected_old_only
}

#[test]
fn fixed_joint_election_holds() {
    let checker = JointModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_elects_on_old_only() {
    let checker = JointModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-joint").is_some(),
        "AS-IS must elect with 2/3 C-old during joint add (0064 tooth)"
    );
}
