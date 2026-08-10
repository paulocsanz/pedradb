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
}

/// Convenience `Result` alias used throughout the crate.
pub type Result<T> = std::result::Result<T, CoreError>;
