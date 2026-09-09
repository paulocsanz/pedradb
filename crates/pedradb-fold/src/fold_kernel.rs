//! Pure range-tombstone coverage for fold last-per-key / apply (F169).
//!
//! **Term:** this file is what `rustc` links. Aeneas extracts that body
//! (`scripts/aeneas_fold.sh`). A u64 view of rustc `&[u8]` bounds is a
//! model twin — not last-wins (deleted).
//!
//!   ./scripts/aeneas_fold.sh --required
//!
//! CHANGELOG records a range delete as `ChangeKind::DeleteRange` with the
//! **start** as the entry key and the exclusive end as the value. Fold used
//! to map that to a point `FoldUpdate::Delete` of the start key only
//! (`entry_to_update`, `last_per_key` last-write-wins). Covered keys stayed
//! live on the replica while the source `get` hid them — silent-wrong CDC.
//!
//! Fix shape: a range tombstone hides every key in `[start, end)` (same
//! half-open rule as [`pedradb_core::range_tombstone_covers`]). Point
//! deletes still hide only the exact key.

#![forbid(unsafe_code)]

/// F169 kernel: does changelog event `(range, start, end)` hide `key`?
#[must_use]
pub fn fold_event_hides_key(is_range: bool, start: &[u8], end: &[u8], key: &[u8]) -> bool {
    if is_range {
        key >= start && key < end
    } else {
        key == start
    }
}

/// AS-IS F169: a range delete hides only its start key (covered keys stay live).
#[must_use]
pub fn fold_event_hides_key_as_is(is_range: bool, start: &[u8], _end: &[u8], key: &[u8]) -> bool {
    let _ = is_range;
    key == start
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_kernel_has_no_verus_cartoon() {
        let src = include_str!("fold_kernel.rs");
        let block = concat!("verus", "!", " {");
        let cfg = concat!("cfg(", "verus", "_keep", "_ghost)");
        assert!(
            !src.contains(block),
            "u64 stand-in is not last-wins of rustc &[u8] fold range"
        );
        assert!(
            !src.contains(cfg),
            "cfg split hides rustc types from the prover"
        );
    }

    #[test]
    fn range_hides_covered_not_outside() {
        assert!(fold_event_hides_key(true, b"k-b", b"k-d", b"k-c"));
        assert!(fold_event_hides_key(true, b"k-b", b"k-d", b"k-b"));
        assert!(!fold_event_hides_key(true, b"k-b", b"k-d", b"k-d"));
        assert!(!fold_event_hides_key(true, b"k-b", b"k-d", b"k-a"));
        assert!(!fold_event_hides_key(true, b"k-b", b"k-d", b"k-e"));
    }

    /// F169 teeth: AS-IS misses the covered key.
    #[test]
    fn as_is_only_hides_start() {
        assert!(fold_event_hides_key_as_is(true, b"k-b", b"k-d", b"k-b"));
        assert!(!fold_event_hides_key_as_is(true, b"k-b", b"k-d", b"k-c"));
    }

    /// Catalog three-teeth plant. Direct `as_is_only_hides_start` is **not** this tooth.
    #[test]
    fn fold_event_hides_key_on_live_fold_is_not_ok() {
        assert!(fold_event_hides_key(true, b"k-b", b"k-d", b"k-c"));
        assert!(
            !fold_event_hides_key_as_is(true, b"k-b", b"k-d", b"k-c"),
            "AS-IS dente: range delete hides only the start key"
        );
        let dir = std::env::temp_dir().join(format!(
            "fold-range-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = pedradb_core::Db::open_with(
            &dir,
            pedradb_core::OpenOptions {
                exclusive: true,
                ..pedradb_core::OpenOptions::default()
            },
        )
        .unwrap();
        db.put(b"k-b", b"vb").unwrap();
        db.put(b"k-c", b"vc").unwrap();
        db.delete_range(b"k-b", b"k-d").unwrap();
        let prefs = crate::PrefixSet::one(b"k-");
        let sync = crate::last_per_key(&db, &prefs);
        let live: Vec<&[u8]> = sync
            .iter()
            .filter(|u| matches!(u, crate::FoldUpdate::Put { .. }))
            .map(crate::FoldUpdate::key)
            .collect();
        assert!(
            !live.iter().any(|k| *k == b"k-c"),
            "live last_per_key must drop covered k-c after DeleteRange [k-b,k-d); live={live:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn point_delete_is_exact() {
        assert!(fold_event_hides_key(false, b"k-c", b"", b"k-c"));
        assert!(!fold_event_hides_key(false, b"k-c", b"", b"k-b"));
    }
}
