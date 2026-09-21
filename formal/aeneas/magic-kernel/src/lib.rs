//! Shim: production `sst/magic_kernel.rs` reads `super::table::SST_MAGIC`.
//! `sst/table.rs` does not compile standalone, so the const is re-exposed
//! here byte-for-byte; `scripts/aeneas_magic.sh` FAILS if the copy drifts
//! from the production `table.rs` line. Production files are the source of
//! truth — never edit copies.

pub mod table {
    /// Pinned to `crates/pedradb-core/src/sst/table_kernel.rs` by aeneas_magic.sh.
    pub const SST_MAGIC: &[u8; 8] = b"PEDRSST\0";
}

#[path = "../../../../crates/pedradb-core/src/sst/magic_kernel.rs"]
pub mod magic_kernel;
