//! Shim: production `t1_modelo_kernel.rs` uses `crate::txn_kernel`.
//! Charon walks this crate; production files are `#[path]`.

#[path = "../../../../crates/pedradb-store/src/txn_kernel.rs"]
pub mod txn_kernel;

#[path = "../../../../crates/pedradb-store/src/t1_modelo_kernel.rs"]
pub mod t1_modelo_kernel;

pub use t1_modelo_kernel::*;
