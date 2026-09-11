//! Client-axis pipeline policy (RFC-0201). Integer arithmetic, no I/O.
//!
//! The 1-op async pipeline is the default concurrent write path, and two of
//! its policies were blind to the client axis (writers vs CPUs):
//! (a) the leader's drain cap serialized big generations into convoys —
//! [`pipeline_drain_cap`] takes every already-queued waiter up to a hard
//! misuse floor (the AS-IS twin pins the old RFC-0044-era cap 8, a
//! WriteThread-shape artifact);
//! (b) the follower spin assumed the leader always has a CPU to run on —
//! [`oversubscription_spin_policy`] applies the adaptive-mutex rule (spin
//! only while the owner runs) at the process scale: `Spin` while writers
//! fit the CPUs, `Park` immediately when oversubscribed.
//!
//! G1 remains uncapped by fd-sharing (see `async_group_drain_cap`); this
//! kernel only owns the async pipeline's drain bound and the spin decision.

#![forbid(unsafe_code)]

/// Hard misuse bound for one async leader's drain (RFC-0201 P0.1). Full
/// drain is O(queued) in walk/encode/complete work — all cheap and
/// off-mutex — but an unbounded drain on a hostile caller (thousands of
/// parked waiters) would hold the leader cycle too long. Above this the
/// remainder relinks exactly like the AS-IS excess path.
pub const PIPELINE_DRAIN_MAX_MEMBERS: usize = 256;

/// AS-IS twin: the cap the async pipeline shipped before RFC-0201
/// (`ASYNC_GROUP_MAX_MEMBERS`, born from the uncapped-herd measurement of
/// 0044 in the dead WriteThread-merge shape). Kept as the named dente the
/// tests pin — the WriteThread multi-op merge still uses it.
pub const PIPELINE_DRAIN_CAP_AS_IS: usize = 8;

/// Members one leader may drain from `queued` already-queued waiters.
/// Full drain up to the misuse floor; at least the leader itself.
#[must_use]
pub fn pipeline_drain_cap(queued: usize) -> usize {
    queued.clamp(1, PIPELINE_DRAIN_MAX_MEMBERS)
}

/// AS-IS twin of [`pipeline_drain_cap`] — the pre-0198 cap-8 convoy.
#[must_use]
pub fn pipeline_drain_cap_as_is(queued: usize) -> usize {
    queued.clamp(1, PIPELINE_DRAIN_CAP_AS_IS)
}

/// Serial leader convoys a generation of `queued` waiters pays under
/// `cap` — the O(ceil) lock+reserve+encode+pwrite cycles the full drain
/// collapses to one. mc50 under the AS-IS cap: `ceil(50/8) == 7`; under
/// RFC-0201: `ceil(50/256) == 1`.
#[must_use]
pub fn drain_convoy_count(queued: usize, cap: usize) -> u64 {
    if queued == 0 {
        return 0;
    }
    let cap = cap.max(1) as u64;
    (queued as u64 + cap - 1) / cap
}

/// Follower wait policy for the pipeline's completion spin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpinDecision {
    /// Writers fit the CPUs — the 0189 adaptive spin stands (mc4 quiet
    /// −17% p50, v5): heartbeats observable, spinning is cheaper than a
    /// park/unpark pair on the leader's critical cycle.
    Spin,
    /// Oversubscribed — every spinning follower steals CPU from the very
    /// leader that would complete it. Park immediately (adaptive-mutex
    /// rule at process scale).
    Park,
}

/// `Spin` iff `writers <= ncpu` (a degenerate `ncpu == 0` spins — the
/// decision must never deadlock a single-CPU box with one writer).
#[must_use]
pub fn oversubscription_spin_policy(writers: usize, ncpu: usize) -> SpinDecision {
    if ncpu == 0 || writers <= ncpu {
        SpinDecision::Spin
    } else {
        SpinDecision::Park
    }
}

/// AS-IS twin of [`oversubscription_spin_policy`] — the 0189 spin is
/// blind to CPU count: it always spins.
#[must_use]
pub fn oversubscription_spin_policy_as_is(writers: usize, ncpu: usize) -> SpinDecision {
    let _ = (writers, ncpu);
    SpinDecision::Spin
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC-0201 P0.1: full drain up to the misuse floor, leader floor 1.
    #[test]
    fn rfc0201_drain_cap_full_below_misuse_floor() {
        assert_eq!(pipeline_drain_cap(0), 1, "leader alone is its own group");
        assert_eq!(pipeline_drain_cap(1), 1);
        assert_eq!(pipeline_drain_cap(50), 50, "mc50 drains in one group");
        assert_eq!(pipeline_drain_cap(255), 255);
        assert_eq!(
            pipeline_drain_cap(256), 256,
            "boundary: exactly the misuse floor"
        );
        assert_eq!(
            pipeline_drain_cap(1_000), PIPELINE_DRAIN_MAX_MEMBERS,
            "hostile caller clamps at the misuse floor"
        );
    }

    /// RFC-0201 P0.1 dente: the AS-IS twin pins the 0044-era cap-8 convoy
    /// math the cut removes (mc50 = 7 serial convoys vs 1).
    #[test]
    fn rfc0201_drain_convoy_count_as_is_vs_full() {
        assert_eq!(PIPELINE_DRAIN_CAP_AS_IS, 8);
        assert_eq!(pipeline_drain_cap_as_is(50), 8);
        assert_eq!(
            drain_convoy_count(50, pipeline_drain_cap_as_is(50)),
            7,
            "AS-IS twin: the cap-8 convoy the cut removes"
        );
        assert_eq!(drain_convoy_count(50, 8), 7, "AS-IS mc50: ceil(50/8)");
        assert_eq!(
            drain_convoy_count(50, pipeline_drain_cap(50)),
            1,
            "0198 mc50: one convoy"
        );
        assert_eq!(drain_convoy_count(0, 8), 0);
        assert_eq!(drain_convoy_count(9, 8), 2);
        assert_eq!(drain_convoy_count(300, PIPELINE_DRAIN_MAX_MEMBERS), 2);
    }

    /// RFC-0201 P0.2: spin while writers fit the CPUs, park when
    /// oversubscribed; the degenerate boxes never park a lone writer.
    #[test]
    fn rfc0201_spin_policy_both_sides() {
        assert_eq!(oversubscription_spin_policy(1, 1), SpinDecision::Spin);
        assert_eq!(oversubscription_spin_policy(1, 0), SpinDecision::Spin);
        assert_eq!(
            oversubscription_spin_policy(4, 4),
            SpinDecision::Spin,
            "mc4 on the 4-vCPU bench box keeps the v5 spin"
        );
        assert_eq!(
            oversubscription_spin_policy(50, 4),
            SpinDecision::Park,
            "mc50 on 4 vCPU: the spin-herd starves the leader"
        );
        assert_eq!(oversubscription_spin_policy(11, 10), SpinDecision::Park);
        assert_eq!(oversubscription_spin_policy(10, 10), SpinDecision::Spin);
    }

    /// RFC-0201 P0.2 dente: the 0189 spin never parks by policy — the
    /// cegueira the cut names.
    #[test]
    fn rfc0201_spin_policy_as_is_always_spins() {
        assert_eq!(
            oversubscription_spin_policy_as_is(50, 4),
            SpinDecision::Spin
        );
        assert_eq!(
            oversubscription_spin_policy_as_is(1_000, 1),
            SpinDecision::Spin
        );
    }
}
