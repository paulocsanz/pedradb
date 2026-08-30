// Verus proof of pure lease decisions (RFC-0002 P6 / F7 / F56).
//
// Source of truth remains `src/lease_kernel.rs`. Not linked into production.
//
//   ./scripts/verus_lease_live.sh

use vstd::prelude::*;

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

/// F56: death is monotone in the clock (reopen must not reset `now_ms`).
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
