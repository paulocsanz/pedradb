//! Shim: production `membership_kernel.rs` calls `crate::commit_kernel`
//! (`replica_served_ok` / RFC-0227 C1). Aeneas crate is not pedradb-raft.

#[path = "../../../../crates/pedradb-raft/src/commit_kernel.rs"]
pub mod commit_kernel;

#[path = "../../../../crates/pedradb-raft/src/membership_kernel.rs"]
pub mod membership_kernel;

pub use membership_kernel::*;
