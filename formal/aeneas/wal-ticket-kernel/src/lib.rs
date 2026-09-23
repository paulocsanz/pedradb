//! Shim: production `wal_ticket_kernel.rs` uses `crate::write_admission_kernel`.

#[path = "../../../../crates/pedradb-core/src/write_admission_kernel.rs"]
pub mod write_admission_kernel;

#[path = "../../../../crates/pedradb-core/src/wal_ticket_kernel.rs"]
pub mod wal_ticket_kernel;

pub use wal_ticket_kernel::*;
