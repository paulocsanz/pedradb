// Verus twin, catalog pair `leveling_pick` (atom tier): the two selection
// decisions of the leveled-compaction kernel, on a u64-key stand-in
// domain ("model: stand-in domain, not production types").
//
// Source of truth for production remains `src/leveling.rs`
// (`pick_l0_to_l1`, `pick_pushdown` — called by `Db::prepare_l0_compact`
// and `Db::prepare_pushdown_compact`). The ladder fn is the close-tier
// pair `leveling`, twin in `verus/leveling.rs`.
//
// Proved here:
//   - the L0→L1 slice is EXACTLY the dst files overlapping the selected-L0
//     hull (never the whole level), and the input cap is respected;
//   - the pushdown refuses a non-disjoint destination.
// The AS-IS mutants (absorb-whole-level / ignore-cap / blind pushdown)
// carry their true as-is contracts, and one divergence fn per mutant pins
// the witness state where model and as-is part ways.
//
// Model gaps (stated, not hidden):
//   - keys are u64, production keys are Vec<u8> (lex order == integer
//     order under zero-padded keys — the kernel's own property test notes
//     the non-padded shape as one production never produces);
//   - `disjoint` is the PAIRWISE semantics; production implements it as
//     sort-by-lo + adjacent windows. sort⇔pairwise equivalence is
//     unproven glue (standard model-tier assumption); `is_disjoint_model`
//     is an O(n²) pairwise stand-in for the kernel's sort-based scan.
//
//   ./scripts/verus_leveling.sh
//
// Do not link this into the production crate — it is a twin of the pure kernel.

use vstd::prelude::*;

verus! {

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

} // verus!
