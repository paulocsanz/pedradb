//! Stateright model over the **real** [`pedradb_store::si_kernel`] (F42 / F84).
//!
//! Faithfulness: fold order and point-get watermark are production
//! `si_reader_beats` / `point_get_watermark`. Persist of SI hist is an axiom.
//!
//! - **Inv-no-ids0-poison:** a partitioned `ids[0]` never wins if a later
//!   peer beats it (F42).
//! - **Inv-range-applied:** point get ranks by per-range `applied`, not
//!   global `last_sequence` (F84).
//! - AS-IS mutants must produce a counterexample (teeth).

use pedradb_store::{
    point_get_prefer_applied, point_get_prefer_applied_as_is, point_get_watermark,
    point_get_watermark_as_is, si_reader_beats, si_reader_beats_as_is,
};
use stateright::{Checker, Model, Property};

const N: usize = 3;
const CAP: u8 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
struct Peer {
    part: bool,
    leader: bool,
    applied: u8,
    global: u8,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct St {
    peers: [Peer; N],
    poisoned: bool,
    stale_range: bool,
    saw_fold: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Partition { i: u8 },
    Heal { i: u8 },
    Elect { i: u8 },
    AdvanceApplied { i: u8 },
    BumpGlobal { i: u8 },
    FoldApplied,
    PointGet { i: u8 },
}

#[derive(Clone)]
struct SiModel {
    fixed: bool,
}

impl SiModel {
    #[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
    fn beats(
        &self,
        c_lead: bool,
        c_part: bool,
        c_self: bool,
        c_app: u64,
        b_lead: bool,
        b_part: bool,
        b_self: bool,
        b_app: u64,
    ) -> bool {
        if self.fixed {
            si_reader_beats(c_lead, c_part, c_self, c_app, b_lead, b_part, b_self, b_app)
        } else {
            si_reader_beats_as_is(c_lead, c_part, c_self, c_app, b_lead, b_part, b_self, b_app)
        }
    }

    fn watermark(&self, applied: u64, global: u64) -> u64 {
        if self.fixed {
            point_get_watermark(applied, global)
        } else {
            point_get_watermark_as_is(applied, global)
        }
    }

    fn prefer_applied(&self) -> bool {
        if self.fixed {
            point_get_prefer_applied()
        } else {
            point_get_prefer_applied_as_is()
        }
    }

    /// Same fold as `best_applied_reader`: first id, then `si_reader_beats`.
    fn fold_applied(&self, s: &St) -> usize {
        let mut best = 0usize;
        for i in 1..N {
            let c = s.peers[i];
            let b = s.peers[best];
            if self.beats(
                c.leader,
                c.part,
                i == 0,
                u64::from(c.applied),
                b.leader,
                b.part,
                best == 0,
                u64::from(b.applied),
            ) {
                best = i;
            }
        }
        best
    }
}

impl Model for SiModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            peers: [
                Peer {
                    part: true,
                    leader: false,
                    applied: 0,
                    global: 0,
                },
                Peer {
                    part: true,
                    leader: false,
                    applied: 0,
                    global: 0,
                },
                Peer {
                    part: true,
                    leader: false,
                    applied: 0,
                    global: 0,
                },
            ],
            poisoned: false,
            stale_range: false,
            saw_fold: false,
        }]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        for i in 0u8..3 {
            actions.push(Act::Partition { i });
            actions.push(Act::Heal { i });
            actions.push(Act::Elect { i });
            actions.push(Act::AdvanceApplied { i });
            actions.push(Act::BumpGlobal { i });
            actions.push(Act::PointGet { i });
        }
        actions.push(Act::FoldApplied);
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::Partition { i } => {
                let i = usize::from(i);
                if i >= N || !state.peers[i].part {
                    return None;
                }
                next.peers[i].part = false;
                next.peers[i].leader = false;
            }
            Act::Heal { i } => {
                let i = usize::from(i);
                if i >= N || state.peers[i].part {
                    return None;
                }
                next.peers[i].part = true;
            }
            Act::Elect { i } => {
                let i = usize::from(i);
                if i >= N || !state.peers[i].part {
                    return None;
                }
                for p in &mut next.peers {
                    p.leader = false;
                }
                next.peers[i].leader = true;
            }
            Act::AdvanceApplied { i } => {
                let i = usize::from(i);
                if i >= N || state.peers[i].applied >= CAP {
                    return None;
                }
                next.peers[i].applied += 1;
            }
            Act::BumpGlobal { i } => {
                let i = usize::from(i);
                if i >= N || state.peers[i].global >= CAP {
                    return None;
                }
                next.peers[i].global += 1;
            }
            Act::FoldApplied => {
                next.saw_fold = true;
                let best = self.fold_applied(state);
                let first = state.peers[0];
                // Poison: fold kept ids[0] while the FIXED ranking says a later
                // peer wins (AS-IS never beats, so first always stays).
                if best == 0 && !first.part {
                    for c in state.peers.iter().skip(1) {
                        if si_reader_beats(
                            c.leader,
                            c.part,
                            false,
                            u64::from(c.applied),
                            first.leader,
                            first.part,
                            true,
                            u64::from(first.applied),
                        ) {
                            next.poisoned = true;
                        }
                    }
                }
            }
            Act::PointGet { i } => {
                let i = usize::from(i);
                if i >= N {
                    return None;
                }
                let p = state.peers[i];
                let wm = if self.prefer_applied() {
                    self.watermark(u64::from(p.applied), u64::from(p.global))
                } else {
                    point_get_watermark_as_is(u64::from(p.applied), u64::from(p.global))
                };
                if wm > u64::from(p.applied) {
                    next.stale_range = true;
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-no-ids0-poison", inv_no_ids0_poison),
            Property::always("Inv-range-applied", inv_range_applied),
            Property::sometimes("non-vacuity-fold", non_vacuity_fold),
        ]
    }
}

fn inv_no_ids0_poison(_: &SiModel, s: &St) -> bool {
    !s.poisoned
}

fn inv_range_applied(_: &SiModel, s: &St) -> bool {
    !s.stale_range
}

fn non_vacuity_fold(_: &SiModel, s: &St) -> bool {
    s.saw_fold
}

#[test]
fn fixed_si_holds() {
    let checker = SiModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_keeps_partitioned_first() {
    let checker = SiModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-no-ids0-poison").is_some(),
        "AS-IS must keep partitioned ids[0] (F42 teeth)"
    );
}

#[test]
fn as_is_ranks_by_global_seq() {
    let checker = SiModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-range-applied").is_some(),
        "AS-IS point get must use last_sequence over range applied (F84 teeth)"
    );
}
