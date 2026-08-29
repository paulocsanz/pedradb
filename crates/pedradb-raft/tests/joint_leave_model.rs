//! RFC-0094 / RFC-0066 P1.1: Stateright over production [`joint_still_active`].
//!
//! After a joint add 3→4 **commits** and leave has not, store
//! `election_has_joint_quorum` still requires C-old∧C-new because
//! `pending_joint_on` keeps the joint while `old != new`. The model calls
//! those **same** kernels — not a paraphrase.
//!
//! - **Inv-leave:** a leader is never elected on C-old majority alone
//!   while the joint is still active.
//! - AS-IS (`joint_still_active_as_is`) must discover the invariant
//!   (the 0066 hole: treat committed joint as single C-old).

use pedradb_raft::{
    joint_election_ok, joint_still_active, joint_still_active_as_is,
};
use stateright::{Checker, Model, Property};

const OLD: [u64; 3] = [1, 2, 3];
const NEW: [u64; 4] = [1, 2, 3, 4];

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct St {
    /// Bit i set ⇒ voter (i+1) granted.
    granted: u8,
    /// Leave-joint committed (`old == new` = C-new only).
    left: bool,
    elected: bool,
    /// Elected without C-new majority while joint still active.
    elected_old_only: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Grant { voter: u8 },
    CommitLeave,
    TryElect,
}

#[derive(Clone)]
struct LeaveModel {
    fixed: bool,
}

impl LeaveModel {
    fn pending(&self, left: bool) -> Option<(&'static [u64], &'static [u64])> {
        let (old, new): (&[u64], &[u64]) = if left {
            (&NEW, &NEW)
        } else {
            (&OLD, &NEW)
        };
        let active = if self.fixed {
            joint_still_active(old, new)
        } else {
            joint_still_active_as_is(old, new)
        };
        if active {
            Some((old, new))
        } else {
            None
        }
    }

    fn elect_ok(&self, granted: u8, left: bool) -> bool {
        let granted_ids = |n: usize| -> u64 {
            (0..n)
                .filter(|i| granted & (1 << i) != 0)
                .count() as u64
        };
        match self.pending(left) {
            Some((old, new)) => {
                let old_yes = granted_ids(old.len());
                let new_yes = granted_ids(new.len());
                joint_election_ok(
                    old_yes,
                    old.len() as u64,
                    Some((new_yes, new.len() as u64)),
                )
            }
            None => {
                // Production: no pending joint → `ids` only (C-old until leave
                // applies; C-new after leave).
                let ids_n = if left { NEW.len() } else { OLD.len() };
                let yes = granted_ids(ids_n);
                joint_election_ok(yes, ids_n as u64, None)
            }
        }
    }
}

impl Model for LeaveModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            granted: 0,
            left: false,
            elected: false,
            elected_old_only: false,
        }]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        if state.elected {
            return;
        }
        for v in 0..4u8 {
            if state.granted & (1 << v) == 0 {
                actions.push(Act::Grant { voter: v });
            }
        }
        if !state.left {
            actions.push(Act::CommitLeave);
        }
        if state.granted != 0 {
            actions.push(Act::TryElect);
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::Grant { voter } => {
                next.granted |= 1 << voter;
            }
            Act::CommitLeave => {
                next.left = true;
            }
            Act::TryElect => {
                if self.elect_ok(state.granted, state.left) {
                    next.elected = true;
                    let old_yes = (0..OLD.len())
                        .filter(|i| state.granted & (1 << i) != 0)
                        .count() as u64;
                    let new_yes = (0..NEW.len())
                        .filter(|i| state.granted & (1 << i) != 0)
                        .count() as u64;
                    if !state.left && old_yes >= 2 && new_yes < 3 {
                        next.elected_old_only = true;
                    }
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-leave", inv_leave),
            Property::sometimes("non-vacuity-leave-elect", non_vacuity),
        ]
    }
}

fn inv_leave(_: &LeaveModel, s: &St) -> bool {
    !s.elected_old_only
}

fn non_vacuity(_: &LeaveModel, s: &St) -> bool {
    s.elected && !s.elected_old_only
}

#[test]
fn kernels_match_0066_contract() {
    assert!(joint_still_active(&OLD, &NEW));
    assert!(!joint_still_active_as_is(&OLD, &NEW));
    assert!(!joint_still_active(&NEW, &NEW));
}

#[test]
fn fixed_leave_joint_holds() {
    let checker = LeaveModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_elects_without_leave() {
    let checker = LeaveModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-leave").is_some(),
        "AS-IS must elect with 2/3 C-old after committed joint without leave (0066 tooth)"
    );
}

/// RFC-0094 P2.2: this BFS is a campaign, not ∀ Raft traces.
/// `R-joint` stays continuous; freeze of the catalog model is not a theorem.
#[test]
fn leave_joint_campaign_is_not_forall_traces() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let residuals = std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
        .expect("residuals.json");
    assert!(
        residuals.contains("\"id\": \"R-joint\""),
        "R-joint must stay in the residual catalog"
    );
    assert!(
        residuals.contains("campaign not a theorem"),
        "R-joint close must refuse forall traces"
    );
    let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
        .expect("catalog.json");
    assert!(
        catalog.contains("joint_leave_model"),
        "Stateright leave stays a catalog model, not a theorem of all schedules"
    );
}
