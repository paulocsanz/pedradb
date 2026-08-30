//! Iterator window retain vs snapshot `visible_at` (RFC-0151 P1).
//!
//! A windowed CF iterator must not emit a key the snapshot merge hid
//! (deletion / covering range tombstone). AS-IS keeps every row.

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
    fn iter_window_keep_kernel_hides_snapshot_dead() {
        assert!(!iter_window_keep(false));
        assert!(
            iter_window_keep_as_is(false),
            "AS-IS dente: hidden row stays in the window"
        );
        assert!(iter_window_keep(true));
    }
}
