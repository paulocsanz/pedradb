//! Stateright model over the **real** [`pedradb_store`] snapshot kernel (F38 / F40 / F41).
//!
//! Faithfulness: export / wipe / payload-apply / txn-meta clear are production
//! `snapshot_kernel`. Persist and the range scan are axioms.
//!
//! - **Inv-no-reserved-export:** `\0store/*` never ships in a snapshot (F41).
//! - **Inv-no-reserved-wipe:** install does not delete reserved keys (F38).
//! - **Inv-txn-meta-cleared:** install clears intent/txn meta (F40).
//! - AS-IS mutants must produce a counterexample (teeth).

use pedradb_store::{
    snapshot_needs_txn_meta_clear, snapshot_needs_txn_meta_clear_as_is, snapshot_touches_user_key,
    snapshot_touches_user_key_as_is,
};
use stateright::{Checker, Model, Property};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct St {
    user: bool,
    reserved: bool,
    intent: bool,
    exported_reserved: bool,
    wiped_reserved: bool,
    leftover_intent: bool,
    saw_install: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    PutUser,
    PutReserved,
    PutIntent,
    Export,
    Install {
        payload_user: bool,
        payload_reserved: bool,
    },
}

#[derive(Clone)]
struct SnapshotModel {
    fixed: bool,
}

impl SnapshotModel {
    fn touches(&self, is_reserved: bool) -> bool {
        if self.fixed {
            snapshot_touches_user_key(is_reserved)
        } else {
            snapshot_touches_user_key_as_is(is_reserved)
        }
    }

    fn clear_txn_meta(&self) -> bool {
        if self.fixed {
            snapshot_needs_txn_meta_clear()
        } else {
            snapshot_needs_txn_meta_clear_as_is()
        }
    }
}

impl Model for SnapshotModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            user: true,
            reserved: true,
            intent: true,
            exported_reserved: false,
            wiped_reserved: false,
            leftover_intent: false,
            saw_install: false,
        }]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        actions.extend([Act::PutUser, Act::PutReserved, Act::PutIntent, Act::Export]);
        for payload_user in [false, true] {
            for payload_reserved in [false, true] {
                actions.push(Act::Install {
                    payload_user,
                    payload_reserved,
                });
            }
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::PutUser => next.user = true,
            Act::PutReserved => next.reserved = true,
            Act::PutIntent => next.intent = true,
            Act::Export => {
                if state.reserved && self.touches(true) {
                    next.exported_reserved = true;
                }
            }
            Act::Install {
                payload_user,
                payload_reserved,
            } => {
                // Wipe then apply — same order as `on_install_snapshot`.
                if state.user && self.touches(false) {
                    next.user = false;
                }
                if state.reserved && self.touches(true) {
                    next.reserved = false;
                    next.wiped_reserved = true;
                }
                if self.clear_txn_meta() {
                    next.intent = false;
                } else if state.intent {
                    next.leftover_intent = true;
                }
                if payload_user && self.touches(false) {
                    next.user = true;
                }
                if payload_reserved && self.touches(true) {
                    next.reserved = true;
                    next.exported_reserved = true;
                }
                next.saw_install = true;
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-no-reserved-export", inv_no_reserved_export),
            Property::always("Inv-no-reserved-wipe", inv_no_reserved_wipe),
            Property::always("Inv-txn-meta-cleared", inv_txn_meta_cleared),
            Property::sometimes("non-vacuity-install", non_vacuity_install),
        ]
    }
}

fn inv_no_reserved_export(_: &SnapshotModel, s: &St) -> bool {
    !s.exported_reserved
}

fn inv_no_reserved_wipe(_: &SnapshotModel, s: &St) -> bool {
    !s.wiped_reserved
}

fn inv_txn_meta_cleared(_: &SnapshotModel, s: &St) -> bool {
    !s.leftover_intent
}

fn non_vacuity_install(_: &SnapshotModel, s: &St) -> bool {
    s.saw_install
}

#[test]
fn fixed_snapshot_holds() {
    let checker = SnapshotModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_exports_reserved() {
    let checker = SnapshotModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-no-reserved-export").is_some(),
        "AS-IS must ship \\0store/* in the snapshot (F41 teeth)"
    );
}

#[test]
fn as_is_wipes_reserved() {
    let checker = SnapshotModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-no-reserved-wipe").is_some(),
        "AS-IS must wipe reserved keys on install (F38 teeth)"
    );
}

#[test]
fn as_is_leaves_orphan_intent() {
    let checker = SnapshotModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-txn-meta-cleared").is_some(),
        "AS-IS must skip txn-meta clear (F40 teeth)"
    );
}
