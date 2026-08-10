//! PedraDB core — clean-room LSM-tree storage engine in `Rust`.
//!
//! This crate implements `RocksDB`-style storage concepts (`WAL`, `MemTable`,
//! `SSTable`, flush, compaction) from scratch in idiomatic `Rust`. The real
//! `RocksDB` (C++) is used only as an external test oracle via the
//! `pedradb-oracle` crate — no C++ code is linked into `pedradb-core`.
//!
//! Delivery is structured in vertical slices; see `docs/architecture.md`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]

pub mod error;
pub mod wal;

pub use error::{CoreError, Result};
