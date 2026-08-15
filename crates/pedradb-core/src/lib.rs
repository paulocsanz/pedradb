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
pub mod bloom;
/// Optional DST buggify annotation sites (RFC-0018 P2.5; no-op unless feature).
pub mod buggify_hooks;
pub mod cache;
pub mod change_feed;
pub mod concurrent;
pub mod db;
pub mod env;
pub mod error;
pub mod host;
pub mod key;
pub mod lock;
pub mod manifest;
pub mod memtable;
pub mod merge;
pub mod occ;
pub mod prefix;
pub mod rng;
pub mod sst;
pub mod time;
pub mod tx;
pub mod vlog;
pub mod wal;

pub use batch::{WriteOp, WriteRecord, WRITE_RECORD_VERSION};
pub use bloom::{BloomFilter, DEFAULT_BITS_PER_KEY};
pub use cache::{BlockCache, TableCache};
pub use change_feed::{
    decode_changelog, ChangeEntry, ChangeKind, ChangeLog, CHANGELOG_FILE_NAME,
};
pub use concurrent::ConcurrentDb;
pub use db::{
    copy_db_directory, read_checkpoint_meta, BatchOp, CheckpointMeta, CompactOptions, Db, DbStats,
    OpenOptions, ScanProjection, Snapshot, WriteOptions, CHECKPOINT_META_FILE,
    L0_COMPACTION_TRIGGER, MAX_LSM_LEVEL, WAL_FILE_NAME,
};
pub use occ::OccTransaction;
pub use vlog::{
    decode_vlog_ref, encode_vlog_ref, ValueLog, VlogRewriteStats, VLOG_FILE_NAME, VLOG_NEW_NAME,
    VLOG_VALUE_PREFIX,
};
pub use env::{Env, EnvFile, StdEnv};
pub use error::{CoreError, Result};
pub use host::{DetHost, Host, StdHost};
pub use key::{
    pack_sequence_and_type, unpack_sequence_and_type, InternalKey, SequenceNumber, ValueType,
    MAX_SEQUENCE_NUMBER,
};
pub use prefix::{
    key_in_prefix_range, prefix_exclusive_end, prefix_exclusive_end_as_is,
};
pub use lock::{DirLock, LOCK_FILE};
pub use manifest::{VersionSet, CURRENT_FILE, MANIFEST_PREFIX};
pub use memtable::{Lookup, MemTable};
pub use merge::{
    collect_range_tombstones, gc_compact_entries, range_deleted, user_key_in_range, visible_range,
    visible_range_limited, CompactGcOptions, RangeTombstone, StreamingVisibleIter, VisibleKv,
};
pub use rng::{mix_seed, Rng, SeedRng, SystemRng};
pub use sst::{
    write_sst, write_sst_entries, write_sst_entries_on, write_sst_on, SstTable,
};
pub use time::{Clock, ManualClock, SystemClock};
pub use tx::Transaction;
