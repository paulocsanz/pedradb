//! Shim: production `write_cycle_kernel.rs` uses `crate::write_admission_kernel`.

#[path = "../../../../crates/pedradb-core/src/write_admission_kernel.rs"]
pub mod write_admission_kernel;

#[path = "../../../../crates/pedradb-core/src/write_cycle_kernel.rs"]
pub mod write_cycle_kernel;

pub use write_cycle_kernel::*;
