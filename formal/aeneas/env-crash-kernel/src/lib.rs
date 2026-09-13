//! Shim: production `env_crash_kernel.rs` uses
//! `crate::group_commit_kernel::fsync_promotes_pending`. Charon walks this
//! crate; the production files are the source of truth — never edit copies.

#[path = "../../../../crates/pedradb-core/src/group_commit_kernel.rs"]
pub mod group_commit_kernel;

#[path = "../../../../crates/pedradb-core/src/env_crash_kernel.rs"]
pub mod env_crash_kernel;

pub use env_crash_kernel::*;
