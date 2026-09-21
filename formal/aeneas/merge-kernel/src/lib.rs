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

#[path = "../../../../crates/pedradb-core/src/merge_kernel.rs"]
pub mod merge;

pub use merge::*;
