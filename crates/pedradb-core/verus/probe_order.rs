// Verus proof of the probe-order kernel
// (RFC-0164 P0.1 — newest-first candidate order for point probes).
//
// Source of truth for production remains `src/probe_order_kernel.rs`.
// This file is the machine-checked theorem: exec == spec, and on an
// equal-lo tie the fixed kernel probes the NEWEST covering candidate
// first while the AS-IS twin (the historical descending-lo walk, the
// db.rs `.rev()` loops) probes the OLDEST — the resurrected delete of
// findings/2026-09-04-reopen-delete-resurrected.
//
//   ./scripts/verus_probe_order.sh
//
// Do not link this into the production crate — it is a twin of the pure kernel.

use vstd::prelude::*;

verus! {

/// Closed-form spec — same body as `probe_order_kernel::
/// first_probe_on_equal_lo`.
pub open spec fn first_probe_on_equal_lo_spec(newer: usize, older: usize) -> usize {
    newer
}

/// AS-IS mutant: the historical stable-sort-by-lo + reverse walk probes
/// the OLDEST tied candidate first.
pub open spec fn first_probe_on_equal_lo_as_is_spec(newer: usize, older: usize) -> usize {
    older
}

/// Executable decision — must match `pedradb_core::probe_order_kernel::
/// first_probe_on_equal_lo` bit-for-bit.
#[verifier::when_used_as_spec(first_probe_on_equal_lo_spec)]
pub fn first_probe_on_equal_lo(newer: usize, _older: usize) -> (p: usize)
    ensures
        p == first_probe_on_equal_lo_spec(newer, _older),
        p == newer,
{
    newer
}

/// Executable mutant — must match `probe_order_kernel::
/// first_probe_on_equal_lo_as_is` bit-for-bit.
#[verifier::when_used_as_spec(first_probe_on_equal_lo_as_is_spec)]
pub fn first_probe_on_equal_lo_as_is(_newer: usize, older: usize) -> (p: usize)
    ensures
        p == first_probe_on_equal_lo_as_is_spec(_newer, older),
        p == older,
{
    older
}

/// P0.1 named lemma (the measured failure shape): two covering candidates
/// tied at lo — the fixed order probes the newer table (the tombstone)
/// first.
proof fn lemma_equal_lo_probes_newest_first(newer: usize, older: usize)
    requires
        newer != older,
    ensures
        first_probe_on_equal_lo(newer, older) == newer,
        first_probe_on_equal_lo(newer, older) != first_probe_on_equal_lo_as_is(newer, older),
{
}

/// Teeth: the AS-IS walk probes the OLDEST tied candidate first exactly
/// where the fixed kernel probes the newest — the older `Found` that
/// shadows the newer tombstone.
proof fn lemma_as_is_probes_oldest_on_equal_lo(newer: usize, older: usize)
    requires
        newer != older,
    ensures
        first_probe_on_equal_lo_as_is(newer, older) == older,
        first_probe_on_equal_lo(newer, older) != first_probe_on_equal_lo_as_is(newer, older),
{
}

// RFC-0164 P1.2 — strict-disjoint fast-path arm (model tier: u64 keys
// stand in for [u8] under the same total order — the scan_guard pattern;
// the kernel/mutant disagreement is at equality, which the scalar order
// preserves exactly).

pub open spec fn run_pairwise_disjoint_los_spec(los: Seq<u64>, his: Seq<u64>) -> bool {
    let n = if los.len() <= his.len() { los.len() } else { his.len() };
    n >= 2 && forall|i: int| 1 <= i < n ==> #[trigger] his[i - 1] < los[i]
}

/// AS-IS mutant spec: the non-strict arm `hi[i-1] <= lo[i]` — it arms the
/// single-candidate bisect path on the equal-lo tie.
pub open spec fn run_pairwise_disjoint_los_as_is_spec(los: Seq<u64>, his: Seq<u64>) -> bool {
    let n = if los.len() <= his.len() { los.len() } else { his.len() };
    n >= 2 && forall|i: int| 1 <= i < n ==> #[trigger] his[i - 1] <= los[i]
}

/// Executable decision — must match `probe_order_kernel::
/// run_pairwise_disjoint_los` on the model domain.
pub fn run_pairwise_disjoint_los(los: &[u64], his: &[u64]) -> (b: bool)
    ensures
        b == run_pairwise_disjoint_los_spec(los@, his@),
{
    let n = if los.len() <= his.len() { los.len() } else { his.len() };
    if n < 2 {
        return false;
    }
    let mut i: usize = 1;
    while i < n
        invariant
            1 <= i <= n,
            n >= 2,
            n <= los.len(),
            n <= his.len(),
            forall|j: int| 1 <= j < i ==> his[j - 1] < los[j],
        decreases n - i,
    {
        if his[i - 1] >= los[i] {
            assert(his[i as int - 1] >= los[i as int]);
            return false;
        }
        i += 1;
    }
    assert(n as int == if los@.len() <= his@.len() { los@.len() } else { his@.len() });
    true
}

/// Executable mutant — must match `probe_order_kernel::
/// run_pairwise_disjoint_los_as_is` on the model domain.
pub fn run_pairwise_disjoint_los_as_is(los: &[u64], his: &[u64]) -> (b: bool)
    ensures
        b == run_pairwise_disjoint_los_as_is_spec(los@, his@),
{
    let n = if los.len() <= his.len() { los.len() } else { his.len() };
    if n < 2 {
        return false;
    }
    let mut i: usize = 1;
    while i < n
        invariant
            1 <= i <= n,
            n >= 2,
            n <= los.len(),
            n <= his.len(),
            forall|j: int| 1 <= j < i ==> his[j - 1] <= los[j],
        decreases n - i,
    {
        if his[i - 1] > los[i] {
            assert(his[i as int - 1] > los[i as int]);
            return false;
        }
        i += 1;
    }
    assert(n as int == if los@.len() <= his@.len() { los@.len() } else { his@.len() });
    true
}

/// P1.2 named lemma (the measured shape): two tables tied at lo = hi —
/// the strict arm keeps the bisect path OFF (the newest-first walk owns
/// the tie), while the AS-IS non-strict arm takes it.
proof fn lemma_equal_lo_keeps_arm_off() {
    let los = Seq::new(2, |_i: int| 7u64);
    let his = Seq::new(2, |_i: int| 7u64);
    assert(los.len() == 2int && his.len() == 2int);
    assert(los[1int] == 7u64 && his[0int] == 7u64);
    assert(his[1int - 1] == 7u64 && los[1int] == 7u64);
    assert(!(his[1int - 1] < los[1int]));
    assert(run_pairwise_disjoint_los_spec(los, his) == false);
    assert(his[0int] <= los[1int]);
    assert(run_pairwise_disjoint_los_as_is_spec(los, his) == true);
    assert(run_pairwise_disjoint_los_spec(los, his) != run_pairwise_disjoint_los_as_is_spec(los, his));
}

/// Teeth: a real gap (hi[0] < lo[1] strictly) arms both rules — the guard
/// is not vacuous.
proof fn lemma_strict_gap_arms_both() {
    let los = Seq::new(2, |i: int| if i == 0 { 5u64 } else { 9u64 });
    let his = Seq::new(2, |i: int| if i == 0 { 4u64 } else { 8u64 });
    assert(his[0int] == 4u64 && los[1int] == 9u64);
    assert(his[0int] < los[1int]);
    assert(run_pairwise_disjoint_los_spec(los, his) == true);
    assert(run_pairwise_disjoint_los_as_is_spec(los, his) == true);
}

} // verus!
