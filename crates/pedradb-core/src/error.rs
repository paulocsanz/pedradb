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

    /// A zero type+len WAL header at a fresh alignment with non-zero bytes
    /// after it (F170). The writer only ever pads `< HEADER_SIZE` zero
    /// bytes, so this shape is corruption, never padding or prealloc.
    #[error("wal zero-length header mid-block with non-zero tail at offset {offset}")]
    WalZeroHeader {
        /// Byte offset where the zero header began.
        offset: u64,
    },

    /// Repeated fail-stop WAL corruption at open: the corruption journal hit
    /// its escalation limit (RFC-0038 option D). A single CRC event cannot
    /// tell isolated bitflip from dying media; only history can. The Nth
    /// recorded event refuses open — in every recovery mode — so the node is
    /// evacuated instead of silently serving from failing hardware.
    ///
    /// A clean WAL still opens (repair/evacuate, then reopen).
    #[error(
        "corruption escalation: {events} fail-stop wal events recorded (limit {limit}); \
         repair/replace the WAL or evacuate — see CORRUPTLOG in the db directory"
    )]
    CorruptionEscalated {
        /// Events now recorded in `CORRUPTLOG`.
        events: u32,
        /// Escalation threshold that fired.
        limit: u32,
    },

    /// An internal invariant was violated.
    #[error("internal error: {0}")]
    Internal(String),

    /// MANIFEST/`CURRENT` were renamed (the new version IS the committed
    /// one on disk) but the final directory fsync failed (F196). Callers
    /// must NOT roll back in-memory state past this point — fence instead
    /// (same shape as `compact_vlog_promote`).
    #[error("manifest committed (CURRENT swung) but final dir sync failed: {source}")]
    ManifestCommittedUnsynced {
        /// The directory fsync I/O error.
        source: std::io::Error,
    },

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

    /// History-tier segment / remote object is unreadable or fails its CRC
    /// (RFC-0046): fail-closed — never upload, serve, or replay corrupt
    /// history bytes.
    #[error("corrupt history: {0}")]
    CorruptHistory(String),

    /// A value-log record (large value) failed its CRC / length check or its
    /// backing file cannot satisfy the pointer: fail-closed — reads surface
    /// this as an error (or fail-stop on Option-shaped APIs), never a miss.
    #[error("corrupt value log: {0}")]
    CorruptValue(String),

    /// A required WAL `sync_data` failed after append; this `Db` refuses further
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
    #[error("snapshot too old: requested sequence {requested}, earliest readable {earliest}")]
    SnapshotTooOld {
        /// Sequence the caller asked to read at.
        requested: crate::key::SequenceNumber,
        /// Lowest sequence still guaranteed readable after GC.
        earliest: crate::key::SequenceNumber,
    },

    /// Write refused because L0 has too many files (open-items §2.3 option a).
    ///
    /// Honest stall: no artificial delay. Caller should compact / wait for
    /// auto-compact, then retry. Off by default (`set_write_stall_l0`).
    #[error("write stall: L0 has {l0_files} files (limit {limit})")]
    WriteStall {
        /// Current L0 SST count.
        l0_files: usize,
        /// Configured stall threshold.
        limit: usize,
    },

    /// Write refused because the active memtable is too large (open-items §2.3 option c).
    ///
    /// Bound against unbounded mem growth when flush cannot keep up. Off by default
    /// (`set_write_stall_mem_bytes`). With drain enabled, one flush is attempted first.
    #[error("write stall: memtable ~{mem_bytes}B (limit {limit}B)")]
    WriteStallMem {
        /// Approximate active memtable bytes.
        mem_bytes: usize,
        /// Configured stall threshold in bytes.
        limit: usize,
    },
}

/// Convenience `Result` alias used throughout the crate.
pub type Result<T> = std::result::Result<T, CoreError>;
