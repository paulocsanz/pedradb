//! Prefix follow over Pedra CHANGELOG / Montanha applied log.

use crate::{FoldCursor, FoldUpdate};
use pedradb_core::{ChangeEntry, ChangeKind, Db, Env};
use pedradb_store::StoreCluster;

/// One or more key prefixes (host fold = `/host/{id}/` plus owned VM keys).
#[derive(Debug, Clone, Default)]
pub struct PrefixSet {
    prefixes: Vec<Vec<u8>>,
}

impl PrefixSet {
    /// Empty set matches nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Single prefix.
    #[must_use]
    pub fn one(prefix: impl AsRef<[u8]>) -> Self {
        Self {
            prefixes: vec![prefix.as_ref().to_vec()],
        }
    }

    /// Add a prefix.
    pub fn push(&mut self, prefix: impl AsRef<[u8]>) {
        self.prefixes.push(prefix.as_ref().to_vec());
    }

    /// Borrowed prefixes.
    #[must_use]
    pub fn iter(&self) -> impl Iterator<Item = &[u8]> {
        self.prefixes.iter().map(Vec::as_slice)
    }
}

/// Whether `key` is in any prefix of `set`.
#[must_use]
pub fn in_prefixes(key: &[u8], set: &PrefixSet) -> bool {
    set.prefixes.iter().any(|p| key.starts_with(p))
}

/// CHANGELOG tail after `from`, filtered to `prefixes`.
#[must_use]
pub fn follow_prefix<E: Env>(
    db: &Db<E>,
    prefixes: &PrefixSet,
    from: FoldCursor,
) -> Vec<FoldUpdate> {
    let last = db.last_sequence();
    db.changes_after(from.0)
        .into_iter()
        .filter(|e| e.sequence <= last && in_prefixes(e.key.as_ref(), prefixes))
        .map(entry_to_update)
        .collect()
}

/// Follow the best local applied Pedra on a Montanha cluster (in-process).
#[must_use]
pub fn follow_store_prefix<E: Env>(
    cluster: &StoreCluster<E>,
    prefixes: &PrefixSet,
    from: FoldCursor,
) -> Vec<FoldUpdate> {
    cluster
        .changelog_after(from.0)
        .into_iter()
        .filter(|e| in_prefixes(e.key.as_ref(), prefixes))
        .map(entry_to_update)
        .collect()
}

pub(crate) fn entry_to_update(e: ChangeEntry) -> FoldUpdate {
    match e.kind {
        ChangeKind::Delete | ChangeKind::DeleteRange => FoldUpdate::Delete {
            key: e.key.to_vec(),
            seq: e.sequence,
        },
        ChangeKind::Put => FoldUpdate::Put {
            key: e.key.to_vec(),
            value: e.value.to_vec(),
            seq: e.sequence,
        },
    }
}
