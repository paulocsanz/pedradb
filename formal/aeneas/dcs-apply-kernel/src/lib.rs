//! Shim: production `apply_kernel.rs` uses `crate::{DcsError, Result}`.
//! Those live in `dcs/src/lib.rs` with `thiserror` + `CoreError` (Aeneas
//! refuses that file the same way as `key.rs`). The decision functions are
//! production via `#[path]`; the enum here is the match surface
//! (`CasFailed` vs other) without thiserror.

pub enum DcsError {
    /// Engine I/O (payload omitted — Charon cannot take `CoreError`).
    Core,
    /// CAS / create precondition failed.
    CasFailed(&'static str),
    /// Unknown or expired lease.
    LeaseNotFound(u64),
    /// Encoding / corruption of DCS metadata.
    Corrupt(String),
}

/// Result alias matching the parent crate.
pub type Result<T> = std::result::Result<T, DcsError>;

#[path = "../../../../crates/pedradb-dcs/src/apply_kernel.rs"]
pub mod apply_kernel;

pub use apply_kernel::*;
