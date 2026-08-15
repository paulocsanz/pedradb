//! Stateright model over the **real** [`pedradb_core::changelog_needs_sst_rebuild`] (F53).
//!
//! Faithfulness: whether open rebuilds the feed from Mem ∪ SST is production
//! `changelog_kernel`. Scan and persist of `CHANGELOG` are axioms.
//!
//! After flush the WAL is truncated. A missing CHANGELOG plus live sequence
//! must rebuild — otherwise fold/journal see `changes_after(0) == []`.
//!
//! - **Inv-no-blind-fold:** empty feed + `last_sequence > 0` rebuilds.
//! - **Inv-fresh:** seq 0 never rebuilds (and is not “blind”).
//! - AS-IS WAL-only must leave the feed empty (teeth).

use pedradb_core::{changelog_needs_sst_rebuild, changelog_needs_sst_rebuild_as_is};
use stateright::{Checker, Model, Property};

const SEQ_CAP: u64 = 3;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct St {
    seq: u64,
    feed_empty: bool,
    fold_blind: bool,
    rebuilt: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Put,
    FlushDropChangelog,
    OpenRecover,
}

#[derive(Clone)]
struct ChangelogModel {
    fixed: bool,
}

impl ChangelogModel {
    fn needs(&self, feed_empty: bool, seq: u64) -> bool {
        if self.fixed {
            changelog_needs_sst_rebuild(feed_empty, seq)
        } else {
            changelog_needs_sst_rebuild_as_is(feed_empty, seq)
        }
    }
}

impl Model for ChangelogModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            seq: 0,
            feed_empty: true,
            fold_blind: false,
            rebuilt: false,
        }]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        actions.extend([Act::Put, Act::FlushDropChangelog, Act::OpenRecover]);
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::Put => {
                if state.seq >= SEQ_CAP {
                    return None;
                }
                next.seq = state.seq + 1;
                next.feed_empty = false;
            }
            Act::FlushDropChangelog => {
                // Flush truncates WAL; missing CHANGELOG → empty feed, seq stays.
                if state.seq == 0 {
                    return None;
                }
                next.feed_empty = true;
            }
            Act::OpenRecover => {
                if self.needs(state.feed_empty, state.seq) {
                    next.feed_empty = false;
                    next.rebuilt = true;
                } else if state.feed_empty && state.seq > 0 {
                    next.fold_blind = true;
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-no-blind-fold", inv_no_blind),
            Property::sometimes("non-vacuity-rebuild", non_vacuity_rebuild),
            Property::sometimes("non-vacuity-put", non_vacuity_put),
        ]
    }
}

fn inv_no_blind(_: &ChangelogModel, s: &St) -> bool {
    !s.fold_blind
}

fn non_vacuity_rebuild(_: &ChangelogModel, s: &St) -> bool {
    s.rebuilt
}

fn non_vacuity_put(_: &ChangelogModel, s: &St) -> bool {
    s.seq > 0
}

#[test]
fn fixed_changelog_holds() {
    let checker = ChangelogModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_leaves_fold_blind() {
    let checker = ChangelogModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-no-blind-fold").is_some(),
        "AS-IS WAL-only must leave empty feed after flush (F53 teeth)"
    );
}
