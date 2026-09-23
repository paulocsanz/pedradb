//! Shim WAL: production `recover_kernel` uses `super::format`.

pub mod format {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum RecordType {
        Zero,
        Full,
        First,
        Middle,
        Last,
    }
}

#[path = "../../../../crates/pedradb-core/src/wal/recover_kernel.rs"]
pub mod recover_kernel;

#[path = "../../../../crates/pedradb-core/src/wal/wal_state_kernel.rs"]
pub mod wal_state_kernel;
