//! RFC-0199 counting ladder — Rust twin of `lsm_compact_work_bound`
//! (Lean: `formal/aeneas/lean/LsmCompactCount.lean`, count row
//! `catalog:lsm_compact`).
//!
//! The theorem's claim in Rust terms: `lsm_compact` is ONE-PASS over the
//! stored entries — work ≤ entries stored strictly below `depth` plus one
//! bookkeeping iteration per drained level, independent of how often
//! compact is called or how big the deeper (untouched) levels are.
//!
//! The REAL kernel is driven on every shape below; an in-test counted
//! mirror of the walk's index skeleton (one step per consumed entry, one
//! bookkeeping step per drained level — the same shape the Lean bridges
//! prove for the extract: every `cont` step increments the index by
//! exactly 1, and the drain loop steps down exactly one level at a time)
//! is checked against the bound. The mirror never reimplements the merge
//! — put/remove decisions stay inside the real kernel.
//!
//! Asserted on the REAL kernel (not the mirror): every level below
//! `depth` comes back drained (its entries were consumed exactly once),
//! levels above `depth` come back untouched, the merged level holds at
//! most `entries_below` entries, `next_seq` is unchanged, and the merged
//! content answers the newest version.

use pedradb_core::lsm_r1_kernel::{
    inv_lsm, lsm_compact, lsm_flush, lsm_probe, lsm_state_of, lsm_write, LsmState, MAX_LEVELS,
};

/// The theorem's bound input: entries stored strictly below `depth`
/// (`lsm_entries_below` in Lean).
fn entries_below(s: &LsmState, depth: usize) -> usize {
    (0..depth).map(|lvl| s.levels[lvl].len).sum()
}

/// The theorem's work twin: one step per consumed entry plus one
/// bookkeeping iteration per drained level (`lsm_compact_src_steps` in
/// Lean). Pure index arithmetic over the input state — no merge logic.
fn work_twin(s: &LsmState, depth: usize) -> usize {
    let mut steps = 0;
    let mut src_lvl = depth;
    while src_lvl > 0 {
        src_lvl -= 1;
        steps += 1; // the drain loop's own iteration
        steps += s.levels[src_lvl].len; // one pass over that level
    }
    steps
}

/// Drive the real kernel on one shape and assert the one-pass
/// consequences against the twin bound.
fn assert_one_pass(s: &LsmState, depth: usize) {
    let bound = entries_below(s, depth) + depth;
    assert_eq!(
        work_twin(s, depth),
        bound,
        "twin must equal the bound arithmetic (Lean proves it for all states)"
    );
    let Some(merged) = lsm_compact(s, depth) else {
        // capacity abort: the walk stopped EARLY, so real work < twin
        return;
    };
    assert!(inv_lsm(&merged), "merged state must satisfy Inv-LSM");
    for lvl in 0..depth {
        assert_eq!(
            merged.levels[lvl].len, 0,
            "level {lvl} below depth must be drained (entries consumed once)"
        );
    }
    for lvl in depth + 1..MAX_LEVELS {
        assert_eq!(
            merged.levels[lvl], s.levels[lvl],
            "level {lvl} above depth must be untouched"
        );
    }
    assert!(
        merged.levels[depth].len <= entries_below(s, depth),
        "merged size bounded by consumed entries"
    );
    assert_eq!(
        merged.next_seq, s.next_seq,
        "compact consumes no sequence numbers"
    );
}

#[test]
fn compact_count_bound_single_key_chain() {
    let s0 = lsm_state_of(1);
    let s1 = lsm_write(&s0, 7, false);
    let s2 = lsm_flush(&s1).expect("flush");
    assert_one_pass(&s2, 2);
    let merged = lsm_compact(&s2, 2).expect("compact");
    assert_eq!(lsm_probe(&merged, 7).map(|e| e.seq), Some(1));
}

#[test]
fn compact_count_bound_multi_key_multi_level() {
    // 2 distinct keys in L0 -> flush -> L1; then 2 more in L0; compact
    // depth 3 drains L2 (empty) + L1 + L0 — 4 distinct keys fit CAP.
    let mut s = lsm_state_of(1);
    for k in [10u64, 11] {
        s = lsm_write(&s, k, false);
    }
    s = lsm_flush(&s).expect("flush 1");
    for k in [20u64, 21] {
        s = lsm_write(&s, k, false);
    }
    assert!(inv_lsm(&s));
    assert_one_pass(&s, 3);
    let merged = lsm_compact(&s, 3).expect("compact depth 3");
    for k in [10u64, 11, 20, 21] {
        assert!(lsm_probe(&merged, k).is_some(), "key {k} survived");
    }
}

#[test]
fn compact_count_bound_deep_placement_house_shape() {
    // value pushed to L2 by hand (house pattern from the in-module tests),
    // tombstone in L0: compact 0..=1 keeps the tombstone.
    let s0 = lsm_state_of(1);
    let s1 = lsm_write(&s0, 7, false);
    let s2 = lsm_flush(&s1).expect("flush");
    let s3 = lsm_write(&s2, 7, true);
    let mut deep = s3;
    deep.levels[2] = deep.levels[1];
    deep.levels[1] = pedradb_core::lsm_r1_kernel::LsmLevel::empty();
    assert!(inv_lsm(&deep));
    assert_one_pass(&deep, 1);
    let merged = lsm_compact(&deep, 1).expect("compact 0..=1");
    assert_eq!(lsm_probe(&merged, 7).map(|e| e.tomb), Some(true));
}

#[test]
fn compact_count_bound_bottom_retires_tombstone() {
    let s0 = lsm_state_of(1);
    let s1 = lsm_write(&s0, 7, false);
    let mut s = s1;
    s.levels[MAX_LEVELS - 1] = s.levels[0];
    s.levels[0] = pedradb_core::lsm_r1_kernel::LsmLevel::empty();
    let s2 = lsm_write(&s, 7, true);
    assert!(inv_lsm(&s2));
    assert_one_pass(&s2, MAX_LEVELS - 1);
    let merged = lsm_compact(&s2, MAX_LEVELS - 1).expect("bottom compact");
    assert_eq!(lsm_probe(&merged, 7), None, "key fully retired");
}

#[test]
fn compact_count_capacity_abort_is_early_exit() {
    // 4 distinct keys in L0 + 1 different key already at L1: merging
    // needs 5 distinct slots > CAP, so the real walk aborts mid-pass —
    // real work strictly below the twin bound.
    let mut s = lsm_state_of(1);
    for k in [1u64, 2, 3] {
        s = lsm_write(&s, k, false);
    }
    s = lsm_flush(&s).expect("flush"); // L0 -> L1 (3 keys)
    s = lsm_write(&s, 9, false); // L0: 1 key
    // add one more distinct key to L1 by hand: total distinct 5 > CAP
    s.levels[1].entries[s.levels[1].len] = pedradb_core::lsm_r1_kernel::LsmEntry {
        key: 5,
        seq: 99,
        tomb: false,
    };
    s.levels[1].len += 1;
    assert_one_pass(&s, 1);
    assert!(
        lsm_compact(&s, 1).is_none(),
        "6 distinct keys cannot fit in one CAP-4 level: capacity abort"
    );
    // and the twin still counts the full (upper) bound:
    assert_eq!(work_twin(&s, 1), entries_below(&s, 1) + 1);
}

#[test]
fn compact_count_bound_empty_and_refused_depths() {
    let s = lsm_state_of(42);
    assert_one_pass(&s, 3);
    assert_eq!(lsm_compact(&s, 0), None, "depth 0 refused");
    assert_eq!(lsm_compact(&s, MAX_LEVELS), None, "depth >= MAX refused");
    let merged = lsm_compact(&s, 3).expect("empty compact");
    assert_eq!(merged, s, "compact of an empty stack is identity");
    assert_eq!(work_twin(&s, 3), 3, "only bookkeeping steps remain");
}

#[test]
fn compact_count_bound_overwrite_chain_one_slot() {
    // same key overwritten repeatedly: L0.len stays 1 — overwrites do not
    // grow the walk; the twin counts stored entries, not put calls.
    let mut s = lsm_state_of(1);
    for _ in 0..5 {
        s = lsm_write(&s, 7, false);
    }
    assert_eq!(s.levels[0].len, 1);
    assert_one_pass(&s, 1);
    let merged = lsm_compact(&s, 1).expect("compact");
    assert_eq!(lsm_probe(&merged, 7).map(|e| e.seq), Some(5));
}
