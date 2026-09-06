//! The durable-fold contract shared by the on-disk benchmark backends.
//!
//! Ported from beyondoss/slipstream `src/snapshot.rs` (MIT), trimmed to the
//! trait surface the `snapshot_backends` bench drives: the append-log backend
//! and the artifact export/import machinery are not part of this harness.
//!
//! ## Invariants every implementation must hold
//!
//! - **Pure function of the applied log.** Delete the store, replay every
//!   update with revision `>` the persisted cursor, and you get
//!   byte-identical state. The store caches the fold; it is never the source
//!   of truth.
//! - **Cursor-after-apply.** A persisted cursor `C` implies every update with
//!   revision `≤ C` is durably folded in. [`apply`](SnapshotStore::apply)
//!   writes data and cursor together so the cursor never names a revision
//!   whose data is absent.
//!
//! ## Threading
//!
//! Methods are **synchronous** and may block on I/O.

use std::path::Path;

use crate::kv::{KvEntry, KvUpdate, WatchCursor};

/// Errors from snapshot store operations.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("snapshot I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid snapshot format: {0}")]
    InvalidFormat(String),
    #[error("snapshot corrupted (CRC mismatch)")]
    Corrupted,
    /// A pluggable [`SnapshotStore`] backend (fjall, RocksDB, PedraDB)
    /// reported an error. Kept backend-agnostic — no backend type leaks into
    /// this enum's signature — so enabling a backend feature does not change
    /// the public error surface.
    #[error("snapshot backend error: {0}")]
    Backend(String),
}

/// A durable, resumable, queryable fold of a KV update stream.
///
/// Consumers pick the backend; the benchmark compares them on the same
/// workload.
pub trait SnapshotStore: Sized + Send {
    /// Open (or resume) the store at `path`.
    ///
    /// Returns the persisted resume cursor — [`WatchCursor::none`] when the
    /// store is fresh — and the store ready to
    /// [`apply`](Self::apply)/query.
    ///
    /// Backends with tuning knobs (compaction threshold, sync mode) expose
    /// them on their own constructors; this uses safe defaults.
    fn load(path: &Path) -> Result<(WatchCursor, Self), SnapshotError>;

    /// Atomically fold `batch` into the store and advance the resume cursor.
    ///
    /// Data and cursor become durable together (see the cursor-after-apply
    /// invariant). `cursor` is the highest revision received in the batch.
    fn apply(&mut self, batch: &[KvUpdate], cursor: &WatchCursor) -> Result<(), SnapshotError>;

    /// Look up the live entry for `key`. `None` if absent or deleted.
    fn get(&self, key: &str) -> Result<Option<KvEntry>, SnapshotError>;

    /// All live entries whose key starts with `prefix`, in ascending key
    /// order.
    ///
    /// Buffers the whole match set into a `Vec`. Convenient for bounded
    /// prefixes, but a broad prefix against an on-disk fold materializes
    /// every match at once — and an empty `prefix` is an unbounded full scan.
    /// Use [`for_each_in_range`](Self::for_each_in_range) for either.
    fn range(&self, prefix: &str) -> Result<Vec<KvEntry>, SnapshotError>;

    /// Stream every live entry whose key starts with `prefix`, in ascending
    /// key order, invoking `f` once per entry — without buffering the whole
    /// match set in memory.
    ///
    /// The provided implementation delegates to [`range`](Self::range) —
    /// correct for in-RAM backends. On-disk backends override it to stream
    /// straight from storage.
    fn for_each_in_range(
        &self,
        prefix: &str,
        mut f: impl FnMut(KvEntry) -> Result<(), SnapshotError>,
    ) -> Result<(), SnapshotError> {
        for entry in self.range(prefix)? {
            f(entry)?;
        }
        Ok(())
    }

    /// The most recently applied (and durably persisted) resume cursor —
    /// [`WatchCursor::none`] when nothing has been applied.
    fn cursor(&self) -> WatchCursor;
}
