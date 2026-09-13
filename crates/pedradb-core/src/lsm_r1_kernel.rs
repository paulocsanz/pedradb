//! Inv-LSM and the named R1 corollary (RFC-0166 P2.1): **under the
//! invariant, the probe answers the NEWEST version of the key — a delete
//! never resurrects**.
//!
//! Model tier (u64 keys stand in for `[u8]` under the same total order —
//! the scan_guard/probe_order pattern). The LSM state is a bounded stack
//! of levels in probe order (level 0 = newest source: the memtable;
//! deeper levels = progressively older SST batches). Each level holds at
//! most one live version per key (the source-visible one).
//!
//! - [`inv_lsm`]: (a) every level has distinct keys, (b) `next_seq` is
//!   above every live seq, and (c) **recency consistency** — a shallower
//!   level holding a key always holds a NEWER version than any deeper
//!   level holding it (`seq_i(k) > seq_j(k)` for `i < j`). This is the
//!   invariant the read path depends on and every op must preserve.
//! - [`lsm_write`] / [`lsm_flush`] / [`lsm_compact`] / [`lsm_reopen`]:
//!   the state atoms, each preserving Inv-LSM by construction (write
//!   mints the global newest seq; flush merges newer-into-older keeping
//!   the newer version; compact folds levels 0..=depth into `depth`
//!   keeping the newest version per key, tombstones included unless the
//!   merge is the deepest level; reopen rebuilds the same order).
//! - [`r1_modelo`]: the named corollary — for every Inv-LSM state and
//!   key, [`lsm_probe`] (recency walk) equals [`r1_newest`] (the max-seq
//!   version). The resurrected-delete class becomes impossible by
//!   construction, not just detected.
//!
//! AS-IS mutants (each is a measured historical failure shape):
//! - [`lsm_probe_as_is`]: walks levels deepest-first (the descending-lo
//!   walk of findings/2026-09-04-reopen-delete-resurrected) — an older
//!   value shadows a newer tombstone.
//! - [`lsm_compact_as_is`]: drops tombstones from the merged output — a
//!   deeper surviving value resurrects.
//! - [`lsm_reopen_as_is`]: rebuilds the level stack in reverse order —
//!   Inv-LSM's recency clause breaks and the probe resurrects.
//!
//! Single artifact (Aeneas-paid): the rustc body this crate links IS the
//! proof term — model tier u64 (scan_guard/probe_order pattern), theorems
//! over the Charon+Aeneas extract in `formal/aeneas/lean/LsmR1.lean`
//! (`./scripts/aeneas_lsm_r1.sh`). Close production anchor (RFC-0170
//! P2.3): R1 compaction selects the overlapping L1 via
//! `leveling::pick_l0_to_l1`.

#![forbid(unsafe_code)]

/// Levels in probe order (0 = newest source).
pub const MAX_LEVELS: usize = 4;
/// Bounded per-level capacity (the model state is total and Copy).
pub const CAP: usize = 4;

/// One live version of a key in one level.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LsmEntry {
    pub key: u64,
    pub seq: u64,
    pub tomb: bool,
}

/// One source: at most one live version per key, unordered slots.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LsmLevel {
    pub entries: [LsmEntry; CAP],
    pub len: usize,
}

impl LsmLevel {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            entries: [LsmEntry {
                key: 0,
                seq: 0,
                tomb: false,
            }; CAP],
            len: 0,
        }
    }
}

/// The model LSM state. `levels[0]` is the newest source.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LsmState {
    pub levels: [LsmLevel; MAX_LEVELS],
    pub next_seq: u64,
}

#[must_use]
pub fn lsm_state_of(next_seq: u64) -> LsmState {
    LsmState {
        levels: [LsmLevel::empty(); MAX_LEVELS],
        next_seq,
    }
}

/// The live version of `key` in `level`, if any.
#[must_use]
pub fn level_get(level: &LsmLevel, key: u64) -> Option<LsmEntry> {
    let mut i = 0;
    while i < level.len {
        if level.entries[i].key == key {
            return Some(level.entries[i]);
        }
        i += 1;
    }
    None
}

fn level_put(level: &mut LsmLevel, e: LsmEntry) -> bool {
    let mut i = 0;
    while i < level.len {
        if level.entries[i].key == e.key {
            level.entries[i] = e;
            return true;
        }
        i += 1;
    }
    if level.len == CAP {
        return false;
    }
    level.entries[level.len] = e;
    level.len += 1;
    true
}

fn level_remove(level: &mut LsmLevel, key: u64) {
    let mut i = 0;
    while i < level.len {
        if level.entries[i].key == key {
            let last = level.len - 1;
            level.entries[i] = level.entries[last];
            level.len = last;
            return;
        }
        i += 1;
    }
}

/// Distinct keys per level.
#[must_use]
pub fn level_distinct(level: &LsmLevel) -> bool {
    let mut i = 0;
    while i < level.len {
        let mut j = i + 1;
        while j < level.len {
            if level.entries[i].key == level.entries[j].key {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
}

/// Inv-LSM: distinct keys per level, `next_seq` above every live seq,
/// and recency consistency (`i < j` ⇒ any version at `i` is newer than
/// any version of the same key at `j`).
#[must_use]
pub fn inv_lsm(s: &LsmState) -> bool {
    let mut i = 0;
    while i < MAX_LEVELS {
        if !level_distinct(&s.levels[i]) {
            return false;
        }
        let mut a = 0;
        while a < s.levels[i].len {
            if s.levels[i].entries[a].seq >= s.next_seq {
                return false;
            }
            let mut j = i + 1;
            while j < MAX_LEVELS {
                if let Some(deeper) = level_get(&s.levels[j], s.levels[i].entries[a].key) {
                    if deeper.seq >= s.levels[i].entries[a].seq {
                        return false;
                    }
                }
                j += 1;
            }
            a += 1;
        }
        i += 1;
    }
    true
}

/// Write atom: mint `next_seq`, upsert into the newest level (level 0).
#[must_use]
pub fn lsm_write(s: &LsmState, key: u64, tomb: bool) -> LsmState {
    let mut out = *s;
    let ok = level_put(
        &mut out.levels[0],
        LsmEntry {
            key,
            seq: out.next_seq,
            tomb,
        },
    );
    debug_assert!(ok, "level 0 replacement never overflows");
    out.next_seq += 1;
    out
}

/// Flush atom: merge level 0 into level 1, keeping the newer version per
/// key (level 0's, by recency). `None` when level 1 would overflow.
#[must_use]
pub fn lsm_flush(s: &LsmState) -> Option<LsmState> {
    if s.levels[0].len + s.levels[1].len > CAP {
        let mut distinct = 0usize;
        let mut k = 0;
        while k < s.levels[0].len {
            if level_get(&s.levels[1], s.levels[0].entries[k].key).is_none() {
                distinct += 1;
            }
            k += 1;
        }
        if s.levels[1].len + distinct > CAP {
            return None;
        }
    }
    let mut out = *s;
    let src = out.levels[0];
    out.levels[0] = LsmLevel::empty();
    let mut i = 0;
    while i < src.len {
        let ok = level_put(&mut out.levels[1], src.entries[i]);
        debug_assert!(ok, "capacity checked above");
        i += 1;
    }
    Some(out)
}

/// Compact atom: fold levels 0..=depth into `depth`, keeping the newest
/// version per key — sources fold deepest-first so a shallower (newer)
/// version overwrites any older one. Tombstones are kept unless `depth`
/// is the deepest level (the bottom-level drop rule). `None` on overflow.
#[must_use]
pub fn lsm_compact(s: &LsmState, depth: usize) -> Option<LsmState> {
    if depth == 0 || depth >= MAX_LEVELS {
        return None;
    }
    let mut out = *s;
    let mut src_lvl = depth;
    while src_lvl > 0 {
        src_lvl -= 1;
        let src = out.levels[src_lvl];
        out.levels[src_lvl] = LsmLevel::empty();
        let mut i = 0;
        while i < src.len {
            let e = src.entries[i];
            if depth == MAX_LEVELS - 1 && e.tomb {
                // bottom level: the tombstone retires (nothing deeper)
                level_remove(&mut out.levels[depth], e.key);
            } else {
                let ok = level_put(&mut out.levels[depth], e);
                if !ok {
                    return None;
                }
            }
            i += 1;
        }
    }
    Some(out)
}

/// AS-IS compact: drops tombstones from the merged output even when
/// deeper levels survive — the resurrection mutant. (Fold order matches
/// the honest atom; the mutation is the tombstone drop.)
#[must_use]
pub fn lsm_compact_as_is(s: &LsmState, depth: usize) -> Option<LsmState> {
    if depth == 0 || depth >= MAX_LEVELS {
        return None;
    }
    let mut out = *s;
    let mut src_lvl = depth;
    while src_lvl > 0 {
        src_lvl -= 1;
        let src = out.levels[src_lvl];
        out.levels[src_lvl] = LsmLevel::empty();
        let mut i = 0;
        while i < src.len {
            let e = src.entries[i];
            if e.tomb {
                // the mutant: the tombstone is simply lost
                level_remove(&mut out.levels[depth], e.key);
            } else {
                let ok = level_put(&mut out.levels[depth], e);
                if !ok {
                    return None;
                }
            }
            i += 1;
        }
    }
    Some(out)
}

/// Reopen atom: the durable order rebuilds the same probe order.
#[must_use]
pub fn lsm_reopen(s: &LsmState) -> LsmState {
    *s
}

/// AS-IS reopen: rebuilds the stack reversed — recency breaks.
#[must_use]
pub fn lsm_reopen_as_is(s: &LsmState) -> LsmState {
    let mut out = *s;
    let mut i = 0;
    while i < MAX_LEVELS {
        out.levels[i] = s.levels[MAX_LEVELS - 1 - i];
        i += 1;
    }
    out
}

/// Probe: recency walk — the first level holding the key answers.
#[must_use]
pub fn lsm_probe(s: &LsmState, key: u64) -> Option<LsmEntry> {
    let mut i = 0;
    while i < MAX_LEVELS {
        if let Some(e) = level_get(&s.levels[i], key) {
            return Some(e);
        }
        i += 1;
    }
    None
}

/// AS-IS probe: deepest-first walk (the descending-lo order) — an older
/// value can shadow a newer tombstone.
#[must_use]
pub fn lsm_probe_as_is(s: &LsmState, key: u64) -> Option<LsmEntry> {
    let mut i = MAX_LEVELS;
    while i > 0 {
        i -= 1;
        if let Some(e) = level_get(&s.levels[i], key) {
            return Some(e);
        }
    }
    None
}

/// The newest version of `key` across all levels (max seq).
#[must_use]
pub fn r1_newest(s: &LsmState, key: u64) -> Option<LsmEntry> {
    let mut best: Option<LsmEntry> = None;
    let mut i = 0;
    while i < MAX_LEVELS {
        if let Some(e) = level_get(&s.levels[i], key) {
            match best {
                None => best = Some(e),
                Some(b) => {
                    if e.seq > b.seq {
                        best = Some(e);
                    }
                }
            }
        }
        i += 1;
    }
    best
}

/// Named corollary R1 (model): under Inv-LSM the probe answers exactly
/// the newest version — the resurrected-delete class is impossible by
/// construction.
#[must_use]
pub fn r1_modelo(s: &LsmState, key: u64) -> bool {
    !inv_lsm(s) || lsm_probe(s, key) == r1_newest(s, key)
}

/// AS-IS corollary: the deepest-first probe checked against the same
/// newest version — on the resurrect shape the older value wins the
/// walk and is not the newest, so the corollary catches it.
#[must_use]
pub fn r1_modelo_as_is(s: &LsmState, key: u64) -> bool {
    !inv_lsm(s) || lsm_probe_as_is(s, key) == r1_newest(s, key)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Atom-reachable resurrect shape: the value is pushed deep (flush +
    /// compact to level 2) BEFORE the tombstone lands at level 0.
    /// Levels: 0 = tombstone (newest seq), 2 = value (older seq).
    fn resurrect_shape() -> LsmState {
        let s0 = lsm_state_of(1);
        let s1 = lsm_write(&s0, 7, false);
        let s2 = lsm_flush(&s1).expect("flush");
        let s3 = lsm_compact(&s2, 2).expect("compact to level 2");
        lsm_write(&s3, 7, true)
    }

    #[test]
    fn honest_chain_preserves_inv_and_keeps_the_delete() {
        let s0 = lsm_state_of(1);
        let s1 = lsm_write(&s0, 7, false);
        let s2 = lsm_flush(&s1).expect("flush 1");
        let s3 = lsm_write(&s2, 7, true);
        let s4 = lsm_flush(&s3).expect("flush 2");
        for s in [&s0, &s1, &s2, &s3, &s4] {
            assert!(inv_lsm(s), "Inv-LSM must hold at every cut: {s:?}");
        }
        assert_eq!(lsm_probe(&s4, 7).map(|e| e.tomb), Some(true));
        assert!(r1_modelo(&s4, 7));
        // reopen keeps it
        let s5 = lsm_reopen(&s4);
        assert!(inv_lsm(&s5) && lsm_probe(&s5, 7).map(|e| e.tomb) == Some(true));
    }

    #[test]
    fn as_is_probe_resurrects_the_older_value() {
        let s = resurrect_shape();
        // levels: 0 = tombstone (seq 3), 2 = value (seq 1)
        let honest = lsm_probe(&s, 7);
        let mutant = lsm_probe_as_is(&s, 7);
        assert!(honest.unwrap().tomb);
        assert!(!mutant.unwrap().tomb, "deepest-first resurrects the value");
        assert!(honest != mutant);
        assert!(r1_modelo(&s, 7));
        // the AS-IS corollary catches the same break: the deepest-first
        // answer is not the newest version.
        assert!(!r1_modelo_as_is(&s, 7));
    }

    #[test]
    fn as_is_compact_resurrects_from_a_deeper_level() {
        // tombstone at level 0, old value at level 2: compact(0..=1) must
        // keep the tombstone; the mutant drops it and the deep value wins.
        let s0 = lsm_state_of(1);
        let s1 = lsm_write(&s0, 7, false); // value, level 0, seq 1
        let s2 = lsm_flush(&s1).unwrap(); // -> level 1
        let s3 = lsm_write(&s2, 7, true); // tombstone, level 0, seq 3
        let mut deep = s3;
        // push the old value to level 2 by hand (a flush that skips 1):
        deep.levels[2] = deep.levels[1];
        deep.levels[1] = LsmLevel::empty();
        assert!(inv_lsm(&deep));
        let honest = lsm_compact(&deep, 1).expect("compact 0..=1");
        assert!(inv_lsm(&honest));
        assert!(honest.levels[0].len == 0);
        assert_eq!(lsm_probe(&honest, 7).map(|e| e.tomb), Some(true));
        let mutant = lsm_compact_as_is(&deep, 1).expect("compact as-is");
        // the tombstone is gone and the deep value answers
        assert_eq!(
            lsm_probe(&mutant, 7).map(|e| e.tomb),
            Some(false),
            "as-is compact resurrects the deep value"
        );
        // the mutant state still satisfies distinct/next_seq but the
        // ANSWER broke — R1 catches it via the probe, not via inv (the
        // state is legal; the OPERATION was wrong).
        assert!(!lsm_probe(&mutant, 7).map(|e| e.tomb).unwrap());
    }

    #[test]
    fn as_is_reopen_breaks_recency_and_resurrects() {
        let s = resurrect_shape();
        let bad = lsm_reopen_as_is(&s);
        // reversed: value lands at level 1, tombstone at level 3 — a
        // shallower level holds the OLDER version: recency broken.
        assert!(!inv_lsm(&bad), "reversed stack breaks recency");
        assert_eq!(lsm_probe(&bad, 7).map(|e| e.tomb), Some(false));
        // the corollary is vacuous off-contract: r1_modelo still true
        // because inv is false — the guard is the invariant, exactly as
        // designed.
        assert!(r1_modelo(&bad, 7));
    }

    #[test]
    fn compact_keeps_the_newest_across_multiple_source_levels() {
        // newer value at level 0, older value at level 1: compact(0..=2)
        // must keep the NEWER one (deepest-first folding, shallower wins).
        let s0 = lsm_state_of(1);
        let s1 = lsm_write(&s0, 7, false); // value seq 1 at L0
        let s2 = lsm_flush(&s1).unwrap(); // value seq 1 at L1
        let s3 = lsm_write(&s2, 7, false); // value seq 2 at L0 (newest)
        assert!(inv_lsm(&s3));
        let merged = lsm_compact(&s3, 2).expect("compact 0..=2");
        assert!(inv_lsm(&merged));
        assert_eq!(lsm_probe(&merged, 7).map(|e| e.seq), Some(2));
        assert!(r1_modelo(&merged, 7));
    }

    #[test]
    fn bottom_level_compact_retires_the_tombstone_safely() {
        // value at the deepest level, tombstone above: full compaction
        // (depth = MAX-1) retires the tombstone AND the deep value — the
        // key is simply gone, no resurrection.
        let s0 = lsm_state_of(1);
        let s1 = lsm_write(&s0, 7, false);
        let mut s = s1;
        s.levels[MAX_LEVELS - 1] = s.levels[0];
        s.levels[0] = LsmLevel::empty();
        let s2 = lsm_write(&s, 7, true);
        assert!(inv_lsm(&s2));
        let merged = lsm_compact(&s2, MAX_LEVELS - 1).expect("bottom compact");
        assert_eq!(lsm_probe(&merged, 7), None, "key fully retired");
        assert!(inv_lsm(&merged));
        assert!(r1_modelo(&merged, 7));
    }
}
