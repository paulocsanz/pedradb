//! Shim: production `sst/scan_kernel.rs` uses `crate::wal::crc::crc_match_ok`.
//! Charon walks this crate; the production files are the source of truth —
//! never edit copies.

pub mod wal;

#[path = "../../../../crates/pedradb-core/src/sst/scan_kernel.rs"]
pub mod scan_kernel;

pub use scan_kernel::*;
