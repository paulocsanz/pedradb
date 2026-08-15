//! Montanha Fold — Slipstream-class materializer (RFC-0024).
//!
//! The fold is **not** a Raft member and **not** linearizable. Reads are
//! LocalApplied at a pinned cursor. The SoR remains Montanha.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod applied;
mod caixote;
mod export;
mod follow;
mod roles;
mod ship;
mod store;
mod watch;

pub use applied::{watch_applied, watch_applied_prefix, WatchApplied};
pub use caixote::{caixote_host_filter, fold_get_local, IntentObservedDelta, SeqSyncState};
pub use export::{export_fold, import_fold};
pub use follow::{follow_prefix, follow_store_prefix, in_prefixes, PrefixSet};
pub use roles::FoldRole;
pub use ship::{ship_pull, FoldShip};
pub use store::{FoldStore, PedraFold};
pub use watch::{
    cursor_expired, last_per_key, resume_window_ok, resync_expired, state_sync_then_tail,
    CursorExpired,
};

use thiserror::Error;

/// Last **applied** sequence (exclusive lower bound for the next follow).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct FoldCursor(pub u64);

impl FoldCursor {
    /// No revisions applied yet.
    #[must_use]
    pub const fn none() -> Self {
        Self(0)
    }

    /// Raw sequence.
    #[must_use]
    pub const fn seq(self) -> u64 {
        self.0
    }
}

/// One change to fold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldUpdate {
    /// Put / overwrite.
    Put {
        /// User key.
        key: Vec<u8>,
        /// Value (empty on a proxy that evicted values).
        value: Vec<u8>,
        /// Source sequence.
        seq: u64,
    },
    /// Point delete.
    Delete {
        /// User key.
        key: Vec<u8>,
        /// Source sequence.
        seq: u64,
    },
}

impl FoldUpdate {
    /// Sequence of this update.
    #[must_use]
    pub fn seq(&self) -> u64 {
        match self {
            Self::Put { seq, .. } | Self::Delete { seq, .. } => *seq,
        }
    }

    /// User key.
    #[must_use]
    pub fn key(&self) -> &[u8] {
        match self {
            Self::Put { key, .. } | Self::Delete { key, .. } => key,
        }
    }
}

/// Fold / watch / ship errors.
#[derive(Debug, Error)]
pub enum FoldError {
    /// Pedra / I/O.
    #[error("fold store: {0}")]
    Store(#[from] pedradb_core::CoreError),
    /// WAL ship.
    #[error("fold ship: {0}")]
    Ship(#[from] pedradb_replicate::ShipError),
    /// Store cluster.
    #[error("fold montanha: {0}")]
    Montanha(#[from] pedradb_store::StoreError),
    /// Log compacted / WAL rotated past the pin.
    #[error("cursor expired: pin={pin} first_retained={first_retained}")]
    CursorExpired {
        /// Persisted pin.
        pin: u64,
        /// First sequence still in the source (or 0 if unknown).
        first_retained: u64,
    },
    /// Transient apply; caller must re-queue (pin must not advance).
    #[error("transient apply: {0}")]
    TransientApply(String),
    /// Protocol / usage.
    #[error("{0}")]
    Msg(String),
}

/// Result alias.
pub type Result<T> = std::result::Result<T, FoldError>;

/// In-process counters (RFC-0024 P0 telemetry).
#[derive(Debug, Default, Clone)]
pub struct FoldMetrics {
    /// Successful apply batches.
    pub fold_apply_batches: u64,
    /// Last pinned cursor.
    pub fold_cursor: u64,
    /// Failed applies.
    pub fold_apply_err: u64,
    /// Watch resyncs (P1).
    pub fold_watch_resync: u64,
    /// Cursor-expired events (P1).
    pub fold_cursor_expired: u64,
    /// Delta keys applied (P1 federation).
    pub sync_delta_keys: u64,
    /// Full-dump backstop uses (P1).
    pub sync_full_dump: u64,
    /// Proxy cache hits / misses (P2).
    pub cache_hit: u64,
    /// Proxy value misses.
    pub cache_miss: u64,
}
