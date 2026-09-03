//! Sorted-ingest latch (pure decision kernel) — RFC-0159 P0.1.
//!
//! A workload that applies batches whose keys are strictly ascending and
//! above every key already in the column family is an **append-only
//! stream**: its natural on-disk form is a sequence of disjoint
//! bottom-level runs, reachable in one write pass instead of the
//! memtable → L0 → L1 → … pushdown ladder (which rewrites the stream
//! several times). The slipstream hydrate loop is exactly such a stream
//! (fixed-width ascending `route.svc-*` keys, batch k sorts before
//! batch k+1; the per-batch META cursor put lives in another family).
//!
//! This module only **decides**: given one batch's ops (family-resolved)
//! and the family's current on-disk maximum key, it routes each family
//! to `Bulk` or `Ladder` for that batch and advances the latch state.
//! The builder/installer that consumes `Bulk` batches is P0.2.
//!
//! # Why per-family
//!
//! Families are key-prefix partitions, so one batch routinely mixes
//! families (data puts + a repeated cursor key). A whole-Db latch would
//! see the repeated cursor key as a duplicate and never engage; each
//! family's stream is judged on its own key slice.
//!
//! # Safety of `Bulk` routing
//!
//! A family is routed `Bulk` for a batch only when every op for it in
//! that batch is a `Put`, the puts are strictly ascending within the
//! batch, and the first is strictly above the family's high-water mark —
//! which ratchets over **every** key ever observed for the family (bulk,
//! ladder, or ineligible batches alike) and, on first observation, must
//! clear the family's pre-existing on-disk maximum. Installing such a
//! batch as a new run above everything below is therefore order-correct
//! without any merge. Deletes and range-deletes never route `Bulk`; they
//! go to the ladder and do not break a live stream (their sequence
//! numbers resolve normally against later bulk puts).
//!
//! The latch is **conservative**: any violation (duplicate key, descent
//! across batches, first batch below the on-disk maximum) kills the
//! family permanently for the Db session — the workload was not
//! append-only after all, and the ladder path is unchanged for it.

use bytes::Bytes;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

/// Consecutive fully-admissible batches before a family latches into
/// bulk mode (8 × 1024 ops ≈ 8k entries at the slipstream batch size —
/// cheap to reach on a real hydrate, never reached by incidental
/// ascending pairs).
pub(crate) const BULK_LATCH_STREAK: u32 = 8;

/// Adjacent inversions allowed in an otherwise-append batch before the
/// family is killed (RFC-0159 P2.1). A real descent (min key ≤
/// high-water) or a duplicate still kills. The write path sorts an
/// admissible nearly-sorted span before [`crate::bulk_run::BulkRun`].
pub(crate) const BULK_NEARLY_SORTED_WINDOW: usize = 8;

/// Kill switch for the whole fast path (`PEDRA_BULK=0`): every batch
/// routes `Ladder` and the latch never engages. A/B and rollback lever.
pub(crate) fn bulk_enabled() -> bool {
    match std::env::var("PEDRA_BULK") {
        Ok(v) => v.trim() != "0",
        Err(_) => true,
    }
}

/// One family-resolved op, borrowed from the batch being classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BulkOp<'a> {
    Put {
        family: &'a str,
        key: &'a [u8],
    },
    Delete {
        family: &'a str,
        key: &'a [u8],
    },
    /// Range-delete; `start` and `end` may resolve to different families
    /// (a span), in which case **both** families route ladder for the
    /// batch.
    DeleteRange {
        start_family: &'a str,
        start: &'a [u8],
        end_family: &'a str,
        end: &'a [u8],
    },
}

impl<'a> BulkOp<'a> {
    /// Families this op touches.
    fn families(&self) -> (&'a str, Option<&'a str>) {
        match self {
            BulkOp::Put { family, .. } | BulkOp::Delete { family, .. } => (family, None),
            BulkOp::DeleteRange {
                start_family,
                end_family,
                ..
            } => (start_family, Some(end_family)),
        }
    }
}

/// Where one family's ops in one batch go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FamilyRoute {
    /// Normal path (memtable → flush → ladder). Unchanged behavior.
    Ladder,
    /// Append-only fast path (P0.2 builder + disjoint bottom install).
    Bulk,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FamilyState {
    /// Ascending stream observed; `streak` consecutive admissible
    /// batches so far. Ladder until the streak reaches the threshold.
    Probing { streak: u32 },
    /// Append-only confirmed: admissible batches route `Bulk`.
    Latched,
    /// A violation was observed; this family never bulks again this
    /// session.
    Dead,
}

/// Per-Db sorted-ingest latch (one per write path that sees **all**
/// writes; enabling it on a Db with unclassified writers is a wiring
/// bug, not a state this type can detect).
#[derive(Debug, Default)]
pub(crate) struct BulkLatch {
    /// Admissible-batch streak before `Latched`.
    threshold: u32,
    state: HashMap<String, FamilyState>,
    /// Max user key ever observed for the family, across all routes.
    high_water: HashMap<String, Bytes>,
    /// Diagnostics counters (`PEDRA_BULK_DIAG`).
    pub(crate) bulk_batches: u64,
    pub(crate) ladder_batches: u64,
    pub(crate) killed: u64,
}

impl BulkLatch {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            threshold: BULK_LATCH_STREAK,
            ..Self::default()
        }
    }

    /// Test constructor with a smaller latch threshold.
    #[cfg(test)]
    #[must_use]
    fn with_threshold(threshold: u32) -> Self {
        Self {
            threshold,
            ..Self::default()
        }
    }

    /// Whether the family's append-only streak reached the threshold
    /// (qualifying flush spans may install at the bottom level).
    #[must_use]
    pub(crate) fn is_latched(&self, family: &str) -> bool {
        self.state.get(family) == Some(&FamilyState::Latched)
    }

    /// First observation of `family` still needs `family_max_in_db`.
    #[must_use]
    pub(crate) fn has_high_water(&self, family: &str) -> bool {
        self.high_water.contains_key(family)
    }

    fn kill(&mut self, family: &str) {
        if self.state.get(family) != Some(&FamilyState::Dead) {
            self.killed += 1;
        }
        self.state.insert(family.to_owned(), FamilyState::Dead);
    }

    /// Ratchet the family's high-water over `key` (every observed key,
    /// whatever routed it, must gate future bulk puts).
    fn ratchet(&mut self, family: &str, key: &[u8]) {
        match self.high_water.entry(family.to_owned()) {
            Entry::Occupied(mut e) => {
                if e.get().as_ref() < key {
                    e.insert(Bytes::copy_from_slice(key));
                }
            }
            Entry::Vacant(e) => {
                e.insert(Bytes::copy_from_slice(key));
            }
        }
    }

    /// Classify one batch. `family_max_in_db` answers "the largest user
    /// key this family holds on disk/in memtable right now" and is
    /// consulted only on a family's **first** observation (later batches
    /// are checked against the latch's own high-water, which covers
    /// everything observed since). Returns the route for every family
    /// present in the batch.
    pub(crate) fn classify_batch<'a>(
        &mut self,
        ops: &[BulkOp<'a>],
        family_max_in_db: &dyn Fn(&str) -> Option<Bytes>,
    ) -> HashMap<&'a str, FamilyRoute> {
        // Pass 1 (no mutation): families present and each family's ops
        // in batch order; family-spanning range-deletes are flagged (the
        // touched families cannot bulk this batch). Verdicts in pass 2
        // are judged against the high-water as of the *previous* batch;
        // this batch's keys ratchet it afterwards.
        let mut families: Vec<&str> = Vec::with_capacity(4);
        let mut per_family: HashMap<&str, Vec<(bool, &[u8])>> = HashMap::new();
        let mut spanning: HashSet<&str> = HashSet::new();
        for op in ops {
            let (f1, f2) = op.families();
            if !families.contains(&f1) {
                families.push(f1);
            }
            if let Some(f2v) = f2 {
                if !families.contains(&f2v) {
                    families.push(f2v);
                }
                if f2v != f1 {
                    // A family-spanning range delete: neither side can
                    // bulk this batch.
                    spanning.insert(f1);
                    spanning.insert(f2v);
                }
            }
            match op {
                BulkOp::Put { family, key } => {
                    per_family.entry(family).or_default().push((true, key));
                }
                BulkOp::Delete { family, key } => {
                    per_family.entry(family).or_default().push((false, key));
                }
                BulkOp::DeleteRange {
                    start_family,
                    start,
                    end_family,
                    end,
                } => {
                    per_family
                        .entry(start_family)
                        .or_default()
                        .push((false, start));
                    per_family.entry(end_family).or_default().push((false, end));
                }
            }
        }

        let mut routes = HashMap::with_capacity(families.len());
        for family in families {
            let empty: &[(bool, &[u8])] = &[];
            let ops_f = per_family.get(family).map_or(empty, Vec::as_slice);
            let route = self.classify_family(family, ops_f, spanning.contains(family), || {
                family_max_in_db(family)
            });
            routes.insert(family, route);
        }
        routes
    }

    /// Verdict + state transition for one family's ops within a batch.
    /// `ops` is `(is_put, key)` in batch order. After the verdict the
    /// high-water ratchets over every observed key of the family.
    pub(crate) fn classify_family(
        &mut self,
        family: &str,
        ops: &[(bool, &[u8])],
        spanning: bool,
        family_max_in_db: impl FnOnce() -> Option<Bytes>,
    ) -> FamilyRoute {
        // A family absent from the state map starts probing (streak 0);
        // its first batch is judged against the on-disk maximum inside
        // `judge`.
        let state = self
            .state
            .get(family)
            .cloned()
            .unwrap_or(FamilyState::Probing { streak: 0 });
        let verdict = self.judge(family, &state, ops, spanning, family_max_in_db);
        // Ratchet over every observed key regardless of the verdict.
        for (_, key) in ops {
            self.ratchet(family, key);
        }
        let was_latched = state == FamilyState::Latched;
        match verdict {
            Verdict::Admissible => {
                let new_state = match state {
                    FamilyState::Dead => FamilyState::Dead,
                    FamilyState::Latched => FamilyState::Latched,
                    FamilyState::Probing { streak } => {
                        let streak = streak + 1;
                        if streak >= self.threshold {
                            FamilyState::Latched
                        } else {
                            FamilyState::Probing { streak }
                        }
                    }
                };
                let bulk = matches!(new_state, FamilyState::Latched);
                self.state.insert(family.to_owned(), new_state);
                if bulk {
                    self.bulk_batches += 1;
                    FamilyRoute::Bulk
                } else {
                    self.ladder_batches += 1;
                    FamilyRoute::Ladder
                }
            }
            Verdict::Ineligible => {
                // Deletes / spanning ranges / mixed: ladder this batch.
                // A probing family restarts its streak; a latched family
                // stays latched (a delete does not break append-above).
                if !was_latched {
                    self.state
                        .insert(family.to_owned(), FamilyState::Probing { streak: 0 });
                }
                self.ladder_batches += 1;
                FamilyRoute::Ladder
            }
            Verdict::Kill => {
                self.kill(family);
                self.ladder_batches += 1;
                FamilyRoute::Ladder
            }
        }
    }

    /// Latched-family happy path: keys are already owned `Bytes`.
    /// Admissible span ratchets **only the last key** (high-water is
    /// monotone). Descent / duplicate / below-water falls through to
    /// [`Self::classify_family`] so the kill/ratchet contract stays one
    /// implementation.
    pub(crate) fn observe_latched_span(&mut self, family: &str, keys: &[Bytes]) -> FamilyRoute {
        if !self.is_latched(family) {
            let ops: Vec<(bool, &[u8])> = keys.iter().map(|k| (true, k.as_ref())).collect();
            return self.classify_family(family, &ops, false, || None);
        }
        if keys.is_empty() {
            return FamilyRoute::Bulk;
        }
        let mut ok = true;
        for w in keys.windows(2) {
            if w[1].as_ref() <= w[0].as_ref() {
                ok = false;
                break;
            }
        }
        if ok {
            if let Some(hw) = self.high_water.get(family) {
                if keys[0].as_ref() <= hw.as_ref() {
                    ok = false;
                }
            }
        }
        if !ok {
            let ops: Vec<(bool, &[u8])> = keys.iter().map(|k| (true, k.as_ref())).collect();
            return self.classify_family(family, &ops, false, || None);
        }
        if let Some(last) = keys.last() {
            self.ratchet(family, last.as_ref());
        }
        self.bulk_batches += 1;
        FamilyRoute::Bulk
    }

    /// Pure judgement of one family's batch slice against its state.
    fn judge(
        &self,
        family: &str,
        state: &FamilyState,
        ops: &[(bool, &[u8])],
        spanning: bool,
        family_max_in_db: impl FnOnce() -> Option<Bytes>,
    ) -> Verdict {
        if matches!(state, FamilyState::Dead) {
            return Verdict::Ineligible;
        }
        if spanning {
            return Verdict::Ineligible;
        }
        // All puts. Strictly ascending is the fast path; a bounded
        // adjacent-inversion window (RFC-0159 P2.1) still admits if the
        // unique sorted span sits strictly above high-water. Duplicates
        // and a real descent (min ≤ high-water) still kill.
        let mut prev: Option<&[u8]> = None;
        let mut inversions = 0usize;
        for &(is_put, key) in ops {
            if !is_put {
                return Verdict::Ineligible;
            }
            if let Some(p) = prev {
                if key == p {
                    return Verdict::Kill;
                }
                if key < p {
                    inversions = inversions.saturating_add(1);
                    if inversions > BULK_NEARLY_SORTED_WINDOW {
                        return Verdict::Kill;
                    }
                }
            }
            prev = Some(key);
        }
        if inversions > 0 && keys_have_duplicate(ops) {
            return Verdict::Kill;
        }
        let Some(first) = ops.iter().map(|&(_, k)| k).min() else {
            // No ops for this family in this batch (only possible via a
            // spanning range-delete naming it): ladder.
            return Verdict::Ineligible;
        };
        // Strictly above everything observed before this batch — or, on
        // first observation, above the family's pre-existing data.
        match self.high_water.get(family) {
            Some(hw) => {
                if first <= hw.as_ref() {
                    return Verdict::Kill;
                }
            }
            None => {
                if let Some(max) = family_max_in_db() {
                    if first <= max.as_ref() {
                        // The stream starts inside existing data — not
                        // append-above; never bulk this family.
                        return Verdict::Kill;
                    }
                }
            }
        }
        Verdict::Admissible
    }
}

enum Verdict {
    Admissible,
    Ineligible,
    Kill,
}

fn keys_have_duplicate(ops: &[(bool, &[u8])]) -> bool {
    let mut keys: Vec<&[u8]> = ops.iter().map(|&(_, k)| k).collect();
    keys.sort_unstable();
    keys.windows(2).any(|w| w[0] == w[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put<'a>(family: &'a str, key: &'a [u8]) -> BulkOp<'a> {
        BulkOp::Put { family, key }
    }

    fn del<'a>(family: &'a str, key: &'a [u8]) -> BulkOp<'a> {
        BulkOp::Delete { family, key }
    }

    fn route_of(routes: &HashMap<&str, FamilyRoute>, family: &str) -> FamilyRoute {
        *routes
            .get(family)
            .unwrap_or_else(|| panic!("family {family} absent from routes"))
    }

    fn no_db_max(_: &str) -> Option<Bytes> {
        None
    }

    #[test]
    fn observe_latched_span_ratchets_last_and_kills_on_descent() {
        let mut latch = BulkLatch::with_threshold(1);
        let a = Bytes::from_static(b"a");
        let b = Bytes::from_static(b"b");
        let c = Bytes::from_static(b"c");
        assert_eq!(
            latch.classify_family("data", &[(true, b"a".as_ref())], false, || None),
            FamilyRoute::Bulk
        );
        assert_eq!(
            latch.observe_latched_span("data", &[b.clone(), c.clone()]),
            FamilyRoute::Bulk
        );
        assert_eq!(
            latch.high_water.get("data").map(Bytes::as_ref),
            Some(b"c".as_ref())
        );
        assert_eq!(
            latch.observe_latched_span("data", &[a.clone()]),
            FamilyRoute::Ladder
        );
        assert!(!latch.is_latched("data"));
    }

    #[test]
    fn latch_engages_after_streak_of_ascending_batches() {
        let mut latch = BulkLatch::with_threshold(3);
        for i in 0..5u32 {
            let k0 = format!("k{i}-a");
            let k1 = format!("k{i}-b");
            let routes = latch.classify_batch(
                &[put("data", k0.as_bytes()), put("data", k1.as_bytes())],
                &no_db_max,
            );
            let expected = if i >= 2 {
                FamilyRoute::Bulk
            } else {
                FamilyRoute::Ladder
            };
            assert_eq!(route_of(&routes, "data"), expected, "batch {i}");
        }
    }

    #[test]
    fn duplicate_key_within_batch_kills_family() {
        let mut latch = BulkLatch::with_threshold(1);
        let routes = latch.classify_batch(&[put("data", b"k1"), put("data", b"k1")], &no_db_max);
        assert_eq!(route_of(&routes, "data"), FamilyRoute::Ladder);
        let routes = latch.classify_batch(&[put("data", b"k9")], &no_db_max);
        assert_eq!(route_of(&routes, "data"), FamilyRoute::Ladder);
    }

    #[test]
    fn descent_across_batches_kills_family() {
        let mut latch = BulkLatch::with_threshold(1);
        latch.classify_batch(&[put("data", b"k5")], &no_db_max);
        // k4 < k5: the stream went backwards.
        let routes = latch.classify_batch(&[put("data", b"k4")], &no_db_max);
        assert_eq!(route_of(&routes, "data"), FamilyRoute::Ladder);
        // Dead is permanent even for a later above-water key.
        let routes = latch.classify_batch(&[put("data", b"k6")], &no_db_max);
        assert_eq!(route_of(&routes, "data"), FamilyRoute::Ladder);
    }

    #[test]
    fn equal_key_across_batches_kills_family() {
        let mut latch = BulkLatch::with_threshold(1);
        latch.classify_batch(&[put("meta", b"cursor")], &no_db_max);
        // The slipstream shape: the same cursor key every batch.
        let routes = latch.classify_batch(&[put("meta", b"cursor")], &no_db_max);
        assert_eq!(route_of(&routes, "meta"), FamilyRoute::Ladder);
    }

    #[test]
    fn delete_routes_ladder_but_keeps_latched_family_alive() {
        let mut latch = BulkLatch::with_threshold(1);
        latch.classify_batch(&[put("data", b"k1")], &no_db_max);
        let routes = latch.classify_batch(&[del("data", b"k1")], &no_db_max);
        assert_eq!(route_of(&routes, "data"), FamilyRoute::Ladder);
        // Still latched: the next ascending put bulks again.
        let routes = latch.classify_batch(&[put("data", b"k2")], &no_db_max);
        assert_eq!(route_of(&routes, "data"), FamilyRoute::Bulk);
    }

    #[test]
    fn delete_in_probing_batch_resets_streak() {
        let mut latch = BulkLatch::with_threshold(2);
        latch.classify_batch(&[put("data", b"k1")], &no_db_max);
        latch.classify_batch(&[del("data", b"k0")], &no_db_max);
        // Streak restarted: this is admissible batch 1 of 2.
        let routes = latch.classify_batch(&[put("data", b"k2")], &no_db_max);
        assert_eq!(route_of(&routes, "data"), FamilyRoute::Ladder);
        let routes = latch.classify_batch(&[put("data", b"k3")], &no_db_max);
        assert_eq!(route_of(&routes, "data"), FamilyRoute::Bulk);
    }

    #[test]
    fn spanning_range_delete_routes_both_families_ladder() {
        let mut latch = BulkLatch::with_threshold(1);
        latch.classify_batch(&[put("a", b"k1"), put("b", b"k1")], &no_db_max);
        let routes = latch.classify_batch(
            &[BulkOp::DeleteRange {
                start_family: "a",
                start: b"k1",
                end_family: "b",
                end: b"k9",
            }],
            &no_db_max,
        );
        assert_eq!(route_of(&routes, "a"), FamilyRoute::Ladder);
        assert_eq!(route_of(&routes, "b"), FamilyRoute::Ladder);
        // Both stay latched (spanning deletes are not stream kills); the
        // next puts must clear each family's ratcheted high-water (b's
        // was ratcheted to the range end k9 by the ineligible batch).
        let routes = latch.classify_batch(&[put("a", b"k2"), put("b", b"z1")], &no_db_max);
        assert_eq!(route_of(&routes, "a"), FamilyRoute::Bulk);
        assert_eq!(route_of(&routes, "b"), FamilyRoute::Bulk);
    }

    #[test]
    fn first_batch_below_db_max_kills_family() {
        let mut latch = BulkLatch::with_threshold(1);
        let routes = latch.classify_batch(&[put("data", b"m5")], &|f| {
            assert_eq!(f, "data");
            Some(Bytes::copy_from_slice(b"m9"))
        });
        assert_eq!(route_of(&routes, "data"), FamilyRoute::Ladder);
        // Dead permanently.
        let routes = latch.classify_batch(&[put("data", b"z1")], &no_db_max);
        assert_eq!(route_of(&routes, "data"), FamilyRoute::Ladder);
    }

    #[test]
    fn high_water_ratchets_over_ineligible_batches() {
        let mut latch = BulkLatch::with_threshold(1);
        latch.classify_batch(&[put("data", b"k5")], &no_db_max);
        // A delete of a high key ratchets the high-water to k9 even
        // though the batch is ineligible.
        latch.classify_batch(&[del("data", b"k9")], &no_db_max);
        // k7 is above k5 but below the ratcheted k9: kill.
        let routes = latch.classify_batch(&[put("data", b"k7")], &no_db_max);
        assert_eq!(route_of(&routes, "data"), FamilyRoute::Ladder);
    }

    #[test]
    fn mixed_families_classify_independently() {
        // Threshold 2: the data stream latches on its second admissible
        // batch; the repeated meta cursor kills itself in probing before
        // it can ever bulk.
        let mut latch = BulkLatch::with_threshold(2);
        let routes = latch.classify_batch(
            &[
                put("data", b"k1"),
                put("data", b"k2"),
                put("meta", b"cursor"),
            ],
            &no_db_max,
        );
        assert_eq!(route_of(&routes, "data"), FamilyRoute::Ladder);
        assert_eq!(route_of(&routes, "meta"), FamilyRoute::Ladder);
        let routes =
            latch.classify_batch(&[put("data", b"k3"), put("meta", b"cursor")], &no_db_max);
        assert_eq!(route_of(&routes, "data"), FamilyRoute::Bulk);
        assert_eq!(route_of(&routes, "meta"), FamilyRoute::Ladder);
        // Meta is dead (repeated key); data keeps bulking.
        let routes =
            latch.classify_batch(&[put("data", b"k4"), put("meta", b"cursor")], &no_db_max);
        assert_eq!(route_of(&routes, "data"), FamilyRoute::Bulk);
        assert_eq!(route_of(&routes, "meta"), FamilyRoute::Ladder);
    }

    #[test]
    fn empty_batch_returns_no_routes() {
        let mut latch = BulkLatch::new();
        let routes = latch.classify_batch(&[], &no_db_max);
        assert!(routes.is_empty());
    }

    #[test]
    fn nearly_sorted_adjacent_swap_stays_latched() {
        let mut latch = BulkLatch::with_threshold(1);
        latch.classify_batch(&[put("data", b"k1")], &no_db_max);
        // One inversion inside the window: k3 then k2, both above k1.
        let routes = latch.classify_batch(&[put("data", b"k3"), put("data", b"k2")], &no_db_max);
        assert_eq!(route_of(&routes, "data"), FamilyRoute::Bulk);
        assert!(latch.is_latched("data"));
        // High-water ratchets over the max observed key (k3).
        assert_eq!(
            latch.high_water.get("data").map(Bytes::as_ref),
            Some(b"k3".as_ref())
        );
        let routes = latch.classify_batch(&[put("data", b"k4")], &no_db_max);
        assert_eq!(route_of(&routes, "data"), FamilyRoute::Bulk);
    }

    #[test]
    fn nearly_sorted_too_many_inversions_kills() {
        let mut latch = BulkLatch::with_threshold(1);
        latch.classify_batch(&[put("data", b"a")], &no_db_max);
        let mut ops = Vec::new();
        // 9 adjacent descents > BULK_NEARLY_SORTED_WINDOW (8).
        let keys: Vec<Vec<u8>> = (0..10u8).rev().map(|i| vec![b'z', i]).collect();
        for k in &keys {
            ops.push(put("data", k));
        }
        let routes = latch.classify_batch(&ops, &no_db_max);
        assert_eq!(route_of(&routes, "data"), FamilyRoute::Ladder);
        assert!(!latch.is_latched("data"));
    }

    #[test]
    fn nearly_sorted_dip_below_high_water_kills() {
        let mut latch = BulkLatch::with_threshold(1);
        latch.classify_batch(&[put("data", b"k5")], &no_db_max);
        // k4 < high-water k5: a real descent, not a within-batch shuffle.
        let routes = latch.classify_batch(&[put("data", b"k7"), put("data", b"k4")], &no_db_max);
        assert_eq!(route_of(&routes, "data"), FamilyRoute::Ladder);
        assert!(!latch.is_latched("data"));
    }
}
