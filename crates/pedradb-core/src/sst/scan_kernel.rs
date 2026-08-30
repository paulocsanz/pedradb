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

/// Tiny SST files may predate the CRC trailer (RFC-0077).
pub const SST_LEGACY_NO_CRC_MAX: usize = 32;

/// What `SstTable::decode` does with a magic file's trailing CRC32C (RFC-0077).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SstCrcFate {
    /// Stored CRC matches: payload is the file minus the 4-byte trailer.
    StripTrailer,
    /// Mismatch on a tiny file: parse the whole buffer (no trailer).
    WholeBuffer,
    /// Mismatch on a modern file: reject. Never a table.
    Reject,
}

/// Admit the SST file CRC. Mismatch on a modern file is never a table.
///
/// AS-IS always [`SstCrcFate::StripTrailer`] (silent-wrong table).
#[must_use]
pub fn sst_crc_fate(stored: u32, computed: u32, buf_len: usize) -> SstCrcFate {
    if crate::wal::crc::crc_match_ok(stored, computed) {
        SstCrcFate::StripTrailer
    } else if buf_len < SST_LEGACY_NO_CRC_MAX {
        SstCrcFate::WholeBuffer
    } else {
        SstCrcFate::Reject
    }
}

/// AS-IS RFC-0077: any checksum is a match (corruption served as a table).
#[must_use]
pub fn sst_crc_fate_as_is(_stored: u32, _computed: u32, _buf_len: usize) -> SstCrcFate {
    SstCrcFate::StripTrailer
}

/// Admit a per-block data CRC (RFC-0077 P1.1). Same gate as the file trailer.
#[must_use]
pub fn sst_block_crc_ok(stored: u32, computed: u32) -> bool {
    crate::wal::crc::crc_match_ok(stored, computed)
}

/// AS-IS: any block checksum matches (corruption served as a block).
#[must_use]
pub fn sst_block_crc_ok_as_is(_stored: u32, _computed: u32) -> bool {
    true
}

/// RFC-0077 P2.2 / R-glue: zero remaining glue (handlers proven, `db.rs`
/// extracted). Always false. `sst_crc_fate` is cataloged; glue stays TCB.
#[must_use]
pub fn zero_glue_admitted() -> bool {
    false
}

/// AS-IS: extracting `sst_crc_fate` looks like glue is gone (the 0077 P2.2 hole).
#[must_use]
pub fn zero_glue_admitted_as_is() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC-0157 P1.2 — property sweep over `sst_crc_fate` /
    /// `sst_block_crc_ok`: seeded CRC pairs × boundary buffer lengths
    /// (including the legacy 32-byte threshold from both sides). Pins:
    /// match ⇒ StripTrailer at any length; mismatch on a tiny file ⇒
    /// WholeBuffer; mismatch on a modern file ⇒ Reject — a mismatch is
    /// NEVER StripTrailer (no silent-wrong table); block gate ==
    /// `crc_match_ok`.
    #[test]
    fn rfc0157_property_sweep_sst_crc_fate() {
        use crate::{Rng, SeedRng};
        // Concrete boundary: 31 is tiny, 32 is modern, both mismatched.
        assert_eq!(
            sst_crc_fate(0xDEAD_BEEF, 0x0BAD_C0DE, SST_LEGACY_NO_CRC_MAX - 1),
            SstCrcFate::WholeBuffer
        );
        assert_eq!(
            sst_crc_fate(0xDEAD_BEEF, 0x0BAD_C0DE, SST_LEGACY_NO_CRC_MAX),
            SstCrcFate::Reject
        );
        // Concrete AS-IS divergence tooth: mismatch served as a table.
        assert_eq!(
            sst_crc_fate_as_is(0xDEAD_BEEF, 0x0BAD_C0DE, 4096),
            SstCrcFate::StripTrailer,
            "AS-IS dente: any checksum is a match"
        );

        let mut viol: Option<String> = None;
        'trials: for trial in 0..20_000u64 {
            let rng = SeedRng::new(0x0157_77C3 ^ trial);
            let stored = (rng.next_u64() >> 32) as u32;
            let computed = if rng.gen_range(4) == 0 {
                stored // force real matches into the sweep
            } else {
                (rng.next_u64() >> 32) as u32
            };
            for buf_len in [0usize, 1, 31, 32, 33, 4096] {
                let fate = sst_crc_fate(stored, computed, buf_len);
                let expect = if stored == computed {
                    SstCrcFate::StripTrailer
                } else if buf_len < SST_LEGACY_NO_CRC_MAX {
                    SstCrcFate::WholeBuffer
                } else {
                    SstCrcFate::Reject
                };
                if fate != expect {
                    viol = Some(format!(
                        "trial={trial} stored={stored:08x} computed={computed:08x} len={buf_len} fate={fate:?}"
                    ));
                    break 'trials;
                }
                if stored != computed && fate == SstCrcFate::StripTrailer {
                    viol = Some(format!(
                        "trial={trial} SILENT-WRONG: mismatch len={buf_len} served as a table"
                    ));
                    break 'trials;
                }
            }
            if sst_block_crc_ok(stored, computed) != crate::wal::crc::crc_match_ok(stored, computed)
            {
                viol = Some(format!("trial={trial} block gate != crc_match_ok"));
                break 'trials;
            }
        }
        assert_eq!(viol, None, "rfc0157 sweep counterexample: {viol:?}");
    }

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

    /// Catalog three-teeth plant. Direct `as_is_misses_spanning_tombstone` is **not** this tooth.
    #[test]
    fn scan_reads_file_on_live_sst_is_not_ok() {
        let tombs: [(&[u8], &[u8]); 1] = [(b"k-b", b"k-f")];
        assert!(scan_reads_file(
            Some(b"k-b"),
            Some(b"k-b"),
            &tombs,
            Bound::Included(b"k-e"),
            Bound::Included(b"k-g"),
        ));
        assert!(
            !scan_reads_file_as_is(
                Some(b"k-b"),
                Some(b"k-b"),
                &tombs,
                Bound::Included(b"k-e"),
                Bound::Included(b"k-g"),
            ),
            "AS-IS dente: bounds-only skip of spanning tombstone file"
        );
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pedra-scan-guard-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut db = crate::Db::open_with(
            &dir,
            crate::OpenOptions {
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                ..crate::OpenOptions::default()
            },
        )
        .unwrap();
        db.set_defer_auto_compact(true);
        db.put(b"k-e", b"live").unwrap();
        db.flush().unwrap();
        db.put(b"k-b", b"start").unwrap();
        db.delete_range(b"k-b", b"k-f").unwrap();
        db.flush().unwrap();
        assert!(
            db.live_sst_meta().len() >= 2,
            "need a point SST and a spanning-tombstone SST, meta={:?}",
            db.live_sst_meta()
        );
        assert_eq!(
            db.get(b"k-e").as_deref(),
            None,
            "point get must see the spanning tombstone"
        );
        let scan: Vec<Vec<u8>> = db
            .range_limited(
                Bound::Included(b"k-e".as_ref()),
                Bound::Included(b"k-g".as_ref()),
                None,
            )
            .into_iter()
            .map(|(k, _)| k.to_vec())
            .collect();
        assert!(
            !scan.iter().any(|k| k.as_slice() == b"k-e"),
            "scan must hide k-e; AS-IS skip of the tombstone SST would leak it: {scan:?}"
        );
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
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
    fn sst_crc_mismatch_is_reject() {
        assert_eq!(sst_crc_fate(1, 1, 100), SstCrcFate::StripTrailer);
        assert_eq!(sst_crc_fate(1, 2, 100), SstCrcFate::Reject);
        assert_eq!(sst_crc_fate(1, 2, 16), SstCrcFate::WholeBuffer);
        assert_eq!(
            sst_crc_fate_as_is(1, 2, 100),
            SstCrcFate::StripTrailer,
            "AS-IS dente: ignore mismatch"
        );
    }

    /// RFC-0076 P1.1: SST file-trailer fate uses `crc_match_ok` (RFC-0077 P0).
    #[test]
    fn sst_file_crc_uses_crc_match_ok() {
        assert!(crate::wal::crc::crc_match_ok(1, 1));
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: ignore mismatch"
        );
        assert_eq!(sst_crc_fate(1, 1, 100), SstCrcFate::StripTrailer);
        assert_eq!(sst_crc_fate(1, 2, 100), SstCrcFate::Reject);
        assert_eq!(
            sst_crc_fate_as_is(1, 2, 100),
            SstCrcFate::StripTrailer,
            "AS-IS would strip a flipped trailer"
        );
    }

    /// RFC-0077 P1.1: per-block admit is `crc_match_ok` (no tiny-legacy path).
    #[test]
    fn sst_block_crc_uses_crc_match_ok() {
        assert!(sst_block_crc_ok(1, 1));
        assert!(!sst_block_crc_ok(1, 2));
        assert!(
            sst_block_crc_ok_as_is(1, 2),
            "AS-IS dente: ignore block mismatch"
        );
        assert_eq!(sst_block_crc_ok(7, 7), crate::wal::crc::crc_match_ok(7, 7));
    }

    /// RFC-0077 P2.2: cataloging `sst_crc_fate` is not zero glue.
    #[test]
    fn zero_glue_is_a_trajectory() {
        assert!(!zero_glue_admitted());
        assert!(
            zero_glue_admitted_as_is(),
            "AS-IS dente: extracting sst_crc_fate looks like glue is gone"
        );
        let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(
            crate_dir.join("verus/sst_crc_fate.rs").is_file(),
            "RFC-0077 P2.1: sst_crc_fate twin must exist"
        );
        assert!(
            crate_dir.join("verus/scan_guard.rs").is_file(),
            "RFC-0077 P2.1: scan_guard F167 twin must stay"
        );
        assert!(
            crate_dir.join("src/db.rs").is_file(),
            "RFC-0077 P2.2: do not extract db.rs"
        );
        let residuals = crate_dir.join("../../scripts/formal/residuals.json");
        let text = std::fs::read_to_string(&residuals).expect("residuals.json");
        assert!(
            text.contains("\"db_rs_extracted\": false"),
            "glue.db_rs_extracted must stay false"
        );
        assert!(
            text.contains("\"id\": \"R-glue\""),
            "R-glue remains a residual"
        );
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
