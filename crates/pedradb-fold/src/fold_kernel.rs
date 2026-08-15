//! Pure range-tombstone coverage for fold last-per-key / apply (F169).
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

    #[test]
    fn point_delete_is_exact() {
        assert!(fold_event_hides_key(false, b"k-c", b"", b"k-c"));
        assert!(!fold_event_hides_key(false, b"k-c", b"", b"k-b"));
    }
}
