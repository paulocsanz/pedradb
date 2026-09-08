//! Pure range-tombstone coverage for fold last-per-key / apply (F169).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). Slice compare is caller; the u64
//! half-open rule is the term. No twin-cópia.
//!
//!   ./scripts/verus_fold_range.sh
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
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn fold_event_hides_key(is_range: bool, start: &[u8], end: &[u8], key: &[u8]) -> bool {
    if is_range {
        key >= start && key < end
    } else {
        key == start
    }
}

/// AS-IS F169: a range delete hides only its start key (covered keys stay live).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn fold_event_hides_key_as_is(is_range: bool, start: &[u8], _end: &[u8], key: &[u8]) -> bool {
    let _ = is_range;
    key == start
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn fold_event_hides_key_spec(
    is_range: bool,
    start: u64,
    end: u64,
    key: u64,
) -> bool {
    if is_range {
        key >= start && key < end
    } else {
        key == start
    }
}

pub fn fold_event_hides_key(is_range: bool, start: u64, end: u64, key: u64) -> (r: bool)
    ensures
        r == fold_event_hides_key_spec(is_range, start, end, key),
        is_range && key >= start && key < end ==> r,
        is_range && (key < start || key >= end) ==> !r,
        !is_range ==> r == (key == start),
{
    if is_range {
        key >= start && key < end
    } else {
        key == start
    }
}

pub fn fold_event_hides_key_as_is(_is_range: bool, start: u64, _end: u64, key: u64) -> (r: bool)
    ensures
        r == (key == start),
{
    key == start
}

proof fn lemma_as_is_misses_cover()
    ensures
        fold_event_hides_key_spec(true, 2, 4, 3),
        !fold_event_hides_key_spec(true, 2, 4, 4),
        fold_event_hides_key_spec(true, 2, 4, 2),
{
}

} // verus!

#[cfg(test)]
mod tests {
    use super::*;

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
