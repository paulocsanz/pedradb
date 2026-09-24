//! Leveled compaction scheduling (pure selection kernel).
//! kernel: leveling — enrolled in residuals.json glue.kernel_paths; the
//! suffix-less enrollment tooth requires this marker (2026-08-31, findings/
//! 2026-08-31-leveling-kernel-unenrolled).
//!
//! **Term:** this file is what `rustc` links. Aeneas extracts that body
//! (`scripts/aeneas_leveling.sh`). A Verus u64-key stand-in of rustc
//! `LevelFile { lo: Vec<u8>, hi: Vec<u8> }` is a model twin — not last-wins
//! (deleted).
//!
//!   ./scripts/aeneas_leveling.sh --required
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
//! Aeneas of the rustc body is the term. A Verus stand-in is not last-wins.

/// Size multiplier between consecutive levels (RocksDB `fanout` shape).
pub(crate) const LEVEL_FANOUT: u64 = 10;

/// Kill switch for the leveled scheduler (`PEDRA_LEVELED=0`): jobs fall back
/// to L0-only stacking and settle to a whole-level rewrite (the pre-leveled
/// shape). A/B and emergency-rollback lever for guest runs.
pub(crate) fn leveled_enabled() -> bool {
    match std::env::var("PEDRA_LEVELED") {
        Ok(v) => v.trim() != "0",
        Err(_) => true,
    }
}

/// AS-IS (pair `leveled_enabled`): the rollback lever is decor — the
/// scheduler stays leveled even under `PEDRA_LEVELED=0`, so the emergency
/// fallback to pre-leveled stacking never engages.
#[cfg(test)]
#[must_use]
pub(crate) fn leveled_enabled_as_is() -> bool {
    true
}

/// Byte target of level `level` (1-based). Level 0 has no target (L0 is
/// drained, not sized); the caller treats the maximum level as unbounded.
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
#[derive(Debug, Clone)]
pub(crate) struct LevelFile {
    pub idx: usize,
    pub lo: Vec<u8>,
    pub hi: Vec<u8>,
    pub bytes: u64,
}

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
#[must_use]
pub(crate) fn is_disjoint(files: &[LevelFile]) -> bool {
    let mut sorted: Vec<&LevelFile> = files.iter().collect();
    sorted.sort_by(|a, b| a.lo.cmp(&b.lo));
    sorted
        .windows(2)
        .all(|w| w[0].hi.as_slice() < w[1].lo.as_slice())
}

/// AS-IS (pair `leveling_disjoint`): equal boundary user keys pass as
/// disjoint (`<=`) — a legacy-stacked set reads disjoint and overlap-sliced
/// jobs run on it, degrading to the whole-level cascade the gate refuses.
#[cfg(test)]
#[must_use]
pub(crate) fn is_disjoint_as_is(files: &[LevelFile]) -> bool {
    let mut sorted: Vec<&LevelFile> = files.iter().collect();
    sorted.sort_by(|a, b| a.lo.cmp(&b.lo));
    sorted
        .windows(2)
        .all(|w| w[0].hi.as_slice() <= w[1].lo.as_slice())
}

/// Total bytes of a level view.
#[must_use]
pub(crate) fn total_bytes(files: &[LevelFile]) -> u64 {
    files.iter().map(|f| f.bytes).sum()
}

/// AS-IS (pair `leveling_total_bytes`): the file count stands in for bytes —
/// level pressure is invisible, so an over-target level reads far under
/// target and pushdown sizing is garbage.
#[cfg(test)]
#[must_use]
pub(crate) fn total_bytes_as_is(files: &[LevelFile]) -> u64 {
    files.len() as u64
}

/// AS-IS (pair `leveling_overlaps`): a file whose `lo` equals the hull end
/// is treated as outside the hull (strict `<`), so the boundary-touching
/// file stays out of the job slice and the level keeps an overlapping chunk
/// after the job.
#[cfg(test)]
#[must_use]
pub(crate) fn overlaps_as_is(f: &LevelFile, hull_lo: &[u8], hull_hi: &[u8]) -> bool {
    f.lo.as_slice() < hull_hi && f.hi.as_slice() >= hull_lo
}

/// Inputs for an L0→L1 job: the oldest `max_l0` L0 files plus the disjoint-L1
/// slice overlapping their hull.
///
/// Returns `None` when there is no L0 input. The caller has already verified
/// the L1 view is disjoint ([`is_disjoint`]) — over a stacked L1 the slice
/// would be the whole level (see module docs).
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
fn pick_l0_to_l1_as_is_whole_level(
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
fn pick_l0_to_l1_as_is_uncapped(l0: &[LevelFile], _max_l0: usize) -> Option<Vec<usize>> {
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

/// AS-IS (pair `leveling_pushdown`): the pushdown skips the disjoint-
/// destination gate, so a stacked level gets rewritten one file at a
/// time — the unbounded cascade the gate exists to refuse. Non-test:
/// the `leveling_pushdown` close-twin token (the extract covers it).
#[cfg_attr(not(test), allow(dead_code))]
#[must_use]
fn pick_pushdown_as_is_blind(src: &[LevelFile], dst: &[LevelFile]) -> Option<(usize, Vec<usize>)> {
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

    #[test]
    fn leveling_has_no_verus_cartoon() {
        let src = include_str!("leveling_kernel.rs");
        let block = concat!("verus", "!", " {");
        let cfg = concat!("cfg(", "verus", "_keep", "_ghost)");
        assert!(
            !src.contains(block),
            "u64-key stand-in is not last-wins of rustc Vec<u8> LevelFile"
        );
        assert!(
            !src.contains(cfg),
            "cfg split hides rustc types from the prover"
        );
    }

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

    /// Plant (pair `leveled_enabled`): `PEDRA_LEVELED=0` must disengage the
    /// leveled scheduler — the as-is mutant keeps leveling (rollback is
    /// decor). Env is restored before returning (serial tests only).
    #[test]
    fn leveled_env_switch_is_not_ok() {
        std::env::set_var("PEDRA_LEVELED", "0");
        assert!(!leveled_enabled());
        assert!(
            leveled_enabled_as_is(),
            "AS-IS dente: PEDRA_LEVELED=0 ignored — rollback lever is decor"
        );
        std::env::remove_var("PEDRA_LEVELED");
        assert!(leveled_enabled());
    }

    /// Plant (pair `leveling_disjoint`): shared boundary user keys mean a
    /// legacy-stacked set — `is_disjoint` refuses it, the as-is mutant
    /// (`<=`) lets sliced jobs run on the stack.
    #[test]
    fn is_disjoint_on_live_stack_is_not_ok() {
        let touching = vec![f(0, "a", "d", 1), f(1, "d", "z", 1)];
        assert!(!is_disjoint(&touching));
        assert!(
            is_disjoint_as_is(&touching),
            "AS-IS dente: shared boundary passes as disjoint — whole-level cascade"
        );
    }

    /// Plant (pair `leveling_overlaps`): a chunk whose `lo` equals the hull
    /// end overlaps the hull (`overlaps` keeps it in the slice); the as-is
    /// strict side drops it and the level keeps an overlapping chunk.
    #[test]
    fn overlaps_on_live_slice_is_not_ok() {
        let chunk = f(1, "e", "k", 1);
        let (hull_lo, hull_hi) = (b"a".as_slice(), b"e".as_slice());
        assert!(chunk.overlaps(hull_lo, hull_hi));
        assert!(
            !overlaps_as_is(&chunk, hull_lo, hull_hi),
            "AS-IS dente: boundary-touching chunk left out of the slice — overlap survives the job"
        );
    }

    /// Plant (pair `leveling_total_bytes`): bytes are bytes — the as-is
    /// count stands in for them and level pressure disappears.
    #[test]
    fn total_bytes_on_live_level_is_not_ok() {
        let level = vec![f(0, "a", "c", 10), f(1, "d", "f", 20)];
        assert_eq!(total_bytes(&level), 30);
        assert_eq!(
            total_bytes_as_is(&level),
            2,
            "AS-IS dente: file count standing in for bytes — over-target level reads under target"
        );
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
