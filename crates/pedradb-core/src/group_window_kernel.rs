//! RFC-0217 P0.1 — bounded collection window for the async group leader.
//!
//! Measured state (p211p/p211q, PHASE/wg): on every clean-arm cell
//! `avg_grp == 1.00` — the default policy keeps async writers on the
//! bypass at writers ≤ ncpu (one full serial section per commit), and
//! where they do merge the leader drains immediately, so each group is
//! the leader alone. The window is the missing half: with
//! `PEDRA_GROUP_WINDOW_US > 0` (or `ConcurrentDb::set_group_window`),
//! merge becomes eligible at ≥ 2 writers and the leader holds the group
//! open for a bounded µs window — arrivals inside the window share this
//! flight's single `write()`/encode pass instead of each paying the
//! serial section alone.
//!
//! An async-only group has no fd to break even against (that is the
//! RFC-0044 P0.5 reason the catch-up hold was skipped), so the bound is
//! the flat clamped window. P0.1b (smoke 2026-09-13: `avg_grp == 1.16`
//! at mc2 with the flat window alone — the peer parks only when it
//! submits during the leader's WAL section): the missing writer is
//! usually in its client-side gap, invisible to `active`, so a recent
//! peer opens the collect through the gap and a quiet quiescence slice
//! ends it. The lone path (`active == 1`, no recent peer) never merges
//! and never waits — single-client p50 is untouched by construction.

/// Misuse ceiling for the window (µs). The window must stay far below a
/// real fdatasync (hundreds of µs) and below park/unpark convoys; larger
/// requested values clamp here instead of taxing every group.
pub const GROUP_WINDOW_MAX_US: u64 = 1_000;

/// Parse `PEDRA_GROUP_WINDOW_US`. `None`/`0`/garbage → 0 (off — the
/// AS-IS behavior); anything above [`GROUP_WINDOW_MAX_US`] clamps.
#[must_use]
pub fn group_window_us(raw: Option<&str>) -> u64 {
    match raw {
        Some(s) => s
            .parse::<u64>()
            .map(|v| v.min(GROUP_WINDOW_MAX_US))
            .unwrap_or(0),
        None => 0,
    }
}

/// Window on ⇒ async merge is eligible from 2 writers up (the 0201
/// boundary — writers > ncpu — keeps its own arm; this ORs in below it).
/// P0.1b: a **recent peer** counts too — the peer in its client-side
/// gap (get between puts, RMW read) is invisible to `active`, and
/// without it the submitter falls to the write-lock bypass (no leader,
/// no collect) exactly in the low-concurrency regime the window exists
/// for (measured: ycsb_f mc2 pinned avg_grp 1.30 until this arm).
#[must_use]
pub fn merge_eligible(writers: usize, window_us: u64, peers_recent: bool) -> bool {
    window_us > 0 && (writers >= 2 || peers_recent)
}

/// How long an async-only group leader may hold the group open (µs):
/// the flat window, zero when the window is off or nobody can arrive.
/// Two ways to be missing: a counted writer absent from the batch
/// (`active > batch_len`), or a **gap ghost** — a peer between
/// reply-consumption and its next submit, invisible to `active` (the
/// counter drops at reply consumption). `peers_recent` (another writer
/// submitted within the recent-concurrency horizon) opens the collect
/// through the ghost's client-side gap; the loop in `lead` exits on
/// arrival (condvar) or a quiet quiescence slice, so the window is a
/// bound, not a sleep.
#[must_use]
pub fn async_catchup_bound_us(
    window_us: u64,
    active: usize,
    batch_len: usize,
    peers_recent: bool,
) -> u64 {
    if window_us == 0 {
        0
    } else if active > batch_len || peers_recent {
        window_us
    } else {
        0
    }
}

/// RFC-0226 P0.2 (was RFC-0211 follow-up, measured 2026-09-14): seal an
/// async-only group at the **first drain** when the leader is still
/// alone in the batch (`batch_len == 1`) and the writers do not
/// outnumber the CPUs. A multi-member first drain already *is* a group
/// — collect stays (the mc16 −13% of the 2026-09-14 cut: `submit_active
/// ≤ ncpu` was firing the seal on a 16-client box because not every
/// client sat inside `submit()` at once).
///
/// RFC-0233 P1.4 (2026-09-16, negative result): gating the seal on live
/// `inflight ≤ batch_len` (keep collect while joiners are in `submit`)
/// formed real groups (avg_group 1.10→2.17, lock_wait 4.28→0.49µs) but
/// **halved** throughput (min-of-3 0.732→0.429 vs the same-day peer):
/// the async WAL is an mmap memcpy (~0.4µs), there is no fd to
/// amortize, so the ~10µs collect is a pure latency tax. Singleton
/// seal stays; the convoy is paid down in the critical section, not by
/// batching.
///
/// Stays false (collect as before) when any member syncs (the waits are
/// the fd amortization), when an explicit `PEDRA_GROUP_WINDOW_US` is on,
/// when writers outnumber the CPUs, or when the first drain already
/// has 2+ members.
#[must_use]
pub fn seal_async_first_drain(
    writers: usize,
    ncpu: usize,
    any_sync: bool,
    window_us: u64,
    batch_len: usize,
) -> bool {
    // `writers` is the max submit-time `active` over the group's members —
    // writers piled up inside `submit` (the oversubscription signal). A
    // peer in its client-side gap is invisible to it (the RFC-0217 P0.1b
    // gap ghost), so no lower bound is placed on it: a lone leader with a
    // recent peer seals too (waiting for the ghost is the measured loss).
    // RFC-0226 P0.2: `batch_len == 1` is the other bound — a first drain
    // that already absorbed 2+ members keeps collect (mc16 named cost).
    !any_sync && window_us == 0 && writers <= ncpu && batch_len == 1
}

/// AS-IS twin of [`seal_async_first_drain`] — the 2026-09-14 shape:
/// seals whenever writers ≤ ncpu, ignoring `batch_len` (the mc16
/// false-positive). `PEDRA_SEAL_ASYNC=0` at the call site is the older
/// pre-2026-09-14 twin (never seals).
#[must_use]
pub fn seal_async_first_drain_as_is(
    writers: usize,
    ncpu: usize,
    any_sync: bool,
    window_us: u64,
    _batch_len: usize,
) -> bool {
    !any_sync && window_us == 0 && writers <= ncpu
}

/// RFC-0226 P1.1: a first-drain leader who is still alone (empty queue,
/// nobody in `submit`) takes the existing 1-op commit path instead of
/// the group serial section. Group-of-1 paying Mutex+mpsc was the
/// bottom of the U (`avg_group=1.04` at mc2). Twin AS-IS always keeps
/// the group path.
#[must_use]
pub fn solo_leader_bypass(batch_len: usize, queue_len: usize, active: usize) -> bool {
    batch_len == 1 && queue_len == 0 && active <= 1
}

/// RFC-0227 P2.6: opt-in flag for the solo-leader bypass.
#[must_use]
pub fn solo_bypass_armed(opt_in: bool) -> bool {
    opt_in
}

/// AS-IS twin of [`solo_leader_bypass`]: every first drain stays in
/// `lead()` (the group serial section).
#[must_use]
pub fn solo_leader_bypass_as_is(_batch_len: usize, _queue_len: usize, _active: usize) -> bool {
    false
}

/// Quiescence slice (µs) during the collect window. A group publish
/// releases all followers within µs of each other, so arrivals come in
/// bursts; a quiet slice after the first absorb means the burst drained.
pub const COLLECT_QUIESCE_US: u64 = 20;

/// When in-flight writers outnumber the batch they are inside `submit()`,
/// not in a client gap. Spin this many µs for them to queue before
/// sealing the WAL frame. 10µs condvar packed avg_group=3.78 but parked
/// (cw=23µs/grp) and lost QPS; the same bound as a **spin** (no
/// `wait_for`) is half a Darwin `write()` with no park tax. AS-IS = 0.
pub const HERD_COLLECT_US: u64 = 10;

/// Stop spinning once this many members are in the frame (mc4). More
/// is extra wait for a syscall that already amortizes 4 puts.
pub const HERD_TARGET: usize = 4;

/// After a fair unlock, yield-loop this long so waiters can `push` before
/// the leader snapshots the queue (Rocks `LinkOne` then walk the list).
/// Stops early at [`join_complete`] (1c: instant). 20µs = one Darwin
/// `write()` — break-even at +1 op.
pub const JOIN_LOOK_US: u64 = 20;

/// The WAL frame has everyone in `submit()` (or the mc4 herd).
#[must_use]
pub fn join_complete(batch_len: usize, active: usize) -> bool {
    herd_full(batch_len) || (active > 0 && batch_len >= active)
}

/// AS-IS: seal after the first drain.
#[must_use]
pub fn join_complete_as_is(_batch_len: usize, _active: usize) -> bool {
    true
}

/// Frame has the mc4 herd — do not wait more.
#[must_use]
pub fn herd_full(batch_len: usize) -> bool {
    batch_len >= HERD_TARGET
}

/// AS-IS: the frame is never full (would keep spinning).
#[must_use]
pub fn herd_full_as_is(_batch_len: usize) -> bool {
    false
}

/// Spin [`HERD_COLLECT_US`] when the frame is short AND someone else is
/// in-flight (`active > batch`) or just got a reply (`peers_recent` —
/// the leader is alone in `active` after publish, peers are in the
/// put-loop gap). `herd_full` ⇒ 0 (frame is full).
#[must_use]
pub fn herd_collect_us(active: usize, batch_len: usize, peers_recent: bool) -> u64 {
    if herd_full(batch_len) {
        0
    } else if active > batch_len || peers_recent {
        HERD_COLLECT_US
    } else {
        0
    }
}

/// AS-IS: never wait for the in-flight herd (seal with whoever drained).
#[must_use]
pub fn herd_collect_us_as_is(_active: usize, _batch_len: usize) -> u64 {
    0
}

/// After a multi-member group publishes, peers are in the reply→put gap
/// (~µs) while the leader is already on the next drain with `active=1`.
/// Spin [`HERD_COLLECT_US`] so those puts join this frame. Lone groups
/// (prev < 2) do not wait — that was avg_group=1.01 / 17 kQPS when
/// `peers_recent` (250µs MULTI_HOLD) forced a 10µs spin on every group.
#[must_use]
pub fn post_group_grace_us(prev_len: usize, batch_len: usize) -> u64 {
    if prev_len >= 2 && !herd_full(batch_len) {
        HERD_COLLECT_US
    } else {
        0
    }
}

/// AS-IS: never grace-spin after a multi-member publish.
#[must_use]
pub fn post_group_grace_us_as_is(_prev_len: usize, _batch_len: usize) -> u64 {
    0
}

/// Collect-loop break: only after at least one arrival beyond the entry
/// batch went quiet. A silent slice with no arrivals keeps waiting (the
/// ghost may still be inside the window bound).
#[must_use]
pub fn collect_should_break(quiesce_timed_out: bool, batch_len: usize, initial_len: usize) -> bool {
    quiesce_timed_out && batch_len > initial_len
}

/// Lone-path peer horizon (µs) with the window on. A peer whose
/// inter-submit gap (RMW get, client think time) exceeds `base_us`
/// (MULTI_HOLD, 250 µs) would let the lone bypass steal the leader
/// before the collect window can absorb it — measured: ycsb_f mc2 flat
/// window pinned `avg_grp == 1.31` while the pure-put twin pinned
/// 1.88. The horizon extends to the window itself so the ghost stays
/// collectable; window 0 keeps `base_us` exactly (AS-IS twin).
#[must_use]
pub fn peer_horizon_us(window_us: u64, base_us: u64) -> u64 {
    window_us.max(base_us)
}

/// P0.4 (opened by P0.3b): seed for the flight EMA before the first
/// real group sample (µs) — same convention as the fd EMA seed
/// (RFC-0042 P1.1). The bootstrap must open a small real window once so
/// a group can form and measure the true flight; after that the EMA
/// owns the cap.
pub const GROUP_FLIGHT_SEED_US: u64 = 25;

/// Parse `PEDRA_GROUP_WINDOW_CAP_TO_FLIGHT`. Off by default — the flat
/// clamped window (the P0.3 shape).
#[must_use]
pub fn group_window_cap_to_flight(raw: Option<&str>) -> bool {
    matches!(raw, Some(s) if s == "1" || s.eq_ignore_ascii_case("true"))
}

/// P0.4: the collect window may not exceed the previous group's
/// measured **flight** — the off-lock WAL section (`write()`, plus
/// `fdatasync` on sync groups). P0.3 measured the flat 1000 µs window
/// LOSING mc2–mc4 (0.10–0.14 vs clean): the wait runs before the flight
/// with nothing in flight to overlap, so it is pure added latency.
/// Capping at the flight keeps the hold bounded by a real serial
/// section: on ext4 the per-group `write()` is the p201r2 mc4 owner
/// (one syscall per op on the bypass — the window rides it); on
/// Darwin-async the measured flight is 4.74 µs/commit (10M-scale
/// write10m PHASE split, 2026-09-13) — below one quiescence slice, so
/// the window collapses to off and the AS-IS behavior returns (no
/// regression by construction). A capped window below one quiescence
/// slice cannot complete a collect — collapse to 0 rather than pay a
/// lock+park per group. Unsampled flight seeds at
/// [`GROUP_FLIGHT_SEED_US`] so the first group can form and measure.
#[must_use]
pub fn flight_capped_window_us(window_us: u64, flight_ema_us: u64, cap_to_flight: bool) -> u64 {
    if !cap_to_flight || window_us == 0 {
        return window_us;
    }
    let flight = if flight_ema_us == 0 {
        GROUP_FLIGHT_SEED_US
    } else {
        flight_ema_us
    };
    let capped = window_us.min(flight);
    if capped < COLLECT_QUIESCE_US {
        0
    } else {
        capped
    }
}

/// AS-IS: the P0.3 flat window — never cap to flight. Darwin-async
/// ~2µs flight still pays the full 1000µs collect (measured 0.10–0.14×).
#[must_use]
pub fn flight_capped_window_us_as_is(
    window_us: u64,
    _flight_ema_us: u64,
    _cap_to_flight: bool,
) -> u64 {
    window_us
}

/// AS-IS twin of [`merge_eligible`] — the 0044-era default: no window,
/// no low-writer merge (the 0201 `writers > ncpu` arm alone).
#[must_use]
pub fn merge_eligible_as_is(_writers: usize, _window_us: u64) -> bool {
    false
}

/// AS-IS twin of [`async_catchup_bound_us`] — async-only groups never
/// wait (RFC-0044 P0.5 shape: `avg_grp == 1.00`).
#[must_use]
pub fn async_catchup_bound_us_as_is(_window_us: u64, _active: usize, _batch_len: usize) -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC-0222 P0.7: production concurrent.rs calls the kernel, not an
    /// inlined window predicate.
    #[test]
    fn concurrent_calls_group_window_kernel() {
        let body = include_str!("concurrent_kernel.rs");
        assert!(
            body.contains("group_window_kernel::merge_eligible("),
            "submit path must call merge_eligible"
        );
        assert!(
            body.contains("group_window_kernel::flight_capped_window_us("),
            "effective window must call flight_capped_window_us"
        );
        assert!(
            body.contains("group_window_kernel::herd_collect_us("),
            "async lead must wait for in-flight submitters before sealing the WAL frame"
        );
        assert!(
            body.contains("group_window_kernel::seal_async_first_drain("),
            "lead must derive the first-drain seal from the kernel"
        );
        assert!(
            body.contains("group_window_kernel::solo_leader_bypass("),
            "lead must hand a still-alone first drain to commit_async_one"
        );
        assert!(
            body.contains("group_window_kernel::solo_bypass_armed("),
            "lead opt-in must call solo_bypass_armed"
        );
        assert!(
            body.contains("write_admission_kernel::one_op_commit("),
            "solo lead 1-op vs batch must call one_op_commit"
        );
        assert!(
            body.contains("group_window_kernel::post_group_grace_us("),
            "lead must grace-spin after a multi-member publish"
        );
        assert!(
            body.contains("group_window_kernel::join_complete("),
            "lead must snapshot the join list like WriteThread"
        );
    }

    #[test]
    fn herd_collect_waits_only_for_missing_inflight() {
        assert_eq!(herd_collect_us(4, 1, false), HERD_COLLECT_US);
        assert_eq!(
            herd_collect_us(1, 1, true),
            HERD_COLLECT_US,
            "gap after publish"
        );
        assert_eq!(
            herd_collect_us(4, 4, true),
            0,
            "frame already at HERD_TARGET"
        );
        assert_eq!(herd_collect_us(1, 1, false), 0, "lone 1c never waits");
        assert!(herd_full(4));
        assert!(!herd_full(3));
        assert_eq!(herd_collect_us_as_is(8, 1), 0, "AS-IS seals immediately");
        assert!(!herd_full_as_is(4), "AS-IS tooth: frame never full");
        assert_eq!(post_group_grace_us(4, 1), HERD_COLLECT_US);
        assert_eq!(post_group_grace_us(1, 1), 0, "after a lone group, no grace");
        assert_eq!(post_group_grace_us(4, 4), 0, "already full");
        assert_eq!(
            post_group_grace_us_as_is(4, 1),
            0,
            "AS-IS tooth: never grace-spin"
        );
        assert!(join_complete(4, 1), "herd full");
        assert!(join_complete(3, 3), "everyone in submit is in the frame");
        assert!(!join_complete(1, 4), "3 joiners still outside");
        assert!(join_complete_as_is(1, 8), "AS-IS seals after first drain");
    }

    #[test]
    fn rfc0217_group_window_env_parse_off_by_default() {
        assert_eq!(group_window_us(None), 0);
        assert_eq!(group_window_us(Some("")), 0);
        assert_eq!(group_window_us(Some("0")), 0);
        assert_eq!(group_window_us(Some("garbage")), 0);
        assert_eq!(group_window_us(Some("250")), 250);
    }

    /// RFC-0226 P0.2: singleton first drain seals; a multi-member first
    /// drain under the CPU cap keeps collect (the mc16 −13% of the
    /// 2026-09-14 cut). Sync / window-on / writers>ncpu / ncpu=0 stay
    /// false. AS-IS twin is the 2026-09-14 policy (ignores batch_len).
    /// RFC-0233 P1.4 negative result (2026-09-16): an `inflight ≤ batch`
    /// gate kept collect for in-submit joiners, formed avg_group 2.17
    /// groups, and still halved throughput — reverted.
    #[test]
    fn seal_async_first_drain_boundary() {
        assert!(
            seal_async_first_drain(4, 12, false, 0, 1),
            "mc4 singleton first drain on a 12-CPU box seals"
        );
        assert!(
            !seal_async_first_drain(4, 12, false, 0, 4),
            "first drain with batch≥2 under the CPU cap keeps collect"
        );
        assert!(
            !seal_async_first_drain(4, 12, false, 0, 2),
            "a two-member first drain is already a group"
        );
        assert!(
            seal_async_first_drain(4, 4, false, 0, 1),
            "writers == ncpu on the 4-vCPU cartaz, singleton"
        );
        assert!(
            seal_async_first_drain(2, 12, false, 0, 1),
            "mc2 — the avg_group=1.04 solo-leader pathology"
        );
        assert!(
            !seal_async_first_drain(4, 12, true, 0, 1),
            "a syncing member keeps the whole catch-up machinery"
        );
        assert!(
            !seal_async_first_drain(4, 12, false, 250, 1),
            "an explicit group window keeps the collect"
        );
        assert!(
            !seal_async_first_drain(50, 12, false, 0, 1),
            "oversubscribed async (kvrocks_set_mc50 regime) keeps the collect"
        );
        assert!(
            seal_async_first_drain(1, 12, false, 0, 1),
            "a lone leader (peers in client gaps — the P0.1b gap ghost) seals too"
        );
        assert!(
            !seal_async_first_drain(16, 0, false, 0, 1),
            "degenerate boxes keep the AS-IS behavior"
        );
        assert!(
            seal_async_first_drain_as_is(4, 12, false, 0, 4),
            "AS-IS 2026-09-14: seals on writers≤ncpu even with batch≥2 (the mc16 false-positive)"
        );
        assert!(
            !seal_async_first_drain_as_is(50, 12, false, 0, 1),
            "AS-IS still refuses oversubscription"
        );
    }

    /// RFC-0226 P1.1: (batch=1, queue empty, active≤1) ⇒ bypass; any
    /// extra queued or in-submit peer ⇒ keep group; AS-IS always keeps.
    #[test]
    fn solo_leader_bypass_boundary() {
        assert!(
            solo_leader_bypass(1, 0, 1),
            "alone at first drain: take commit_async_one"
        );
        assert!(
            solo_leader_bypass(1, 0, 0),
            "active already 0 (reply consumed) still bypasses"
        );
        assert!(
            !solo_leader_bypass(1, 1, 1),
            "someone queued behind the leader: keep the group"
        );
        assert!(
            !solo_leader_bypass(1, 0, 2),
            "a peer is inside submit: keep the group"
        );
        assert!(
            !solo_leader_bypass(2, 0, 2),
            "first drain already has two members"
        );
        assert!(
            !solo_leader_bypass_as_is(1, 0, 1),
            "AS-IS always keeps the group serial section"
        );
        assert!(!solo_leader_bypass_as_is(1, 0, 0), "AS-IS never bypasses");
        let open = include_str!("concurrent_kernel.rs");
        let arm = open
            .split("solo_bypass: match")
            .nth(1)
            .expect("solo_bypass default");
        assert!(
            arm.contains("Ok(\"0\")") && arm.contains("_ => true"),
            "RFC-0233 P1.4: solo_bypass default ON (ycsb_f_mc4 group-of-1)"
        );
    }

    #[test]
    fn rfc0217_group_window_env_clamps_to_ceiling() {
        assert_eq!(group_window_us(Some("1000")), GROUP_WINDOW_MAX_US);
        assert_eq!(group_window_us(Some("999999")), GROUP_WINDOW_MAX_US);
    }

    #[test]
    fn rfc0217_group_window_merge_eligibility_boundary() {
        assert!(
            !merge_eligible(1, 500, false),
            "a lone writer with no recent peer never groups"
        );
        assert!(
            merge_eligible(2, 1, false),
            "two writers + window on: eligible"
        );
        assert!(
            merge_eligible(1, 500, true),
            "gap ghost: recent peer makes the lone submitter eligible"
        );
        assert!(
            !merge_eligible(50, 0, true),
            "window off keeps the 0201-only arm (mc50 policy decides)"
        );
    }

    #[test]
    fn rfc0217_group_window_bound_zero_when_nobody_missing() {
        assert_eq!(
            async_catchup_bound_us(500, 1, 1, false),
            0,
            "alone, no recent peer: no wait"
        );
        assert_eq!(
            async_catchup_bound_us(500, 3, 3, false),
            0,
            "everyone already queued, no recent peer: no wait"
        );
        assert_eq!(
            async_catchup_bound_us(500, 4, 1, false),
            500,
            "missing counted writers: flat window"
        );
        assert_eq!(
            async_catchup_bound_us(0, 4, 1, true),
            0,
            "window off: async groups keep draining instantly"
        );
    }

    #[test]
    fn rfc0217_group_window_collects_through_client_gap() {
        assert_eq!(
            async_catchup_bound_us(500, 1, 1, true),
            500,
            "gap ghost: peer recent but invisible to active — collect"
        );
        assert_eq!(
            async_catchup_bound_us(500, 3, 3, true),
            500,
            "ghost burst may still land after everyone queued"
        );
    }

    #[test]
    fn rfc0217_group_window_quiesce_break_after_first_arrival() {
        assert!(
            !collect_should_break(true, 1, 1),
            "silent slice, no arrival: keep waiting for the ghost"
        );
        assert!(
            collect_should_break(true, 2, 1),
            "arrival absorbed then quiet: burst drained"
        );
        assert!(
            !collect_should_break(false, 2, 1),
            "condvar wake (real arrival): keep collecting"
        );
    }

    #[test]
    fn rfc0217_group_window_peer_horizon_extends_with_window() {
        assert_eq!(
            peer_horizon_us(0, 250),
            250,
            "window off: MULTI_HOLD exactly (AS-IS twin)"
        );
        assert_eq!(
            peer_horizon_us(1000, 250),
            1000,
            "window on: ghost stays collectable through its gap"
        );
        assert_eq!(peer_horizon_us(250, 250), 250);
    }

    #[test]
    fn rfc0217_group_window_as_is_twins_pin_the_old_shape() {
        assert!(!merge_eligible_as_is(2, 1_000));
        assert!(!merge_eligible_as_is(8, 1_000));
        assert_eq!(async_catchup_bound_us_as_is(1_000, 4, 1), 0);
    }

    #[test]
    fn rfc0217_p04_env_parse_off_by_default() {
        assert!(!group_window_cap_to_flight(None));
        assert!(!group_window_cap_to_flight(Some("")));
        assert!(!group_window_cap_to_flight(Some("0")));
        assert!(!group_window_cap_to_flight(Some("garbage")));
        assert!(group_window_cap_to_flight(Some("1")));
        assert!(group_window_cap_to_flight(Some("true")));
        assert!(group_window_cap_to_flight(Some("TRUE")));
    }

    #[test]
    fn rfc0217_p04_cap_off_is_the_flat_window_twin() {
        assert_eq!(
            flight_capped_window_us(1_000, 2, false),
            1_000,
            "cap off: full window even with ~0 flight (the P0.3 shape)"
        );
        assert_eq!(
            flight_capped_window_us(0, 500, true),
            0,
            "window off stays off regardless of the cap"
        );
    }

    #[test]
    fn rfc0217_p04_window_never_exceeds_flight() {
        assert_eq!(
            flight_capped_window_us(1_000, 300, true),
            300,
            "flight 300µs: the hold is bounded by the real serial section"
        );
        assert_eq!(
            flight_capped_window_us(200, 300, true),
            200,
            "window below the flight keeps the configured window"
        );
        assert_eq!(
            flight_capped_window_us_as_is(1_000, 2, true),
            1_000,
            "AS-IS tooth: cap_to_flight still pays the flat 1000µs window"
        );
        assert_eq!(
            flight_capped_window_us(1_000, 2, true),
            0,
            "repaired: Darwin-async ~2µs flight collapses the collect"
        );
    }

    #[test]
    fn rfc0217_p04_flight_below_quiesce_collapses_to_off() {
        assert_eq!(
            flight_capped_window_us(1_000, 2, true),
            0,
            "Darwin-async flight ≈1–2µs: nothing to ride, collect off"
        );
        assert_eq!(
            flight_capped_window_us(1_000, COLLECT_QUIESCE_US, true),
            COLLECT_QUIESCE_US,
            "exactly one quiescence slice is the smallest usable window"
        );
        assert_eq!(
            flight_capped_window_us(1_000, COLLECT_QUIESCE_US - 1, true),
            0
        );
    }

    #[test]
    fn rfc0217_p04_unsampled_flight_seeds_a_bootstrap_window() {
        assert_eq!(
            flight_capped_window_us(1_000, 0, true),
            GROUP_FLIGHT_SEED_US,
            "no sample yet: a small real window so the first group can form and measure"
        );
    }
}
