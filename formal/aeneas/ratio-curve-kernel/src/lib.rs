//! Shim: production `ratio_curve_kernel.rs` uses `crate::scale_kernel`
//! and `crate::write_cycle_kernel` (which uses `crate::write_admission_kernel`).

#[path = "../../../../crates/pedradb-core/src/write_admission_kernel.rs"]
pub mod write_admission_kernel;

#[path = "../../../../crates/pedradb-core/src/scale_kernel.rs"]
pub mod scale_kernel;

#[path = "../../../../crates/pedradb-core/src/write_cycle_kernel.rs"]
pub mod write_cycle_kernel;

#[path = "../../../../crates/pedradb-core/src/ratio_curve_kernel.rs"]
pub mod ratio_curve_kernel;

pub use ratio_curve_kernel::*;
