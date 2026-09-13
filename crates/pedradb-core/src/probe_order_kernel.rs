//! RFC-0164 P0.1: probe-order kernel — candidate order for point probes.
//!
//! **Single artifact (pair `probe_order`):** this file is what `rustc` links
//! *and* what Verus proves (`cfg(verus_keep_ghost)`). Slice covering walk
//! stays rustc. No twin-cópia.
//!
//!   ./scripts/verus_probe_order.sh
//!
//! Spec S: the tables probed for a point lookup must be visited newest-first
//! among the candidates whose `[lo, hi]` covers the key. The engine's
//! historical walk sorted candidates by `lo` ascending and walked them in
//! reverse (descending `lo`); on an equal-`lo` tie that probes the OLDEST
//! table first, so a newer tombstone is never consulted and a deleted value
//! is resurrected (findings/2026-09-04-reopen-delete-resurrected, db.rs
//! `.rev()` loops). This kernel owns the order; the wire (P0.2) lands after
//! the read-path fix.

#![forbid(unsafe_code)]

macro_rules! first_probe_on_equal_lo_body {
    ($newer:expr, $older:expr) => {{
        let _ = $older;
        $newer
    }};
}

macro_rules! first_probe_on_equal_lo_as_is_body {
    ($newer:expr, $older:expr) => {{
        let _ = $newer;
        $older
    }};
}

/// Decision core (theorem-ready): among two covering candidates tied at
/// `lo`, the probe order visits the NEWEST first — a newer tombstone or
/// overwrite must never be shadowed by an older table's `Found`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn first_probe_on_equal_lo(newer: usize, _older: usize) -> usize {
    first_probe_on_equal_lo_body!(newer, _older)
}

/// AS-IS twin (recorded mutant): the historical descending-`lo` walk
/// (stable sort + `.rev()`) visits the OLDEST tied candidate first —
/// `Found` wins there, the newer tombstone is never consulted, and the
/// deleted value is resurrected.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn first_probe_on_equal_lo_as_is(_newer: usize, older: usize) -> usize {
    first_probe_on_equal_lo_as_is_body!(_newer, older)
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn first_probe_on_equal_lo_spec(newer: usize, _older: usize) -> usize {
    newer
}

pub open spec fn first_probe_on_equal_lo_as_is_spec(_newer: usize, older: usize) -> usize {
    older
}

pub fn first_probe_on_equal_lo(newer: usize, older: usize) -> (r: usize)
    ensures
        r == first_probe_on_equal_lo_spec(newer, older),
        r == newer,
{
    first_probe_on_equal_lo_body!(newer, older)
}

pub fn first_probe_on_equal_lo_as_is(newer: usize, older: usize) -> (r: usize)
    ensures
        r == first_probe_on_equal_lo_as_is_spec(newer, older),
        r == older,
{
    first_probe_on_equal_lo_as_is_body!(newer, older)
}

proof fn lemma_as_is_picks_oldest_on_tie()
    ensures
        first_probe_on_equal_lo_spec(1, 0) == 1,
        first_probe_on_equal_lo_as_is_spec(1, 0) == 0,
{
}

} // verus!

/// Candidate tables for a point probe, newest-first (the spec).
///
/// `los[i]`/`his[i]` bound table `i`; `newest_first` lists table indices in
/// recency order (newest first). Returns the subsequence of `newest_first`
/// whose range covers `key`. Total function: out-of-range indices are
/// skipped, never panic. P0.2 wires the engine walk to `probe_order`; the
/// transitional dead-code allow ends there.
#[cfg(not(verus_keep_ghost))]
#[cfg_attr(not(test), allow(dead_code))]
fn probe_order(los: &[&[u8]], his: &[&[u8]], newest_first: &[usize], key: &[u8]) -> Vec<usize> {
    newest_first
        .iter()
        .copied()
        .filter(|&i| i < los.len() && i < his.len() && los[i] <= key && key <= his[i])
        .collect()
}

/// Packed `hi(pos) >= key`. Index form so Charon sees a total fn, not `impl Fn`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
fn covering_hi_ge(his: &[&[u8]], pos: usize, key: &[u8]) -> bool {
    pos < his.len() && his[pos] >= key
}

/// Position of `i` in `by_lo`, or `by_lo.len()` if missing (kept, like the
/// engine walk). Index loop — `Iterator::position` CFailure on this pin.
#[cfg(not(verus_keep_ghost))]
#[must_use]
fn covering_pos(by_lo: &[usize], i: usize) -> usize {
    let mut pos = 0usize;
    while pos < by_lo.len() {
        if by_lo[pos] == i {
            return pos;
        }
        pos += 1;
    }
    pos
}

/// Engine-facing packed image of [`probe_order`] (RFC-0164 P0.2): members of
/// `newest_first` whose packed `[lo, hi]` covers `key`, newest-first.
/// `prefix_end` is the caller's `partition_point_gt(key)` over the los
/// (`pos < prefix_end` ⟺ `lo(pos) <= key`). A table missing from `by_lo`
/// is kept. Index `while` (not `filter`/`position`/`impl Iterator`) so the
/// Aeneas pin emits a `def`.
#[cfg(not(verus_keep_ghost))]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn probe_order_covering(
    newest_first: &[usize],
    by_lo: &[usize],
    prefix_end: usize,
    his: &[&[u8]],
    key: &[u8],
) -> Vec<usize> {
    let mut out = Vec::with_capacity(newest_first.len());
    let mut k = 0usize;
    while k < newest_first.len() {
        let i = newest_first[k];
        let pos = covering_pos(by_lo, i);
        if pos >= by_lo.len() || (pos < prefix_end && covering_hi_ge(his, pos, key)) {
            out.push(i);
        }
        k += 1;
    }
    out
}

/// AS-IS dente: same covering test, oldest-first (historical `.rev()` walk).
#[cfg(not(verus_keep_ghost))]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn probe_order_covering_as_is(
    newest_first: &[usize],
    by_lo: &[usize],
    prefix_end: usize,
    his: &[&[u8]],
    key: &[u8],
) -> Vec<usize> {
    let n = newest_first.len();
    let mut out = Vec::with_capacity(n);
    let mut j = 0usize;
    while j < n {
        let i = newest_first[n - 1 - j];
        let pos = covering_pos(by_lo, i);
        if pos >= by_lo.len() || (pos < prefix_end && covering_hi_ge(his, pos, key)) {
            out.push(i);
        }
        j += 1;
    }
    out
}

/// Strict-disjoint fast-path arm (RFC-0164 P1.2): a run indexed by `lo`
/// may take the single-candidate bisect path only when every adjacent
/// pair is STRICTLY disjoint — `hi[i-1] < lo[i]`. Equal-`lo` ties (the
/// put/tombstone shape of findings/2026-09-04-reopen-delete-resurrected)
/// and overlaps must stay on the newest-first walk
/// ([`probe_order_covering`]). `SstRun::pairwise_disjoint` wires here;
/// the engine passes parallel arrays in `by_lo` order, and `min` keeps
/// the fn total on mismatched lengths.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn run_pairwise_disjoint_los(los: &[&[u8]], his: &[&[u8]]) -> bool {
    let n = los.len().min(his.len());
    n >= 2 && (1..n).all(|i| his[i - 1] < los[i])
}

/// AS-IS twin (recorded mutant): the non-strict arm — `hi[i-1] <= lo[i]`.
/// On the equal-`lo` tie it arms the single-candidate bisect path; the
/// stable sort keeps newest-first among ties, so `by_lo[p-1]` lands on
/// the OLDER table and the deleted value is resurrected — the fast-path
/// variant of the measured failure (the P0.1 walk mutant is the other).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn run_pairwise_disjoint_los_as_is(los: &[&[u8]], his: &[&[u8]]) -> bool {
    let n = los.len().min(his.len());
    n >= 2 && (1..n).all(|i| his[i - 1] <= los[i])
}

/// AS-IS twin: the engine's historical candidate walk (recorded mutant).
///
/// Sorts ALL tables by `lo` ascending (stable over `newest_first`, matching
/// `SstRun::sorted_by_lo`), keeps the prefix with `lo <= key`
/// (`partition_point_gt`), and walks it in reverse, skipping non-covering
/// tables — the two `.rev()` loops in `Db::lookup`/`lookup_sst_packed`.
/// On an equal-`lo` tie the stable sort preserves newest-first, so the
/// reverse walk probes the OLDEST table first: the stale pick.
#[cfg(not(verus_keep_ghost))]
#[cfg_attr(not(test), allow(dead_code))]
fn probe_order_as_is(
    los: &[&[u8]],
    his: &[&[u8]],
    newest_first: &[usize],
    key: &[u8],
) -> Vec<usize> {
    let mut by_lo: Vec<usize> = newest_first
        .iter()
        .copied()
        // The engine only ever sorts valid table indices; dropping stale
        // ones keeps the twin callable standalone.
        .filter(|&i| i < los.len())
        .collect();
    by_lo.sort_by(|&a, &b| los[a].cmp(los[b]));
    let p = by_lo.partition_point(|&i| los[i] <= key);
    by_lo[..p]
        .iter()
        .rev()
        .copied()
        .filter(|&i| i < his.len() && his[i] >= key)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measured failure shape: put@1 (older) and tombstone@2 (newer)
    /// over the same key — two L0 tables with equal lo = hi = k.
    #[test]
    fn equal_lo_tie_probes_newest_first() {
        let k = b"k".as_slice();
        let los = [k, k];
        let his = [k, k];
        let newest_first = [1, 0];
        let spec = probe_order(&los, &his, &newest_first, k);
        assert_eq!(spec, vec![1, 0]);
        // Decision core agrees with the order's first pick.
        assert_eq!(spec.first().copied(), Some(first_probe_on_equal_lo(1, 0)));
        // AS-IS tooth: the historical descending-lo walk picks 0 (the older
        // put) first — the resurrected delete.
        let mutant = probe_order_as_is(&los, &his, &newest_first, k);
        assert_eq!(mutant, vec![0, 1]);
        assert_eq!(
            mutant.first().copied(),
            Some(first_probe_on_equal_lo_as_is(1, 0))
        );
    }

    /// Overlapping ranges with distinct los: older table's lo is above the
    /// newer's — descending-lo still hits the older table first.
    #[test]
    fn overlap_distinct_los_still_inverts_as_is() {
        let (a, b, m, z) = (
            b"a".as_slice(),
            b"b".as_slice(),
            b"m".as_slice(),
            b"z".as_slice(),
        );
        // table 0 (older): [b, z]; table 1 (newer): [a, m]; key = b.
        let los = [b, a];
        let his = [z, m];
        let newest_first = [1, 0];
        assert_eq!(probe_order(&los, &his, &newest_first, b), vec![1, 0]);
        assert_eq!(probe_order_as_is(&los, &his, &newest_first, b), vec![0, 1]);
    }

    /// Strictly disjoint run: one candidate — both orders agree (the fast
    /// path stays legitimate exactly here).
    #[test]
    fn disjoint_run_agrees() {
        let (a, b, c, d) = (
            b"a".as_slice(),
            b"b".as_slice(),
            b"c".as_slice(),
            b"d".as_slice(),
        );
        let los = [a, c];
        let his = [b, d];
        let newest_first = [1, 0];
        assert_eq!(probe_order(&los, &his, &newest_first, c), vec![1]);
        assert_eq!(probe_order_as_is(&los, &his, &newest_first, c), vec![1]);
        // Key covered only by the older table.
        assert_eq!(probe_order(&los, &his, &newest_first, a), vec![0]);
    }

    /// Total function: stale indices and empty inputs never panic.
    #[test]
    fn total_on_degenerate_inputs() {
        let k = b"k".as_slice();
        assert!(probe_order(&[], &[], &[], k).is_empty());
        assert!(probe_order(&[k], &[k], &[7], k).is_empty());
        assert!(probe_order_as_is(&[k], &[k], &[7], k).is_empty());
    }

    /// P0.2 wire equivalence: the packed image yields the same order as the
    /// unpacked spec `probe_order` on the same run — a hi-bounded miss and
    /// the equal-lo tie (the measured failure shape).
    #[test]
    fn packed_covering_matches_probe_order() {
        let (a, b, c, k, z) = (
            b"a".as_slice(),
            b"b".as_slice(),
            b"c".as_slice(),
            b"k".as_slice(),
            b"z".as_slice(),
        );
        // Run: table 0 (older) [b,z] covers k; table 1 (newer) [a,c] misses
        // (hi c < k). by_lo sorts by lo: [1 (a), 0 (b)]; his in that order
        // are [c, z].
        let los = [b, a];
        let his = [z, c];
        let newest_first = [1usize, 0];
        let by_lo = [1usize, 0];
        let his_sorted = [c, z];
        assert_eq!(
            probe_order(&los, &his, &newest_first, k),
            vec![0],
            "only the older table covers k (newer hi c < k)"
        );
        let packed = probe_order_covering(&newest_first, &by_lo, 2, &his_sorted, k);
        assert_eq!(packed, vec![0]);

        // Equal-lo tie: both tables [k,k]; the packed image keeps the
        // newest-first order exactly like the spec.
        let los_tie = [k, k];
        let his_tie = [k, k];
        assert_eq!(
            probe_order(&los_tie, &his_tie, &newest_first, k),
            vec![1, 0]
        );
        let tie = probe_order_covering(&newest_first, &by_lo, 2, &his_tie, k);
        assert_eq!(tie, vec![1, 0]);
        let mutant = probe_order_covering_as_is(&newest_first, &by_lo, 2, &his_tie, k);
        assert_eq!(mutant, vec![0, 1]);
    }

    /// Finite-domain theorem: on EVERY distinct equal-lo tie the decision
    /// core orders newest-first and the AS-IS twin oldest-first — they
    /// disagree exactly on every tie.
    #[test]
    fn theorem_first_probe_on_every_distinct_tie() {
        for newer in 0..4usize {
            for older in 0..4usize {
                if newer == older {
                    continue;
                }
                assert_eq!(first_probe_on_equal_lo(newer, older), newer);
                assert_eq!(first_probe_on_equal_lo_as_is(newer, older), older);
                assert_ne!(
                    first_probe_on_equal_lo(newer, older),
                    first_probe_on_equal_lo_as_is(newer, older),
                    "mutant must differ from fixed on every distinct tie"
                );
            }
        }
    }

    /// The measured shape (RFC-0164 P1.2): two single-key tables tied at
    /// lo = hi = k — the bisect arm must stay OFF; the non-strict mutant
    /// arms it (fast path onto the older put — resurrection).
    #[test]
    fn equal_lo_tie_keeps_bisect_arm_off() {
        let k = b"k".as_slice();
        let los = [k, k];
        let his = [k, k];
        assert!(!run_pairwise_disjoint_los(&los, &his));
        assert!(run_pairwise_disjoint_los_as_is(&los, &his));
    }

    /// A real gap (hi[i-1] < lo[i]) arms both rules — the guard is not
    /// vacuous.
    #[test]
    fn strict_gap_arms_bisect() {
        let los = [b"b".as_slice(), b"k".as_slice()];
        let his = [b"a".as_slice(), b"j".as_slice()];
        assert!(run_pairwise_disjoint_los(&los, &his));
        assert!(run_pairwise_disjoint_los_as_is(&los, &his));
    }

    /// Overlapping bounds (hi[i-1] > lo[i]) never arm, under either rule.
    #[test]
    fn overlap_never_arms_bisect() {
        let los = [b"a".as_slice(), b"b".as_slice()];
        let his = [b"z".as_slice(), b"z".as_slice()];
        assert!(!run_pairwise_disjoint_los(&los, &his));
        assert!(!run_pairwise_disjoint_los_as_is(&los, &his));
    }

    /// Single-table runs keep the walk: the bisect path needs >= 2 tables.
    #[test]
    fn single_table_keeps_bisect_arm_off() {
        let los = [b"k".as_slice()];
        let his = [b"k".as_slice()];
        assert!(!run_pairwise_disjoint_los(&los, &his));
        assert!(!run_pairwise_disjoint_los_as_is(&los, &his));
    }

    /// Live shape helper: two REAL SSTs from the production writer — older
    /// put@1, newer tombstone@2 over one key — returning (older, newer,
    /// key, paths) with the measured lo/hi bounds of both tables.
    fn live_equal_lo_tables() -> (
        crate::sst::SstTable,
        crate::sst::SstTable,
        Vec<u8>,
        Vec<std::path::PathBuf>,
    ) {
        use crate::env::StdEnv;
        use crate::key::{InternalKey, ValueType};
        use bytes::Bytes;

        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let k = format!("pedra-probe-tie-{n}").into_bytes();
        let dir = std::env::temp_dir();
        let older_path = dir.join(format!("pedra-probe-older-{n}-{}", std::process::id()));
        let newer_path = dir.join(format!("pedra-probe-newer-{n}-{}", std::process::id()));
        let older = crate::sst::write_sst_try_sorted_on(
            &StdEnv,
            &older_path,
            [Ok((
                InternalKey::new(k.clone(), 1, ValueType::Value),
                Bytes::from_static(b"v"),
            ))],
            1,
        )
        .expect("write older put SST");
        let newer = crate::sst::write_sst_try_sorted_on(
            &StdEnv,
            &newer_path,
            [Ok((
                InternalKey::new(k.clone(), 2, ValueType::Deletion),
                Bytes::from_static(b""),
            ))],
            1,
        )
        .expect("write newer tombstone SST");
        (older, newer, k, vec![older_path, newer_path])
    }

    /// Catalog three-teeth plant (`run_disjoint`): the live equal-lo tie —
    /// put then tombstone over one key both land at lo = hi = k. The
    /// strict kernel keeps the covering walk (disjoint = false, the newer
    /// tombstone is consulted); the AS-IS `<=` mutant arms the bisect
    /// fast path onto the older put — the resurrected delete of
    /// findings/2026-09-04-reopen-delete-resurrected.
    #[test]
    fn run_disjoint_on_live_equal_lo_is_not_ok() {
        use crate::key::SequenceNumber;
        use crate::memtable::Lookup;

        let (older, newer, k, paths) = live_equal_lo_tables();
        for p in &paths {
            let _ = std::fs::remove_file(p);
        }
        // Measured live bounds: both single-key tables tie at lo = hi = k.
        let (lo_n, hi_n) = (
            newer.smallest_user_key().expect("newer lo"),
            newer.largest_user_key().expect("newer hi"),
        );
        let (lo_o, hi_o) = (
            older.smallest_user_key().expect("older lo"),
            older.largest_user_key().expect("older hi"),
        );
        assert_eq!(lo_n, hi_n, "single-key table ties at lo = hi");
        assert_eq!(lo_n, k.as_slice());
        assert_eq!(lo_o, lo_n, "the measured shape: equal lo across tables");
        let los = [lo_n, lo_o];
        let his = [hi_n, hi_o];
        assert!(
            !run_pairwise_disjoint_los(&los, &his),
            "kernel: the tie keeps the covering walk"
        );
        assert!(
            run_pairwise_disjoint_los_as_is(&los, &his),
            "AS-IS dente: `<=` arms the bisect on the equal-lo tie"
        );
        // Live anchor: each table alone answers put vs tombstone — the
        // walk order is what decides which one the point lookup sees.
        assert!(matches!(
            older.get(&k, SequenceNumber::MAX),
            Lookup::Found(_)
        ));
        assert!(matches!(
            newer.get(&k, SequenceNumber::MAX),
            Lookup::Deleted
        ));
    }

    /// Catalog three-teeth plant (`probe_order_covering`): the same live
    /// equal-lo tie through the packed zero-alloc covering image. The
    /// kernel gate visits the newest (tombstone) table first; the AS-IS
    /// gate visits the older put first — the not-ok order the engine
    /// historically walked (`.rev()` over lo-ascending).
    #[test]
    fn probe_order_covering_on_live_gate_is_not_ok() {
        use crate::key::SequenceNumber;
        use crate::memtable::Lookup;

        let (older, newer, k, paths) = live_equal_lo_tables();
        for p in &paths {
            let _ = std::fs::remove_file(p);
        }
        let hi_n = newer.largest_user_key().expect("newer hi");
        let hi_o = older.largest_user_key().expect("older hi");
        assert_eq!(hi_n, hi_o, "the measured shape: equal lo/hi tie");
        // by_lo stable sort on a tie keeps the given order; newest_first
        // says table 1 (the tombstone) is newer.
        let newest_first = [1usize, 0];
        let by_lo = [1usize, 0];
        let his_sorted = [hi_n, hi_o];
        let gate = probe_order_covering(&newest_first, &by_lo, 2, &his_sorted, &k);
        assert_eq!(gate, vec![1, 0], "kernel gate: newest tombstone first");
        let mutant = probe_order_covering_as_is(&newest_first, &by_lo, 2, &his_sorted, &k);
        assert_eq!(
            mutant,
            vec![0, 1],
            "AS-IS dente: older put first — the resurrecting order"
        );
        // Live anchor: the gate's first pick is the table the lookup
        // consults — the newest answers Deleted.
        assert!(matches!(
            newer.get(&k, SequenceNumber::MAX),
            Lookup::Deleted
        ));
    }

    /// RFC-0199 P0.3 counting ladder — Rust twin of
    /// `probe_order_covering_work_bound` (Lean:
    /// `formal/aeneas/lean/ProbeLadderCount.lean`, count row
    /// `catalog:probe_order_covering`).
    ///
    /// The theorem's claim in Rust terms: `probe_order_covering` pays at
    /// most one `by_lo` scan plus one ladder step per candidate — work
    /// ≤ candidates × (by_lo.len() + 1), whatever the deeper engine
    /// structures hold. The REAL kernel is driven on every shape; an
    /// in-test counted mirror of the index skeleton (the same shape the
    /// Lean bridges prove for the extract: every `cont` of the ladder
    /// consumes exactly one candidate, every `cont` of the scan advances
    /// exactly one position) is checked against the bound arithmetic.
    /// The mirror never reimplements the covering decision — keep/skip
    /// stays inside the real kernel.
    fn ladder_work_twin(candidates: usize, scan_len: usize) -> usize {
        let mut work = 0;
        let mut k = 0;
        while k < candidates {
            // one by_lo scan (≤ scan_len steps) + one ladder step
            work += scan_len + 1;
            k += 1;
        }
        work
    }

    /// Drive the real kernel on one shape: the twin equals the bound
    /// arithmetic, the real decision matches the expected subsequence,
    /// and the output keeps `newest_first`'s relative order.
    fn assert_ladder_bound(
        newest_first: &[usize],
        by_lo: &[usize],
        prefix_end: usize,
        his: &[&[u8]],
        key: &[u8],
        expected: &[usize],
    ) {
        let bound = newest_first.len() * (by_lo.len() + 1);
        assert_eq!(
            ladder_work_twin(newest_first.len(), by_lo.len()),
            bound,
            "twin must equal the bound arithmetic (Lean proves it for all shapes)"
        );
        let out = probe_order_covering(newest_first, by_lo, prefix_end, his, key);
        assert_eq!(out, expected, "real kernel decision on this shape");
        assert!(out.len() <= newest_first.len());
        let mut cursor = newest_first.iter();
        for cand in &out {
            assert!(
                cursor.any(|c| c == cand),
                "output must keep newest_first's relative order"
            );
        }
    }

    /// The measured failure shape (equal-lo tie): both candidates cover;
    /// the ladder consumes exactly one scan + one step each.
    #[test]
    fn covering_ladder_count_bound_tie_shape() {
        let k = b"k".as_slice();
        assert_ladder_bound(&[1usize, 0], &[1usize, 0], 2, &[k, k], k, &[1, 0]);
    }

    /// Hi-bounded miss: the newer table's hi is below the key — skipped
    /// by the real decision, still one scan + one step on the twin.
    #[test]
    fn covering_ladder_count_bound_hi_miss_skips() {
        let (c, k, z) = (b"c".as_slice(), b"k".as_slice(), b"z".as_slice());
        assert_ladder_bound(&[1usize, 0], &[1usize, 0], 2, &[c, z], k, &[0]);
    }

    /// A candidate missing from `by_lo` (pos = len) is KEPT — the scan
    /// runs off the end, exactly `by_lo.len()` steps, never more.
    #[test]
    fn covering_ladder_count_bound_missing_candidate_kept() {
        let k = b"k".as_slice();
        assert_ladder_bound(&[7usize, 0], &[0usize], 1, &[k], k, &[7, 0]);
    }

    /// Prefix gate: pos past `prefix_end` skips the candidate even when
    /// the hi would cover — the lo partition is the authority.
    #[test]
    fn covering_ladder_count_bound_prefix_gate() {
        let (k, z) = (b"k".as_slice(), b"z".as_slice());
        assert_ladder_bound(&[1usize], &[1usize], 0, &[z], k, &[]);
    }

    /// Empty candidate list: zero work, empty output.
    #[test]
    fn covering_ladder_count_bound_empty_candidates() {
        let k = b"k".as_slice();
        assert_ladder_bound(&[], &[1usize, 0], 2, &[k, k], k, &[]);
        assert_eq!(ladder_work_twin(0, 3), 0);
    }

    /// The AS-IS mutant walks the same index skeleton backwards — same
    /// work bound shape (each step one scan + one step), opposite order.
    /// Work is not correctness: the mutant pays the same and is wrong.
    #[test]
    fn covering_ladder_count_bound_as_is_same_skeleton() {
        let k = b"k".as_slice();
        let newest_first = [1usize, 0];
        let by_lo = [1usize, 0];
        assert_eq!(
            ladder_work_twin(newest_first.len(), by_lo.len()),
            newest_first.len() * (by_lo.len() + 1)
        );
        let mutant = probe_order_covering_as_is(&newest_first, &by_lo, 2, &[k, k], k);
        assert_eq!(mutant, vec![0, 1], "AS-IS dente: oldest first");
    }
}
