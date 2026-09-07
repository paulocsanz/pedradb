//! Iterator window retain vs snapshot `visible_at` (RFC-0151 P1).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). No twin-cópia.
//!
//!   ./scripts/verus_iter_window.sh
//!
//! A windowed CF iterator must not emit a key the snapshot merge hid
//! (deletion / covering range tombstone). AS-IS keeps every row.
//!
//! The rustc bodies stay token-identical with the clone in
//! `pedradb_core::merge` (`catalog` `iter_window_merge`). Verus proofs
//! sit in the `cfg(verus_keep_ghost)` block above them (last-wins for
//! the clone lint is the rustc body).

#![forbid(unsafe_code)]

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

/// SI snapshot visibility at the iterator: keep iff the snapshot merge
/// marked the row live. Same class as lifting the deployed SI gate into
/// the exec function the binary calls (arxiv 2606.17182).
pub open spec fn iter_window_keep_spec(snapshot_live: bool) -> bool {
    snapshot_live
}

pub open spec fn iter_window_keep_as_is_spec(_snapshot_live: bool) -> bool {
    true
}

pub fn iter_window_keep(snapshot_live: bool) -> (d: bool)
    ensures
        d == iter_window_keep_spec(snapshot_live),
{
    snapshot_live
}

pub fn iter_window_keep_as_is(_snapshot_live: bool) -> (d: bool)
    ensures
        d == true,
{
    true
}

proof fn lemma_as_is_emits_hidden()
    ensures
        !iter_window_keep_spec(false),
        iter_window_keep_as_is_spec(false),
{
}

} // verus!

/// Keep a window row that snapshot merge marked live.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn iter_window_keep(snapshot_live: bool) -> bool {
    snapshot_live
}

/// AS-IS scan leak: emit a hidden version (deleted / range-covered).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn iter_window_keep_as_is(_snapshot_live: bool) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iter_window_keep_kernel_hides_snapshot_dead() {
        assert!(!iter_window_keep(false));
        assert!(
            iter_window_keep_as_is(false),
            "AS-IS dente: hidden row stays in the window"
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
