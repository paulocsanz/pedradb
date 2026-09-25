//! PedraDB core — LSM-tree storage engine in Rust.
//!
//! The embed handle is [`ConcurrentDb`]. One writer pays one atomic on the
//! in-flight counter (`active`) and then the lone commit — the same WAL
//! `fdatasync`-before-Ok as the engine. [`db::Db`] is that engine, not a
//! second database to open from outside this crate.
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

#[path = "batch_kernel.rs"]
pub mod batch;
#[path = "bloom_kernel.rs"]
pub mod bloom;
/// Optional DST buggify annotation sites (RFC-0018 P2.5; no-op unless feature).
#[path = "buggify_hooks_kernel.rs"]
pub mod buggify_hooks;
#[path = "bulk_ingest_kernel.rs"]
pub mod bulk_ingest;
#[path = "bulk_run_kernel.rs"]
pub(crate) mod bulk_run;

#[path = "cache_kernel.rs"]
pub mod cache;
pub mod cf_kernel;
#[path = "change_feed_kernel.rs"]
pub mod change_feed;
pub mod changelog_kernel;
pub mod client_axis_kernel;
pub mod compact_kernel;
#[path = "concurrent_kernel.rs"]
pub mod concurrent;
#[path = "corrupt_kernel.rs"]
pub mod corrupt;
pub mod d1_modelo_kernel;
#[doc(hidden)]
#[path = "db_kernel.rs"]
pub mod db;
pub mod durability_spine_kernel;
#[path = "env_kernel.rs"]
pub mod env;
pub mod env_crash_kernel;
#[path = "error_kernel.rs"]
pub mod error;
pub mod flush_kernel;
pub mod group_commit_kernel;
pub mod group_window_kernel;
/// Health assessment, self-diagnosis, and telemetry exporters.
pub mod health_kernel;
#[path = "history_kernel.rs"]
pub mod history;
#[path = "host_kernel.rs"]
pub mod host;
#[path = "key_kernel.rs"]
pub mod key;
#[path = "leveling_kernel.rs"]
mod leveling;
pub mod lsm_r1_kernel;
/// RFC-0278: LSM Inductive Bisimulation & Snapshot Equivalence Kernel.
pub mod lsm_bisimulation_kernel;
/// RFC-0278: FSCQ-class Mechanized Crash-Recovery Refinement Kernel.
pub mod crash_refinement_kernel;
/// RFC-0278: Bounded Dynamic Resources & Stack Limits Kernel.
pub mod bounded_alloc_kernel;
/// RFC-0279: Manifest Confluence & Church-Rosser VersionSet Kernel.
pub mod manifest_confluence_kernel;
/// RFC-0279: Liveness, Starvation-Freedom & Deadlock-Free Backpressure Kernel.
pub mod liveness_progress_kernel;
/// RFC-0279: Serializable Snapshot Isolation (SSI) & Anti-Dependency Kernel.
pub mod ssi_conflict_kernel;
/// RFC-0279: Merge Operator Associativity & CRDT Determinism Kernel.
pub mod merge_determinism_kernel;
/// RFC-0279: Causal Seam Order & Handler Invariant Kernel.
pub mod causal_seam_kernel;
/// RFC-0280: VLog Referential Integrity & Blob Pointer Kernel.
pub mod vlog_integrity_kernel;
/// RFC-0280: RAM Pre-Flush Barrier & DRAM Bit-Rot Protection Kernel.
pub mod ram_preflush_barrier_kernel;
/// RFC-0280: Iterator Pinning & Hazard Reference Counting Kernel.
pub mod iterator_pinning_kernel;
/// RFC-0280: Hybrid Logical Clock & Monotonic Time Horizon Kernel.
pub mod monotonic_clock_kernel;
/// RFC-0280: Amortized Space & Write Amplification Bounds Kernel.
pub mod space_amplification_kernel;
/// RFC-0281: Bloom Filter Zero False Negative Soundness Kernel.
pub mod bloom_soundness_kernel;
/// RFC-0281: Dual-Log Recovery & Anti-Zombie Resurrection Kernel.
pub mod dual_log_recovery_kernel;
/// RFC-0281: Tombstone Soundness & Unmasking Prevention Kernel.
pub mod tombstone_soundness_kernel;
/// RFC-0281: Strict Weak Ordering Key Comparator Axiom Kernel.
pub mod comparator_axiom_kernel;
/// RFC-0281: Direct I/O 4096-Byte Alignment & DMA Coherence Contract Kernel.
pub mod direct_io_contract_kernel;
/// RFC-0282: RC11 Relaxed Memory Causality Kernel.
pub mod rc11_relaxed_memory_kernel;
/// RFC-0282: Torn Sector Heal & Dual-Boundary Generation Envelope Kernel.
pub mod torn_sector_heal_kernel;
/// RFC-0282: Asymmetric Network Partition & Leader Lease Kernel.
pub mod asymmetric_lease_kernel;
/// RFC-0282: Range Scan Snapshot Linearizability Kernel.
pub mod range_scan_linear_kernel;
/// RFC-0282: VLog Garbage Collection Tri-Color Reachability Barrier Kernel.
pub mod vlog_gc_barrier_kernel;
/// RFC-0282: Compaction Termination & Lyapunov Metric Kernel.
pub mod compaction_lyapunov_kernel;
/// RFC-0282: Volatile Zeroization & Residual Entropy Elimination Kernel.
pub mod zeroize_entropy_kernel;
/// RFC-0282: Dense Time Scheduler & Epsilon Perturbation Kernel.
pub mod dense_time_scheduler_kernel;
/// RFC-0282: Two-Phase Commit Distributed Recovery Confluence Kernel.
pub mod twopc_confluence_kernel;
/// RFC-0282: Schema Homomorphism & Categorical Evolution Kernel.
pub mod schema_homomorphism_kernel;
/// RFC-0283: Starvation Freedom & K-Bounded Overtaking Kernel.
pub mod starvation_freedom_kernel;
/// RFC-0283: Lossless Codec Inversion & Bijective Roundtrip Kernel.
pub mod codec_inversion_kernel;
/// RFC-0283: Prefix Delta-Encoding & Restart Point Soundness Kernel.
pub mod prefix_delta_restart_kernel;
/// RFC-0283: Cross-Column Family Isolation & Replay Independence Kernel.
pub mod cross_cf_isolation_kernel;
/// RFC-0283: In-Memory Block Cache Canary Sentinel Kernel.
pub mod cache_canary_sentinel_kernel;
/// RFC-0283: Dynamic SSI Serialization Graph Cycle Detector Kernel.
pub mod ssi_cycle_detector_kernel;
/// RFC-0283: Bounded Snapshot Epoch Lease & Anti-Space-Explosion Kernel.
pub mod snapshot_epoch_lease_kernel;
/// RFC-0283: Priority Inversion Freedom & Dual-Lane Admission Kernel.
pub mod priority_inversion_freedom_kernel;
/// RFC-0283: POSIX Parent Directory Synchronization Order Kernel.
pub mod posix_dir_sync_kernel;
/// RFC-0283: Replication Snapshot-to-Log Catch-Up Boundary Kernel.
pub mod replication_catchup_boundary_kernel;
/// RFC-0284: Prefix-Free Composite Key Injective Encoding Kernel.
pub mod prefix_free_key_kernel;
/// RFC-0284: Per-Table Entropy and Hash-Flooding Resilient Bloom Kernel.
pub mod bloom_hash_entropy_kernel;
/// RFC-0284: Compaction Debt Pacing and Write-Stall Prevention Kernel.
pub mod compaction_pacing_kernel;
/// RFC-0284: Super-Atomic MultiGet Snapshot Coherence Kernel.
pub mod super_atomic_multiget_kernel;
/// RFC-0284: Flash Translation Layer (FTL) Erase-Block Alignment Kernel.
pub mod ftl_erase_boundary_kernel;
/// RFC-0284: Cross-Device (EXDEV) Safe Migration Protocol Kernel.
pub mod cross_device_barrier_kernel;
/// RFC-0284: Monotonic Sequence Number Horizon and Rollover Prevention Kernel.
pub mod sequence_horizon_kernel;
/// RFC-0284: Pre-Manifest Orphan SST Recovery and Idempotent Cleanup Kernel.
pub mod orphan_sst_cleanup_kernel;
/// RFC-0284: Bitemporal Secondary Index Coherence Kernel.
pub mod bitemporal_index_kernel;
/// RFC-0284: Async Cancellation Safety and Leader Handover Kernel.
pub mod async_cancellation_kernel;
/// RFC-0285: Zero-Allocation Hot-Path and Static Slab Reservation Kernel.
pub mod hot_path_zero_alloc_kernel;
/// RFC-0285: MemTable-to-SST Flush Bisimulation and Strict Monotonicity Kernel.
pub mod memtable_flush_bisimulation_kernel;
/// RFC-0285: Superblock Cryptographic Identity and Anti-Inode-Reuse Kernel.
pub mod file_identity_superblock_kernel;
/// RFC-0285: Memory-Mapped Region Quiescence and Safe Unmap Epoch Barrier Kernel.
pub mod mmap_quiescence_barrier_kernel;
/// RFC-0285: Block Cache Key Disambiguation and Anti-Incarnation Collision Kernel.
pub mod block_cache_disambiguation_kernel;
/// RFC-0285 Pilar 1: Lease Expiration and Runtime Freeze Guard Kernel.
pub mod lease_expiration_guard_kernel;
/// RFC-0285 Pilar 2: Mesh MTU Fragmentation and Blackhole Defense Kernel.
pub mod mesh_mtu_fragmentation_kernel;
/// RFC-0285 Pilar 3: Federated Cursor Continuity and Bisimulation Kernel.
pub mod federated_cursor_continuity_kernel;
/// RFC-0285 Pilar 4: Metal Parent Directory POSIX Fsync Barrier Kernel.
pub mod metal_fsync_barrier_kernel;
/// RFC-0285 Pilar 5: Multi-Tenant Prefix Isolation and Non-Interference Kernel.
pub mod multitenant_prefix_isolation_kernel;
/// RFC-0285 Pilar 6: Async Pool Decoupling and Starvation-Free Group Commit Kernel.
pub mod async_pool_decoupling_kernel;
/// RFC-0285 Pilar 7: Asymmetric Partition Quorum and Bipartite Confluence Kernel.
pub mod asymmetric_partition_quorum_kernel;
/// RFC-0285 Pilar 8: File Descriptor Quota Governor and EMFILE Prevention Kernel.
pub mod fd_quota_governor_kernel;
/// RFC-0285 Pilar 9: Boot-ID Stale Lockfile Reclaim and Crash Recovery Kernel.
pub mod boot_id_lockfile_kernel;
/// RFC-0285 Pilar 10: Bidirectional Schema Homomorphism and Unknown-Field Preservation Kernel.
pub mod rolling_upgrade_homomorphism_kernel;
pub mod product_crown_kernel;
pub mod rmw_sched_kernel;
pub mod wal_buffer_kernel;
pub mod wal_ticket_kernel;
pub mod filter_partition_kernel;
pub mod workload_class_kernel;
pub mod write_ack_kernel;
pub mod write_cycle_kernel;

/// Disk-pressure watermarks (RFC-0179): refuse writes before ENOSPC, keep reads up.
pub mod disk_pressure_kernel;
/// RAM-pressure watermarks and backpressure admission (OOM prevention).
pub mod ram_pressure_kernel;
/// Unified backpressure and protection kernel (RFC-0274).
pub mod backpressure_kernel;
pub mod leftover_page_kernel;
#[path = "lock_kernel.rs"]
pub mod lock;
pub mod lookup_kernel;
#[path = "manifest_mod_kernel.rs"]
pub mod manifest;
pub mod manifest_kernel;
#[path = "memtable_kernel.rs"]
pub mod memtable;
#[path = "merge_kernel.rs"]
pub mod merge;
#[path = "occ_kernel.rs"]
pub mod occ;
/// Cooperative PCT turnstile hooks (RFC-0051 P0; feature `pct` only).
#[cfg(feature = "pct")]
#[path = "pct_hooks_kernel.rs"]
pub mod pct_hooks;
#[path = "prefix_kernel.rs"]
pub mod prefix;
pub mod probe_order_kernel;
pub mod ratio_curve_kernel;
#[path = "rng_kernel.rs"]
pub mod rng;
/// One-process scale model (RFC-0176): probes, WARM cap, spectrum clock.
pub mod scale_kernel;
pub mod scan_readahead_kernel;
#[path = "sst/mod_kernel.rs"]
pub mod sst;
#[path = "time_kernel.rs"]
pub mod time;
#[path = "tx_kernel.rs"]
pub mod tx;
#[path = "verified_kernel.rs"]
pub mod verified;
/// At-rest CRC scrub (RFC-0060).
#[path = "verify_kernel.rs"]
pub mod verify;
#[path = "vlog_kernel.rs"]
pub mod vlog;
pub mod vlog_gc_kernel;
#[path = "wal/mod_kernel.rs"]
pub mod wal;
pub mod write_admission_kernel;

pub use batch::{
    write_record_count_ok, write_record_count_ok_as_is, WriteOp, WriteRecord, WRITE_RECORD_VERSION,
};
pub use bloom::{
    bloom_decode_count, bloom_header_ok, bloom_header_ok_as_is, filter_bytes_loaded,
    filter_part_load_count, reset_bloom_decode_count, reset_filter_part_load_count, BloomFilter,
    DEFAULT_BITS_PER_KEY, MAX_K,
};
pub use filter_partition_kernel::{
    filter_nparts, filter_nparts_as_is, filter_partition, filter_partition_as_is,
};
pub use cache::{BlockCache, SstPayloadPool, TableCache};
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
    copy_db_directory, escape_inline_value, read_checkpoint_meta, BatchOp, BlobGcCandidate,
    CheckpointMeta, CompactOptions, Db, DbStats, FenceClass, FenceRecovery, FenceReport,
    HistoryHorizon, HistoryOptions, OpenOptions, PreparedL0Compact, ReadProbeSnap, RecoveryReport,
    ScanProjection, Snapshot, SnapshotPin, SstLiveMeta, WalRecovery, WriteOptions, WritePhaseStats,
    CHECKPOINT_META_FILE, DEFAULT_SST_PAYLOAD_BUDGET_BYTES, L0_COMPACTION_TRIGGER, MAX_LSM_LEVEL,
    WAL_FILE_NAME,
};
pub use disk_pressure_kernel::{
    compact_refuse, disk_pressure_admit, disk_probe_or_unknown, external_write_admitted,
    DiskPressureAdmit, DISK_HARD_FREE_BYTES, DISK_SOFT_FREE_BYTES,
};
pub use backpressure_kernel::{
    BackpressureConfig, CompactionDebtVerdict, CompactionIoPacer, ConcurrencyVerdict,
    SnapshotPinVerdict, VlogGcVerdict,
};
pub use health_kernel::{
    evaluate_db_health, format_json_status, format_prometheus_metrics, DbHealthReport,
    HealthConfig, HealthIssue, HealthStatus, RecommendedIntervention,
};
pub use env::{
    admit_disk_write, probe_available_bytes, AdviseKind, Env, EnvFile, EnvSource, SstFileSource,
    StdEnv,
};
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
pub use sst::{
    block_hash_probe_count, point_hash_probe_count, prefix_full_key_bytes, prefix_shared_len,
    prefix_trunc_key_bytes, reset_block_hash_probe_count, write_sst, write_sst_entries,
    write_sst_entries_on, write_sst_on, SstTable,
};
pub use time::{Clock, ManualClock, SystemClock};
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
pub use workload_class_kernel::{workload_class, workload_class_as_is, WorkloadClass};
pub mod sync_kernel;
