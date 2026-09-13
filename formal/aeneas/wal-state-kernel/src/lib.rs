//! Shim: production `wal/wal_state_kernel.rs` uses `crate::env_crash_kernel`.
//! Charon walks this crate; the production files are the source of truth —
//! never edit copies.

#[path = "../../../../crates/pedradb-core/src/group_commit_kernel.rs"]
pub mod group_commit_kernel;

#[path = "../../../../crates/pedradb-core/src/env_crash_kernel.rs"]
pub mod env_crash_kernel;

pub mod wal;

pub use wal::wal_state_kernel::*;
