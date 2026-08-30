// Verus proof of the WAL-ship rotation guard (F165).
// Twin of `pedradb-replicate/src/ship_kernel.rs` — byte stamps modeled as u64
// fingerprints (`Option<u64>` stands for `Option<&[u8]>`; equal fingerprint =
// unchanged prefix, the append-only axiom).
//
//   ./scripts/verus_ship_guard.sh

use vstd::prelude::*;

verus! {

pub enum PullPlan {
    Rotated { file_len: u64, cursor: u64 },
    UpToDate,
    Ship { bytes: u64 },
}

/// Fingerprint form of `ship_kernel::stamp_changed`.
pub open spec fn stamp_changed_spec(stamp_then: u64, stamp_now: u64) -> bool {
    stamp_then != stamp_now
}

/// Whether the primary stream still continues the shipped cursor: the file is
/// present, has not shrunk past the cursor, and its captured prefix is intact.
pub open spec fn continues_spec(
    file_len: Option<u64>,
    cursor: u64,
    stamp_then: Option<u64>,
    stamp_now: u64,
) -> bool {
    match file_len {
        Some(len) => len >= cursor && (stamp_then.is_none() || stamp_then == Some(stamp_now)),
        None => cursor == 0 && stamp_then.is_none(),
    }
}

/// `min(len - cursor, max_pull)` as a spec fn (u64 wrapper around int math).
pub open spec fn capped_residual(len: u64, cursor: u64, max_pull: u64) -> u64 {
    if len - cursor >= max_pull {
        max_pull
    } else {
        (len - cursor) as u64
    }
}

pub open spec fn pull_plan_spec(
    file_len: Option<u64>,
    cursor: u64,
    max_pull: u64,
    stamp_then: Option<u64>,
    stamp_now: u64,
) -> PullPlan {
    match file_len {
        None => if cursor > 0 || stamp_then.is_some() {
            PullPlan::Rotated { file_len: 0, cursor: cursor }
        } else {
            PullPlan::UpToDate
        },
        Some(len) => if len < cursor {
            PullPlan::Rotated { file_len: len, cursor: cursor }
        } else if stamp_then.is_some() && stamp_changed_spec(stamp_then.unwrap(), stamp_now) {
            PullPlan::Rotated { file_len: len, cursor: cursor }
        } else if len == cursor {
            PullPlan::UpToDate
        } else {
            PullPlan::Ship { bytes: capped_residual(len, cursor, max_pull) }
        },
    }
}

pub open spec fn is_rotated(p: PullPlan) -> bool {
    match p {
        PullPlan::Rotated { .. } => true,
        PullPlan::UpToDate => false,
        PullPlan::Ship { .. } => false,
    }
}

pub open spec fn is_ship(p: PullPlan) -> bool {
    match p {
        PullPlan::Rotated { .. } => false,
        PullPlan::UpToDate => false,
        PullPlan::Ship { .. } => true,
    }
}

pub fn pull_plan(
    file_len: Option<u64>,
    cursor: u64,
    max_pull: u64,
    stamp_then: Option<u64>,
    stamp_now: u64,
) -> (plan: PullPlan)
    ensures
        plan == pull_plan_spec(file_len, cursor, max_pull, stamp_then, stamp_now),
        // Fail-closed: a stream that no longer continues the cursor never
        // ships or claims up-to-date.
        !continues_spec(file_len, cursor, stamp_then, stamp_now) ==> is_rotated(plan),
        // Sound: shipping / up-to-date only on a continuing stream.
        is_ship(plan) ==> continues_spec(file_len, cursor, stamp_then, stamp_now),
        plan == PullPlan::UpToDate ==> continues_spec(file_len, cursor, stamp_then, stamp_now),
{
    match file_len {
        None => if cursor > 0 || stamp_then.is_some() {
            PullPlan::Rotated { file_len: 0, cursor: cursor }
        } else {
            PullPlan::UpToDate
        },
        Some(len) => if len < cursor {
            PullPlan::Rotated { file_len: len, cursor: cursor }
        } else if let Some(then) = stamp_then {
            if then != stamp_now {
                PullPlan::Rotated { file_len: len, cursor: cursor }
            } else if len == cursor {
                PullPlan::UpToDate
            } else {
                let residual = len - cursor;
                PullPlan::Ship { bytes: residual.min(max_pull) }
            }
        } else if len == cursor {
            PullPlan::UpToDate
        } else {
            let residual = len - cursor;
            PullPlan::Ship { bytes: residual.min(max_pull) }
        },
    }
}

/// A `Ship` plan always ships the capped residual.
proof fn lemma_ship_capped(
    len: u64,
    cursor: u64,
    max_pull: u64,
    stamp_then: Option<u64>,
    stamp_now: u64,
    bytes: u64,
)
    requires
        pull_plan_spec(Some(len), cursor, max_pull, stamp_then, stamp_now)
            == (PullPlan::Ship { bytes: bytes }),
    ensures
        bytes == capped_residual(len, cursor, max_pull),
        len > cursor,
        stamp_then.is_none() || stamp_then == Some(stamp_now),
{
}

/// AS-IS F165: length-only check (no stamp, missing file is "up to date").
pub open spec fn pull_plan_as_is_spec(
    file_len: Option<u64>,
    cursor: u64,
    max_pull: u64,
    _stamp_then: Option<u64>,
    _stamp_now: u64,
) -> PullPlan {
    match file_len {
        None => PullPlan::UpToDate,
        Some(len) => if len < cursor {
            PullPlan::Rotated { file_len: len, cursor: cursor }
        } else if len == cursor {
            PullPlan::UpToDate
        } else {
            PullPlan::Ship { bytes: capped_residual(len, cursor, max_pull) }
        },
    }
}

/// Teeth 1: rotate-then-regrow past the cursor. The guard fails closed; the
/// AS-IS length check ships misaligned bytes.
proof fn lemma_as_is_ships_after_regrow(
    len: u64,
    cursor: u64,
    max_pull: u64,
    stamp_then: u64,
    stamp_now: u64,
)
    requires
        cursor > 0,
        len > cursor,
        stamp_then != stamp_now,
    ensures
        pull_plan_spec(Some(len), cursor, max_pull, Some(stamp_then), stamp_now)
            == (PullPlan::Rotated { file_len: len, cursor: cursor }),
        pull_plan_as_is_spec(Some(len), cursor, max_pull, Some(stamp_then), stamp_now)
            == (PullPlan::Ship { bytes: capped_residual(len, cursor, max_pull) }),
{
}

/// Teeth 2: the WAL vanished under an advanced cursor. The guard fails
/// closed; AS-IS reports up-to-date (silently stale replica).
proof fn lemma_as_is_silent_on_vanished(cursor: u64, max_pull: u64, stamp_then: u64)
    requires
        cursor > 0,
    ensures
        pull_plan_spec(None, cursor, max_pull, Some(stamp_then), 0)
            == (PullPlan::Rotated { file_len: 0, cursor: cursor }),
        pull_plan_as_is_spec(None, cursor, max_pull, Some(stamp_then), 0)
            == PullPlan::UpToDate,
{
}

} // verus!
