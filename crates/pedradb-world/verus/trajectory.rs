// Verus proof of trajectory monotonicity (RFC-0059 P2.2).
// Twin of `src/world_kernel.rs` (HashMap/String fold is caller).
//
//   ./scripts/verus_world_trajectory.sh

use vstd::prelude::*;

verus! {

/// Decision domain: the three monotone raft coordinates of a sample
/// (step/node/range are grouping keys, caller side).
pub struct Sample {
    pub term: u64,
    pub snapshot_index: u64,
    pub applied_index: u64,
}

// Verdict codes: 0 = no violation, 1 = term, 2 = snapshot, 3 = applied.
pub const NO_VIOLATION: u8 = 0;
pub const TERM: u8 = 1;
pub const SNAPSHOT: u8 = 2;
pub const APPLIED: u8 = 3;

/// FIXED: first regressed coordinate, checked in cascade; `NO_VIOLATION`
/// iff the pair is monotone on all three coordinates.
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

/// AS-IS: single-coordinate rule — only the term is checked; watermark
/// regressions are blessed.
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

/// Monotone pair: FIXED and AS-IS agree on NO_VIOLATION (the campaign
/// green path is stable under both rules).
proof fn lemma_monotone_pair_agrees(p: Sample, c: Sample)
    requires
        p.term <= c.term,
        p.snapshot_index <= c.snapshot_index,
        p.applied_index <= c.applied_index,
    ensures
        trajectory_violation_spec(p, c) == NO_VIOLATION,
        trajectory_violation_as_is_spec(p, c) == NO_VIOLATION,
{
}

/// Resurrection window: applied watermark regresses while term and
/// snapshot are monotone — FIXED reports APPLIED, AS-IS blesses it.
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

/// Stale snapshot: snapshot watermark regresses while term is monotone
/// and applied did not regress — FIXED reports SNAPSHOT, AS-IS blesses
/// it (term-only rule).
proof fn lemma_as_is_blesses_snapshot_regression(p: Sample, c: Sample)
    requires
        p.term <= c.term,
        c.snapshot_index < p.snapshot_index,
        p.applied_index <= c.applied_index,
    ensures
        trajectory_violation_spec(p, c) == SNAPSHOT,
        trajectory_violation_as_is_spec(p, c) == NO_VIOLATION,
{
}

/// RFC-0170 close: production check_trajectory.
pub fn check_trajectory(p: Sample, c: Sample) -> (d: u8)
    ensures
        d == trajectory_violation_spec(p, c),
        d != NO_VIOLATION ==> c.term < p.term || c.snapshot_index < p.snapshot_index
            || c.applied_index < p.applied_index,
{
    trajectory_violation(p, c)
}

} // verus!
