//! RFC-0059 P2.2: trajectory monotonicity kernel.
//!
//! **Single artifact (pair `world_trajectory`):** this file is what `rustc`
//! links *and* what Verus proves (`cfg(verus_keep_ghost)`). HashMap/String
//! fold is caller. Pair `world_trajectory_fold` stays a twin-cópia until
//! its turn.
//!
//!   ./scripts/verus_world_trajectory.sh
//!
//! Production `World::exchange` and the exported fold [`check_trajectory`]
//! call [`trajectory_violation`]. Catalog pairs `world_trajectory` /
//! `world_trajectory_fold`; Verus proves the u8 cascade.

#![forbid(unsafe_code)]

#[cfg(not(verus_keep_ghost))]
use std::collections::HashMap;

/// Per-node intra-run trajectory sample (RFC-0059 P2.2): raft
/// coordinates observed right after a net exchange. A live node's term,
/// snapshot_index and applied_index are monotone across the whole run —
/// including install-snapshot catch-up (stale snapshots are rejected by
/// the store's commit guard) and membership exit/rejoin (state is kept,
/// only the role demotes).
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrajectorySample {
    /// Schedule step the sample was taken after.
    pub step: u32,
    /// Node id (1-based).
    pub node: u64,
    /// Range id.
    pub range: u64,
    /// Raft term at sample time.
    pub term: u64,
    /// Snapshot watermark at sample time.
    pub snapshot_index: u64,
    /// Applied watermark at sample time.
    pub applied_index: u64,
}

/// Which coordinate regressed between two samples of the same
/// (node, range): `None` when the pair is monotone.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn trajectory_violation(
    prev: &TrajectorySample,
    cur: &TrajectorySample,
) -> Option<&'static str> {
    if cur.term < prev.term {
        Some("term")
    } else if cur.snapshot_index < prev.snapshot_index {
        Some("snapshot_index")
    } else if cur.applied_index < prev.applied_index {
        Some("applied_index")
    } else {
        None
    }
}

/// AS-IS (pair `world_trajectory`): only the term is checked — applied
/// and snapshot watermark regressions are blessed (the resurrection
/// window the cascade exists to refuse).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn trajectory_violation_as_is(
    prev: &TrajectorySample,
    cur: &TrajectorySample,
) -> Option<&'static str> {
    if cur.term < prev.term {
        Some("term")
    } else {
        None
    }
}

/// Fold a full sample sequence (any interleaving of nodes/ranges) into
/// per-(node, range) monotonicity violations. The run itself checks the
/// same invariant incrementally through [`trajectory_violation`]; this
/// fold is the exported form (mutant tests + forensics on a captured
/// trajectory).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn check_trajectory(samples: &[TrajectorySample]) -> Vec<String> {
    let mut prev: HashMap<(u64, u64), TrajectorySample> = HashMap::new();
    let mut out = Vec::new();
    for s in samples {
        match prev.get(&(s.node, s.range)) {
            Some(p) => {
                if let Some(what) = trajectory_violation(p, s) {
                    out.push(format!(
                        "n{} r{} {} regressed {}->{} @step {} (after {})",
                        s.node,
                        s.range,
                        what,
                        match what {
                            "term" => p.term,
                            "snapshot_index" => p.snapshot_index,
                            _ => p.applied_index,
                        },
                        match what {
                            "term" => s.term,
                            "snapshot_index" => s.snapshot_index,
                            _ => s.applied_index,
                        },
                        s.step,
                        p.step
                    ));
                }
            }
            None => {}
        }
        prev.insert((s.node, s.range), s.clone());
    }
    out
}

/// AS-IS (pair `world_trajectory_fold`): the fold uses the term-only
/// rule, so a resurrected applied watermark is silent.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn check_trajectory_as_is(samples: &[TrajectorySample]) -> Vec<String> {
    let mut prev: HashMap<(u64, u64), TrajectorySample> = HashMap::new();
    let mut out = Vec::new();
    for s in samples {
        match prev.get(&(s.node, s.range)) {
            Some(p) => {
                if let Some(what) = trajectory_violation_as_is(p, s) {
                    out.push(format!(
                        "n{} r{} {} regressed {}->{} @step {} (after {})",
                        s.node,
                        s.range,
                        what,
                        match what {
                            "term" => p.term,
                            "snapshot_index" => p.snapshot_index,
                            _ => p.applied_index,
                        },
                        match what {
                            "term" => s.term,
                            "snapshot_index" => s.snapshot_index,
                            _ => s.applied_index,
                        },
                        s.step,
                        p.step
                    ));
                }
            }
            None => {}
        }
        prev.insert((s.node, s.range), s.clone());
    }
    out
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

/// Decision domain: the three monotone raft coordinates of a sample
/// (step/node/range are grouping keys, caller side).
pub struct Sample {
    pub term: u64,
    pub snapshot_index: u64,
    pub applied_index: u64,
}

pub const NO_VIOLATION: u8 = 0;
pub const TERM: u8 = 1;
pub const SNAPSHOT: u8 = 2;
pub const APPLIED: u8 = 3;

pub open spec fn trajectory_violation_spec(p: Sample, c: Sample) -> u8 {
    if c.term < p.term {
        TERM
    } else if c.snapshot_index < p.snapshot_index {
        SNAPSHOT
    } else if c.applied_index < p.applied_index {
        APPLIED
    } else {
        NO_VIOLATION
    }
}

pub fn trajectory_violation(p: Sample, c: Sample) -> (d: u8)
    ensures
        d == trajectory_violation_spec(p, c),
        d != NO_VIOLATION ==> c.term < p.term || c.snapshot_index < p.snapshot_index
            || c.applied_index < p.applied_index,
{
    if c.term < p.term {
        TERM
    } else if c.snapshot_index < p.snapshot_index {
        SNAPSHOT
    } else if c.applied_index < p.applied_index {
        APPLIED
    } else {
        NO_VIOLATION
    }
}

pub open spec fn trajectory_violation_as_is_spec(p: Sample, c: Sample) -> u8 {
    if c.term < p.term {
        TERM
    } else {
        NO_VIOLATION
    }
}

pub fn trajectory_violation_as_is(p: Sample, c: Sample) -> (d: u8)
    ensures
        d == trajectory_violation_as_is_spec(p, c),
        d == NO_VIOLATION || d == TERM,
{
    if c.term < p.term {
        TERM
    } else {
        NO_VIOLATION
    }
}

proof fn lemma_as_is_blesses_applied_regression(p: Sample, c: Sample)
    requires
        p.term <= c.term,
        p.snapshot_index <= c.snapshot_index,
        c.applied_index < p.applied_index,
    ensures
        trajectory_violation_spec(p, c) == APPLIED,
        trajectory_violation_as_is_spec(p, c) == NO_VIOLATION,
{
}

} // verus!

#[cfg(test)]
mod tests {
    use super::*;

    fn s(step: u32, node: u64, term: u64, snap: u64, applied: u64) -> TrajectorySample {
        TrajectorySample {
            step,
            node,
            range: 1,
            term,
            snapshot_index: snap,
            applied_index: applied,
        }
    }

    #[test]
    fn trajectory_violation_on_applied_regression_is_not_ok() {
        let p = s(1, 1, 1, 4, 7);
        let c = s(2, 1, 1, 4, 3);
        assert_eq!(trajectory_violation(&p, &c), Some("applied_index"));
        assert_eq!(trajectory_violation_as_is(&p, &c), None);
    }

    #[test]
    fn check_trajectory_on_resurrected_watermark_is_not_ok() {
        let samples = vec![s(1, 1, 1, 4, 7), s(2, 1, 1, 4, 3)];
        let hits = check_trajectory(&samples);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].contains("applied_index"), "{hits:?}");
        assert!(check_trajectory_as_is(&samples).is_empty());
    }
}
