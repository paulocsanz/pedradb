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
//! the flat clamped window; the leader only waits when a writer is
//! actually missing (`active > batch_len`). The lone path (`active == 1`)
//! never merges and never waits — single-client p50 is untouched by
//! construction.

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
#[must_use]
pub fn merge_eligible(writers: usize, window_us: u64) -> bool {
    window_us > 0 && writers >= 2
}

/// How long an async-only group leader may hold the group open (µs):
/// the flat window, but zero when the window is off or nobody is
/// missing (`active <= batch_len` — waiting for a straggler that does
/// not exist is pure latency).
#[must_use]
pub fn async_catchup_bound_us(window_us: u64, active: usize, batch_len: usize) -> u64 {
    if window_us == 0 || active <= batch_len {
        0
    } else {
        window_us
    }
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
        assert!(!merge_eligible(1, 500), "a lone writer never groups");
        assert!(merge_eligible(2, 1), "two writers + window on: eligible");
        assert!(
            !merge_eligible(50, 0),
            "window off keeps the 0201-only arm (mc50 policy decides)"
        );
    }

    #[test]
    fn rfc0217_group_window_bound_zero_when_nobody_missing() {
        assert_eq!(async_catchup_bound_us(500, 1, 1), 0, "alone: no wait");
        assert_eq!(
            async_catchup_bound_us(500, 3, 3),
            0,
            "everyone already queued: no wait"
        );
        assert_eq!(
            async_catchup_bound_us(500, 4, 1),
            500,
            "missing writers: flat window"
        );
        assert_eq!(
            async_catchup_bound_us(0, 4, 1),
            0,
            "window off: async groups keep draining instantly"
        );
    }

    #[test]
    fn rfc0217_group_window_as_is_twins_pin_the_old_shape() {
        assert!(!merge_eligible_as_is(2, 1_000));
        assert!(!merge_eligible_as_is(8, 1_000));
        assert_eq!(async_catchup_bound_us_as_is(1_000, 4, 1), 0);
    }
}
