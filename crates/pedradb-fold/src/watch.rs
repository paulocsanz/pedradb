//! State-sync then tail, and cursor-expired resync (RFC-0024 P1).

use crate::follow::{in_prefixes, PrefixSet};
use crate::{FoldCursor, FoldStore, FoldUpdate, Result};
use pedradb_core::{ChangeKind, Db, Env};
use std::collections::{BTreeMap, BTreeSet};

/// Source compacted past the pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorExpired {
    /// Pin that is no longer in the retained window.
    pub pin: u64,
    /// First retained sequence (if known).
    pub first_retained: u64,
}

/// Resume is sound iff `first_sequence <= pin + 1`.
#[must_use]
pub fn resume_window_ok(pin: u64, first_sequence: u64) -> bool {
    first_sequence <= pin.saturating_add(1)
}

/// Last-write-wins per key from `changes_after(0)` (state-sync seed).
#[must_use]
pub fn last_per_key<E: Env>(db: &Db<E>, prefixes: &PrefixSet) -> Vec<FoldUpdate> {
    let last = db.last_sequence();
    let mut map: BTreeMap<Vec<u8>, FoldUpdate> = BTreeMap::new();
    for e in db.changes_after(0) {
        if e.sequence > last || !in_prefixes(e.key.as_ref(), prefixes) {
            continue;
        }
        let u = crate::follow::entry_to_update(e);
        map.insert(u.key().to_vec(), u);
    }
    map.into_values().collect()
}

/// No-cursor: last-per-key then live tail after `max(seed seq)`.
/// With cursor: tail only (`seq > pin`), unless the window expired.
///
/// # Errors
/// [`crate::FoldError::CursorExpired`] when `first_retained` is past the pin.
pub fn state_sync_then_tail<E: Env>(
    db: &Db<E>,
    prefixes: &PrefixSet,
    from: Option<FoldCursor>,
    first_retained: u64,
) -> Result<Vec<FoldUpdate>> {
    match from {
        None => {
            let mut seed = last_per_key(db, prefixes);
            let high = seed.iter().map(FoldUpdate::seq).max().unwrap_or(0);
            let tail = crate::follow::follow_prefix(db, prefixes, FoldCursor(high));
            seed.extend(tail);
            Ok(seed)
        }
        Some(pin) => {
            if !resume_window_ok(pin.0, first_retained) {
                return Err(crate::FoldError::CursorExpired {
                    pin: pin.0,
                    first_retained,
                });
            }
            Ok(crate::follow::follow_prefix(db, prefixes, pin))
        }
    }
}

/// True when the pin is behind the retained head.
#[must_use]
pub fn cursor_expired(pin: FoldCursor, first_retained: u64) -> bool {
    !resume_window_ok(pin.0, first_retained)
}

/// Synthetic deletes for fold keys missing from `live`, then last-per-key puts.
///
/// Delete-then-put order: vanished keys first (unknown seq = `pin`), then re-list.
///
/// # Errors
/// Store range.
pub fn resync_expired<E: Env>(
    store: &impl FoldStore,
    db: &Db<E>,
    prefixes: &PrefixSet,
    pin: FoldCursor,
) -> Result<Vec<FoldUpdate>> {
    let mut live: BTreeSet<Vec<u8>> = BTreeSet::new();
    for e in db.changes_after(0) {
        if !in_prefixes(e.key.as_ref(), prefixes) {
            continue;
        }
        match e.kind {
            ChangeKind::Delete | ChangeKind::DeleteRange => {
                live.remove(e.key.as_ref());
            }
            ChangeKind::Put => {
                live.insert(e.key.to_vec());
            }
        }
    }
    let mut out = Vec::new();
    for pref in prefixes.iter() {
        for (k, _) in store.range(pref)? {
            if !live.contains(&k) {
                out.push(FoldUpdate::Delete { key: k, seq: pin.0 });
            }
        }
    }
    out.extend(last_per_key(db, prefixes));
    Ok(out)
}
