//! Stateright model over the **real** [`pedradb_fold::fold_event_hides_key`]
//! (F169).
//!
//! Faithfulness: the predicate is what `last_per_key` / dest apply use to
//! drop keys under a changelog range tombstone. A `false` result for a
//! covered key is the silent-wrong resurrection the repro pinned.
//!
//! Domain: `u8` keys (same total order as `[u8]`).
//!
//! - **Inv-range-covers:** range event + `start <= key < end` ⇒ hides.
//! - **Inv-range-outside:** range event + key outside ⇒ does not hide.
//! - **Inv-point-exact:** point event hides only the exact key.
//! - AS-IS must miss a covered key (teeth).

use pedradb_fold::{fold_event_hides_key, fold_event_hides_key_as_is};
use stateright::{Checker, Model, Property};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct St {
    decided: bool,
    missed_cover: bool,
    hid_outside: bool,
    point_wrong: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Ask,
}

#[derive(Clone)]
struct FoldRangeModel {
    is_range: bool,
    start: u8,
    end: u8,
    key: u8,
    fixed: bool,
}

impl FoldRangeModel {
    fn hides(&self) -> bool {
        let s = [self.start];
        let e = [self.end];
        let k = [self.key];
        if self.fixed {
            fold_event_hides_key(self.is_range, &s, &e, &k)
        } else {
            fold_event_hides_key_as_is(self.is_range, &s, &e, &k)
        }
    }

    fn covered(&self) -> bool {
        self.is_range && self.start <= self.key && self.key < self.end
    }
}

impl Model for FoldRangeModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<St> {
        vec![St {
            decided: false,
            missed_cover: false,
            hid_outside: false,
            point_wrong: false,
        }]
    }

    fn actions(&self, s: &St, actions: &mut Vec<Act>) {
        if !s.decided {
            actions.push(Act::Ask);
        }
    }

    fn next_state(&self, s: &St, _a: Act) -> Option<St> {
        let mut next = s.clone();
        next.decided = true;
        let h = self.hides();
        if self.covered() && !h {
            next.missed_cover = true;
        }
        if self.is_range && !self.covered() && h {
            next.hid_outside = true;
        }
        if !self.is_range {
            let want = self.key == self.start;
            if h != want {
                next.point_wrong = true;
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-range-covers", |_m, s: &St| !s.missed_cover),
            Property::always("Inv-range-outside", |_m, s: &St| !s.hid_outside),
            Property::always("Inv-point-exact", |_m, s: &St| !s.point_wrong),
            Property::sometimes("non-vacuity", |_m, s: &St| s.decided),
        ]
    }
}

fn worlds() -> Vec<FoldRangeModel> {
    let mut out = Vec::new();
    let keys = [0u8, 1, 2, 3, 4, 5];
    for is_range in [true, false] {
        for start in keys {
            for end in keys {
                for key in keys {
                    for fixed in [true, false] {
                        out.push(FoldRangeModel {
                            is_range,
                            start,
                            end,
                            key,
                            fixed,
                        });
                    }
                }
            }
        }
    }
    out
}

#[test]
fn fixed_guard_holds_in_all_worlds() {
    for model in worlds() {
        if !model.fixed {
            continue;
        }
        model.checker().spawn_bfs().join().assert_properties();
    }
}

/// F169 teeth: AS-IS hides only the start, missing k-c in [k-b, k-d).
#[test]
fn as_is_misses_covered_key() {
    let model = FoldRangeModel {
        is_range: true,
        start: 2,
        end: 4,
        key: 3,
        fixed: false,
    };
    assert!(
        !model.hides(),
        "AS-IS must miss the covered key for the teeth to mean anything"
    );
    assert!(model.covered());
    let checker = model.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-range-covers").is_some(),
        "AS-IS must miss a covered key (F169 teeth)"
    );
}
