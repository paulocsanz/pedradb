//! Pure scan-file relevance decision for SST range reads (F167).
//!
//! `SstTable::entries_in_user_range` fast-rejects whole files whose point keys
//! all fall outside `[start, end)` using only `smallest/largest_user_key`.
//! A range tombstone's **end key is stored as the entry value**, so it never
//! extends `largest_user_key`: a file whose point keys all precede the scan
//! window can still carry a tombstone spanning into it. Skipping such a file
//! lets covered keys scan as live while point `get` (which collects tombstones
//! unbounded) correctly hides them — a silent-wrong range read.
//!
//! Fix shape: a file must be read when its point bounds overlap the window
//! **or** any stored tombstone can straddle the window (end past its start,
//! start before its end — the second clause keeps the prune sharp for files
//! living after the window under a far-reaching tombstone).

use std::ops::Bound;

/// Whether a tombstone `[t_start, t_end)` can cover any key of `[start, end)`.
///
/// Both sides matter: the end must reach strictly past the window start
/// (half-open coverage), and the start must not lie beyond the window end.
#[must_use]
pub fn tombstone_reaches_window(
    t_start: &[u8],
    t_end: &[u8],
    start: Bound<&[u8]>,
    end: Bound<&[u8]>,
) -> bool {
    let reaches_start = match start {
        Bound::Unbounded => true,
        Bound::Included(s) | Bound::Excluded(s) => t_end > s,
    };
    let starts_before_end = match end {
        Bound::Unbounded => true,
        Bound::Included(e) => t_start <= e,
        Bound::Excluded(e) => t_start < e,
    };
    reaches_start && starts_before_end
}

/// AS-IS F167: file bounds only — tombstone span ignored, spanning files skipped.
#[must_use]
pub fn tombstone_reaches_window_as_is(
    _t_start: &[u8],
    _t_end: &[u8],
    _start: Bound<&[u8]>,
    _end: Bound<&[u8]>,
) -> bool {
    false
}

/// Whether a file with these point bounds may hold keys in `[start, end)`.
#[must_use]
pub fn point_bounds_overlap(
    smallest: Option<&[u8]>,
    largest: Option<&[u8]>,
    start: Bound<&[u8]>,
    end: Bound<&[u8]>,
) -> bool {
    let (Some(lo), Some(hi)) = (smallest, largest) else {
        return true;
    };
    let file_before_end = match end {
        Bound::Unbounded => true,
        Bound::Included(e) => lo <= e,
        Bound::Excluded(e) => lo < e,
    };
    let file_after_start = match start {
        Bound::Unbounded => true,
        Bound::Included(s) => hi >= s,
        Bound::Excluded(s) => hi > s,
    };
    file_before_end && file_after_start
}

/// Whether `[start, end)` contains `key` (same semantics as
/// [`crate::merge::user_key_in_range`]; model twin input).
#[must_use]
pub fn key_in_window(key: &[u8], start: Bound<&[u8]>, end: Bound<&[u8]>) -> bool {
    let after_start = match start {
        Bound::Unbounded => true,
        Bound::Included(s) => key >= s,
        Bound::Excluded(s) => key > s,
    };
    let before_end = match end {
        Bound::Unbounded => true,
        Bound::Included(e) => key <= e,
        Bound::Excluded(e) => key < e,
    };
    after_start && before_end
}

/// F167 kernel: must a scan of `[start, end)` read this file?
///
/// True when point bounds overlap the window or any tombstone can straddle it
/// (the tombstone start key is a point key in the file, so only the end can
/// escape `largest`; only a start beyond the window end can make straddling
/// impossible).
#[must_use]
pub fn scan_reads_file(
    smallest: Option<&[u8]>,
    largest: Option<&[u8]>,
    tombs: &[(&[u8], &[u8])],
    start: Bound<&[u8]>,
    end: Bound<&[u8]>,
) -> bool {
    if point_bounds_overlap(smallest, largest, start, end) {
        return true;
    }
    tombs
        .iter()
        .any(|&(t_start, t_end)| tombstone_reaches_window(t_start, t_end, start, end))
}

/// AS-IS F167: bounds-only fast-reject (spanning tombstones lost).
#[must_use]
pub fn scan_reads_file_as_is(
    smallest: Option<&[u8]>,
    largest: Option<&[u8]>,
    _tombs: &[(&[u8], &[u8])],
    start: Bound<&[u8]>,
    end: Bound<&[u8]>,
) -> bool {
    point_bounds_overlap(smallest, largest, start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spanning_tombstone_keeps_file() {
        // File points all before the window; tombstone [k-b, k-f) spans into it.
        let tombs: [(&[u8], &[u8]); 1] = [(b"k-b", b"k-f")];
        assert!(scan_reads_file(
            Some(b"k-b"),
            Some(b"k-b"),
            &tombs,
            Bound::Included(b"k-e"),
            Bound::Included(b"k-g"),
        ));
    }

    /// F167 teeth: AS-IS skips the spanning file (covered keys scan live).
    #[test]
    fn as_is_misses_spanning_tombstone() {
        let tombs: [(&[u8], &[u8]); 1] = [(b"k-b", b"k-f")];
        assert!(!scan_reads_file_as_is(
            Some(b"k-b"),
            Some(b"k-b"),
            &tombs,
            Bound::Included(b"k-e"),
            Bound::Included(b"k-g"),
        ));
    }

    #[test]
    fn disjoint_files_still_skipped() {
        // Window strictly after every point and after the tombstone end.
        let tombs: [(&[u8], &[u8]); 1] = [(b"k-b", b"k-f")];
        assert!(!scan_reads_file(
            Some(b"k-b"),
            Some(b"k-b"),
            &tombs,
            Bound::Included(b"k-g"),
            Bound::Included(b"k-z"),
        ));
        // Window strictly before every point AND before the tombstone start:
        // the tombstone covers nothing at/below the window end (sharp prune).
        let far: [(&[u8], &[u8]); 1] = [(b"k-b", b"k-z")];
        assert!(!scan_reads_file(
            Some(b"k-b"),
            Some(b"k-c"),
            &far,
            Bound::Included(b"k-a"),
            Bound::Included(b"k-a"),
        ));
    }

    #[test]
    fn half_open_boundaries() {
        // end == Included start: [.., k-e) covers nothing at/after k-e.
        assert!(!tombstone_reaches_window(
            b"k-b",
            b"k-e",
            Bound::Included(b"k-e"),
            Bound::Included(b"k-g"),
        ));
        assert!(tombstone_reaches_window(
            b"k-b",
            b"k-f",
            Bound::Included(b"k-e"),
            Bound::Included(b"k-g"),
        ));
        // Tombstone starts at the last window key (Included end): that key is
        // in the window and covered — must keep the file.
        assert!(tombstone_reaches_window(
            b"k-f",
            b"k-z",
            Bound::Included(b"k-a"),
            Bound::Included(b"k-f"),
        ));
        assert!(tombstone_reaches_window(
            b"k-f",
            b"k-z",
            Bound::Included(b"k-a"),
            Bound::Excluded(b"k-g"),
        ));
        // Tombstone starts strictly past every window key — prune.
        assert!(!tombstone_reaches_window(
            b"k-f",
            b"k-z",
            Bound::Included(b"k-a"),
            Bound::Excluded(b"k-f"),
        ));
        assert!(!tombstone_reaches_window(
            b"k-f",
            b"k-z",
            Bound::Included(b"k-a"),
            Bound::Included(b"k-e"),
        ));
        // Unbounded window end: only the start side can prune.
        assert!(tombstone_reaches_window(
            b"k-b",
            b"k-f",
            Bound::Included(b"k-e"),
            Bound::Unbounded,
        ));
        assert!(!tombstone_reaches_window(
            b"k-b",
            b"k-e",
            Bound::Included(b"k-e"),
            Bound::Unbounded,
        ));
    }

    #[test]
    fn unbounded_start_keeps_files_via_point_bounds() {
        // Unbounded start never rejects by the start side.
        assert!(point_bounds_overlap(
            Some(b"k-b"),
            Some(b"k-c"),
            Bound::Unbounded,
            Bound::Included(b"k-z"),
        ));
    }
}
