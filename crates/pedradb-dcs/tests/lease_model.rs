//! Stateright model over the **real** [`pedradb_dcs::lease_kernel`] (F7 / F56).
//!
//! Faithfulness: grant id, table miss, and live-deadline are production
//! `lease_kernel`, not a paraphrase. Persist of meta and the clock object
//! are axioms.
//!
//! - **Inv-unknown-expired:** a process-local miss is expired (F7).
//! - **Inv-no-id-reuse:** next grant id is above disk max (F7 reanimation).
//! - **Inv-no-reanimate:** a dead lease does not become live without a grant
//!   (F56: RAM clock reset).
//! - AS-IS mutants must produce a counterexample (teeth).

use pedradb_dcs::{
    lease_live, lease_table_expired, lease_table_expired_as_is, next_lease_id_after,
    next_lease_id_as_is,
};
use stateright::{Checker, Model, Property};

const NOW_MAX: u64 = 4;
const DISK_MAX_CAP: u64 = 3;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct St {
    now: u64,
    /// Absolute deadline; `0` means no grant yet (also the immortal encoding).
    lease: u64,
    known: bool,
    disk_max: u64,
    saw_dead: bool,
    unknown_live: bool,
    reused: bool,
    reanimated: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Tick,
    ResetClock,
    Grant,
    GetTable,
    GetLive,
}

#[derive(Clone)]
struct LeaseModel {
    fixed: bool,
}

impl LeaseModel {
    fn table_expired(&self, hit: Option<bool>) -> bool {
        if self.fixed {
            lease_table_expired(hit)
        } else {
            lease_table_expired_as_is(hit)
        }
    }

    fn next_id(&self, disk_max: u64) -> u64 {
        if self.fixed {
            next_lease_id_after(disk_max)
        } else {
            next_lease_id_as_is(disk_max)
        }
    }
}

impl Model for LeaseModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            now: 0,
            lease: 0,
            known: false,
            disk_max: 0,
            saw_dead: false,
            unknown_live: false,
            reused: false,
            reanimated: false,
        }]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        if state.now < NOW_MAX {
            actions.push(Act::Tick);
        }
        // F56 teeth: only the AS-IS caller resets the RAM clock.
        if !self.fixed {
            actions.push(Act::ResetClock);
        }
        if state.disk_max < DISK_MAX_CAP {
            actions.push(Act::Grant);
        }
        actions.push(Act::GetTable);
        actions.push(Act::GetLive);
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::Tick => {
                if state.now >= NOW_MAX {
                    return None;
                }
                next.now = state.now + 1;
                if state.lease != 0 && !lease_live(state.lease, next.now) {
                    next.saw_dead = true;
                }
            }
            Act::ResetClock => next.now = 0,
            Act::Grant => {
                if state.disk_max >= DISK_MAX_CAP {
                    return None;
                }
                let n = self.next_id(state.disk_max);
                if state.disk_max > 0 && n <= state.disk_max {
                    next.reused = true;
                }
                next.disk_max = state.disk_max.max(n);
                next.lease = state.now.saturating_add(2);
                next.known = true;
                next.saw_dead = false;
            }
            Act::GetTable => {
                let clock_expired = state.lease != 0 && state.now >= state.lease;
                let hit = if state.known {
                    Some(clock_expired)
                } else {
                    None
                };
                if !self.table_expired(hit) && !state.known {
                    next.unknown_live = true;
                }
            }
            Act::GetLive => {
                if state.lease != 0 && lease_live(state.lease, state.now) && state.saw_dead {
                    next.reanimated = true;
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-unknown-expired", inv_unknown_expired),
            Property::always("Inv-no-id-reuse", inv_no_id_reuse),
            Property::always("Inv-no-reanimate", inv_no_reanimate),
            Property::sometimes("non-vacuity-grant", non_vacuity_grant),
            Property::sometimes("non-vacuity-dead", non_vacuity_dead),
        ]
    }
}

fn inv_unknown_expired(_: &LeaseModel, s: &St) -> bool {
    !s.unknown_live
}

fn inv_no_id_reuse(_: &LeaseModel, s: &St) -> bool {
    !s.reused
}

fn inv_no_reanimate(_: &LeaseModel, s: &St) -> bool {
    !s.reanimated
}

fn non_vacuity_grant(_: &LeaseModel, s: &St) -> bool {
    s.known
}

fn non_vacuity_dead(_: &LeaseModel, s: &St) -> bool {
    s.saw_dead
}

#[test]
fn fixed_lease_holds() {
    let checker = LeaseModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_unknown_is_live() {
    let checker = LeaseModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-unknown-expired").is_some(),
        "AS-IS unknown id must look live (F7 teeth)"
    );
}

#[test]
fn as_is_reuses_disk_id() {
    let checker = LeaseModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-no-id-reuse").is_some(),
        "AS-IS next id must restart at 1 (F7 reanimation teeth)"
    );
}

#[test]
fn as_is_clock_reset_reanimates() {
    let checker = LeaseModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-no-reanimate").is_some(),
        "AS-IS RAM clock reset must revive a dead lease (F56 teeth)"
    );
}
