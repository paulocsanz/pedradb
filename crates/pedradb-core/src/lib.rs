//! PedraDB core — LSM-tree storage engine in Rust.
//!
//! LSM storage concepts (WAL, MemTable, SSTable, flush, compaction)
//! implemented from scratch in idiomatic Rust. The real RocksDB (C++) is
//! used only as an external test oracle via the `pedradb-oracle` crate —
//! no C++ code is linked into `pedradb-core`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]

pub mod batch;
pub mod bloom;
/// Optional DST buggify annotation sites (RFC-0018 P2.5; no-op unless feature).
pub mod buggify_hooks;

pub mod cache;
pub mod cf_kernel;
pub mod change_feed;
pub mod changelog_kernel;
pub mod compact_kernel;
pub mod concurrent;
pub mod corrupt;
pub mod db;
pub mod env;
pub mod error;
pub mod flush_kernel;
pub mod group_commit_kernel;
pub mod history;
pub mod host;
pub mod key;
pub mod lock;
pub mod manifest;
pub mod manifest_kernel;
pub mod memtable;
pub mod merge;
pub mod occ;
/// Cooperative PCT turnstile hooks (RFC-0051 P0; feature `pct` only).
#[cfg(feature = "pct")]
pub mod pct_hooks;
pub mod prefix;
pub mod rng;
pub mod sst;
pub mod time;
pub mod tx;
pub mod verified;
/// At-rest CRC scrub (RFC-0060).
pub mod verify;
pub mod vlog;
pub mod vlog_gc_kernel;
pub mod wal;

pub use batch::{
    write_record_count_ok, write_record_count_ok_as_is, WriteOp, WriteRecord, WRITE_RECORD_VERSION,
};
pub use bloom::{bloom_header_ok, bloom_header_ok_as_is, BloomFilter, DEFAULT_BITS_PER_KEY, MAX_K};
pub use cache::{BlockCache, TableCache};
pub use cf_kernel::{
    cf_encode_effective, cf_family_of, compact_rewrites_sst_cf, compact_rewrites_sst_cf_as_is,
    decode_cf_key, encode_cf_key, infer_sst_cf, key_in_cf_family, key_in_cf_family_as_is,
};
pub use change_feed::{
    decode_changelog, ChangeEntry, ChangeKind, ChangeLog, CHANGELOG_CORRUPT_FILE_NAME,
    CHANGELOG_FILE_NAME,
};
pub use changelog_kernel::{
    changelog_needs_sst_rebuild, changelog_needs_sst_rebuild_as_is, changelog_should_store,
    changelog_should_store_as_is, DEFAULT_CHANGELOG_INTERVAL,
};
pub use concurrent::ConcurrentDb;
pub use db::{
    copy_db_directory, read_checkpoint_meta, BatchOp, BlobGcCandidate, CheckpointMeta,
    CompactOptions, Db, DbStats, FenceClass, FenceRecovery, FenceReport, HistoryHorizon,
    HistoryOptions, OpenOptions, PreparedL0Compact, ReadProbeSnap, RecoveryReport, ScanProjection,
    Snapshot, SnapshotPin, SstLiveMeta, WalRecovery, WriteOptions, WritePhaseStats,
    CHECKPOINT_META_FILE, L0_COMPACTION_TRIGGER, MAX_LSM_LEVEL, WAL_FILE_NAME,
};
pub use env::{AdviseKind, Env, EnvFile, StdEnv};
pub use error::{CoreError, Result};
pub use host::{DetHost, Host, StdHost};
pub use key::{
    ikey_seq_cmp, pack_sequence_and_type, unpack_sequence_and_type, InternalKey, SequenceNumber,
    ValueType, MAX_SEQUENCE_NUMBER,
};
pub use lock::{DirLock, LOCK_FILE};
pub use manifest::{VersionSet, CURRENT_FILE, MANIFEST_PREFIX};
pub use memtable::{Lookup, MemTable};
pub use merge::{
    collect_range_tombstones, gc_compact_entries, iter_window_keep, iter_window_keep_as_is,
    range_deleted, range_tombstone_covers, range_tombstone_covers_as_is, user_key_in_range,
    visible_at, visible_at_as_is, visible_range, visible_range_limited, CompactGcOptions,
    RangeTombstone, StreamingVisibleIter, VisibleKv, WindowKv, WindowKvIter,
};
pub use occ::OccTransaction;
pub use prefix::{key_in_prefix_range, prefix_exclusive_end, prefix_exclusive_end_as_is};
pub use rng::{mix_seed, Rng, SeedRng, SystemRng};
pub use sst::{write_sst, write_sst_entries, write_sst_entries_on, write_sst_on, SstTable};
pub use time::{Clock, ManualClock, SystemClock};
pub use tx::Transaction;
pub use verified::{
    profile_report, ring_model_admitted, ring_model_admitted_as_is, ring_twin_admitted,
    ring_twin_admitted_as_is, verified_admits_ring, verified_admits_ring_as_is,
    wal_on_sqe_admitted, wal_on_sqe_admitted_as_is, ProfileComponent, ProfileState,
    VerifiedProfile, PROFILE_VERSION,
};
pub use verify::{verify_at_rest, xor_durable_bits, VerifyFailure, VerifyReport};
pub use vlog::{
    blob_path, decode_vlog_ptr, decode_vlog_ref, encode_vlog_ptr, encode_vlog_ref, list_blob_nums,
    ValueLog, VlogPtr, VlogRewriteStats, VLOG_BLOB_PREFIX, VLOG_FILE_NAME, VLOG_NEW_NAME,
    VLOG_VALUE_PREFIX,
};
