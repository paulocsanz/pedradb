//! Stateright model over the **real** [`pedradb_core::range_tombstone_covers`] (F30).
//!
//! Faithfulness: `[start, end)` cover is production `merge`. Persist of the
//! tombstone and OCC conflict probe are axioms.
//!
//! - **Inv-interior:** a key strictly inside `[start, end)` is covered.
//! - **Inv-start:** `start` is covered (inclusive).
//! - **Inv-end-exclusive:** `end` is not covered.
//! - AS-IS only matches `start` and must miss the interior (teeth).

use pedradb_core::{range_tombstone_covers, range_tombstone_covers_as_is};
use stateright::{Checker, Model, Property};

const START: &[u8] = b"a";
const END: &[u8] = b"z";
const INTERIOR: &[u8] = b"m";
const BEFORE: &[u8] = b"0";
const AFTER: &[u8] = b"{";

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct St {
    interior_missed: bool,
    start_missed: bool,
    end_covered: bool,
    outside_covered: bool,
    saw_interior: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Start,
    Interior,
    End,
    Before,
    After,
}

#[derive(Clone)]
struct RangeModel {
    fixed: bool,
}

impl RangeModel {
    fn covers(&self, key: &[u8]) -> bool {
        if self.fixed {
            range_tombstone_covers(START, END, key)
        } else {
            range_tombstone_covers_as_is(START, END, key)
        }
    }
}

impl Model for RangeModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            interior_missed: false,
            start_missed: false,
            end_covered: false,
            outside_covered: false,
            saw_interior: false,
        }]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        actions.extend([Act::Start, Act::Interior, Act::End, Act::Before, Act::After]);
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::Start => {
                if !self.covers(START) {
                    next.start_missed = true;
                }
            }
            Act::Interior => {
                next.saw_interior = true;
                if !self.covers(INTERIOR) {
                    next.interior_missed = true;
                }
            }
            Act::End => {
                if self.covers(END) {
                    next.end_covered = true;
                }
            }
            Act::Before => {
                if self.covers(BEFORE) {
                    next.outside_covered = true;
                }
            }
            Act::After => {
                if self.covers(AFTER) {
                    next.outside_covered = true;
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-interior", inv_interior),
            Property::always("Inv-start", inv_start),
            Property::always("Inv-end-exclusive", inv_end),
            Property::always("Inv-outside", inv_outside),
            Property::sometimes("non-vacuity-interior", non_vacuity_interior),
        ]
    }
}

fn inv_interior(_: &RangeModel, s: &St) -> bool {
    !s.interior_missed
}

fn inv_start(_: &RangeModel, s: &St) -> bool {
    !s.start_missed
}

fn inv_end(_: &RangeModel, s: &St) -> bool {
    !s.end_covered
}

fn inv_outside(_: &RangeModel, s: &St) -> bool {
    !s.outside_covered
}

fn non_vacuity_interior(_: &RangeModel, s: &St) -> bool {
    s.saw_interior
}

#[test]
fn fixed_range_holds() {
    let checker = RangeModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_misses_interior() {
    let checker = RangeModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-interior").is_some(),
        "AS-IS must miss a covering delete on an interior key (F30 teeth)"
    );
}
