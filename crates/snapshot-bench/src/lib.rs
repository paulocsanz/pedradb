//! `snapshot_backends` benchmark harness: the sorted-ingest route-fold
//! workload used for the published Pedra vs RocksDB vs fjall tables.
//!
//! Ported from [beyondoss/slipstream](https://github.com/beyondoss/slipstream)
//! (MIT license), PR #19 / branch `cursor/pedradb-snapshot-adapter-cb1b`
//! @ `d3bc6a4`, trimmed to the surface the bench exercises:
//!
//! - `kv.rs` — `KvEntry`/`KvUpdate`/`VersionToken`/`WatchCursor` (verbatim).
//! - `snapshot.rs` — the `SnapshotStore` trait and `SnapshotError`, minus the
//!   artifact export/import methods (no `object_store`/`tar` here).
//! - `snapshot_record.rs` — the shared `[ver_len][version][value]` record
//!   codec (verbatim).
//! - `snapshot_{fjall,rocksdb,pedradb}.rs` — the three on-disk backends with
//!   the same tuning constants as upstream; `export_to`/`import` omitted.
//!
//! The append-log backend, NATS watch machinery, and artifact transport of
//! the upstream crate are intentionally absent — this crate exists so the
//! benchmark is reproducible from this repository alone, with Pedra pinned to
//! the in-tree `rocksdb-compat`.

#![deny(unsafe_code)]

mod kv;
pub mod snapshot;
#[cfg(feature = "fjall")]
mod snapshot_fjall;
#[cfg(feature = "pedradb")]
mod snapshot_pedradb;
#[cfg(any(feature = "fjall", feature = "rocksdb", feature = "pedradb"))]
mod snapshot_record;
#[cfg(feature = "rocksdb")]
mod snapshot_rocksdb;

pub use kv::{KvEntry, KvUpdate, VersionToken, WatchCursor};
#[cfg(feature = "fjall")]
pub use snapshot_fjall::{FjallConfig, FjallReader, FjallSnapshot};
#[cfg(feature = "pedradb")]
pub use snapshot_pedradb::{PedraDbConfig, PedraDbReader, PedraDbSnapshot};
#[cfg(feature = "rocksdb")]
pub use snapshot_rocksdb::{RocksDbConfig, RocksDbReader, RocksDbSnapshot};
pub use snapshot::SnapshotStore;
