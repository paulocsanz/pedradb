//! Shim: production `durability_spine_kernel.rs` uses `crate::write_ack_kernel`.
//! Charon walks this crate; the production files are the source of truth.

#[path = "../../../../crates/pedradb-core/src/group_commit_kernel.rs"]
pub mod group_commit_kernel;

#[path = "../../../../crates/pedradb-core/src/env_crash_kernel.rs"]
pub mod env_crash_kernel;

#[path = "../../../../crates/pedradb-core/src/d1_modelo_kernel.rs"]
pub mod d1_modelo_kernel;

pub mod wal;

#[path = "../../../../crates/pedradb-core/src/write_ack_kernel.rs"]
pub mod write_ack_kernel;

#[path = "../../../../crates/pedradb-core/src/durability_spine_kernel.rs"]
pub mod durability_spine_kernel;

pub use durability_spine_kernel::*;
