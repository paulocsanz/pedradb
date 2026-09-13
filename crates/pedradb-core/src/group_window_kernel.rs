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

/// Quiescence slice (µs) during the collect window. A group publish
/// releases all followers within µs of each other, so arrivals come in
/// bursts; a quiet slice after the first absorb means the burst drained.
pub const COLLECT_QUIESCE_US: u64 = 20;

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

/// AS-IS twin of [`merge_eligible`] — the 0044-era default: no window,
/// no low-writer merge (the 0201 `writers > ncpu` arm alone).
#[must_use]
pub fn merge_eligible_as_is(_writers: usize, _window_us: u64) -> bool {
    false
}

/// AS-IS twin of [`async_catchup_bound_us`] — async-only groups never
/// wait (RFC-0044 P0.5 shape: `avg_grp == 1.00`).
#[must_use]
pub fn async_catchup_bound_us_as_is(
    _window_us: u64,
    _active: usize,
    _batch_len: usize,
) -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc0217_group_window_env_parse_off_by_default() {
        assert_eq!(group_window_us(None), 0);
        assert_eq!(group_window_us(Some("")), 0);
        assert_eq!(group_window_us(Some("0")), 0);
        assert_eq!(group_window_us(Some("garbage")), 0);
        assert_eq!(group_window_us(Some("250")), 250);
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
        assert!(merge_eligible(2, 1, false), "two writers + window on: eligible");
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
}
