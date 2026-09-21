//! Shim: expose the production WAL kernels at the crate root so
//! `super::format::RecordType` inside `recover_kernel` resolves here
//! (Charon walks this crate; the production files are the source of
//! truth — never edit copies).

#[path = "../../../../crates/pedradb-core/src/wal/format_kernel.rs"]
pub mod format;

#[path = "../../../../crates/pedradb-core/src/wal/recover_kernel.rs"]
pub mod recover_kernel;

pub use recover_kernel::*;
