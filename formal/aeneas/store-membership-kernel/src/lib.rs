//! Shim: production store `membership_kernel.rs` calls `crate::commit_kernel`
//! (`replica_served_ok` / RFC-0227 C1).

#[path = "../../../../crates/pedradb-store/src/commit_kernel.rs"]
pub mod commit_kernel;

#[path = "../../../../crates/pedradb-store/src/membership_kernel.rs"]
pub mod membership_kernel;

pub use membership_kernel::*;
