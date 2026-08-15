// Verus proof of the SI snapshot GC gate (F168).
// Twin of `src/si_kernel.rs::snapshot_read_plan`. Not linked into production.
//
//   ./scripts/verus_si_read.sh

use vstd::prelude::*;

verus! {

pub open spec fn snapshot_read_spec(snapshot: u64, watermark: u64) -> bool {
    // true = Serve, false = TooOld
    if watermark == 0 {
        true
    } else {
        snapshot + 1 >= watermark
    }
}

/// F168 kernel twin: reject a snapshot older than the GC floor
/// (`watermark - 1`); everything else serves.
pub fn snapshot_read_plan(snapshot: u64, watermark: u64) -> (serve: bool)
    ensures
        serve == snapshot_read_spec(snapshot, watermark),
        // TooOld is sound: the floor cannot cover the snapshot.
        !serve ==> snapshot + 1 < watermark,
        // Serve is never fabricated: some history entry (the floor at
        // `watermark - 1`, or the gen-0 anchor when no GC ran) covers it.
        serve ==> watermark == 0 || snapshot >= watermark - 1,
        // Never overflow-rejects fresh reads at the top of the space.
        snapshot == u64::MAX ==> serve,
{
    if watermark == 0 {
        true
    } else if watermark - 1 > snapshot {
        false
    } else {
        true
    }
}

/// AS-IS F168: always "serve" — pruned history answers fabricated absence.
pub fn snapshot_read_plan_as_is(snapshot: u64, watermark: u64) -> (serve: bool)
    ensures serve == true,
{
    let _ = snapshot;
    let _ = watermark;
    true
}

/// Teeth: in the F168 world (snapshot 1, watermark 7 — the repro), AS-IS
/// serves a snapshot whose floor (6) does not cover it.
proof fn lemma_as_is_serves_uncovered() {
    // AS-IS serves unconditionally (ensures serve == true); the guarded spec
    // rejects the repro world.
    let guarded_spec = snapshot_read_spec(1, 7);
    assert(guarded_spec == false); // 1 + 1 < 7
}

/// Non-vacuity: the floor semantics — snapshot exactly at `watermark - 1`
/// serves (the floor entry is readable there).
proof fn lemma_floor_boundary_serves() {
    assert(snapshot_read_spec(6, 7)); // 6 + 1 >= 7
    assert(!snapshot_read_spec(5, 7)); // 5 + 1 < 7
    assert(snapshot_read_spec(0, 0)); // no GC ever ran
    assert(snapshot_read_spec(u64::MAX, u64::MAX)); // no overflow reject
}

fn main() {}
}
