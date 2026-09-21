//! Shim: production `batch.rs` uses `crate::error` and `crate::key`.
//! CoreError without thiserror (same refuse as `error.rs`). Production
//! files are `#[path]`.

pub mod error {
    pub enum CoreError {
        Internal(String),
    }
    pub type Result<T> = std::result::Result<T, CoreError>;
}

#[path = "../../../../crates/pedradb-core/src/key_kernel.rs"]
pub mod key;

#[path = "../../../../crates/pedradb-core/src/write_admission_kernel.rs"]
pub mod write_admission_kernel;

#[path = "../../../../crates/pedradb-core/src/batch_kernel.rs"]
pub mod batch;

pub use batch::*;
