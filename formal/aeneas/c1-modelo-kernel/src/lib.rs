//! Shim: production `c1_modelo_kernel.rs` uses `crate::{joint_election_ok,
//! may_commit_at, propose_ack_ok}` from membership + commit kernels.

#[path = "../../../../crates/pedradb-raft/src/membership_kernel.rs"]
pub mod membership_kernel;

pub use membership_kernel::{joint_election_ok, joint_election_ok_as_is};

#[path = "../../../../crates/pedradb-raft/src/commit_kernel.rs"]
pub mod commit_kernel;

pub use commit_kernel::{
    may_commit_at, may_commit_at_as_is, propose_ack_ok, propose_ack_ok_as_is,
};

#[path = "../../../../crates/pedradb-raft/src/c1_modelo_kernel.rs"]
pub mod c1_modelo_kernel;

pub use c1_modelo_kernel::*;
