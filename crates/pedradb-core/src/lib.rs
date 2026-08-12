//! PedraDB core — clean-room LSM-tree storage engine in Rust.
//!
//! This crate implements RocksDB-style storage concepts (WAL, MemTable,
//! SSTable, flush, compaction) from scratch in idiomatic Rust. The real
//! RocksDB (C++) is used only as an external test oracle via the
//! `pedradb-oracle` crate — no C++ code is linked into `pedradb-core`.
//!
//! Delivery is structured in vertical slices; see `docs/architecture.md`
//! and `docs/rfc/0001-pedradb-high-level-spec.md`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]

pub mod batch;
pub mod db;
pub mod env;
pub mod error;
pub mod key;
pub mod lock;
pub mod manifest;
pub mod memtable;
pub mod merge;
pub mod sst;
pub mod tx;
pub mod wal;

pub use batch::{WriteOp, WriteRecord, WRITE_RECORD_VERSION};
pub use db::{BatchOp, CompactOptions, Db, OpenOptions, Snapshot, WriteOptions, WAL_FILE_NAME};
pub use env::{Env, EnvFile, StdEnv};
pub use error::{CoreError, Result};
pub use key::{
    pack_sequence_and_type, unpack_sequence_and_type, InternalKey, SequenceNumber, ValueType,
    MAX_SEQUENCE_NUMBER,
};
pub use lock::{DirLock, LOCK_FILE};
pub use manifest::{VersionSet, CURRENT_FILE, MANIFEST_PREFIX};
pub use memtable::{Lookup, MemTable};
pub use merge::{
    gc_compact_entries, user_key_in_range, visible_range, CompactGcOptions, VisibleKv,
};
pub use sst::{
    write_sst, write_sst_entries, write_sst_entries_on, write_sst_on, SstTable,
};
pub use tx::Transaction;
