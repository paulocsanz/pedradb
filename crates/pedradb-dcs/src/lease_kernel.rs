//! Pure lease decisions (Beyond-style kernel, F7 / F56).
//!
//! # Contract
//!
//! - **No I/O, no clock object, no RNG.** Time enters as a `u64` or a
//!   precomputed `bool` (caller already compared `now` to expiry).
//! - Production [`crate::Dcs::get`] / [`crate::command::dcs_get_at`] call
//!   these helpers; Verus/Lean must use **this same module**.
//!
//! Spec page: `determinismo/pedradb-dst/formal/F7-F56-lease.md`.

#![forbid(unsafe_code)]

/// Absolute-deadline live check (store / replicated DCS path, F56).
///
/// `lease == 0` is immortal. Otherwise live iff `now_ms < lease`.
///
/// # Post-condition
///
/// ```text
/// ensures live == (lease == 0 || now_ms < lease)
/// ensures !live && now2 >= now_ms  ==>  !lease_live(lease, now2)
/// ```
#[must_use]
pub fn lease_live(lease: u64, now_ms: u64) -> bool {
    lease == 0 || now_ms < lease
}

/// AS-IS: every lease is immortal (the F56 hole).
#[must_use]
pub fn lease_live_as_is(_lease: u64, _now_ms: u64) -> bool {
    true
}

/// Process-local table (F7): unknown id is **expired** (fail-safe).
///
/// `table_hit = None` — id not in this process's map (restart / never granted).
/// `table_hit = Some(clock_expired)` — caller already evaluated `now >= expiry`.
#[must_use]
pub fn lease_table_expired(table_hit: Option<bool>) -> bool {
    table_hit.unwrap_or(true)
}

/// Next grant id after scanning disk max (F7 reanimation).
///
/// Never reuse an id that still appears in meta (`max_seen`). Saturating:
/// at `u64::MAX` we stay there (documented residual).
#[must_use]
pub fn next_lease_id_after(max_seen_on_disk: u64) -> u64 {
    max_seen_on_disk.saturating_add(1).max(1)
}

/// AS-IS F7 immortal: unknown id treated as live.
#[must_use]
pub fn lease_table_expired_as_is(table_hit: Option<bool>) -> bool {
    table_hit.unwrap_or_default()
}

/// AS-IS F7 reanimation: always restart the counter at 1.
#[must_use]
pub fn next_lease_id_as_is(_max_seen_on_disk: u64) -> u64 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_lease_is_immortal() {
        assert!(lease_live(0, 0));
        assert!(lease_live(0, u64::MAX));
    }

    #[test]
    fn deadline_exclusive() {
        assert!(lease_live(100, 99));
        assert!(!lease_live(100, 100));
        assert!(!lease_live(100, 101));
    }

    #[test]
    fn monotone_clock_preserves_death() {
        let lease = 50u64;
        let now = 50u64;
        assert!(!lease_live(lease, now));
        for now2 in [now, now + 1, u64::MAX] {
            assert!(
                !lease_live(lease, now2),
                "dead at {now} must stay dead at {now2}"
            );
        }
    }

    #[test]
    fn as_is_clock_reset_reanimates() {
        // F56: expired at now=100, reopen resets RAM clock to 0.
        assert!(!lease_live(100, 100));
        assert!(lease_live(100, 0), "clock reset must be the F56 witness");
    }

    #[test]
    fn unknown_id_is_expired() {
        assert!(lease_table_expired(None));
        assert!(!lease_table_expired(Some(false)));
        assert!(lease_table_expired(Some(true)));
    }

    #[test]
    fn as_is_unknown_is_immortal() {
        assert!(!lease_table_expired_as_is(None));
        assert_ne!(lease_table_expired(None), lease_table_expired_as_is(None));
    }

    #[test]
    fn next_id_never_reuses_disk_max() {
        assert_eq!(next_lease_id_after(0), 1);
        assert_eq!(next_lease_id_after(7), 8);
        assert_eq!(next_lease_id_after(u64::MAX), u64::MAX);
    }

    #[test]
    fn as_is_reuse_reanimates() {
        assert_eq!(next_lease_id_as_is(7), 1);
        assert!(next_lease_id_as_is(7) <= 7);
        assert!(next_lease_id_after(7) > 7);
    }

    #[test]
    fn theorem_lease_live_on_finite_domain() {
        const B: u64 = 5;
        let mut n = 0u64;
        for lease in 0..B {
            for now in 0..B {
                let live = lease_live(lease, now);
                assert_eq!(live, lease == 0 || now < lease);
                if !live {
                    for now2 in now..B {
                        assert!(!lease_live(lease, now2));
                    }
                }
                n += 1;
            }
        }
        assert_eq!(n, B * B);
    }

    #[test]
    fn theorem_table_and_next_on_bool_domain() {
        assert!(lease_table_expired(None));
        for e in [false, true] {
            assert_eq!(lease_table_expired(Some(e)), e);
            assert_eq!(lease_table_expired_as_is(Some(e)), e);
        }
        for max in 0u64..8 {
            let n = next_lease_id_after(max);
            if max < u64::MAX {
                assert!(n > max);
            }
            assert!(n >= 1);
            assert_eq!(next_lease_id_as_is(max), 1);
        }
    }

    #[test]
    fn lease_live_on_live_expiry_is_not_ok() {
        assert!(!lease_live(10, 10));
        assert!(
            lease_live_as_is(10, 10),
            "AS-IS dente: expired lease stays live"
        );
    }
}
