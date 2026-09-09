//! Leveled compaction scheduling (pure selection kernel).
//! kernel: leveling — enrolled in residuals.json glue.kernel_paths; the
//! suffix-less enrollment tooth requires this marker (2026-08-31, findings/
//! 2026-08-31-leveling-kernel-unenrolled).
//!
//! **Single artifact (pairs `leveling`, `leveling_pick`):** this file is
//! what `rustc` links *and* what Verus proves (`cfg(verus_keep_ghost)`).
//! Pair `leveling_pushdown` still has a twin-cópia until its turn.
//!
//!   ./scripts/verus_leveling.sh
//!   ./scripts/verus_leveling_pick.sh
//!
//! Policy: L0→L1 jobs absorb the L1 slice that overlaps the selected L0s, and
//! each level `n ≥ 1` is capped at [`level_target_bytes`]. When a level is over
//! target, one pushdown job moves its **oldest** file into level `n+1` together
//! with the level-`n+1` files it overlaps. Both job shapes are bounded: they
//! never span more than the selected inputs plus one bounded overlap slice.
//!
//! # Why the overlap slice is safe only on a disjoint level
//!
//! A uniform-random L0 run spans the whole key space, so its overlap closure
//! over a *stacked* (mutually overlapping) L1 is every L1 file — a whole-level
//! rewrite per job. Over a *disjoint* L1 the files intersecting the L0 hull
//! are exactly the ones the output replaces, one pass, no cascade: L0 files
//! are never extended by L1 files outside the hull.
//!
//! # Why levels stay disjoint after a job
//!
//! The output of a job covers the hull of its inputs. Any file whose range
//! lies inside that hull necessarily overlaps the hull — hence overlaps the
//! source or one of the already-selected files — and would have been selected.
//! So every unselected file at the target level lies strictly outside the
//! hull, and the new disjoint chunks (split at user-key boundaries) plus the
//! unselected files form a disjoint level again. Inductively the invariant
//! holds from the first job on an empty level.
//!
//! The rustc bodies stay byte-stable so non-`single_artifact` twins still
//! token-match. Verus proofs sit in the `cfg(verus_keep_ghost)` block
//! above them (last-wins for lint is the rustc body).

#[cfg(verus_keep_ghost)]
use vstd::arithmetic::mul::*;
#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

broadcast use vstd::arithmetic::mul::lemma_mul_is_commutative, vstd::arithmetic::mul::lemma_mul_inequality;

pub open spec fn ten_pow(exp: int) -> int
    decreases exp,
{
    if exp <= 0 { 1int } else { 10int * ten_pow(exp - 1) }
}

/// Spec domain is `int`; `level == 0` has no target, the ladder is
/// `l1_target * LEVEL_FANOUT^min(level-1, 18)` with `LEVEL_FANOUT == 10`.
pub open spec fn level_target_bytes_spec(level: int, l1_target: int) -> int {
    if level <= 0 {
        0int
    } else {
        let exp = if level >= 19 { 18int } else { level - 1 };
        l1_target * ten_pow(exp)
    }
}

/// Mirrors rustc `level_target_bytes` on every input where the saturating
/// arms do not engage (the precondition states exactly that).
pub fn level_target_bytes(level: u32, l1_target: u64) -> (t: u64)
    requires
        level_target_bytes_spec(level as int, l1_target as int) <= 0xffff_ffff_ffff_ffffint,
    ensures
        t as int == level_target_bytes_spec(level as int, l1_target as int),
        level == 0 ==> t == 0,
        level >= 1 ==> t >= l1_target,
{
    if level == 0 {
        0
    } else if l1_target == 0 {
        0
    } else {
        let exp: u32 = if level >= 19 { 18 } else { level - 1 };
        let p = ten_pow_exec(exp, l1_target);
        assert(ten_pow(exp as int) >= 1) by {
            lemma_ten_pow_pos(exp as int);
        };
        assert(1int * l1_target as int <= ten_pow(exp as int) * l1_target as int);
        l1_target * p
    }
}

fn ten_pow_exec(exp: u32, cap: u64) -> (p: u64)
    requires
        cap >= 1,
        cap as int * ten_pow(exp as int) <= 0xffff_ffff_ffff_ffffint,
    ensures
        p as int == ten_pow(exp as int),
    decreases exp,
{
    if exp == 0 {
        1
    } else {
        assert(ten_pow((exp - 1) as int) <= ten_pow(exp as int)) by {
            lemma_ten_pow_monotone((exp - 1) as int, exp as int);
        };
        assert(ten_pow((exp - 1) as int) * cap as int <= ten_pow(exp as int) * cap as int);
        assert(ten_pow((exp - 1) as int) * cap as int <= 0xffff_ffff_ffff_ffffint);
        assert(1int * ten_pow(exp as int) <= cap as int * ten_pow(exp as int));
        assert(ten_pow(exp as int) <= 0xffff_ffff_ffff_ffffint);
        let child = ten_pow_exec((exp - 1) as u32, cap);
        child * 10
    }
}

proof fn lemma_ten_pow_pos(exp: int)
    requires
        exp >= 0,
    ensures
        ten_pow(exp) >= 1,
    decreases exp,
{
    if exp == 0 {
    } else {
        lemma_ten_pow_pos(exp - 1);
    }
}

proof fn lemma_ten_pow_monotone(a: int, b: int)
    requires
        0 <= a,
        a <= b,
    ensures
        ten_pow(a) <= ten_pow(b),
    decreases b - a,
{
    if a == b {
    } else {
        lemma_ten_pow_monotone(a, b - 1);
        lemma_ten_pow_pos(a);
    }
}

/// Named lemma (ladder / RocksDB Target_Size(Ln+1) = Target_Size(Ln)*10):
/// targets never shrink as the level grows — wrapping as-is does.
proof fn lemma_targets_monotone(a: int, b: int, l1_target: int)
    requires
        1 <= a,
        a <= b,
        l1_target >= 1,
    ensures
        level_target_bytes_spec(a, l1_target) <= level_target_bytes_spec(b, l1_target),
{
    let ea = if a >= 19 { 18int } else { a - 1 };
    let eb = if b >= 19 { 18int } else { b - 1 };
    assert(ea <= eb) by {
        if a >= 19 {
            assert(eb <= 18);
        } else if b >= 19 {
            assert(a - 1 <= 18);
        }
    };
    lemma_ten_pow_monotone(ea, eb);
    assert(ten_pow(ea) * l1_target <= ten_pow(eb) * l1_target);
}

} // verus!

#[cfg(verus_keep_ghost)]
verus! {

/// Stand-in domain for pair `leveling_pick` (keys are u64; production keys
/// are Vec<u8>). `overlapping_prefix` is GetOverlappingInputs of the hull:
/// files outside the hull never enter the slice (`lemma_prefix_excludes`).
/// rustc `pick_l0_to_l1` / `pick_pushdown` stay last-wins below
/// (`cfg(not(verus_keep_ghost))`).
#[derive(Copy, Clone)]
pub struct MFile {
    pub idx: usize,
    pub lo: u64,
    pub hi: u64,
}

/// Interval overlap on the integer domain (u64 fields lift to int).
#[inline]
pub open spec fn overlaps_spec(flo: int, fhi: int, hull_lo: int, hull_hi: int) -> bool {
    flo <= hull_hi && fhi >= hull_lo
}

fn overlaps_exec(flo: u64, fhi: u64, hull_lo: u64, hull_hi: u64) -> (b: bool)
    ensures
        b <==> overlaps_spec(flo as int, fhi as int, hull_lo as int, hull_hi as int),
{
    flo <= hull_hi && fhi >= hull_lo
}

/// Pairwise disjointness of a level view (model of `leveling::is_disjoint`
/// once sorted; equal boundary keys count as overlap, like the kernel).
pub open spec fn disjoint_spec(dst: Seq<MFile>) -> bool {
    forall |i: int, j: int| #![auto]
        0 <= i && i < dst.len() && 0 <= j && j < dst.len() && i < j ==>
            (dst[i].hi < dst[j].lo || dst[j].hi < dst[i].lo)
}

/// Spec-side filter definition: the files among `dst[0..j]` overlapping
/// `[hlo, hhi]`, in order. The exec loops below mirror this recursion
/// step-for-step, so the slice contract is stated as an equality against
/// this function (the model's definition of "exactly the overlapping
/// files").
pub open spec fn overlapping_prefix(dst: Seq<MFile>, j: int, hlo: u64, hhi: u64) -> Seq<MFile>
    decreases j,
{
    if j <= 0 {
        Seq::<MFile>::empty()
    } else {
        let rest = overlapping_prefix(dst, j - 1, hlo, hhi);
        if overlaps_spec(dst[j - 1].lo as int, dst[j - 1].hi as int, hlo as int, hhi as int) {
            rest.push(dst[j - 1])
        } else {
            rest
        }
    }
}

/// Production entry (RFC-0170 close): same decision as `pick_l0_to_l1_model`.
pub fn pick_l0_to_l1(
    l0: &[MFile],
    l1: &[MFile],
    max_l0: usize,
) -> (r: Option<(Vec<usize>, u64, u64, Vec<MFile>)>)
    requires
        forall |i: int, j: int| 0 <= i && i < l1.len() && 0 <= j && j < l1.len() && i != j
            ==> l1[i].idx != l1[j].idx,
    ensures
        (l0.len() == 0 || max_l0 == 0) <==> (r is None),
        r.is_some() ==> {
            let (sel, hlo, hhi, slice) = r.unwrap();
            &&& sel.len() == if l0.len() < max_l0 { l0.len() } else { max_l0 }
            &&& (forall |k: int| 0 <= k && k < sel.len() ==> sel@[k] == l0@[k].idx)
            &&& (exists |k: int| 0 <= k && k < sel.len() && hlo == l0@[k].lo
                && (forall |m: int| 0 <= m && m < sel.len() ==> hlo <= l0@[m].lo))
            &&& (exists |k: int| 0 <= k && k < sel.len() && hhi == l0@[k].hi
                && (forall |m: int| 0 <= m && m < sel.len() ==> hhi >= l0@[m].hi))
            &&& slice@ == overlapping_prefix(l1@, l1@.len() as int, hlo, hhi)
        },
{
    pick_l0_to_l1_model(l0, l1, max_l0)
}

/// Twin atom: the L0→L1 job. `None` exactly on empty L0 or zero cap;
/// otherwise the L0 side is exactly the first `max_l0` files, the returned
/// hull is an attained min-lo/max-hi of that selection, and the L1 slice
/// is EXACTLY the dst files overlapping that hull.
pub fn pick_l0_to_l1_model(
    l0: &[MFile],
    l1: &[MFile],
    max_l0: usize,
) -> (r: Option<(Vec<usize>, u64, u64, Vec<MFile>)>)
    requires
        forall |i: int, j: int| 0 <= i && i < l1.len() && 0 <= j && j < l1.len() && i != j
            ==> l1[i].idx != l1[j].idx,
    ensures
        (l0.len() == 0 || max_l0 == 0) <==> (r is None),
        r.is_some() ==> {
            let (sel, hlo, hhi, slice) = r.unwrap();
            &&& sel.len() == if l0.len() < max_l0 { l0.len() } else { max_l0 }
            &&& (forall |k: int| 0 <= k && k < sel.len() ==> sel@[k] == l0@[k].idx)
            &&& (exists |k: int| 0 <= k && k < sel.len() && hlo == l0@[k].lo
                && (forall |m: int| 0 <= m && m < sel.len() ==> hlo <= l0@[m].lo))
            &&& (exists |k: int| 0 <= k && k < sel.len() && hhi == l0@[k].hi
                && (forall |m: int| 0 <= m && m < sel.len() ==> hhi >= l0@[m].hi))
            &&& slice@ == overlapping_prefix(l1@, l1@.len() as int, hlo, hhi)
        },
{
    if l0.len() == 0 || max_l0 == 0 {
        None
    } else {
        broadcast use vstd::seq::group_seq_axioms;
        let n = if l0.len() < max_l0 { l0.len() } else { max_l0 };
        let mut hull_lo: u64 = l0[0].lo;
        let mut hull_hi: u64 = l0[0].hi;
        let mut i: usize = 1;
        while i < n
            invariant
                1 <= i,
                i <= n,
                n <= l0.len(),
                n >= 1,
                (forall |m: int| 0 <= m && m < i ==> hull_lo <= l0@[m].lo),
                (forall |m: int| 0 <= m && m < i ==> hull_hi >= l0@[m].hi),
                (exists |m: int| 0 <= m && m < i && hull_lo == l0@[m].lo),
                (exists |m: int| 0 <= m && m < i && hull_hi == l0@[m].hi),
            decreases n - i,
        {
            if l0[i].lo < hull_lo {
                hull_lo = l0[i].lo;
            }
            if l0[i].hi > hull_hi {
                hull_hi = l0[i].hi;
            }
            i = i + 1;
        }
        let mut sel: Vec<usize> = Vec::new();
        let mut k: usize = 0;
        while k < n
            invariant
                k <= n,
                n <= l0.len(),
                n >= 1,
                sel.len() == k,
                forall |m: int| 0 <= m && m < k ==> sel@[m] == l0@[m].idx,
            decreases n - k,
        {
            sel.push(l0[k].idx);
            k = k + 1;
        }
        let mut slice: Vec<MFile> = Vec::new();
        let mut j: usize = 0;
        while j < l1.len()
            invariant
                j <= l1.len(),
                slice@ == overlapping_prefix(l1@, j as int, hull_lo, hull_hi),
            decreases l1.len() - j,
        {
            if overlaps_exec(l1[j].lo, l1[j].hi, hull_lo, hull_hi) {
                slice.push(l1[j]);
            }
            j = j + 1;
        }
        Some((sel, hull_lo, hull_hi, slice))
    }
}

/// Non-overlapping dst files never enter a prefix: with distinct idxs, a
/// file outside the hull cannot appear at any position of the slice
/// definition.
proof fn lemma_prefix_excludes(
    dst: Seq<MFile>,
    j: int,
    hlo: u64,
    hhi: u64,
    p: int,
)
    requires
        0 <= j,
        j <= dst.len(),
        0 <= p && p < dst.len(),
        !overlaps_spec(dst[p].lo as int, dst[p].hi as int, hlo as int, hhi as int),
        forall |i: int, k: int| 0 <= i && i < dst.len() && 0 <= k && k < dst.len() && i != k
            ==> dst[i].idx != dst[k].idx,
    ensures
        forall |q: int| 0 <= q && q < overlapping_prefix(dst, j, hlo, hhi).len()
            ==> overlapping_prefix(dst, j, hlo, hhi)[q].idx != dst[p].idx,
    decreases j,
{
    broadcast use vstd::seq::group_seq_axioms;
    if j <= 0 {
    } else {
        lemma_prefix_excludes(dst, j - 1, hlo, hhi, p);
        let pre = overlapping_prefix(dst, j - 1, hlo, hhi);
        if overlaps_spec(dst[j - 1].lo as int, dst[j - 1].hi as int, hlo as int, hhi as int) {
            // dst[j-1] overlaps the hull and dst[p] does not, so j-1 != p,
            // and distinct idxs keep dst[p]'s idx out of the pushed element.
            assert(dst[j - 1].idx != dst[p].idx);
            let out = pre.push(dst[j - 1]);
            assert(overlapping_prefix(dst, j, hlo, hhi) == out);
            assert forall |q: int| 0 <= q && q < out.len() implies out[q].idx != dst[p].idx by {
                if q < pre.len() {
                    assert(out[q] == pre[q]);
                } else {
                    assert(q == pre.len());
                    assert(out[q] == dst[j - 1]);
                }
            }
        }
    }
}

/// Production entry (RFC-0170 close): same decision as `pick_pushdown_model`.
pub fn pick_pushdown(src: &[MFile], dst: &[MFile]) -> (r: Option<(usize, Vec<MFile>)>)
    requires
        forall |i: int, j: int| 0 <= i && i < dst.len() && 0 <= j && j < dst.len() && i != j
            ==> dst[i].idx != dst[j].idx,
    ensures
        (src.len() == 0 || !disjoint_spec(dst@)) <==> (r is None),
        r.is_some() ==> {
            let (s, slice) = r.unwrap();
            &&& s == src@[0].idx
            &&& slice@ == overlapping_prefix(dst@, dst@.len() as int, src@[0].lo, src@[0].hi)
        },
{
    pick_pushdown_model(src, dst)
}

/// Twin atom: the pushdown. `None` exactly on empty source or a
/// non-disjoint destination; otherwise the slice is EXACTLY the
/// destination files overlapping the oldest source file.
pub fn pick_pushdown_model(src: &[MFile], dst: &[MFile]) -> (r: Option<(usize, Vec<MFile>)>)
    requires
        forall |i: int, j: int| 0 <= i && i < dst.len() && 0 <= j && j < dst.len() && i != j
            ==> dst[i].idx != dst[j].idx,
    ensures
        (src.len() == 0 || !disjoint_spec(dst@)) <==> (r is None),
        r.is_some() ==> {
            let (s, slice) = r.unwrap();
            &&& s == src@[0].idx
            &&& slice@ == overlapping_prefix(dst@, dst@.len() as int, src@[0].lo, src@[0].hi)
        },
{
    if src.len() == 0 {
        None
    } else if !is_disjoint_model(dst) {
        None
    } else {
        broadcast use vstd::seq::group_seq_axioms;
        let s0_lo = src[0].lo;
        let s0_hi = src[0].hi;
        let mut slice: Vec<MFile> = Vec::new();
        let mut j: usize = 0;
        while j < dst.len()
            invariant
                j <= dst.len(),
                slice@ == overlapping_prefix(dst@, j as int, s0_lo, s0_hi),
            decreases dst.len() - j,
        {
            if overlaps_exec(dst[j].lo, dst[j].hi, s0_lo, s0_hi) {
                slice.push(dst[j]);
            }
            j = j + 1;
        }
        assert(src@[0].lo == s0_lo);
        assert(src@[0].hi == s0_hi);
        Some((src[0].idx, slice))
    }
}

/// Exec model of the kernel's `is_disjoint` (sort + adjacent windows),
/// checked pairwise here — O(n²) stand-in, same semantics as the spec.
pub fn is_disjoint_model(files: &[MFile]) -> (d: bool)
    ensures
        d <==> disjoint_spec(files@),
{
    let mut i: usize = 0;
    while i < files.len()
        invariant
            0 <= i <= files.len(),
            forall |a: int, b: int| 0 <= a && a < i && 0 <= b && b < files.len() && a != b
                ==> files@[a].hi < files@[b].lo || files@[b].hi < files@[a].lo,
        decreases files.len() - i,
    {
        let mut j: usize = 0;
        while j < files.len()
            invariant
                0 <= j <= files.len(),
                i < files.len(),
                forall |b: int| 0 <= b && b < j && b != i as int
                    ==> files@[i as int].hi < files@[b].lo || files@[b].hi < files@[i as int].lo,
                forall |a: int, b: int| 0 <= a && a < i && 0 <= b && b < files.len() && a != b
                    ==> files@[a].hi < files@[b].lo || files@[b].hi < files@[a].lo,
            decreases files.len() - j,
        {
            if j != i {
                if !(files[j].hi < files[i].lo || files[i].hi < files[j].lo) {
                    assert(!disjoint_spec(files@)) by {
                        assert(files@[i as int].hi >= files@[j as int].lo);
                        assert(files@[j as int].hi >= files@[i as int].lo);
                        if i < j {
                            assert(!(files@[i as int].hi < files@[j as int].lo || files@[j as int].hi < files@[i as int].lo));
                        } else {
                            assert(!(files@[j as int].hi < files@[i as int].lo || files@[i as int].hi < files@[j as int].lo));
                        }
                    };
                    return false;
                }
            }
            j = j + 1;
        }
        i = i + 1;
    }
    assert(disjoint_spec(files@)) by {
        forall |a: int, b: int| #![auto]
            0 <= a && a < files.len() && 0 <= b && b < files.len() && a != b
                ==> files@[a].hi < files@[b].lo || files@[b].hi < files@[a].lo;
    };
    true
}

// ---------------------------------------------------------------------------
// AS-IS mutants and their divergence witnesses. Each mutant carries its
// TRUE as-is contract (what the broken code does), and each divergence fn
// pins a concrete state where the model's safety property and the as-is
// result part ways.
// ---------------------------------------------------------------------------

/// AS-IS mutant 1 (absorb whole level): the L1 side ignores the hull and
/// returns every dst file — the "compaction never converges" shape, where
/// a freshly split level is reabsorbed in full every L0→L1 cycle.
pub fn pick_l0_to_l1_as_is_whole_level(l0: &[MFile], l1: &[MFile]) -> (r: (Vec<usize>, Vec<MFile>))
    ensures
        r.0.len() == l0.len(),
        forall |k: int| 0 <= k && k < l0.len() ==> r.0@[k] == l0@[k].idx,
        r.1.len() == l1.len(),
        forall |j: int| 0 <= j && j < l1.len() ==> r.1@[j].idx == l1@[j].idx,
{
    let mut sel: Vec<usize> = Vec::new();
    let mut k: usize = 0;
    while k < l0.len()
        invariant
            k <= l0.len(),
            sel.len() == k,
            forall |m: int| 0 <= m && m < k ==> sel@[m] == l0@[m].idx,
        decreases l0.len() - k,
    {
        sel.push(l0[k].idx);
        k = k + 1;
    }
    let mut slice: Vec<MFile> = Vec::new();
    let mut j: usize = 0;
    while j < l1.len()
        invariant
            j <= l1.len(),
            slice.len() == j,
            forall |m: int| 0 <= m && m < j ==> slice@[m].idx == l1@[m].idx,
        decreases l1.len() - j,
    {
        slice.push(l1[j]);
        j = j + 1;
    }
    (sel, slice)
}

/// AS-IS mutant 2 (ignore cap): the L0 side ignores `max_l0` and selects
/// the whole level — an unbounded job the cap exists to prevent.
pub fn pick_l0_to_l1_as_is_uncapped(l0: &[MFile], _max_l0: usize) -> (sel: Vec<usize>)
    ensures
        sel.len() == l0.len(),
        forall |k: int| 0 <= k && k < l0.len() ==> sel@[k] == l0@[k].idx,
{
    let mut sel: Vec<usize> = Vec::new();
    let mut k: usize = 0;
    while k < l0.len()
        invariant
            k <= l0.len(),
            sel.len() == k,
            forall |m: int| 0 <= m && m < k ==> sel@[m] == l0@[m].idx,
        decreases l0.len() - k,
    {
        sel.push(l0[k].idx);
        k = k + 1;
    }
    sel
}

/// AS-IS mutant 3 (blind pushdown): skips the destination disjointness
/// refusal — pushes the oldest source file into a level that already has
/// overlapping ranges, the exact hazard the refusal exists to block.
pub fn pick_pushdown_as_is_blind(src: &[MFile], dst: &[MFile]) -> (r: Option<(usize, Vec<MFile>)>)
    requires
        forall |i: int, j: int| 0 <= i && i < dst.len() && 0 <= j && j < dst.len() && i != j
            ==> dst[i].idx != dst[j].idx,
    ensures
        src.len() == 0 <==> (r is None),
        r.is_some() ==> {
            let (s, slice) = r.unwrap();
            &&& s == src@[0].idx
            &&& slice@ == overlapping_prefix(dst@, dst@.len() as int, src@[0].lo, src@[0].hi)
        },
{
    if src.len() == 0 {
        None
    } else {
        broadcast use vstd::seq::group_seq_axioms;
        let s0_lo = src[0].lo;
        let s0_hi = src[0].hi;
        let mut slice: Vec<MFile> = Vec::new();
        let mut j: usize = 0;
        while j < dst.len()
            invariant
                j <= dst.len(),
                slice@ == overlapping_prefix(dst@, j as int, s0_lo, s0_hi),
            decreases dst.len() - j,
        {
            if overlaps_exec(dst[j].lo, dst[j].hi, s0_lo, s0_hi) {
                slice.push(dst[j]);
            }
            j = j + 1;
        }
        assert(src@[0].lo == s0_lo);
        assert(src@[0].hi == s0_hi);
        Some((src[0].idx, slice))
    }
}

/// Divergence 1 (whole level): dst holds one file inside the hull and one
/// entirely outside it. The model's slice is exactly the overlapping
/// prefix (one file); the as-is mutant reabsorbs both.
pub fn divergence_l0_to_l1_whole_level()
{
    let mut l0: Vec<MFile> = Vec::new();
    l0.push(MFile { idx: 0, lo: 10, hi: 20 });
    let mut l1: Vec<MFile> = Vec::new();
    l1.push(MFile { idx: 1, lo: 15, hi: 25 });
    l1.push(MFile { idx: 2, lo: 1000, hi: 2000 });
    assert(l0@.len() == 1 && l1@.len() == 2);
    assert(l0@[0].lo == 10 && l0@[0].hi == 20);
    assert(l1@[0].idx == 1 && l1@[1].idx == 2);
    assert(overlaps_spec(1000, 2000, 10, 20) == false);
    let mr = pick_l0_to_l1_model(&l0, &l1, 4);
    assert(mr.is_some());
    let (msel, hlo, hhi, mslice) = mr.unwrap();
    assert(msel.len() == 1);
    let (_asel, aslice) = pick_l0_to_l1_as_is_whole_level(&l0, &l1);
    // model: the hull of one L0 file is that file's bounds, and the far
    // file (idx 2, no overlap) never enters the slice.
    assert(hlo == 10 && hhi == 20);
    proof {
        lemma_prefix_excludes(l1@, l1@.len() as int, hlo, hhi, 1);
    }
    assert(forall |q: int| 0 <= q && q < mslice.len() ==> mslice@[q].idx != 2);
    // as-is: whole level reabsorbed, far file included.
    assert(aslice.len() == 2);
    assert(aslice@[1].idx == 2);
}

/// Divergence 2 (cap): three L0 files, cap 1. The model selects exactly
/// one; the as-is mutant selects all three.
pub fn divergence_l0_to_l1_uncapped()
{
    let mut l0: Vec<MFile> = Vec::new();
    l0.push(MFile { idx: 0, lo: 10, hi: 20 });
    l0.push(MFile { idx: 3, lo: 30, hi: 40 });
    l0.push(MFile { idx: 4, lo: 50, hi: 60 });
    let l1: Vec<MFile> = Vec::new();
    assert(l0@.len() == 3);
    let mr = pick_l0_to_l1_model(&l0, &l1, 1);
    assert(mr.is_some());
    let (msel, _hlo, _hhi, _mslice) = mr.unwrap();
    assert(msel.len() == 1);
    let asel = pick_l0_to_l1_as_is_uncapped(&l0, 1);
    assert(asel.len() == 3);
}

/// Divergence 3 (blind pushdown): the destination itself is not disjoint,
/// so the model refuses (None); the blind mutant pushes anyway.
pub fn divergence_pushdown_blind()
{
    let mut src: Vec<MFile> = Vec::new();
    src.push(MFile { idx: 0, lo: 10, hi: 20 });
    let mut dst: Vec<MFile> = Vec::new();
    dst.push(MFile { idx: 1, lo: 5, hi: 15 });
    dst.push(MFile { idx: 2, lo: 12, hi: 25 });
    assert(dst@.len() == 2);
    assert(dst@[0].hi >= dst@[1].lo);
    assert(dst@[1].hi >= dst@[0].lo);
    assert(!disjoint_spec(dst@)) by {
        assert(!(dst@[0].hi < dst@[1].lo || dst@[1].hi < dst@[0].lo));
    };
    let m = pick_pushdown_model(&src, &dst);
    assert(m.is_none());
    let a = pick_pushdown_as_is_blind(&src, &dst);
    assert(a.is_some());
}
} // verus! pick

/// Size multiplier between consecutive levels (RocksDB `fanout` shape).
pub(crate) const LEVEL_FANOUT: u64 = 10;

/// Kill switch for the leveled scheduler (`PEDRA_LEVELED=0`): jobs fall back
/// to L0-only stacking and settle to a whole-level rewrite (the pre-leveled
/// shape). A/B and emergency-rollback lever for guest runs.
#[cfg(not(verus_keep_ghost))]
pub(crate) fn leveled_enabled() -> bool {
    match std::env::var("PEDRA_LEVELED") {
        Ok(v) => v.trim() != "0",
        Err(_) => true,
    }
}

/// Byte target of level `level` (1-based). Level 0 has no target (L0 is
/// drained, not sized); the caller treats the maximum level as unbounded.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub(crate) fn level_target_bytes(level: u32, l1_target: u64) -> u64 {
    if level == 0 {
        return 0;
    }
    let exp = (level - 1).min(18) as u32;
    l1_target.saturating_mul(LEVEL_FANOUT.saturating_pow(exp))
}

/// AS-IS (pair `leveling`): the naive ladder with no exponent cap and
/// wrapping arithmetic. On deep levels the target wraps downward, so an
/// over-target level reads under target — the pre-leveled shape where job
/// sizing is garbage past level 19.
#[cfg(test)]
#[must_use]
pub(crate) fn level_target_bytes_as_is(level: u32, l1_target: u64) -> u64 {
    if level == 0 {
        return 0;
    }
    l1_target.wrapping_mul(LEVEL_FANOUT.wrapping_pow(level - 1))
}

/// One scheduling candidate: live-inventory index plus its user-key range and
/// on-disk size. Key ranges are user keys (internal suffixes only widen a
/// range, and overlap on user keys is the conservative direction).
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone)]
pub(crate) struct LevelFile {
    pub idx: usize,
    pub lo: Vec<u8>,
    pub hi: Vec<u8>,
    pub bytes: u64,
}

#[cfg(not(verus_keep_ghost))]
impl LevelFile {
    /// Overlaps the half-open hull `[hull_lo, hull_hi]` (inclusive both ends:
    /// ranges carry concrete smallest/largest keys).
    pub(crate) fn overlaps(&self, hull_lo: &[u8], hull_hi: &[u8]) -> bool {
        self.lo.as_slice() <= hull_hi && self.hi.as_slice() >= hull_lo
    }
}

/// Whether the files are pairwise disjoint once sorted by smallest key.
///
/// Equal boundary user keys count as overlap: chunks split at user-key
/// boundaries never share a user key, so a shared boundary means the set was
/// not produced by this policy (legacy stacking) and must be repaired before
/// overlap-sliced jobs run on it.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub(crate) fn is_disjoint(files: &[LevelFile]) -> bool {
    let mut sorted: Vec<&LevelFile> = files.iter().collect();
    sorted.sort_by(|a, b| a.lo.cmp(&b.lo));
    sorted
        .windows(2)
        .all(|w| w[0].hi.as_slice() < w[1].lo.as_slice())
}

/// Total bytes of a level view.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub(crate) fn total_bytes(files: &[LevelFile]) -> u64 {
    files.iter().map(|f| f.bytes).sum()
}

/// Inputs for an L0→L1 job: the oldest `max_l0` L0 files plus the disjoint-L1
/// slice overlapping their hull.
///
/// Returns `None` when there is no L0 input. The caller has already verified
/// the L1 view is disjoint ([`is_disjoint`]) — over a stacked L1 the slice
/// would be the whole level (see module docs).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub(crate) fn pick_l0_to_l1(
    l0: &[LevelFile],
    l1: &[LevelFile],
    max_l0: usize,
) -> Option<(Vec<usize>, Vec<usize>)> {
    if l0.is_empty() || max_l0 == 0 {
        return None;
    }
    let sel: Vec<&LevelFile> = l0.iter().take(max_l0).collect();
    let hull_lo = sel.iter().map(|f| f.lo.as_slice()).min()?.to_vec();
    let hull_hi = sel.iter().map(|f| f.hi.as_slice()).max()?.to_vec();
    let slice: Vec<usize> = l1
        .iter()
        .filter(|f| f.overlaps(&hull_lo, &hull_hi))
        .map(|f| f.idx)
        .collect();
    Some((sel.iter().map(|f| f.idx).collect(), slice))
}

/// AS-IS (pair `leveling_pick`): the L0→L1 job reabsorbs the whole L1
/// regardless of overlap — the pre-leveled whole-level-rewrite shape.
#[cfg(test)]
#[must_use]
pub(crate) fn pick_l0_to_l1_as_is_whole_level(
    l0: &[LevelFile],
    l1: &[LevelFile],
) -> Option<(Vec<usize>, Vec<usize>)> {
    if l0.is_empty() {
        return None;
    }
    Some((
        l0.iter().map(|f| f.idx).collect(),
        l1.iter().map(|f| f.idx).collect(),
    ))
}

/// AS-IS (pair `leveling_pick`): every L0 file enters the job, the input
/// cap is ignored — unbounded job size on a deep L0 stack.
#[cfg(test)]
#[must_use]
pub(crate) fn pick_l0_to_l1_as_is_uncapped(l0: &[LevelFile], _max_l0: usize) -> Option<Vec<usize>> {
    if l0.is_empty() {
        return None;
    }
    Some(l0.iter().map(|f| f.idx).collect())
}

/// Inputs for one pushdown job from level `n` to `n+1`: the oldest source
/// file plus the (disjoint) level-`n+1` files overlapping it.
///
/// `src` is caller-ordered oldest-first. Returns `None` when the source level
/// is empty or the destination view is not disjoint.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub(crate) fn pick_pushdown(src: &[LevelFile], dst: &[LevelFile]) -> Option<(usize, Vec<usize>)> {
    let source = src.first().cloned()?;
    if !is_disjoint(dst) {
        return None;
    }
    let slice: Vec<usize> = dst
        .iter()
        .filter(|f| f.overlaps(&source.lo, &source.hi))
        .map(|f| f.idx)
        .collect();
    Some((source.idx, slice))
}

/// AS-IS (pair `leveling_pick`): the pushdown skips the disjoint-
/// destination gate, so a stacked level gets rewritten one file at a
/// time — the unbounded cascade the gate exists to refuse.
#[cfg(test)]
#[must_use]
pub(crate) fn pick_pushdown_as_is_blind(
    src: &[LevelFile],
    dst: &[LevelFile],
) -> Option<(usize, Vec<usize>)> {
    let source = src.first().cloned()?;
    let slice: Vec<usize> = dst
        .iter()
        .filter(|f| f.overlaps(&source.lo, &source.hi))
        .map(|f| f.idx)
        .collect();
    Some((source.idx, slice))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(idx: usize, lo: &str, hi: &str, bytes: u64) -> LevelFile {
        LevelFile {
            idx,
            lo: lo.as_bytes().to_vec(),
            hi: hi.as_bytes().to_vec(),
            bytes,
        }
    }

    #[test]
    fn level_targets_scale_by_fanout() {
        assert_eq!(level_target_bytes(0, 256), 0);
        assert_eq!(level_target_bytes(1, 256), 256);
        assert_eq!(level_target_bytes(2, 256), 2_560);
        assert_eq!(level_target_bytes(3, 256), 25_600);
    }

    #[test]
    fn disjoint_detection_rejects_stacked_runs() {
        let disjoint = vec![f(0, "a", "c", 1), f(1, "d", "f", 1), f(2, "g", "z", 1)];
        assert!(is_disjoint(&disjoint));
        let stacked = vec![f(0, "a", "m", 1), f(1, "b", "z", 1)];
        assert!(!is_disjoint(&stacked));
        // Shared boundary user key = not disjoint.
        let touching = vec![f(0, "a", "d", 1), f(1, "d", "z", 1)];
        assert!(!is_disjoint(&touching));
    }

    #[test]
    fn l0_job_takes_only_the_overlapping_l1_slice() {
        let l0 = vec![f(10, "b", "y", 100)];
        let l1 = vec![
            f(0, "a", "a", 1),
            f(1, "c", "e", 2),
            f(2, "x", "z", 3),
            f(3, "zz", "zzz", 4),
        ];
        let (l0s, slice) = pick_l0_to_l1(&l0, &l1, 2).unwrap();
        assert_eq!(l0s, vec![10]);
        assert_eq!(slice, vec![1, 2]);
    }

    #[test]
    fn l0_job_respects_the_input_cap() {
        let l0 = vec![f(1, "a", "z", 1), f(2, "a", "z", 1), f(3, "a", "z", 1)];
        let (l0s, _) = pick_l0_to_l1(&l0, &[], 2).unwrap();
        assert_eq!(l0s, vec![1, 2]);
    }

    #[test]
    fn pushdown_takes_oldest_source_plus_overlaps() {
        let src = vec![f(7, "m", "p", 5), f(9, "q", "r", 6)];
        let dst = vec![f(0, "a", "m", 1), f(1, "n", "o", 1), f(2, "s", "z", 1)];
        let (s, slice) = pick_pushdown(&src, &dst).unwrap();
        assert_eq!(s, 7);
        assert_eq!(slice, vec![0, 1]);
    }

    #[test]
    fn pushdown_refuses_non_disjoint_destination() {
        let dst = vec![f(0, "a", "m", 1), f(1, "b", "z", 1)];
        assert!(pick_pushdown(&[f(7, "m", "p", 5)], &dst).is_none());
    }

    /// The invariant the module docs argue: after replacing a job's inputs
    /// with its hull, the level is still disjoint. Checked by simulation
    /// over random uniform-shaped levels.
    #[test]
    fn job_output_keeps_level_disjoint() {
        let mut rng_state = 0x853c_49e6_748f_ea9du64;
        let mut rng = move || {
            rng_state ^= rng_state << 13;
            rng_state ^= rng_state >> 7;
            rng_state ^= rng_state << 17;
            rng_state
        };
        for _ in 0..200 {
            // Random disjoint L1 over key space 0..1000. Keys are zero-padded
            // so byte order == numeric order: a plain decimal rendering would
            // invert ranges across digit-count boundaries (lo "98" > hi "105"),
            // a shape production never produces (smallest <= largest user key).
            let key = |k: u32| format!("{k:04}");
            let mut l1: Vec<LevelFile> = Vec::new();
            let mut k = 0u32;
            let mut idx = 0;
            while k < 950 {
                let w = 1 + (rng() % 60) as u32;
                l1.push(f(idx, &key(k), &key(k + w), 10));
                idx += 1;
                k += w + 1 + (rng() % 40) as u32;
            }
            // A random L0 hull.
            let a = (rng() % 400) as u32;
            let b = a + 1 + (rng() % 500) as u32;
            let l0 = vec![f(9000, &key(a), &key(b), 10)];
            let (_, slice) = pick_l0_to_l1(&l0, &l1, 1).unwrap();
            // Simulate: drop the slice, insert the hull as one chunk.
            let mut after: Vec<LevelFile> = l1
                .iter()
                .filter(|x| !slice.contains(&x.idx))
                .cloned()
                .collect();
            after.push(f(9500, &key(a), &key(b), 10));
            assert!(
                is_disjoint(&after),
                "hull [{a},{b}] broke disjointness with slice {slice:?}"
            );
        }
    }

    /// Plant (pair `leveling`, entry `level_target_bytes`): on deep levels
    /// the entry ladder caps the exponent and saturates — never wrapping
    /// downward — while the as-is mutant's target shrinks, so an
    /// over-target level reads under target.
    #[test]
    fn level_target_bytes_on_live_deep_level_is_not_ok() {
        assert_eq!(level_target_bytes(19, 2), level_target_bytes(20, 2));
        assert!(level_target_bytes(20, 2) >= level_target_bytes(19, 2));
        assert_ne!(level_target_bytes(20, 2), level_target_bytes_as_is(20, 2));
        assert!(
            level_target_bytes_as_is(20, 2) < level_target_bytes_as_is(19, 2),
            "AS-IS dente: deep-level target wraps downward"
        );
    }

    /// Plant (pair `leveling_pick`, entry `pick_l0_to_l1`): the entry takes
    /// only the overlapping disjoint slice under the input cap and refuses
    /// non-disjoint pushdowns; each as-is mutant accepts one of those
    /// unbounded job shapes.
    #[test]
    fn pick_l0_to_l1_on_live_slice_is_not_ok() {
        // Whole-level dente: the far L1 file never overlaps the hull, so the
        // entry keeps it out; the mutant reabsorbs the entire level.
        let l0 = vec![f(0, "j", "t", 1)];
        let l1 = vec![f(1, "m", "p", 1), f(2, "zz", "zzz", 1)];
        let (_, mslice) = pick_l0_to_l1(&l0, &l1, 4).unwrap();
        let (_, aslice) = pick_l0_to_l1_as_is_whole_level(&l0, &l1).unwrap();
        assert!(!mslice.contains(&2));
        assert!(aslice.contains(&2), "AS-IS dente: whole level reabsorbed");

        // Uncapped dente: three L0 files, cap 1 — entry selects one, mutant
        // selects all three.
        let l0c = vec![f(0, "a", "z", 1), f(3, "a", "z", 1), f(4, "a", "z", 1)];
        assert_eq!(pick_l0_to_l1(&l0c, &[], 1).unwrap().0.len(), 1);
        assert_eq!(
            pick_l0_to_l1_as_is_uncapped(&l0c, 1).unwrap().len(),
            3,
            "AS-IS dente: input cap ignored"
        );

        // Blind-pushdown dente: a stacked destination is refused by the
        // entry, blindly rewritten by the mutant.
        let dst = vec![f(0, "a", "m", 1), f(1, "b", "z", 1)];
        let src = vec![f(7, "m", "p", 5)];
        assert!(pick_pushdown(&src, &dst).is_none());
        assert!(
            pick_pushdown_as_is_blind(&src, &dst).is_some(),
            "AS-IS dente: stacked destination rewritten anyway"
        );
    }

    /// Plant (pair `leveling_pushdown`, entry `pick_pushdown`): the
    /// pushdown demotes only the oldest source chunk and only over a
    /// disjoint destination — the blind mutant rewrites stacked levels.
    #[test]
    fn pick_pushdown_on_live_pushdown_gate_is_not_ok() {
        // Empty source: no chunk to demote at all.
        assert!(pick_pushdown(&[], &[f(0, "a", "z", 1)]).is_none());

        // Disjoint destination: entry takes the OLDEST chunk's index and
        // exactly its overlapping slice — the far file stays out.
        let src = vec![f(7, "m", "p", 5), f(9, "q", "r", 5)];
        let dst = vec![f(0, "a", "m", 1), f(2, "zz", "zzz", 1)];
        let (sidx, slice) = pick_pushdown(&src, &dst).unwrap();
        assert_eq!(sidx, 7, "oldest chunk demoted first");
        assert_eq!(slice, vec![0]);
        assert!(!slice.contains(&2), "far file never in the slice");

        // Stacked destination: the entry's disjoint gate refuses; the
        // blind mutant returns a job that rewrites the stacked level.
        let stacked = vec![f(0, "a", "m", 1), f(1, "b", "z", 1)];
        assert!(pick_pushdown(&src, &stacked).is_none());
        assert!(
            pick_pushdown_as_is_blind(&src, &stacked).is_some(),
            "AS-IS dente: stacked destination rewritten anyway"
        );
    }
}
