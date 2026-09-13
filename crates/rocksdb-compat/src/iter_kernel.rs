//! Iterator window retain vs snapshot `visible_at` (RFC-0151 P1).
//!
//! **Term:** this file is what `rustc` links. Aeneas extracts that body
//! (`scripts/aeneas_iter.sh`). A Verus cfg-split stand-in billed as last-wins
//! of rustc `iter_window_keep(bool)` is a model twin (deleted).
//!
//!   ./scripts/aeneas_iter.sh --required
//!
//! A windowed CF iterator must not emit a key the snapshot merge hid
//! (deletion / covering range tombstone). AS-IS keeps every row.
//!
//! The rustc bodies stay token-identical with the clone in
//! `pedradb_core::merge` (`catalog` `iter_window_merge`).
//! Aeneas of the rustc body is the term. A Verus stand-in is not last-wins.

#![forbid(unsafe_code)]

/// Keep a window row that snapshot merge marked live.
#[must_use]
pub fn iter_window_keep(snapshot_live: bool) -> bool {
    snapshot_live
}

/// AS-IS scan leak: emit a hidden version (deleted / range-covered).
#[must_use]
pub fn iter_window_keep_as_is(_snapshot_live: bool) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iter_kernel_has_no_verus_cartoon() {
        let src = include_str!("iter_kernel.rs");
        let block = concat!("verus", "!", " {");
        let cfg = concat!("cfg(", "verus", "_keep", "_ghost)");
        assert!(
            !src.contains(block),
            "cfg-split stand-in is not last-wins of rustc iter_window_keep"
        );
        assert!(
            !src.contains(cfg),
            "cfg split hides rustc types from the prover"
        );
    }

    #[test]
    fn iter_window_keep_kernel_hides_snapshot_dead() {
        assert!(!iter_window_keep(false));
        assert!(
            iter_window_keep_as_is(false),
            "AS-IS tooth: hidden row stays in the window"
        );
        assert!(iter_window_keep(true));
    }

    /// Clone twin (catalog `iter_window_merge`): both copies must implement
    /// the same function. This crate already depends on `pedradb-core`, so
    /// the twin lives here (a-side) and calls the core copy live; token
    /// identity (lint) catches one-sided drift, this sweep also catches
    /// both-sides drift at `cargo test` time.
    #[test]
    fn twin_agrees_with_core_merge_iter_window_on_full_domain() {
        use pedradb_core::merge as core;
        let mut checked = 0usize;
        for live in [false, true] {
            assert_eq!(
                iter_window_keep(live),
                core::iter_window_keep(live),
                "iter_window_keep({live})"
            );
            assert_eq!(
                iter_window_keep_as_is(live),
                core::iter_window_keep_as_is(live),
                "iter_window_keep_as_is({live})"
            );
            checked += 2;
        }
        // Identity + as-is teeth on THIS copy too.
        assert_eq!(iter_window_keep(false), false);
        assert_eq!(iter_window_keep(true), true);
        assert_eq!(iter_window_keep_as_is(false), true);
        assert_eq!(checked, 4);
    }
}
