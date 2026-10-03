//! Pure pull/rotation decision for WAL shipping (F165).
//!
//! **Term:** this file is what `rustc` links. Aeneas extracts that body
//! (`scripts/aeneas_ship.sh`). A u64 fingerprint of rustc `&[u8]` stamps
//! is a model twin — not last-wins (deleted).
//!
//!   ./scripts/aeneas_ship.sh --required
//!
//! `Db::flush` rotates `CURRENT.log` by **truncating in place** (same path,
//! offset 0). A byte cursor alone cannot distinguish "grew" from "rotated and
//! regrew past the cursor": length-only detection ships misaligned bytes from
//! the fresh file and silently strands the records below the cursor — a stale
//! replica with no error.
//!
//! Fix shape: capture a **prefix stamp** (first [`SHIP_STAMP_BYTES`] bytes) of
//! the WAL when the cursor is established. Append-only growth never rewrites
//! shipped bytes, so any prefix change (or shrink past the cursor, or the file
//! vanishing under an advanced cursor) is a rotation and must fail closed.

#![forbid(unsafe_code)]

/// Bytes of WAL prefix compared per pull to detect in-place rotation.
pub const SHIP_STAMP_BYTES: usize = 64;

/// Plan for one [`crate::WalShipper::pull`] attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullPlan {
    /// Primary WAL no longer continues the shipped stream; re-bootstrap.
    Rotated {
        /// File length observed at detection (0 when the file is gone).
        file_len: u64,
        /// Stale shipper cursor.
        cursor: u64,
    },
    /// Nothing new since the cursor.
    UpToDate,
    /// Ship `bytes` starting at the cursor.
    Ship {
        /// Bytes to read (already capped by max pull).
        bytes: u64,
    },
}

/// Whether the current WAL prefix stopped matching the captured stamp.
///
/// `stamp_now` is the first `min(stamp_then.len(), file_len)` bytes of the
/// current file. Tail-only shrink (torn-write trim) keeps the prefix and is
/// not a rotation when the cursor still fits; any rewritten byte is.
#[must_use]
pub fn stamp_changed(stamp_then: &[u8], stamp_now: &[u8]) -> bool {
    if stamp_now.len() > stamp_then.len() {
        return true;
    }
    &stamp_then[..stamp_now.len()] != stamp_now
}

/// Decide one pull (F165 kernel).
///
/// Order matters: a missing file under an advanced cursor is a rotation even
/// before lengths are compared; a shrink past the cursor and a stamp change
/// are rotations; only a stable prefix with `len > cursor` ships.
#[must_use]
pub fn pull_plan(
    file_len: Option<u64>,
    cursor: u64,
    max_pull: u64,
    stamp_then: Option<&[u8]>,
    stamp_now: &[u8],
) -> PullPlan {
    let Some(len) = file_len else {
        if cursor > 0 || stamp_then.is_some() {
            return PullPlan::Rotated {
                file_len: 0,
                cursor,
            };
        }
        return PullPlan::UpToDate;
    };
    if len < cursor {
        return PullPlan::Rotated {
            file_len: len,
            cursor,
        };
    }
    if let Some(then) = stamp_then {
        if stamp_changed(then, stamp_now) {
            return PullPlan::Rotated {
                file_len: len,
                cursor,
            };
        }
    }
    if len == cursor {
        return PullPlan::UpToDate;
    }
    PullPlan::Ship {
        bytes: (len - cursor).min(max_pull),
    }
}

/// AS-IS F165: length-only rotation check (misses truncate-then-regrow).
#[must_use]
pub fn pull_plan_as_is(
    file_len: Option<u64>,
    cursor: u64,
    max_pull: u64,
    _stamp_then: Option<&[u8]>,
    _stamp_now: &[u8],
) -> PullPlan {
    let Some(len) = file_len else {
        return PullPlan::UpToDate;
    };
    if len < cursor {
        return PullPlan::Rotated {
            file_len: len,
            cursor,
        };
    }
    if len == cursor {
        return PullPlan::UpToDate;
    }
    PullPlan::Ship {
        bytes: (len - cursor).min(max_pull),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ship_kernel_has_no_verus_cartoon() {
        let src = include_str!("ship_kernel.rs");
        let block = concat!("verus", "!", " {");
        let cfg = concat!("cfg(", "verus", "_keep", "_ghost)");
        assert!(
            !src.contains(block),
            "u64 fingerprint is not last-wins of rustc &[u8] stamps"
        );
        assert!(
            !src.contains(cfg),
            "cfg split hides rustc types from the prover"
        );
    }

    const STAMP: &[u8] = &[7u8; SHIP_STAMP_BYTES];

    #[test]
    fn growth_with_stable_prefix_ships() {
        assert_eq!(
            pull_plan(Some(200), 100, 4_000_000, Some(STAMP), STAMP),
            PullPlan::Ship { bytes: 100 }
        );
        assert_eq!(
            pull_plan(Some(100), 100, 4_000_000, Some(STAMP), STAMP),
            PullPlan::UpToDate
        );
    }

    #[test]
    fn shrink_past_cursor_rotates() {
        assert_eq!(
            pull_plan(Some(50), 100, 4_000_000, Some(STAMP), &STAMP[..50]),
            PullPlan::Rotated {
                file_len: 50,
                cursor: 100
            }
        );
    }

    /// F165 core: truncate-then-regrow past the cursor keeps `len >= cursor`,
    /// so only the rewritten prefix exposes the rotation.
    #[test]
    fn regrow_past_cursor_with_new_prefix_rotates() {
        let mut new_stamp = [9u8; SHIP_STAMP_BYTES];
        new_stamp[0] ^= 0xff;
        assert_eq!(
            pull_plan(Some(500), 300, 4_000_000, Some(STAMP), &new_stamp),
            PullPlan::Rotated {
                file_len: 500,
                cursor: 300
            }
        );
        // AS-IS ships misaligned bytes here (teeth).
        assert_eq!(
            pull_plan_as_is(Some(500), 300, 4_000_000, Some(STAMP), &new_stamp),
            PullPlan::Ship { bytes: 200 }
        );
    }

    /// Catalog three-teeth plant. Direct `regrow_past_cursor_with_new_prefix_rotates` is **not** this tooth.
    #[test]
    fn stamp_changed_on_rewrite_is_not_ok() {
        let then = [7u8; SHIP_STAMP_BYTES];
        let mut now = then;
        now[0] ^= 0xff;
        assert!(stamp_changed(&then, &now));
        assert!(!stamp_changed(&then, &then));
        assert_eq!(
            pull_plan(Some(500), 300, 4_000_000, Some(&then), &now),
            PullPlan::Rotated {
                file_len: 500,
                cursor: 300
            }
        );
        assert_eq!(
            pull_plan_as_is(Some(500), 300, 4_000_000, Some(&then), &now),
            PullPlan::Ship { bytes: 200 },
            "AS-IS dente: length-only ships misaligned bytes on rewrite"
        );
    }

    #[test]
    fn missing_file_under_advanced_cursor_rotates() {
        assert_eq!(
            pull_plan(None, 100, 4_000_000, Some(STAMP), &[]),
            PullPlan::Rotated {
                file_len: 0,
                cursor: 100
            }
        );
        // Fresh primary that never wrote a WAL: benign.
        assert_eq!(pull_plan(None, 0, 4_000_000, None, &[]), PullPlan::UpToDate);
    }

    #[test]
    fn tail_trim_keeps_prefix_and_cursor() {
        // Torn-tail trim below the stamp end but above the cursor: not a rotation.
        assert_eq!(
            pull_plan(Some(50), 40, 4_000_000, Some(STAMP), &STAMP[..50]),
            PullPlan::Ship { bytes: 10 }
        );
        // ...but the prefix must still match what was shipped.
        let mut bad = STAMP[..64].to_vec();
        bad[63] ^= 1;
        assert_eq!(
            pull_plan(Some(80), 60, 4_000_000, Some(STAMP), &bad),
            PullPlan::Rotated {
                file_len: 80,
                cursor: 60
            }
        );
    }

    #[test]
    fn max_pull_caps_ship() {
        assert_eq!(
            pull_plan(Some(1_000), 100, 64, Some(STAMP), STAMP),
            PullPlan::Ship { bytes: 64 }
        );
    }
}
