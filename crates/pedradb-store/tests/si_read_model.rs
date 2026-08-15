//! Stateright model over the **real** [`pedradb_store::snapshot_read_plan`]
//! (F168).
//!
//! Faithfulness: the predicate is the production gate at the top of
//! `StoreCluster::get_at_version`; `TooOld` ⇒ the caller fails with
//! `TransactionTooOld` instead of reading pruned history.
//!
//! Domain: `snapshot`/`watermark` as small `u64`s around the GC floor. After
//! `maybe_gc_versions` runs with watermark `w`, the oldest history entry of
//! every key is the **floor** at generation `w - 1` (entries below were
//! pruned; the floor value is the last pruned value).
//!
//! - **Inv-toold-sound:** `Serve` ⇒ the floor `w - 1` covers the snapshot
//!   (`snapshot >= w - 1`), so some history entry `g <= snapshot` exists —
//!   never a fabricated answer.
//! - **Inv-toold-not-vacuous:** some world is `TooOld` (the gate exists) and
//!   fresh snapshots always `Serve`.
//! - AS-IS (always Serve) must serve a below-floor snapshot (teeth — that is
//!   the fabricated `Ok(None)` of F168).

use pedradb_store::{snapshot_read_plan, snapshot_read_plan_as_is, SnapshotRead};
use stateright::{Checker, Model, Property};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct St {
    decided: bool,
    served_uncovered: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Read,
}

#[derive(Clone)]
struct SiReadModel {
    snapshot: u64,
    watermark: u64,
    fixed: bool,
}

impl SiReadModel {
    fn plan(&self) -> SnapshotRead {
        if self.fixed {
            snapshot_read_plan(self.snapshot, self.watermark)
        } else {
            snapshot_read_plan_as_is(self.snapshot, self.watermark)
        }
    }

    /// Does the post-GC floor (`watermark - 1`) cover this snapshot?
    fn floor_covers(&self) -> bool {
        // watermark == 0: no GC ever ran, full history (gen-0 anchor) covers all.
        self.watermark == 0 || self.snapshot + 1 >= self.watermark
    }
}

impl Model for SiReadModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<St> {
        vec![St {
            decided: false,
            served_uncovered: false,
        }]
    }

    fn actions(&self, s: &St, actions: &mut Vec<Act>) {
        if !s.decided {
            actions.push(Act::Read);
        }
    }

    fn next_state(&self, s: &St, _a: Act) -> Option<St> {
        let mut next = s.clone();
        next.decided = true;
        if self.plan() == SnapshotRead::Serve && !self.floor_covers() {
            next.served_uncovered = true;
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-toold-sound", |_m, s: &St| !s.served_uncovered),
            Property::sometimes("Inv-toold-non-vacuous", |_m, s: &St| s.decided),
        ]
    }
}

fn worlds() -> Vec<SiReadModel> {
    let mut out = Vec::new();
    for snapshot in 0u64..12 {
        for watermark in 0u64..12 {
            for fixed in [true, false] {
                out.push(SiReadModel {
                    snapshot,
                    watermark,
                    fixed,
                });
            }
        }
    }
    // Extreme edge: huge watermark against a tiny snapshot (overflow probe).
    out.push(SiReadModel {
        snapshot: 0,
        watermark: u64::MAX,
        fixed: true,
    });
    out
}

#[test]
fn fixed_plan_holds_in_all_worlds() {
    for model in worlds() {
        if !model.fixed {
            continue;
        }
        let checker = model.checker().spawn_bfs().join();
        checker.assert_properties();
    }
}

#[test]
fn fresh_snapshots_always_serve() {
    // Non-vacuity: the gate never rejects current or no-GC reads.
    assert_eq!(snapshot_read_plan(10, 10), SnapshotRead::Serve);
    assert_eq!(snapshot_read_plan(0, 0), SnapshotRead::Serve);
    assert_eq!(snapshot_read_plan(u64::MAX, u64::MAX), SnapshotRead::Serve);
    // The gate does reject below the floor.
    assert_eq!(snapshot_read_plan(0, 2), SnapshotRead::TooOld);
}

/// F168 teeth: AS-IS serves the below-floor snapshot whose history was
/// pruned — the fabricated `Ok(None)` the repro pinned.
#[test]
fn as_is_serves_pruned_snapshot() {
    let model = SiReadModel {
        snapshot: 1,
        watermark: 7,
        fixed: false,
    };
    assert_eq!(model.plan(), SnapshotRead::Serve);
    assert!(!model.floor_covers(), "snapshot 1 below floor 6");
    let checker = model.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-toold-sound").is_some(),
        "AS-IS must serve an uncovered snapshot (F168 teeth)"
    );
}
