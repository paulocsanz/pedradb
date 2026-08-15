//! Cursor-after-apply combinator (RFC-0024 P0.1).

use crate::{FoldCursor, FoldStore, FoldUpdate, Result};
use pedradb_core::{ChangeEntry, ChangeKind, Db, StdEnv};
use pedradb_journal::JournalConsumer;

/// Receive-then-apply loop: pin advances only after `store.apply` returns.
pub struct WatchApplied<S: FoldStore> {
    store: S,
    pending: Vec<FoldUpdate>,
}

impl<S: FoldStore> WatchApplied<S> {
    /// Wrap an open store.
    pub fn new(store: S) -> Self {
        Self {
            store,
            pending: Vec::new(),
        }
    }

    /// Buffer an update. Does **not** advance the cursor.
    pub fn recv(&mut self, u: FoldUpdate) {
        self.pending.push(u);
    }

    /// Apply pending batch then persist cursor. On store error, pending stays
    /// (re-queue); cursor unchanged.
    ///
    /// # Errors
    /// Store apply.
    pub fn flush(&mut self) -> Result<FoldCursor> {
        if self.pending.is_empty() {
            return Ok(self.store.cursor());
        }
        let high = self.pending.iter().map(FoldUpdate::seq).max().unwrap_or(0);
        let cursor = FoldCursor(high);
        self.store.apply(&self.pending, cursor)?;
        self.pending.clear();
        Ok(cursor)
    }

    /// Borrow the fold.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Mut borrow.
    pub fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }

    /// Unapplied buffer (crash window).
    #[must_use]
    pub fn pending(&self) -> &[FoldUpdate] {
        &self.pending
    }
}

/// Poll CHANGELOG without advancing a journal pin; apply into `store`; then pin.
///
/// # Errors
/// Store apply.
pub fn watch_applied(
    db: &Db<StdEnv>,
    consumer: &mut JournalConsumer,
    store: &mut impl FoldStore,
) -> Result<FoldCursor> {
    watch_applied_prefix(db, consumer, store, None)
}

/// Like [`watch_applied`] but keep only `prefixes` (Caixote host fold).
///
/// # Errors
/// Store apply.
pub fn watch_applied_prefix(
    db: &Db<StdEnv>,
    consumer: &mut JournalConsumer,
    store: &mut impl FoldStore,
    prefixes: Option<&crate::PrefixSet>,
) -> Result<FoldCursor> {
    let last = db.last_sequence();
    let batch: Vec<ChangeEntry> = db
        .changes_after(consumer.pin)
        .into_iter()
        .filter(|e| e.sequence <= last)
        .filter(|e| match prefixes {
            None => true,
            Some(p) => crate::follow::in_prefixes(e.key.as_ref(), p),
        })
        .collect();
    if batch.is_empty() {
        return Ok(store.cursor());
    }
    let updates: Vec<FoldUpdate> = batch
        .iter()
        .map(|e| match e.kind {
            ChangeKind::Delete | ChangeKind::DeleteRange => FoldUpdate::Delete {
                key: e.key.to_vec(),
                seq: e.sequence,
            },
            ChangeKind::Put => FoldUpdate::Put {
                key: e.key.to_vec(),
                value: e.value.to_vec(),
                seq: e.sequence,
            },
        })
        .collect();
    let high = updates.iter().map(FoldUpdate::seq).max().unwrap_or(0);
    store.apply(&updates, FoldCursor(high))?;
    // Pin only after apply returned.
    consumer.pin = high;
    Ok(FoldCursor(high))
}
