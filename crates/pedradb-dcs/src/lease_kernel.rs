//! Pure lease decisions (Beyond-style kernel, F7 / F56).
//!
//! **Single artifact (pair `lease`):** this file is what `rustc` links
//! *and* what Verus proves (`cfg(verus_keep_ghost)`). Pairs `lease_table`
//! and `lease_next_id` still have a twin-cópia until their turn.
//!
//!   ./scripts/verus_lease_live.sh
//!
//! # Contract
//!
//! - **No I/O, no clock object, no RNG.** Time enters as a `u64` or a
//!   precomputed `bool` (caller already compared `now` to expiry).
//! - Production [`crate::Dcs::get`] / [`crate::command::dcs_get_at`] call
//!   these helpers; Verus/Lean must use **this same module**.
//!
//! The rustc bodies stay byte-stable so non-`single_artifact` twins still
//! token-match. Verus proofs sit in the `cfg(verus_keep_ghost)` block
//! above them (last-wins for lint is the rustc body).
//!
//! Spec page: `determinismo/pedradb-dst/formal/F7-F56-lease.md`.

#![forbid(unsafe_code)]

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn lease_live_spec(lease: u64, now_ms: u64) -> bool {
    lease == 0 || now_ms < lease
}

pub fn lease_live(lease: u64, now_ms: u64) -> (d: bool)
    ensures
        d == lease_live_spec(lease, now_ms),
        d == (lease == 0 || now_ms < lease),
{
    lease == 0 || now_ms < lease
}

pub open spec fn lease_live_as_is_spec(_lease: u64, _now_ms: u64) -> bool {
    true
}

pub fn lease_live_as_is(_lease: u64, _now_ms: u64) -> (d: bool)
    ensures
        d,
{
    true
}

/// F56: death is monotone in the clock (reopen must not reset `now_ms`).
/// IronKV host program is the Verus term (verified-ironkv; VeruSAGE IR):
/// the same price here — rustc links this file. A host that rewound its
/// view would reanimate a dead lease; we refuse that.
proof fn lemma_expired_stays_dead(lease: u64, now_ms: u64, now2: u64)
    requires
        !lease_live_spec(lease, now_ms),
        now2 >= now_ms,
    ensures
        !lease_live_spec(lease, now2),
{
}

/// F56 witness: resetting the clock to 0 reanimates a positive deadline.
proof fn lemma_clock_reset_reanimates(lease: u64)
    requires
        lease > 0,
    ensures
        !lease_live_spec(lease, lease),
        lease_live_spec(lease, 0),
{
}

/// AS-IS dente: expired deadline stays live (the F56 hole).
proof fn lemma_as_is_keeps_expired_live(lease: u64)
    requires
        lease > 0,
    ensures
        !lease_live_spec(lease, lease),
        lease_live_as_is_spec(lease, lease),
{
}

pub open spec fn lease_table_expired_spec(table_hit: Option<bool>) -> bool {
    match table_hit {
        None => true,
        Some(e) => e,
    }
}

pub fn lease_table_expired(table_hit: Option<bool>) -> (d: bool)
    ensures
        d == lease_table_expired_spec(table_hit),
        table_hit.is_none() ==> d,
{
    match table_hit {
        None => true,
        Some(e) => e,
    }
}

pub open spec fn lease_table_expired_as_is(table_hit: Option<bool>) -> bool {
    match table_hit {
        None => false,
        Some(e) => e,
    }
}

proof fn lemma_unknown_as_is_is_immortal()
    ensures
        lease_table_expired_spec(None),
        !lease_table_expired_as_is(None),
{
}

pub open spec fn sat_add1(x: u64) -> u64 {
    if x == u64::MAX {
        x
    } else {
        (x + 1) as u64
    }
}

pub open spec fn next_lease_id_spec(max_seen: u64) -> u64 {
    let s = sat_add1(max_seen);
    if s > 1 {
        s
    } else {
        1
    }
}

pub fn next_lease_id_after(max_seen: u64) -> (n: u64)
    ensures
        n == next_lease_id_spec(max_seen),
        max_seen < u64::MAX ==> n > max_seen,
        n >= 1,
{
    let s = if max_seen == u64::MAX {
        max_seen
    } else {
        max_seen + 1
    };
    if s > 1 {
        s
    } else {
        1
    }
}

proof fn lemma_as_is_reuse_can_collide(max_seen: u64)
    requires
        max_seen >= 1,
    ensures
        ({
            let as_is: u64 = 1;
            as_is <= max_seen
        }),
{
}

} // verus!

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
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn lease_live(lease: u64, now_ms: u64) -> bool {
    lease == 0 || now_ms < lease
}

/// AS-IS: every lease is immortal (the F56 hole).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn lease_live_as_is(_lease: u64, _now_ms: u64) -> bool {
    true
}

/// Process-local table (F7): unknown id is **expired** (fail-safe).
///
/// `table_hit = None` — id not in this process's map (restart / never granted).
/// `table_hit = Some(clock_expired)` — caller already evaluated `now >= expiry`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn lease_table_expired(table_hit: Option<bool>) -> bool {
    table_hit.unwrap_or(true)
}

/// Next grant id after scanning disk max (F7 reanimation).
///
/// Never reuse an id that still appears in meta (`max_seen`). Saturating:
/// at `u64::MAX` we stay there (documented residual).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn next_lease_id_after(max_seen_on_disk: u64) -> u64 {
    max_seen_on_disk.saturating_add(1).max(1)
}

/// AS-IS F7 immortal: unknown id treated as live.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn lease_table_expired_as_is(table_hit: Option<bool>) -> bool {
    table_hit.unwrap_or_default()
}

/// AS-IS F7 reanimation: always restart the counter at 1.
#[cfg(not(verus_keep_ghost))]
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
