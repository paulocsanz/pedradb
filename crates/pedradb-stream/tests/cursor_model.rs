//! Stateright model over the **real** [`pedradb_stream::cursor_kernel`] (F54).
//!
//! Faithfulness: peek pin and ack-in-order are production `cursor_kernel`.
//! Persist of the consumer cursor is an axiom.
//!
//! - **Inv-no-hole:** ack never skips an unacked seq.
//! - **Inv-peek-no-pin:** peek does not persist the cursor ahead of apply.
//! - AS-IS mutants must produce a counterexample (teeth).

use pedradb_stream::{
    ack_in_order, ack_in_order_as_is, next_seq, peek_pins_cursor, peek_pins_cursor_as_is,
};
use stateright::{Checker, Model, Property};

const FRONTIER_CAP: u64 = 3;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct St {
    frontier: u64,
    last_acked: u64,
    applied: u64,
    hole: bool,
    pin_ahead: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Publish,
    Peek,
    Apply,
    Ack { seq: u64 },
}

#[derive(Clone)]
struct CursorModel {
    fixed: bool,
}

impl CursorModel {
    fn may_ack(&self, last: u64, seq: u64) -> bool {
        if self.fixed {
            ack_in_order(last, seq)
        } else {
            ack_in_order_as_is(last, seq)
        }
    }

    fn peek_pins(&self) -> bool {
        if self.fixed {
            peek_pins_cursor()
        } else {
            peek_pins_cursor_as_is()
        }
    }
}

impl Model for CursorModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            frontier: 0,
            last_acked: 0,
            applied: 0,
            hole: false,
            pin_ahead: false,
        }]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        actions.extend([Act::Publish, Act::Peek, Act::Apply]);
        for seq in 1..=FRONTIER_CAP {
            actions.push(Act::Ack { seq });
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::Publish => {
                if state.frontier >= FRONTIER_CAP {
                    return None;
                }
                next.frontier = state.frontier + 1;
            }
            Act::Peek => {
                if self.peek_pins() && state.frontier > state.last_acked {
                    next.last_acked = state.frontier;
                    if next.last_acked > next.applied {
                        next.pin_ahead = true;
                    }
                }
            }
            Act::Apply => next.applied = state.frontier,
            Act::Ack { seq } => {
                // Production: ack after the caller applied `seq`.
                if seq == 0 || seq > state.frontier || seq > state.applied {
                    return None;
                }
                if self.may_ack(state.last_acked, seq) {
                    if seq != next_seq(state.last_acked) {
                        next.hole = true;
                    }
                    next.last_acked = seq;
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-no-hole", inv_no_hole),
            Property::always("Inv-peek-no-pin", inv_peek_no_pin),
            Property::sometimes("non-vacuity-publish", non_vacuity_publish),
            Property::sometimes("non-vacuity-ack", non_vacuity_ack),
        ]
    }
}

fn inv_no_hole(_: &CursorModel, s: &St) -> bool {
    !s.hole
}

fn inv_peek_no_pin(_: &CursorModel, s: &St) -> bool {
    !s.pin_ahead
}

fn non_vacuity_publish(_: &CursorModel, s: &St) -> bool {
    s.frontier > 0
}

fn non_vacuity_ack(_: &CursorModel, s: &St) -> bool {
    s.last_acked > 0
}

#[test]
fn fixed_cursor_holds() {
    let checker = CursorModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_skips_unacked() {
    let checker = CursorModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-no-hole").is_some(),
        "AS-IS ack must skip seqs (F54 teeth)"
    );
}

#[test]
fn as_is_peek_pins_ahead() {
    let checker = CursorModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-peek-no-pin").is_some(),
        "AS-IS peek must persist the cursor before apply (F54 teeth)"
    );
}
