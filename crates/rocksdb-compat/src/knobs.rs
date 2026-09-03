//! RFC-0062 P0.3 — machine-checkable inventory of rust-rocksdb knobs on this
//! face. A host that compiles a `set_*` must be able to look up whether Pedra
//! honours it, ignores it, refuses it (G2), or diverges *safer*.
//!
//! Silent no-ops that disable integrity are forbidden. `set_verify_checksums(false)`
//! and friends fail at use/open with [`super::ErrorKind::NotSupported`].

/// How a rust-rocksdb knob is treated on this drop-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum KnobClass {
    /// Changes Pedra state (or is the documented default).
    Wired,
    /// Accepted for compile-compat; has no effect. Named so the host can
    /// tell, never so we can pretend we bought cache/pipeline.
    Inert,
    /// Refused. Returning Ok would be operational silent-wrong (G2).
    NotSupported,
    /// Observable KV differs from Rocks *on purpose* and is stricter.
    SaferDivergent,
}

/// One row of the inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnobEntry {
    /// rust-rocksdb method name (`set_verify_checksums`, …).
    pub name: &'static str,
    /// Classification.
    pub class: KnobClass,
    /// When the class applies (empty = always).
    pub when: &'static str,
    /// What the host actually gets.
    pub note: &'static str,
}

/// Every rust-rocksdb setter this crate exposes, classified.
pub const KNOB_INVENTORY: &[KnobEntry] = &[
    // --- Wired ---
    e(
        "create_if_missing",
        KnobClass::Wired,
        "",
        "creates the directory",
    ),
    e(
        "set_sync",
        KnobClass::Wired,
        "",
        "WAL barrier before Ok; default false (Rocks factory)",
    ),
    e(
        "set_wal_full_fsync",
        KnobClass::Wired,
        "",
        "default true = upstream Darwin Rocks F_FULLFSYNC (CMake HAVE_FULLFSYNC)",
    ),
    e(
        "set_write_buffer_size",
        KnobClass::Wired,
        "",
        "default 64 MiB (Rocks factory 0x4000000)",
    ),
    e(
        "set_enable_blob_files",
        KnobClass::Wired,
        "",
        "spill ≥ min_blob_size to VALUES.vlog",
    ),
    e("set_min_blob_size", KnobClass::Wired, "", "blob threshold"),
    e(
        "set_blob_file_size",
        KnobClass::Wired,
        "",
        "vlog rotate cap",
    ),
    e(
        "set_compaction_filter",
        KnobClass::Wired,
        "",
        "applied on compact",
    ),
    e(
        "set_merge_operator",
        KnobClass::Wired,
        "",
        "full merge; partial ignored",
    ),
    e(
        "set_merge_operator_associative",
        KnobClass::Wired,
        "",
        "full merge",
    ),
    e(
        "set_background_error_listener",
        KnobClass::Wired,
        "",
        "fence callback from compact worker",
    ),
    e(
        "set_wal_recovery_mode",
        KnobClass::Wired,
        "PointInTime|FailClosed|AbsoluteConsistency|TolerateCorruptedTailRecords",
        "PIT is compat default; Absolute/Tolerate map to FailClosed",
    ),
    e(
        "create_missing_column_families",
        KnobClass::Wired,
        "",
        "names passed to open_cf are registered",
    ),
    e(
        "set_snapshot",
        KnobClass::Wired,
        "ReadOptions",
        "pins iterator/get_opt at snapshot seq (F180)",
    ),
    e(
        "set_iterate_lower_bound",
        KnobClass::Wired,
        "ReadOptions",
        "iterator bound",
    ),
    e(
        "set_iterate_upper_bound",
        KnobClass::Wired,
        "ReadOptions",
        "iterator bound",
    ),
    e(
        "set_wait",
        KnobClass::Wired,
        "WaitForCompactOptions",
        "wait vs poll",
    ),
    e(
        "set_timeout",
        KnobClass::Wired,
        "WaitForCompactOptions",
        "wait timeout",
    ),
    e(
        "set_move_files",
        KnobClass::Wired,
        "IngestExternalFileOptions",
        "unlink source after ingest",
    ),
    e(
        "WriteOptions::set_sync",
        KnobClass::Wired,
        "",
        "per-write barrier",
    ),
    e(
        "OptimisticTransactionOptions::set_snapshot",
        KnobClass::Wired,
        "",
        "OCC begin pin",
    ),
    e(
        "TransactionDB::transaction",
        KnobClass::Wired,
        "",
        "pessimistic 2PL exclusive key locks",
    ),
    e(
        "TransactionOptions::set_lock_timeout",
        KnobClass::Wired,
        "",
        "2PL lock wait (ms); 0 = try-once",
    ),
    e(
        "TransactionOptions::set_deadlock_detect",
        KnobClass::Wired,
        "",
        "wait-for cycle → Busy",
    ),
    e(
        "set_env",
        KnobClass::Inert,
        "Options",
        "stored for BackupEngine/compile; Pedra I/O is pedradb_core::Env at open",
    ),
    e(
        "Env::set_background_threads",
        KnobClass::Inert,
        "",
        "stored; Pedra keeps one host compact worker",
    ),
    e(
        "set_sst_file_manager",
        KnobClass::Inert,
        "Options",
        "caps stored; compact does not stall on max space yet",
    ),
    e(
        "set_verify_checksums",
        KnobClass::Wired,
        "true (default)",
        "SST/WAL CRC stays on",
    ),
    e(
        "set_paranoid_checks",
        KnobClass::Wired,
        "true",
        "Pedra is always fail-closed on integrity",
    ),
    e(
        "set_checksum_type",
        KnobClass::Wired,
        "CRC32c (default)",
        "Pedra SST is CRC32C",
    ),
    // --- G2 NotSupported (never Ok that disables integrity) ---
    e(
        "set_verify_checksums",
        KnobClass::NotSupported,
        "false",
        "open/get_opt/iterator_opt → ErrorKind::NotSupported; CRC cannot be turned off",
    ),
    e(
        "set_paranoid_checks",
        KnobClass::NotSupported,
        "false",
        "open → NotSupported; Pedra has no write-through-corrupt mode",
    ),
    e(
        "set_checksum_type",
        KnobClass::NotSupported,
        "NoChecksum",
        "open → NotSupported",
    ),
    e(
        "set_wal_recovery_mode",
        KnobClass::NotSupported,
        "SkipAnyCorruptedRecord",
        "open → NotSupported (G2; skip-any does not exist)",
    ),
    // --- Safer-divergent ---
    e(
        "delete_file_in_range",
        KnobClass::SaferDivergent,
        "",
        "tombstone+compact; never unlink SST",
    ),
    e(
        "ingest_external_file",
        KnobClass::SaferDivergent,
        "",
        "re-apply through WAL then flush (G1); not a hardlink of a Rocks SST",
    ),
    e(
        "set_wal_recovery_mode",
        KnobClass::SaferDivergent,
        "AbsoluteConsistency|TolerateCorruptedTailRecords",
        "mapped to FailClosed; Rocks Absolute also refuses open",
    ),
    // --- Inert (compile, no effect) ---
    e(
        "set_use_fsync",
        KnobClass::Inert,
        "",
        "WAL class is set_sync + set_wal_full_fsync",
    ),
    e("set_manual_wal_flush", KnobClass::Inert, "", ""),
    e("set_wal_bytes_per_sync", KnobClass::Inert, "", ""),
    e(
        "increase_parallelism",
        KnobClass::Inert,
        "",
        "no Rocks thread pool",
    ),
    e(
        "set_max_background_jobs",
        KnobClass::Inert,
        "",
        "host compact worker is one thread",
    ),
    e("set_max_open_files", KnobClass::Inert, "", ""),
    e("set_keep_log_file_num", KnobClass::Inert, "", ""),
    e("set_compaction_readahead_size", KnobClass::Inert, "", ""),
    e("set_max_subcompactions", KnobClass::Inert, "", ""),
    e(
        "set_enable_pipelined_write",
        KnobClass::Inert,
        "",
        "RFC-0055 parked",
    ),
    e("set_wal_size_limit_mb", KnobClass::Inert, "", ""),
    e(
        "set_allow_concurrent_memtable_write",
        KnobClass::Inert,
        "",
        "group-commit, not N memtables",
    ),
    e(
        "set_avoid_unnecessary_blocking_io",
        KnobClass::Inert,
        "",
        "",
    ),
    e(
        "set_enable_write_thread_adaptive_yield",
        KnobClass::Inert,
        "",
        "",
    ),
    e("set_log_level", KnobClass::Inert, "", ""),
    e("set_target_file_size_base", KnobClass::Inert, "", ""),
    e("set_target_file_size_multiplier", KnobClass::Inert, "", ""),
    e(
        "set_bottommost_compression_type",
        KnobClass::Inert,
        "",
        "lz4 is the SST codec",
    ),
    e(
        "set_bottommost_zstd_max_train_bytes",
        KnobClass::Inert,
        "",
        "",
    ),
    e(
        "set_prefix_extractor",
        KnobClass::Inert,
        "",
        "prefix CFs are the layout, not a Rocks extractor",
    ),
    e("set_memtable_prefix_bloom_ratio", KnobClass::Inert, "", ""),
    e("set_compression_per_level", KnobClass::Inert, "", ""),
    e(
        "set_compaction_style",
        KnobClass::Inert,
        "",
        "leveled N→N+1",
    ),
    e(
        "set_level_compaction_dynamic_level_bytes",
        KnobClass::Inert,
        "",
        "",
    ),
    e("set_bytes_per_sync", KnobClass::Inert, "", ""),
    e("set_max_write_buffer_number", KnobClass::Inert, "", ""),
    e(
        "set_min_write_buffer_number_to_merge",
        KnobClass::Inert,
        "",
        "",
    ),
    e(
        "set_level_zero_file_num_compaction_trigger",
        KnobClass::Inert,
        "",
        "core L0 trigger",
    ),
    e(
        "set_level_zero_slowdown_writes_trigger",
        KnobClass::Inert,
        "",
        "",
    ),
    e(
        "set_level_zero_stop_writes_trigger",
        KnobClass::Inert,
        "",
        "",
    ),
    e("set_max_bytes_for_level_base", KnobClass::Inert, "", ""),
    e(
        "set_max_bytes_for_level_multiplier",
        KnobClass::Inert,
        "",
        "",
    ),
    e(
        "set_disable_auto_compactions",
        KnobClass::Inert,
        "",
        "host worker still drains L0",
    ),
    e("set_report_bg_io_stats", KnobClass::Inert, "", ""),
    e("set_optimize_filters_for_hits", KnobClass::Inert, "", ""),
    e(
        "set_enable_blob_gc",
        KnobClass::Inert,
        "",
        "vlog GC is compact_vlog / auto_reclaim",
    ),
    e("set_blob_gc_age_cutoff", KnobClass::Inert, "", ""),
    e("set_blob_compression_type", KnobClass::Inert, "", ""),
    e("set_universal_compaction_options", KnobClass::Inert, "", ""),
    e(
        "set_block_based_table_factory",
        KnobClass::Wired,
        "checksum + block_cache",
        "NoChecksum is NotSupported; set_block_cache sizes SST cache",
    ),
    e(
        "optimize_for_point_lookup",
        KnobClass::Wired,
        "",
        "sizes SST block cache to N MiB (RFC-0153)",
    ),
    e("prepare_for_bulk_load", KnobClass::Inert, "", ""),
    e(
        "set_comparator_with_ts",
        KnobClass::Inert,
        "",
        "UDT/versioned CF is a remaining gap",
    ),
    e("set_block_size", KnobClass::Inert, "BlockBasedOptions", ""),
    e(
        "set_bloom_filter",
        KnobClass::Inert,
        "BlockBasedOptions",
        "SST bloom is Pedra's",
    ),
    e(
        "set_block_cache",
        KnobClass::Wired,
        "Options / BlockBasedOptions",
        "Cache::new_lru_cache is the SST block-cache byte budget (RFC-0153)",
    ),
    e(
        "set_cache_index_and_filter_blocks",
        KnobClass::Inert,
        "BlockBasedOptions",
        "",
    ),
    e(
        "set_pin_l0_filter_and_index_blocks_in_cache",
        KnobClass::Inert,
        "BlockBasedOptions",
        "",
    ),
    e(
        "set_whole_key_filtering",
        KnobClass::Inert,
        "BlockBasedOptions",
        "",
    ),
    e(
        "set_format_version",
        KnobClass::Inert,
        "BlockBasedOptions",
        "",
    ),
    e("set_async_io", KnobClass::Inert, "ReadOptions", ""),
    e("fill_cache", KnobClass::Inert, "ReadOptions", ""),
    e(
        "set_prefix_same_as_start",
        KnobClass::Wired,
        "ReadOptions",
        "From(prefix) scan stops at prefix_exclusive_end",
    ),
    e("set_total_order_seek", KnobClass::Inert, "ReadOptions", ""),
    e("set_timestamp", KnobClass::Inert, "ReadOptions", "UDT gap"),
    e("set_readahead_size", KnobClass::Inert, "ReadOptions", ""),
    e("set_pin_data", KnobClass::Inert, "ReadOptions", ""),
    e(
        "set_read_timestamp_for_validation",
        KnobClass::Inert,
        "Transaction",
        "UDT gap",
    ),
    e(
        "set_commit_timestamp",
        KnobClass::Inert,
        "Transaction",
        "UDT gap",
    ),
    e(
        "set_snapshot_consistency",
        KnobClass::Inert,
        "IngestExternalFileOptions",
        "ingest is a regular write",
    ),
    e(
        "set_allow_global_seqno",
        KnobClass::Inert,
        "IngestExternalFileOptions",
        "",
    ),
    e(
        "set_allow_blocking_flush",
        KnobClass::Inert,
        "IngestExternalFileOptions",
        "ingest already flushes",
    ),
    e(
        "set_ingest_behind",
        KnobClass::Inert,
        "IngestExternalFileOptions",
        "",
    ),
];

const fn e(
    name: &'static str,
    class: KnobClass,
    when: &'static str,
    note: &'static str,
) -> KnobEntry {
    KnobEntry {
        name,
        class,
        when,
        note,
    }
}

/// G2 rows: integrity-disabling requests the face must refuse.
#[must_use]
pub fn g2_not_supported() -> Vec<&'static KnobEntry> {
    KNOB_INVENTORY
        .iter()
        .filter(|k| k.class == KnobClass::NotSupported)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn inventory_has_unique_name_when_pairs_and_g2_rows() {
        let mut seen = HashSet::new();
        for k in KNOB_INVENTORY {
            assert!(
                seen.insert((k.name, k.when, k.class as u8)),
                "duplicate knob row {} / {}",
                k.name,
                k.when
            );
        }
        let g2: Vec<_> = g2_not_supported();
        assert!(
            g2.iter().any(|k| k.name == "set_verify_checksums"),
            "G2 must name set_verify_checksums(false)"
        );
        assert!(
            g2.iter().any(|k| k.name == "set_paranoid_checks"),
            "G2 must name set_paranoid_checks(false)"
        );
        assert!(
            g2.iter()
                .any(|k| k.name.contains("wal_recovery") || k.when.contains("SkipAny")),
            "G2 must name skip-any WAL"
        );
        assert!(
            KNOB_INVENTORY
                .iter()
                .any(|k| k.class == KnobClass::Wired && k.name == "set_block_cache"),
            "set_block_cache sizes the SST cache (RFC-0153)"
        );
        assert!(KNOB_INVENTORY
            .iter()
            .any(|k| k.class == KnobClass::SaferDivergent && k.name == "delete_file_in_range"));
        assert!(KNOB_INVENTORY.len() >= 40, "inventory too thin");
    }
}
