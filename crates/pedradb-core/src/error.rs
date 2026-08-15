//! Error types for the pedradb-core engine.

use std::io;
use thiserror::Error;

/// Top-level error type for the core engine.
#[derive(Debug, Error)]
pub enum CoreError {
    /// An I/O error from the underlying filesystem.
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    /// A WAL record failed its CRC32C integrity check.
    #[error("wal crc mismatch at offset {offset}: expected {expected:#010x}, found {found:#010x}")]
    Crc {
        /// Byte offset in the log file where the bad record header began.
        offset: u64,
        /// Expected checksum computed over (length + type + data).
        expected: u32,
        /// Checksum actually found in the record header.
        found: u32,
    },

    /// The WAL stream was truncated mid-record (likely an unflushed crash).
    #[error("wal truncated record at offset {0}")]
    Truncated(u64),

    /// An internal invariant was violated.
    #[error("internal error: {0}")]
    Internal(String),

    /// Transaction was already committed or aborted.
    #[error("transaction already finished")]
    TransactionFinished,

    /// Transaction is empty and commit was refused (optional policy — unused if empty commit allowed).
    #[error("transaction error: {0}")]
    Transaction(String),

    /// Another process (or non-stolen lock) already has this DB directory open.
    #[error("database already open at {path}: held by pid {holder_pid:?}")]
    AlreadyOpen {
        /// Directory that is locked.
        path: std::path::PathBuf,
        /// PID written in `LOCK`, if parseable.
        holder_pid: Option<u32>,
    },

    /// MANIFEST / CURRENT contents are unreadable or inconsistent.
    #[error("corrupt manifest: {0}")]
    CorruptManifest(String),

    /// A required WAL `sync_all` failed after append; this `Db` refuses further
    /// writes until `close` + `open` (reopen rebuilds mem from WAL).
    ///
    /// The original failure may still have left a complete record on disk
    /// (uncertain outcome). `Ok` still means durable when `sync=true`.
    #[error("database fenced after durability failure; close and reopen to recover")]
    DurabilityFenced,

    /// Optimistic concurrency control: another commit changed a key this TX
    /// read or wrote since its snapshot (RFC-0014 P2.1).
    #[error("transaction conflict: key changed since snapshot")]
    TransactionConflict,

    /// Conditional put failed: key state did not match the expected precondition
    /// (RFC-0019 CAS / `put_if`).
    #[error("compare-and-swap precondition failed")]
    CasMismatch,

    /// Read snapshot is older than the version-GC watermark (open-items §2.1 (c)).
    ///
    /// History required for `requested` may have been dropped by
    /// [`crate::db::Db::compact_reclaim`], `latest_only`, or an explicit GC floor.
    /// Montanha maps the store-level cousin to FDB `transaction_too_old`.
    #[error(
        "snapshot too old: requested sequence {requested}, earliest readable {earliest}"
    )]
    SnapshotTooOld {
        /// Sequence the caller asked to read at.
        requested: crate::key::SequenceNumber,
        /// Lowest sequence still guaranteed readable after GC.
        earliest: crate::key::SequenceNumber,
    },
}

/// Convenience `Result` alias used throughout the crate.
pub type Result<T> = std::result::Result<T, CoreError>;
