// Verus proof of the SI snapshot GC gate (F168).
// Twin of `src/si_kernel.rs::snapshot_read_plan`. Not linked into production.
//
//   ./scripts/verus_si_read.sh

use vstd::prelude::*;

verus! {

/// Same variants as production `si_kernel::SnapshotRead`.
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum SnapshotRead {
    TooOld,
    Serve,
}

pub open spec fn snapshot_read_spec(snapshot: u64, watermark: u64) -> SnapshotRead {
    if watermark == 0 {
        SnapshotRead::Serve
    } else if watermark - 1 > snapshot {
        SnapshotRead::TooOld
    } else {
        SnapshotRead::Serve
    }
}

/// F168 kernel twin: reject a snapshot older than the GC floor
/// (`watermark - 1`); everything else serves.
pub fn snapshot_read_plan(snapshot: u64, watermark: u64) -> (r: SnapshotRead)
    ensures
        r == snapshot_read_spec(snapshot, watermark),
        r == (SnapshotRead::TooOld) ==> snapshot + 1 < watermark,
        r == (SnapshotRead::Serve) ==> watermark == 0 || snapshot >= watermark - 1,
        snapshot == u64::MAX ==> r == (SnapshotRead::Serve),
{
    if watermark == 0 {
        SnapshotRead::Serve
    } else if watermark - 1 > snapshot {
        SnapshotRead::TooOld
    } else {
        SnapshotRead::Serve
    }
}

/// AS-IS F168: always Serve — pruned history answers fabricated absence.
pub fn snapshot_read_plan_as_is(_snapshot: u64, _watermark: u64) -> (r: SnapshotRead)
    ensures
        r == (SnapshotRead::Serve),
{
    SnapshotRead::Serve
}

/// Teeth: in the F168 world (snapshot 1, watermark 7 — the repro), AS-IS
/// serves a snapshot whose floor (6) does not cover it.
proof fn lemma_as_is_serves_uncovered() {
    assert(snapshot_read_spec(1, 7) == SnapshotRead::TooOld); // 7 - 1 = 6 > 1
    assert(snapshot_read_spec(6, 7) == SnapshotRead::Serve);
}

/// Non-vacuity: floor / overflow / no-GC.
proof fn lemma_floor_boundary_serves() {
    assert(snapshot_read_spec(6, 7) == SnapshotRead::Serve);
    assert(snapshot_read_spec(5, 7) == SnapshotRead::TooOld);
    assert(snapshot_read_spec(0, 0) == SnapshotRead::Serve);
    assert(snapshot_read_spec(u64::MAX, u64::MAX) == SnapshotRead::Serve);
}

fn main() {}
}
