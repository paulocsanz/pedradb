//! Stateright model over the **real** [`pedradb_store`] compact kernel (F27 / F28).
//!
//! Faithfulness: who counts in `min(applied)` and whether `through` may drop
//! are production `compact_kernel`. Persist of snap/log and AE catch-up are
//! axioms.
//!
//! - **Inv-offline-floor:** compact never advances past a peer that still
//!   counts (F28: partitioned members freeze the watermark).
//! - **Inv-term-present:** compact refuses `term_at(through) == 0` (F27).
//! - AS-IS mutants must produce a counterexample (teeth).

use pedradb_store::{
    compact_ready, may_compact_through, may_compact_through_as_is, peer_counts_for_compact,
    peer_counts_for_compact_as_is,
};
use stateright::{Checker, Model, Property};

const N: usize = 3;
const APPLIED_CAP: u8 = 3;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct St {
    applied: [u8; N],
    /// Bit `i` set ⇒ peer `i` is participating.
    live: u8,
    snap: u8,
    term_missing: bool,
    dropped_offline: bool,
    compacted_missing: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Advance { peer: u8 },
    Partition { peer: u8 },
    Heal { peer: u8 },
    LoseTerm,
    Compact,
}

#[derive(Clone)]
struct CompactModel {
    fixed: bool,
}

impl CompactModel {
    fn counts(&self, participating: bool) -> bool {
        if self.fixed {
            peer_counts_for_compact(participating)
        } else {
            peer_counts_for_compact_as_is(participating)
        }
    }

    fn may_through(&self, snap: u64, through: u64, term: u64) -> bool {
        if self.fixed {
            may_compact_through(snap, through, term)
        } else {
            may_compact_through_as_is(snap, through, term)
        }
    }

    fn min_applied(&self, s: &St) -> u64 {
        let mut m = u64::MAX;
        for i in 0..N {
            if self.counts(live_bit(s.live, i)) {
                m = m.min(u64::from(s.applied[i]));
            }
        }
        if m == u64::MAX {
            0
        } else {
            m
        }
    }
}

fn live_bit(live: u8, i: usize) -> bool {
    live & (1 << i) != 0
}

fn set_live(live: u8, i: usize, on: bool) -> u8 {
    if on {
        live | (1 << i)
    } else {
        live & !(1 << i)
    }
}

impl Model for CompactModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            applied: [0; N],
            live: (1 << N) - 1,
            snap: 0,
            term_missing: false,
            dropped_offline: false,
            compacted_missing: false,
        }]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        for peer in 0u8..3 {
            actions.push(Act::Advance { peer });
            actions.push(Act::Partition { peer });
            actions.push(Act::Heal { peer });
        }
        actions.push(Act::LoseTerm);
        actions.push(Act::Compact);
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::Advance { peer } => {
                let i = usize::from(peer);
                if i >= N || state.applied[i] >= APPLIED_CAP {
                    return None;
                }
                next.applied[i] = state.applied[i] + 1;
            }
            Act::Partition { peer } => {
                let i = usize::from(peer);
                if i >= N || !live_bit(state.live, i) {
                    return None;
                }
                next.live = set_live(state.live, i, false);
            }
            Act::Heal { peer } => {
                let i = usize::from(peer);
                if i >= N || live_bit(state.live, i) {
                    return None;
                }
                next.live = set_live(state.live, i, true);
            }
            Act::LoseTerm => {
                if state.term_missing {
                    return None;
                }
                next.term_missing = true;
            }
            Act::Compact => {
                let through = self.min_applied(state);
                if !compact_ready(through) {
                    return None;
                }
                let term = u64::from(!state.term_missing);
                if !self.may_through(u64::from(state.snap), through, term) {
                    return None;
                }
                if term == 0 {
                    next.compacted_missing = true;
                }
                for i in 0..N {
                    if u64::from(state.applied[i]) < through {
                        next.dropped_offline = true;
                    }
                }
                next.snap = u8::try_from(through).unwrap_or(u8::MAX);
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-offline-floor", inv_offline_floor),
            Property::always("Inv-term-present", inv_term_present),
            Property::sometimes("non-vacuity-advance", non_vacuity_advance),
            Property::sometimes("non-vacuity-compact", non_vacuity_compact),
        ]
    }
}

fn inv_offline_floor(_: &CompactModel, s: &St) -> bool {
    !s.dropped_offline
}

fn inv_term_present(_: &CompactModel, s: &St) -> bool {
    !s.compacted_missing
}

fn non_vacuity_advance(_: &CompactModel, s: &St) -> bool {
    s.applied.iter().any(|&a| a > 0)
}

fn non_vacuity_compact(_: &CompactModel, s: &St) -> bool {
    s.snap > 0
}

#[test]
fn fixed_compact_holds() {
    let checker = CompactModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_compacts_past_offline() {
    let checker = CompactModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-offline-floor").is_some(),
        "AS-IS must ignore partitioned peers (F28 teeth)"
    );
}

#[test]
fn as_is_compacts_missing_term() {
    let checker = CompactModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-term-present").is_some(),
        "AS-IS must compact when term_at(through)==0 (F27 teeth)"
    );
}
