//! Stateright model over the **real** [`pedradb_core::wal::recover_kernel`] (F4 / F14).
//!
//! EXPLODE-style: every recover observation is a choice; the production `fn`
//! decides Keep / Resync / `KeepPrefix` / `FailStop` / Stop. Bytes and fsync stay
//! axioms (DST / `FailingEnv`).
//!
//! - **Inv-no-silent-empty:** torn/length/unknown at prefix 0 is not clean EOF.
//! - **Inv-crc-fail-stop:** CRC never resyncs.
//! - **Inv-orphan-fail-stop:** orphan Middle/Last is not clean EOF.
//! - AS-IS mutants must produce a counterexample (teeth).

use pedradb_core::wal::recover_kernel::{
    fragment_act, fragment_act_as_is, recover_collect_act, recover_collect_act_as_is, FragAct,
    FragKind, RecoverAct, RecoverKind,
};
use stateright::{Checker, Model, Property};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct St {
    prefix_n: u64,
    skips: u64,
    steps: u8,
    done: bool,
    failed: bool,
    silent_empty: bool,
    crc_resync: bool,
    orphan_eof: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Observe { kind: RecoverKind, can_skip: bool },
    Frag { kind: FragKind, scratch_empty: bool },
}

#[derive(Clone)]
struct RecoverModel {
    fixed: bool,
}

impl RecoverModel {
    fn collect(&self, kind: RecoverKind, prefix_n: u64, can_skip: bool, skips: u64) -> RecoverAct {
        if self.fixed {
            recover_collect_act(kind, prefix_n, can_skip, skips, false)
        } else {
            recover_collect_act_as_is(kind, prefix_n, can_skip, skips)
        }
    }

    fn frag(&self, kind: FragKind, scratch_empty: bool) -> FragAct {
        if self.fixed {
            fragment_act(kind, scratch_empty)
        } else {
            fragment_act_as_is(kind, scratch_empty)
        }
    }
}

impl Model for RecoverModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            prefix_n: 0,
            skips: 0,
            steps: 0,
            done: false,
            failed: false,
            silent_empty: false,
            crc_resync: false,
            orphan_eof: false,
        }]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        if state.steps >= 3 {
            return;
        }
        if !state.done {
            for kind in [
                RecoverKind::Record,
                RecoverKind::CleanEof,
                RecoverKind::Truncated,
                RecoverKind::LengthCorrupt,
                RecoverKind::UnknownType,
                RecoverKind::OrphanFragment,
                RecoverKind::Crc,
                RecoverKind::Other,
            ] {
                actions.push(Act::Observe {
                    kind,
                    can_skip: false,
                });
                actions.push(Act::Observe {
                    kind,
                    can_skip: true,
                });
            }
        }
        for kind in [
            FragKind::Full,
            FragKind::First,
            FragKind::Middle,
            FragKind::Last,
            FragKind::Zero,
        ] {
            actions.push(Act::Frag {
                kind,
                scratch_empty: true,
            });
            actions.push(Act::Frag {
                kind,
                scratch_empty: false,
            });
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        next.steps = state.steps.saturating_add(1);
        match action {
            Act::Observe { kind, can_skip } => {
                if state.done {
                    return None;
                }
                let skips = if can_skip {
                    state.skips.saturating_add(1)
                } else {
                    state.skips
                };
                let act = self.collect(kind, state.prefix_n, can_skip, skips);
                match act {
                    RecoverAct::KeepRecord => {
                        next.prefix_n = state.prefix_n.saturating_add(1);
                        next.skips = 0;
                    }
                    RecoverAct::Resync => {
                        next.skips = skips;
                        if kind == RecoverKind::Crc {
                            next.crc_resync = true;
                        }
                    }
                    RecoverAct::Stop => {
                        if state.prefix_n == 0
                            && matches!(
                                kind,
                                RecoverKind::Truncated
                                    | RecoverKind::LengthCorrupt
                                    | RecoverKind::UnknownType
                            )
                        {
                            next.silent_empty = true;
                        }
                        next.done = true;
                    }
                    RecoverAct::KeepPrefix => next.done = true,
                    RecoverAct::FailStop => {
                        next.failed = true;
                        next.done = true;
                    }
                }
            }
            Act::Frag {
                kind,
                scratch_empty,
            } => {
                if self.frag(kind, scratch_empty) == FragAct::CleanEof {
                    next.orphan_eof = true;
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-no-silent-empty", inv_no_silent_empty),
            Property::always("Inv-crc-fail-stop", inv_crc_fail_stop),
            Property::always("Inv-orphan-fail-stop", inv_orphan_fail_stop),
            Property::sometimes("non-vacuity-record", non_vacuity_record),
            Property::sometimes("non-vacuity-fail", non_vacuity_fail),
        ]
    }
}

fn inv_no_silent_empty(_: &RecoverModel, s: &St) -> bool {
    !s.silent_empty
}

fn inv_crc_fail_stop(_: &RecoverModel, s: &St) -> bool {
    !s.crc_resync
}

fn inv_orphan_fail_stop(_: &RecoverModel, s: &St) -> bool {
    !s.orphan_eof
}

fn non_vacuity_record(_: &RecoverModel, s: &St) -> bool {
    s.prefix_n > 0
}

fn non_vacuity_fail(_: &RecoverModel, s: &St) -> bool {
    s.failed
}

#[test]
fn fixed_recover_holds() {
    let checker = RecoverModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_finds_f4_f14_crc() {
    let checker = RecoverModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-no-silent-empty").is_some(),
        "AS-IS torn/length must look like clean EOF (F4 teeth)"
    );
    assert!(
        checker.discovery("Inv-crc-fail-stop").is_some(),
        "AS-IS CRC resync must be found (SilentWrong teeth)"
    );
    assert!(
        checker.discovery("Inv-orphan-fail-stop").is_some(),
        "AS-IS orphan Middle/Last must look like clean EOF (F14 teeth)"
    );
}
