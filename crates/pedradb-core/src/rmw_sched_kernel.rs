//! rmw scheduling decision for the concurrent async 1-op pipeline
//! (RFC-0211). Integer arithmetic, no I/O.
//!
//! The rank-1 measured hole (same-boot p209b, 3 quiet rounds, min-of-3):
//! `ycsb_f_mc4` floors at 0.532× vs the Rocks `sync=false` peer with the
//! WAL syscall inside the Db write lock, and at 0.780× with it staged —
//! the remainder is the bypass itself. With `writers == ncpu` the 0201
//! boundary (`client_axis_kernel::async_merge_policy`: merge only when
//! writers OUTNUMBER the CPUs) keeps every 1-op async writer on the
//! bypass: each takes the Db write lock and runs the whole commit
//! serialized (encode + WAL write + apply + publish).
//!
//! The drainable group pipeline (leader drains the queued generation,
//! one frame, off-lock WAL write, group apply) already exists, carries
//! the sync/multi-op groups, and is paid in the oversubscribed regime
//! (kvrocks_set_mc50 1.678×). This kernel extends the boundary to the
//! single-op async group — the rmw shape is `get` + one `put`, and the
//! put is the op that sinks (same boot: write-pure mc4 1.035, mix 50/50
//! 1.537, rmw 0.532).
//!
//! Multi-op batches stay on the bypass at `writers <= ncpu` in BOTH
//! regimes: `deps_apply_batch_mc4` / `deps_raftlog_mc4` are paid guards
//! (1.0859 / 1.577) and the new boundary must not move them. No
//! wait-to-grow anywhere (0180/0190 veto): the leader drains what has
//! ALREADY queued, capped by the 0201 misuse floor.
//!
//! 2026-09-14: the wiring default is ON (`PEDRA_RMW_SCHED=0` restores the
//! bypass). Railway overwrite_mc4 0.35× was 4 writers on 48 CPUs taking
//! the 0201 bypass; the 4-vCPU cartaz is the same `writers == ncpu` hole.

#![forbid(unsafe_code)]

/// Pipeline decision for one concurrent async submit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedDecision {
    /// Every writer runs its own commit under the Db write lock (the
    /// Rocks shape; the 0201 default at `writers <= ncpu`).
    Bypass,
    /// Concurrent writers join the drainable group: one leader encodes
    /// the queued generation into one frame, WAL write off the Db lock,
    /// group apply/publish.
    Merge,
}

/// RFC-0211 boundary: merge when writers outnumber the CPUs (the intact
/// 0201 rule) OR when the submit is a single-op async write and at least
/// two writers are active — the `writers == ncpu` rmw regime this RFC
/// owns. `forced` is the explicit `PEDRA_ASYNC_GROUP=1|0` pin and wins in
/// both directions, exactly as in `async_merge_policy`.
#[must_use]
pub fn rmw_group_sched(
    writers: usize,
    ncpu: usize,
    single_op: bool,
    forced: Option<bool>,
) -> SchedDecision {
    match forced {
        Some(pin) => {
            if pin {
                SchedDecision::Merge
            } else {
                SchedDecision::Bypass
            }
        }
        // Degenerate boxes never merge (mirrors `async_merge_policy`:
        // a single-CPU box keeps the lone/mc2 shapes off the leader path)
        // and a lone writer has no group to join.
        None if ncpu == 0 || writers <= 1 => SchedDecision::Bypass,
        None if writers > ncpu => SchedDecision::Merge,
        None if single_op => SchedDecision::Merge,
        None => SchedDecision::Bypass,
    }
}

/// AS-IS twin of [`rmw_group_sched`] — the exact 0201 boundary
/// (`client_axis_kernel::async_merge_policy`): the op shape is
/// irrelevant, only the oversubscription line decides.
#[must_use]
pub fn rmw_group_sched_as_is(
    writers: usize,
    ncpu: usize,
    single_op: bool,
    forced: Option<bool>,
) -> SchedDecision {
    let _ = single_op;
    match forced {
        Some(pin) => {
            if pin {
                SchedDecision::Merge
            } else {
                SchedDecision::Bypass
            }
        }
        None if ncpu > 0 && writers > ncpu => SchedDecision::Merge,
        None => SchedDecision::Bypass,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC-0211 P0.1: the new boundary — single-op async at
    /// `writers == ncpu` merges; the AS-IS twin (0201) keeps the bypass.
    #[test]
    fn rfc0211_boundary_writers_eq_ncpu_single_op_merges() {
        assert_eq!(
            rmw_group_sched(4, 4, true, None),
            SchedDecision::Merge,
            "mc4 on the 4-vCPU cartaz box: the rmw regime this RFC owns"
        );
        assert_eq!(
            rmw_group_sched_as_is(4, 4, true, None),
            SchedDecision::Bypass,
            "AS-IS twin: writers == ncpu keeps the 0201 bypass"
        );
        assert_eq!(
            rmw_group_sched(2, 4, true, None),
            SchedDecision::Merge,
            "mc2 single-op joins the group under the new rule"
        );
        assert_eq!(
            rmw_group_sched_as_is(2, 4, true, None),
            SchedDecision::Bypass
        );
    }

    /// RFC-0211 P0.1: the AS-IS twin IS the 0201 rule — over the whole
    /// (writers × ncpu × pin) grid the twin's decision equals
    /// `client_axis_kernel::async_merge_policy`.
    #[test]
    fn rfc0211_as_is_twin_equals_the_0201_rule_everywhere() {
        for writers in 0..=64usize {
            for ncpu in [0usize, 1, 2, 4, 8, 64] {
                for forced in [None, Some(true), Some(false)] {
                    let policy =
                        crate::client_axis_kernel::async_merge_policy(writers, ncpu, forced);
                    assert_eq!(
                        rmw_group_sched_as_is(writers, ncpu, false, forced),
                        if policy { SchedDecision::Merge } else { SchedDecision::Bypass },
                        "twin != 0201 rule at writers={writers} ncpu={ncpu} forced={forced:?}"
                    );
                }
            }
        }
    }

    /// RFC-0211 P0.1: multi-op batches keep the bypass at
    /// `writers <= ncpu` in BOTH regimes — the paid guards
    /// (`deps_apply_batch_mc4`, `deps_raftlog_mc4`) must not move.
    #[test]
    fn rfc0211_multi_op_stays_bypass_at_eq_ncpu() {
        assert_eq!(rmw_group_sched(4, 4, false, None), SchedDecision::Bypass);
        assert_eq!(
            rmw_group_sched_as_is(4, 4, false, None),
            SchedDecision::Bypass
        );
        assert_eq!(rmw_group_sched(3, 8, false, None), SchedDecision::Bypass);
        assert_eq!(
            rmw_group_sched_as_is(3, 8, false, None),
            SchedDecision::Bypass
        );
    }

    /// RFC-0211 P0.1: `writers > ncpu` merges in both regimes (the 0201
    /// rule is intact — mc50 keeps its paid 1.678× path).
    #[test]
    fn rfc0211_oversubscribed_merges_in_both() {
        assert_eq!(rmw_group_sched(50, 4, true, None), SchedDecision::Merge);
        assert_eq!(rmw_group_sched(50, 4, false, None), SchedDecision::Merge);
        assert_eq!(
            rmw_group_sched_as_is(50, 4, true, None),
            SchedDecision::Merge
        );
        assert_eq!(
            rmw_group_sched_as_is(50, 4, false, None),
            SchedDecision::Merge
        );
    }

    /// RFC-0211 P0.1: the `PEDRA_ASYNC_GROUP` pin wins over the new
    /// boundary exactly as it wins over the old one.
    #[test]
    fn rfc0211_forced_pin_wins_both_directions() {
        assert_eq!(
            rmw_group_sched(4, 4, true, Some(false)),
            SchedDecision::Bypass,
            "pin=0 keeps even the rmw regime off the leader path"
        );
        assert_eq!(
            rmw_group_sched(1, 64, false, Some(true)),
            SchedDecision::Merge,
            "pin=1 merges even where the auto rules never would"
        );
        assert_eq!(
            rmw_group_sched_as_is(4, 4, true, Some(false)),
            SchedDecision::Bypass
        );
        assert_eq!(
            rmw_group_sched_as_is(1, 64, false, Some(true)),
            SchedDecision::Merge
        );
    }

    /// RFC-0211 P0.1 misuse guards: degenerate boxes and lone writers
    /// never merge — the decision must not deadlock a single-CPU box or
    /// route a lone submit through the leader hop.
    #[test]
    fn rfc0211_degenerate_boxes_never_merge() {
        assert_eq!(rmw_group_sched(16, 0, true, None), SchedDecision::Bypass);
        assert_eq!(rmw_group_sched(0, 4, true, None), SchedDecision::Bypass);
        assert_eq!(rmw_group_sched(1, 4, true, None), SchedDecision::Bypass);
        assert_eq!(rmw_group_sched(1, 1, true, None), SchedDecision::Bypass);
        assert_eq!(
            rmw_group_sched_as_is(16, 0, true, None),
            SchedDecision::Bypass
        );
    }
}
