//! Shim: production `write_ack_kernel.rs` uses `crate::d1_modelo_kernel`,
//! `crate::env_crash_kernel`, and `crate::wal::wal_state_kernel`. Charon
//! walks this crate; the production files are the source of truth — never
//! edit copies.

#[path = "../../../../crates/pedradb-core/src/group_commit_kernel.rs"]
pub mod group_commit_kernel;

#[path = "../../../../crates/pedradb-core/src/env_crash_kernel.rs"]
pub mod env_crash_kernel;

#[path = "../../../../crates/pedradb-core/src/d1_modelo_kernel.rs"]
pub mod d1_modelo_kernel;

pub mod wal;

#[path = "../../../../crates/pedradb-core/src/write_ack_kernel.rs"]
pub mod write_ack_kernel;

pub use write_ack_kernel::*;
