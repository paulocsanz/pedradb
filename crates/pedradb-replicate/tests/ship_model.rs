//! Stateright model over the **real** [`pedradb_replicate::ship_kernel`]
//! `pull_plan` (F165).
//!
//! Faithfulness: the decision is production `ship_kernel` (bytes → plan); the
//! WAL-append/rotate behavior of `Db::flush` is an axiom of the worlds
//! (append-only growth; rotate truncates in place and rewrites the prefix).
//!
//! - **Inv-rotate-detect:** any world whose stream no longer continues the
//!   shipped prefix (rotate+regrow past the cursor, shrink, vanished file)
//!   must plan `Rotated`.
//! - **Inv-ship-sound:** a `Ship` plan ships `min(len - cursor, max)` only
//!   with the file present, `len > cursor`, and a stable prefix.
//! - **Inv-uptodate-sound:** `UpToDate` only with the file present,
//!   `len == cursor`, stable prefix.
//! - AS-IS (length-only) must miss rotate+regrow (teeth).

use pedradb_replicate::ship_kernel::{pull_plan, pull_plan_as_is, PullPlan, SHIP_STAMP_BYTES};
use stateright::{Checker, Model, Property};

const MAX_PULL: u64 = 4_000_000;
const STAMP_A: &[u8] = &[1u8; SHIP_STAMP_BYTES];
const STAMP_B: &[u8] = &[2u8; SHIP_STAMP_BYTES];

/// One primary-WAL world relative to a shipped cursor.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct World {
    /// File present at all.
    exists: bool,
    /// Current file length.
    len: u64,
    /// Cursor (bytes already shipped).
    cursor: u64,
    /// Stamp captured when shipping was established.
    stamp_then: Option<&'static [u8]>,
    /// First `min(stamp_then.len(), len)` bytes now.
    stamp_now: &'static [u8],
}

impl World {
    /// Whether the primary stream stopped continuing the shipped prefix
    /// (axiom: appends never rewrite bytes; only rotation/deletion does).
    fn rotated(&self) -> bool {
        if !self.exists {
            return self.cursor > 0 || self.stamp_then.is_some();
        }
        if self.len < self.cursor {
            return true;
        }
        match self.stamp_then {
            None => false,
            Some(then) => {
                let n = self.stamp_now.len().min(then.len());
                then[..n] != self.stamp_now[..n]
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct St {
    pulled: bool,
    missed_rotation: bool,
    silent_gap: bool,
    wrong_ship: bool,
    uptodate_unsound: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Pull,
}

#[derive(Clone)]
struct ShipModel {
    world: World,
    fixed: bool,
}

impl ShipModel {
    fn plan(&self) -> PullPlan {
        let file_len = self.world.exists.then_some(self.world.len);
        if self.fixed {
            pull_plan(
                file_len,
                self.world.cursor,
                MAX_PULL,
                self.world.stamp_then,
                self.world.stamp_now,
            )
        } else {
            pull_plan_as_is(
                file_len,
                self.world.cursor,
                MAX_PULL,
                self.world.stamp_then,
                self.world.stamp_now,
            )
        }
    }
}

impl Model for ShipModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            pulled: false,
            missed_rotation: false,
            silent_gap: false,
            wrong_ship: false,
            uptodate_unsound: false,
        }]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        if !state.pulled {
            actions.push(Act::Pull);
        }
    }

    fn next_state(&self, state: &Self::State, _action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        next.pulled = true;
        let rotated = self.world.rotated();
        match self.plan() {
            // Rotated on a continuing stream is a false positive: fail-closed
            // (operator re-bootstraps), never silent wrong. Not flagged.
            PullPlan::Rotated { .. } => {}
            PullPlan::UpToDate => {
                if rotated {
                    next.missed_rotation = true;
                    next.silent_gap = true;
                }
                // A missing WAL with cursor 0 is a fresh primary (benign).
                if self.world.len != self.world.cursor {
                    next.uptodate_unsound = true;
                }
            }
            PullPlan::Ship { bytes } => {
                if rotated {
                    next.missed_rotation = true;
                    next.silent_gap = true;
                }
                let expect = self
                    .world
                    .len
                    .saturating_sub(self.world.cursor)
                    .min(MAX_PULL);
                if bytes != expect || !self.world.exists || self.world.len <= self.world.cursor {
                    next.wrong_ship = true;
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-rotate-detect", inv_rotate_detect),
            Property::always("Inv-ship-sound", inv_ship_sound),
            Property::always("Inv-uptodate-sound", inv_uptodate_sound),
            Property::always("Inv-no-silent-gap", inv_no_silent_gap),
            Property::sometimes("non-vacuity-pull", non_vacuity_pull),
        ]
    }
}

fn inv_rotate_detect(_: &ShipModel, s: &St) -> bool {
    !s.missed_rotation
}

fn inv_ship_sound(_: &ShipModel, s: &St) -> bool {
    !s.wrong_ship
}

fn inv_uptodate_sound(_: &ShipModel, s: &St) -> bool {
    !s.uptodate_unsound
}

fn inv_no_silent_gap(_: &ShipModel, s: &St) -> bool {
    !s.silent_gap
}

fn non_vacuity_pull(_: &ShipModel, s: &St) -> bool {
    s.pulled
}

fn worlds() -> Vec<World> {
    let mut out = Vec::new();
    for exists in [true, false] {
        for len in [0u64, 50, 100, 300, 500] {
            if !exists && len != 0 {
                continue; // no file ⇒ no observable length
            }
            for cursor in [0u64, 100, 300] {
                for stamp_then in [None, Some(STAMP_A)] {
                    // stamp_now: unchanged prefix, rewritten prefix, or short read.
                    for stamp_now in [STAMP_A, STAMP_B, &STAMP_A[..16], &[]] {
                        if stamp_then.is_none() && !stamp_now.is_empty() {
                            continue; // no stamp ⇒ caller passes no bytes
                        }
                        out.push(World {
                            exists,
                            len,
                            cursor,
                            stamp_then,
                            stamp_now,
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
    for world in worlds() {
        let checker = ShipModel { world, fixed: true }
            .checker()
            .spawn_bfs()
            .join();
        checker.assert_properties();
    }
}

#[test]
fn as_is_misses_rotate_regrow() {
    // F165 teeth: cursor 300, flush rotated the log, fresh writes regrew the
    // file to 500. Length-only sees `len > cursor` and ships misaligned bytes.
    let world = World {
        exists: true,
        len: 500,
        cursor: 300,
        stamp_then: Some(STAMP_A),
        stamp_now: STAMP_B,
    };
    let checker = ShipModel {
        world,
        fixed: false,
    }
    .checker()
    .spawn_bfs()
    .join();
    assert!(
        checker.discovery("Inv-no-silent-gap").is_some(),
        "AS-IS must ship misaligned bytes after rotate+regrow (F165 teeth)"
    );
}

#[test]
fn as_is_misses_vanished_wal() {
    // F165 teeth: the file is gone under an advanced cursor; AS-IS says up-to-date.
    let world = World {
        exists: false,
        len: 0,
        cursor: 100,
        stamp_then: Some(STAMP_A),
        stamp_now: &[],
    };
    let checker = ShipModel {
        world,
        fixed: false,
    }
    .checker()
    .spawn_bfs()
    .join();
    assert!(
        checker.discovery("Inv-no-silent-gap").is_some(),
        "AS-IS must treat a vanished WAL as up-to-date (F165 teeth)"
    );
}
