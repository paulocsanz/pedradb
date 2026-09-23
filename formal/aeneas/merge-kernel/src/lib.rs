//! Shim: production `merge.rs` uses `crate::error`, `crate::key`, and
//! `crate::compact_kernel`. CoreError without thiserror. Production files
//! are `#[path]`.

pub mod error {
    pub enum CoreError {
        Internal(String),
    }
    pub type Result<T> = std::result::Result<T, CoreError>;
}

#[path = "../../../../crates/pedradb-core/src/key_kernel.rs"]
pub mod key;

#[path = "../../../../crates/pedradb-core/src/compact_kernel.rs"]
pub mod compact_kernel;

#[path = "../../../../crates/pedradb-core/src/write_admission_kernel.rs"]
pub mod write_admission_kernel;

/// Honest-sync promote used by [`env_crash_kernel`] (production file).
pub mod group_commit_kernel {
    #[must_use]
    pub fn fsync_promotes_pending(honest: bool) -> bool {
        honest
    }
}

#[path = "../../../../crates/pedradb-core/src/env_crash_kernel.rs"]
pub mod env_crash_kernel;

pub mod wal;

#[path = "../../../../crates/pedradb-core/src/merge_kernel.rs"]
pub mod merge;

pub use merge::*;
