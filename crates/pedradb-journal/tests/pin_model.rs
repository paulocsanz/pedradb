//! Stateright model over the **real** [`pedradb_journal::pin_kernel`] (H1).
//!
//! Fold path: peek → apply → `pin_after_apply`. Peek must not move the pin.
//! AS-IS `peek_pins_cursor` pins on receipt → pin can exceed applied.

use pedradb_journal::pin_kernel::{may_advance_pin, peek_pins_cursor, peek_pins_cursor_as_is};
use stateright::{Checker, Model, Property};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct St {
    frontier: u64,
    applied: u64,
    pin: u64,
    pin_ahead: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Publish,
    Peek,
    Apply,
    PinAfter,
}

#[derive(Clone)]
struct PinModel {
    fixed: bool,
}

impl PinModel {
    fn peek_pins(&self) -> bool {
        if self.fixed {
            peek_pins_cursor()
        } else {
            peek_pins_cursor_as_is()
        }
    }
}

impl Model for PinModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            frontier: 0,
            applied: 0,
            pin: 0,
            pin_ahead: false,
        }]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        actions.extend([Act::Publish, Act::Peek, Act::Apply, Act::PinAfter]);
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::Publish => {
                if next.frontier < 3 {
                    next.frontier += 1;
                }
            }
            Act::Peek => {
                if self.peek_pins() && next.frontier > next.pin {
                    next.pin = next.frontier;
                }
            }
            Act::Apply => next.applied = next.frontier,
            Act::PinAfter => {
                if may_advance_pin(next.pin, next.applied) {
                    next.pin = next.applied;
                }
            }
        }
        if next.pin > next.applied {
            next.pin_ahead = true;
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-pin-le-applied", inv_pin_le_applied),
            Property::sometimes("non-vacuity-publish", non_vacuity_publish),
        ]
    }
}

fn inv_pin_le_applied(_: &PinModel, s: &St) -> bool {
    !s.pin_ahead
}

fn non_vacuity_publish(_: &PinModel, s: &St) -> bool {
    s.frontier > 0
}

#[test]
fn fixed_peek_never_pins_ahead() {
    let checker = PinModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_peek_can_pin_ahead() {
    let checker = PinModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-pin-le-applied").is_some(),
        "AS-IS peek_pins must leave pin > applied (H1 teeth)"
    );
}
