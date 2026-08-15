//! Stateright model over the **real** [`pedradb_fold::isolated_kernel`] (F83).
//!
//! Faithfulness: whether a key is this isolated id or a path child is
//! production `isolated_id_matches`. Prefix-set membership is the caller
//! (`in_prefixes`).
//!
//! - **Inv-no-sibling:** `/vm/vm-ab` is not `/vm/vm-a` (F83).
//! - **Inv-child:** `/vm/vm-a/disk` is a child.
//! - **Inv-exact:** the id itself matches.
//! - AS-IS `starts_with` must leak the sibling (teeth).

use pedradb_fold::{
    isolated_child_byte, isolated_child_byte_as_is, isolated_id_matches, isolated_id_matches_as_is,
};
use stateright::{Checker, Model, Property};

const ID: &[u8] = b"/vm/vm-a";
const EXACT: &[u8] = b"/vm/vm-a";
const CHILD: &[u8] = b"/vm/vm-a/disk";
const SIBLING: &[u8] = b"/vm/vm-ab";
const OTHER: &[u8] = b"/other";

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct St {
    sibling_leaked: bool,
    child_missed: bool,
    exact_missed: bool,
    saw_sibling: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Exact,
    Child,
    Sibling,
    Other,
    NextByte { b: u8 },
}

#[derive(Clone)]
struct IsolatedModel {
    fixed: bool,
}

impl IsolatedModel {
    fn matches(&self, key: &[u8]) -> bool {
        if self.fixed {
            isolated_id_matches(key, ID)
        } else {
            isolated_id_matches_as_is(key, ID)
        }
    }

    fn child_byte(&self, next: u8) -> bool {
        if self.fixed {
            isolated_child_byte(next)
        } else {
            isolated_child_byte_as_is(next)
        }
    }
}

impl Model for IsolatedModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            sibling_leaked: false,
            child_missed: false,
            exact_missed: false,
            saw_sibling: false,
        }]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        actions.extend([Act::Exact, Act::Child, Act::Sibling, Act::Other]);
        // Slash vs a sibling-continuation byte (the F83 witness).
        actions.push(Act::NextByte { b: b'/' });
        actions.push(Act::NextByte { b: b'b' });
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::Exact => {
                if !self.matches(EXACT) {
                    next.exact_missed = true;
                }
            }
            Act::Child => {
                if !self.matches(CHILD) {
                    next.child_missed = true;
                }
            }
            Act::Sibling => {
                next.saw_sibling = true;
                if self.matches(SIBLING) {
                    next.sibling_leaked = true;
                }
            }
            Act::Other => {
                if self.matches(OTHER) {
                    next.sibling_leaked = true;
                }
            }
            Act::NextByte { b } => {
                if b != b'/' && self.child_byte(b) {
                    next.sibling_leaked = true;
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-no-sibling", inv_no_sibling),
            Property::always("Inv-child", inv_child),
            Property::always("Inv-exact", inv_exact),
            Property::sometimes("non-vacuity-sibling", non_vacuity_sibling),
        ]
    }
}

fn inv_no_sibling(_: &IsolatedModel, s: &St) -> bool {
    !s.sibling_leaked
}

fn inv_child(_: &IsolatedModel, s: &St) -> bool {
    !s.child_missed
}

fn inv_exact(_: &IsolatedModel, s: &St) -> bool {
    !s.exact_missed
}

fn non_vacuity_sibling(_: &IsolatedModel, s: &St) -> bool {
    s.saw_sibling
}

#[test]
fn fixed_isolated_holds() {
    let checker = IsolatedModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_leaks_sibling() {
    let checker = IsolatedModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-no-sibling").is_some(),
        "AS-IS starts_with must match /vm/vm-ab under /vm/vm-a (F83 teeth)"
    );
}
