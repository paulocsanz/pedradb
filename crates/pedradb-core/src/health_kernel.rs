//! Comprehensive health evaluation, self-diagnosis, and telemetry for PedraDB.
//!
//! Provides:
//! - [`HealthStatus`]: Tri-state operational health (`Healthy`, `Degraded`, `ActionRequired`).
//! - [`HealthIssue`]: Specific diagnosed root causes (e.g. pin leak, write stall, compaction debt).
//! - [`RecommendedIntervention`]: Actionable operational remediations for humans or automation.
//! - [`DbHealthReport`]: Snapshot of health, issues, recommendations, and sequence watermark.
//! - [`format_prometheus_metrics`]: Zero-dependency OpenMetrics / Prometheus exporter text.
//! - [`format_json_status`]: FoundationDB-style structured status JSON.

#![forbid(unsafe_code)]

use crate::db::DbStats;
use crate::disk_pressure_kernel::DiskPressureAdmit;
use crate::key::SequenceNumber;

/// Tri-state operational health verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HealthStatus {
    /// System is operating normally within all configured parameters.
    Healthy = 0,
    /// System is experiencing performance degradation or resource pressure,
    /// but is still processing operations without blocking.
    Degraded = 1,
    /// Critical condition requiring manual or automated operator intervention
    /// (e.g. write stalls, failing compactions, snapshot leaks, disk pressure, corruption).
    ActionRequired = 2,
}

impl HealthStatus {
    /// True when status is [`HealthStatus::Healthy`].
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        matches!(self, HealthStatus::Healthy)
    }

    /// True when status is [`HealthStatus::Degraded`].
    #[must_use]
    pub fn is_degraded(&self) -> bool {
        matches!(self, HealthStatus::Degraded)
    }

    /// True when status is [`HealthStatus::ActionRequired`].
    #[must_use]
    pub fn is_action_required(&self) -> bool {
        matches!(self, HealthStatus::ActionRequired)
    }

    /// Canonical string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            HealthStatus::Healthy => "healthy",
            HealthStatus::Degraded => "degraded",
            HealthStatus::ActionRequired => "action_required",
        }
    }

    /// Numeric code for metrics and monitoring (0 = Healthy, 1 = Degraded, 2 = ActionRequired).
    #[must_use]
    pub fn status_code(&self) -> u32 {
        *self as u32
    }
}

/// Diagnosed root causes identified during health evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthIssue {
    /// Ingestion is actively stalled due to L0 file accumulation.
    WriteStalledL0 {
        /// Current number of L0 SST files.
        l0_files: u64,
        /// Configured hard stall limit.
        limit: u64,
    },
    /// Ingestion is actively stalled due to memtable byte consumption.
    WriteStalledMem {
        /// Current approximate memtable bytes.
        mem_bytes: usize,
        /// Configured hard memory stall limit.
        limit: u64,
    },
    /// Ingestion is under soft pressure (draining active writes).
    WritePressureL0 {
        /// Current number of L0 SST files.
        l0_files: u64,
        /// Configured soft pressure threshold.
        threshold: u64,
    },
    /// An open snapshot pin is holding back version GC for too long.
    SnapshotPinLeak {
        /// Oldest snapshot sequence number pinned.
        oldest_pin_seq: SequenceNumber,
        /// Current database sequence number.
        current_seq: SequenceNumber,
        /// Sequence lag between current and pinned.
        lag: u64,
        /// Total number of active snapshot pins.
        pin_count: usize,
    },
    /// Background compactions are failing repeatedly.
    AutoCompactFailing {
        /// Total compaction failures observed.
        failures: u64,
        /// Last observed compaction error message.
        last_error: String,
    },
    /// Value log dead space is high due to updates or deletions.
    VlogFragmentationHigh {
        /// Current ratio of live bytes to total on-disk vlog bytes.
        live_ratio_permille: u32,
        /// Estimated garbage / dead bytes in vlog files.
        dead_bytes: u64,
    },
    /// Free filesystem disk space is below soft reclamation threshold.
    DiskSpaceLow {
        /// Bytes available on filesystem.
        available_bytes: u64,
        /// Soft threshold bytes.
        threshold_bytes: u64,
    },
    /// Free filesystem disk space is below hard refusal floor.
    DiskSpaceCritical {
        /// Bytes available on filesystem.
        available_bytes: u64,
        /// Hard refusal floor bytes.
        floor_bytes: u64,
    },
    /// Corrupted files or CRC mismatches were recorded.
    CorruptionDetected {
        /// Total corruption incidents recorded in journal.
        events: u32,
    },
    /// Block cache hit ratio is severely low under sustained read load.
    BlockCacheThrashing {
        /// Hit ratio permille (0 to 1000).
        hit_ratio_permille: u32,
        /// Total requests sampled (hits + misses).
        total_requests: u64,
    },
    /// Table cache hit ratio is severely low under sustained query load.
    TableCacheThrashing {
        /// Hit ratio permille (0 to 1000).
        hit_ratio_permille: u32,
        /// Total requests sampled (hits + misses).
        total_requests: u64,
    },
}

/// Actionable operational recommendations for operators or automation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecommendedIntervention {
    /// Trigger manual or prioritized compaction to drain L0 accumulation.
    RunManualCompaction {
        /// Specific rationale.
        reason: &'static str,
    },
    /// Identify and abort/drop leaked snapshot handles in application code.
    ReleaseLeakedSnapshots {
        /// Oldest pinned sequence.
        oldest_seq: SequenceNumber,
        /// Sequence lag.
        lag: u64,
    },
    /// Run value log garbage collection (`Db::compact_vlog`).
    RunVlogGc {
        /// Estimated reclaimable bytes.
        reclaimable_bytes: u64,
    },
    /// Tune L0 compaction parallelism or adjust stall limits.
    TuneWriteStallThresholds {
        /// Current L0 file count.
        current_l0: u64,
    },
    /// Increase block cache capacity to reduce disk read thrashing.
    IncreaseBlockCacheCapacity {
        /// Current block cache hits.
        current_hits: u64,
        /// Current block cache misses.
        current_misses: u64,
    },
    /// Inspect disk filesystem, permissions, and I/O error logs.
    CheckDiskAndPermissions {
        /// Last observed error message.
        last_error: String,
    },
    /// Stop node, preserve directory, and restore from backup / replica peer.
    IsolateAndRestoreFromBackup {
        /// Corruption events count.
        events: u32,
    },
    /// Allocate or provision additional storage space immediately.
    ProvisionMoreDiskSpace {
        /// Bytes available.
        available_bytes: u64,
    },
    /// No manual action is currently required.
    NoActionRequired,
}

/// Thresholds for database self-diagnosis.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthConfig {
    /// Sequence lag for an active snapshot pin to be considered degraded.
    pub pin_lag_degraded_threshold: u64,
    /// Sequence lag for an active snapshot pin to require immediate intervention.
    pub pin_lag_action_required_threshold: u64,
    /// Minimum vlog file bytes before evaluating vlog fragmentation.
    pub vlog_fragmentation_min_bytes: u64,
    /// Vlog live ratio below which fragmentation is considered degraded.
    pub vlog_live_ratio_threshold: f64,
    /// Minimum cache requests before evaluating cache thrashing.
    pub cache_thrash_min_requests: u64,
    /// Cache hit ratio below which cache is considered thrashing under load.
    pub cache_thrash_hit_ratio: f64,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            pin_lag_degraded_threshold: 5_000,
            pin_lag_action_required_threshold: 20_000,
            vlog_fragmentation_min_bytes: 16 * 1024 * 1024,
            vlog_live_ratio_threshold: 0.50,
            cache_thrash_min_requests: 500,
            cache_thrash_hit_ratio: 0.50,
        }
    }
}

/// Unified health assessment report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbHealthReport {
    /// Overall health status.
    pub status: HealthStatus,
    /// Diagnostic issues detected.
    pub issues: Vec<HealthIssue>,
    /// Recommended operator interventions.
    pub recommendations: Vec<RecommendedIntervention>,
    /// Database sequence number when checked.
    pub checked_at_seq: SequenceNumber,
}

impl DbHealthReport {
    /// True when status is [`HealthStatus::Healthy`].
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        self.status.is_healthy()
    }

    /// True when status is [`HealthStatus::Degraded`].
    #[must_use]
    pub fn is_degraded(&self) -> bool {
        self.status.is_degraded()
    }

    /// True when status is [`HealthStatus::ActionRequired`].
    #[must_use]
    pub fn is_action_required(&self) -> bool {
        self.status.is_action_required()
    }

    /// Compact single-line summary for logging.
    #[must_use]
    pub fn summary_line(&self) -> String {
        format!(
            "status={} issues={} recommendations={} seq={}",
            self.status.as_str(),
            self.issues.len(),
            self.recommendations.len(),
            self.checked_at_seq
        )
    }

    /// Format report as JSON.
    #[must_use]
    pub fn to_json(&self, stats: &DbStats) -> String {
        format_json_status(stats, self)
    }

    /// Format report as Prometheus metrics text.
    #[must_use]
    pub fn to_prometheus_text(&self, stats: &DbStats) -> String {
        format_prometheus_metrics(stats, self)
    }
}

/// Pure health evaluation over statistics and environmental watermarks.
#[must_use]
pub fn evaluate_db_health(
    stats: &DbStats,
    disk_admit: Option<DiskPressureAdmit>,
    corruption_events: u32,
    config: &HealthConfig,
) -> DbHealthReport {
    let mut status = HealthStatus::Healthy;
    let mut issues = Vec::new();
    let mut recommendations = Vec::new();

    // 1. Data Integrity / Corruption check (highest priority: fail-closed / isolate)
    if corruption_events > 0 {
        status = HealthStatus::ActionRequired;
        issues.push(HealthIssue::CorruptionDetected {
            events: corruption_events,
        });
        recommendations.push(RecommendedIntervention::IsolateAndRestoreFromBackup {
            events: corruption_events,
        });
    }

    // 2. Compaction failure check
    if stats.auto_compact_failures > 0 {
        status = HealthStatus::ActionRequired;
        issues.push(HealthIssue::AutoCompactFailing {
            failures: stats.auto_compact_failures,
            last_error: stats.last_auto_compact_error.clone(),
        });
        recommendations.push(RecommendedIntervention::CheckDiskAndPermissions {
            last_error: stats.last_auto_compact_error.clone(),
        });
    }

    // 3. Write stall check (L0 file or memtable exhaustion)
    if stats.write_stall_l0 > 0 && stats.l0_files >= stats.write_stall_l0 {
        status = HealthStatus::ActionRequired;
        issues.push(HealthIssue::WriteStalledL0 {
            l0_files: stats.l0_files,
            limit: stats.write_stall_l0,
        });
        recommendations.push(RecommendedIntervention::RunManualCompaction {
            reason: "L0 file count exceeded hard write stall limit",
        });
    }
    if stats.write_stall_mem_bytes > 0
        && (stats.mem_approx_bytes as u64) >= stats.write_stall_mem_bytes
    {
        status = HealthStatus::ActionRequired;
        issues.push(HealthIssue::WriteStalledMem {
            mem_bytes: stats.mem_approx_bytes,
            limit: stats.write_stall_mem_bytes,
        });
        recommendations.push(RecommendedIntervention::RunManualCompaction {
            reason: "Memtable memory usage exceeded hard write stall limit",
        });
    }

    // 4. Filesystem disk pressure check
    if let Some(admit) = disk_admit {
        match admit {
            DiskPressureAdmit::Refuse { available, need } => {
                status = HealthStatus::ActionRequired;
                issues.push(HealthIssue::DiskSpaceCritical {
                    available_bytes: available,
                    floor_bytes: need,
                });
                recommendations.push(RecommendedIntervention::ProvisionMoreDiskSpace {
                    available_bytes: available,
                });
            }
            DiskPressureAdmit::Reclaim => {
                if status < HealthStatus::Degraded {
                    status = HealthStatus::Degraded;
                }
                issues.push(HealthIssue::DiskSpaceLow {
                    available_bytes: crate::disk_pressure_kernel::DISK_SOFT_FREE_BYTES,
                    threshold_bytes: crate::disk_pressure_kernel::DISK_SOFT_FREE_BYTES,
                });
                recommendations.push(RecommendedIntervention::ProvisionMoreDiskSpace {
                    available_bytes: crate::disk_pressure_kernel::DISK_SOFT_FREE_BYTES,
                });
            }
            DiskPressureAdmit::Ok => {}
        }
    }

    // 5. Snapshot pin leak check
    if let Some(oldest_seq) = stats.oldest_pin_seq {
        let lag = stats.last_sequence.saturating_sub(oldest_seq);
        if lag >= config.pin_lag_action_required_threshold {
            status = HealthStatus::ActionRequired;
            issues.push(HealthIssue::SnapshotPinLeak {
                oldest_pin_seq: oldest_seq,
                current_seq: stats.last_sequence,
                lag,
                pin_count: stats.snapshot_pin_count,
            });
            recommendations.push(RecommendedIntervention::ReleaseLeakedSnapshots {
                oldest_seq,
                lag,
            });
        } else if lag >= config.pin_lag_degraded_threshold {
            if status < HealthStatus::Degraded {
                status = HealthStatus::Degraded;
            }
            issues.push(HealthIssue::SnapshotPinLeak {
                oldest_pin_seq: oldest_seq,
                current_seq: stats.last_sequence,
                lag,
                pin_count: stats.snapshot_pin_count,
            });
            recommendations.push(RecommendedIntervention::ReleaseLeakedSnapshots {
                oldest_seq,
                lag,
            });
        }
    }

    // 6. Write pressure check (soft threshold)
    if stats.write_pressure_l0 > 0 && stats.l0_files >= stats.write_pressure_l0 {
        if status < HealthStatus::Degraded {
            status = HealthStatus::Degraded;
        }
        issues.push(HealthIssue::WritePressureL0 {
            l0_files: stats.l0_files,
            threshold: stats.write_pressure_l0,
        });
        if !recommendations.iter().any(|r| matches!(r, RecommendedIntervention::RunManualCompaction { .. })) {
            recommendations.push(RecommendedIntervention::RunManualCompaction {
                reason: "L0 file count exceeded soft write pressure threshold",
            });
        }
    }

    // 7. Value log fragmentation check
    if stats.vlog_bytes >= config.vlog_fragmentation_min_bytes {
        let live_ratio = stats.vlog_live_ratio();
        if live_ratio < config.vlog_live_ratio_threshold {
            if status < HealthStatus::Degraded {
                status = HealthStatus::Degraded;
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let permille = (live_ratio * 1000.0).clamp(0.0, 1000.0) as u32;
            let dead_bytes = stats.vlog_bytes.saturating_sub(stats.vlog_live_bytes);
            issues.push(HealthIssue::VlogFragmentationHigh {
                live_ratio_permille: permille,
                dead_bytes,
            });
            recommendations.push(RecommendedIntervention::RunVlogGc {
                reclaimable_bytes: dead_bytes,
            });
        }
    }

    // 8. Cache thrashing checks under load
    let total_block_reqs = stats.block_cache_hits.saturating_add(stats.block_cache_misses);
    if total_block_reqs >= config.cache_thrash_min_requests {
        let ratio = stats.block_cache_hit_ratio();
        if ratio < config.cache_thrash_hit_ratio {
            if status < HealthStatus::Degraded {
                status = HealthStatus::Degraded;
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let permille = (ratio * 1000.0).clamp(0.0, 1000.0) as u32;
            issues.push(HealthIssue::BlockCacheThrashing {
                hit_ratio_permille: permille,
                total_requests: total_block_reqs,
            });
            recommendations.push(RecommendedIntervention::IncreaseBlockCacheCapacity {
                current_hits: stats.block_cache_hits,
                current_misses: stats.block_cache_misses,
            });
        }
    }

    let total_table_reqs = stats.table_cache_hits.saturating_add(stats.table_cache_misses);
    if total_table_reqs >= config.cache_thrash_min_requests {
        let ratio = stats.table_cache_hit_ratio();
        if ratio < config.cache_thrash_hit_ratio {
            if status < HealthStatus::Degraded {
                status = HealthStatus::Degraded;
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let permille = (ratio * 1000.0).clamp(0.0, 1000.0) as u32;
            issues.push(HealthIssue::TableCacheThrashing {
                hit_ratio_permille: permille,
                total_requests: total_table_reqs,
            });
        }
    }

    if recommendations.is_empty() {
        recommendations.push(RecommendedIntervention::NoActionRequired);
    }

    DbHealthReport {
        status,
        issues,
        recommendations,
        checked_at_seq: stats.last_sequence,
    }
}

/// Format metrics and health status into standard Prometheus exposition format.
#[must_use]
pub fn format_prometheus_metrics(stats: &DbStats, health: &DbHealthReport) -> String {
    let mut out = String::with_capacity(2048);

    macro_rules! append_metric {
        ($name:expr, $help:expr, $type:expr, $val:expr) => {
            out.push_str(concat!("# HELP ", $name, " ", $help, "\n"));
            out.push_str(concat!("# TYPE ", $name, " ", $type, "\n"));
            out.push_str(&format!("{} {}\n", $name, $val));
        };
    }

    append_metric!(
        "pedradb_health_status",
        "Overall database health status (0=Healthy, 1=Degraded, 2=ActionRequired)",
        "gauge",
        health.status.status_code()
    );
    append_metric!(
        "pedradb_health_issues_count",
        "Number of active diagnostic health issues",
        "gauge",
        health.issues.len()
    );
    append_metric!(
        "pedradb_last_sequence",
        "Highest committed database sequence number",
        "counter",
        stats.last_sequence
    );
    append_metric!(
        "pedradb_mem_approx_bytes",
        "Memtable approximate memory usage in bytes",
        "gauge",
        stats.mem_approx_bytes
    );
    append_metric!(
        "pedradb_mem_entries",
        "Number of internal entries in active and immutable memtables",
        "gauge",
        stats.mem_entries
    );
    append_metric!(
        "pedradb_sst_count",
        "Number of live SST files in LSM tree",
        "gauge",
        stats.sst_count
    );
    append_metric!(
        "pedradb_sst_bytes",
        "Total size of all SST files on disk in bytes",
        "gauge",
        stats.sst_bytes
    );
    append_metric!(
        "pedradb_l0_files",
        "Number of SST files in level 0",
        "gauge",
        stats.l0_files
    );
    append_metric!(
        "pedradb_wal_bytes",
        "WAL file size on disk in bytes",
        "gauge",
        stats.wal_bytes
    );
    append_metric!(
        "pedradb_wal_sync_count_total",
        "Cumulative WAL fsync / fdatasync operations",
        "counter",
        stats.wal_sync_count
    );
    append_metric!(
        "pedradb_vlog_bytes",
        "Total on-disk value log bytes",
        "gauge",
        stats.vlog_bytes
    );
    append_metric!(
        "pedradb_vlog_live_bytes",
        "Live value payload bytes referenced by LSM tree",
        "gauge",
        stats.vlog_live_bytes
    );
    append_metric!(
        "pedradb_vlog_live_ratio",
        "Ratio of live bytes to total on-disk vlog bytes (0.0 to 1.0)",
        "gauge",
        format!("{:.4}", stats.vlog_live_ratio())
    );
    append_metric!(
        "pedradb_total_disk_bytes",
        "Total disk footprint in bytes (SST + WAL + Vlog)",
        "gauge",
        stats.total_disk_bytes
    );
    append_metric!(
        "pedradb_bytes_ingested_total",
        "Total logical user payload bytes ingested via put/apply",
        "counter",
        stats.bytes_ingested
    );
    append_metric!(
        "pedradb_bytes_written_wal_total",
        "Total bytes written to the WAL",
        "counter",
        stats.bytes_written_wal
    );
    append_metric!(
        "pedradb_bytes_written_sst_total",
        "Total bytes written to SST files via flush and compaction",
        "counter",
        stats.bytes_written_sst
    );
    append_metric!(
        "pedradb_write_amplification",
        "Estimated write amplification ratio ((WAL + SST written) / ingested)",
        "gauge",
        format!("{:.4}", stats.write_amplification())
    );
    append_metric!(
        "pedradb_compact_count_total",
        "Cumulative successful SST compaction operations",
        "counter",
        stats.compact_count
    );
    append_metric!(
        "pedradb_auto_compact_failures_total",
        "Cumulative background compaction failures",
        "counter",
        stats.auto_compact_failures
    );
    append_metric!(
        "pedradb_write_stall_count_total",
        "Cumulative write requests refused or halted due to write stall",
        "counter",
        stats.write_stall_count
    );
    append_metric!(
        "pedradb_write_pressure_count_total",
        "Cumulative soft write pressure drain events",
        "counter",
        stats.write_pressure_count
    );
    append_metric!(
        "pedradb_is_write_stalled",
        "Current write stall state (1=actively stalled, 0=not stalled)",
        "gauge",
        if stats.is_write_stalled { 1 } else { 0 }
    );
    append_metric!(
        "pedradb_is_write_pressured",
        "Current soft write pressure state (1=under pressure, 0=normal)",
        "gauge",
        if stats.is_write_pressured { 1 } else { 0 }
    );
    append_metric!(
        "pedradb_snapshot_pin_count",
        "Current count of active snapshot pins holding back GC",
        "gauge",
        stats.snapshot_pin_count
    );
    append_metric!(
        "pedradb_pin_seq_lag",
        "Sequence lag of the oldest active snapshot pin",
        "gauge",
        stats.pin_seq_lag
    );
    append_metric!(
        "pedradb_table_cache_hit_ratio",
        "Table cache hit ratio (0.0 to 1.0)",
        "gauge",
        format!("{:.4}", stats.table_cache_hit_ratio())
    );
    append_metric!(
        "pedradb_block_cache_hit_ratio",
        "Block cache hit ratio (0.0 to 1.0)",
        "gauge",
        format!("{:.4}", stats.block_cache_hit_ratio())
    );
    append_metric!(
        "pedradb_block_cache_bytes",
        "Block cache resident memory in bytes",
        "gauge",
        stats.block_cache_bytes
    );
    append_metric!(
        "pedradb_corruptions_detected_total",
        "Cumulative corruption events detected in journal",
        "counter",
        stats.corruptions_detected
    );

    out
}

/// Format statistics and health report into structured JSON status (FoundationDB-style).
#[must_use]
pub fn format_json_status(stats: &DbStats, health: &DbHealthReport) -> String {
    let mut out = String::with_capacity(2048);

    out.push_str("{\n");
    out.push_str("  \"health\": {\n");
    out.push_str(&format!("    \"status\": \"{}\",\n", health.status.as_str()));
    out.push_str(&format!("    \"status_code\": {},\n", health.status.status_code()));
    out.push_str(&format!("    \"checked_at_seq\": {},\n", health.checked_at_seq));

    // issues list
    out.push_str("    \"issues\": [\n");
    for (i, issue) in health.issues.iter().enumerate() {
        let is_last = i + 1 == health.issues.len();
        let comma = if is_last { "" } else { "," };
        match issue {
            HealthIssue::WriteStalledL0 { l0_files, limit } => {
                out.push_str(&format!(
                    "      {{\"type\": \"write_stalled_l0\", \"l0_files\": {}, \"limit\": {}}}{}\n",
                    l0_files, limit, comma
                ));
            }
            HealthIssue::WriteStalledMem { mem_bytes, limit } => {
                out.push_str(&format!(
                    "      {{\"type\": \"write_stalled_mem\", \"mem_bytes\": {}, \"limit\": {}}}{}\n",
                    mem_bytes, limit, comma
                ));
            }
            HealthIssue::WritePressureL0 { l0_files, threshold } => {
                out.push_str(&format!(
                    "      {{\"type\": \"write_pressure_l0\", \"l0_files\": {}, \"threshold\": {}}}{}\n",
                    l0_files, threshold, comma
                ));
            }
            HealthIssue::SnapshotPinLeak { oldest_pin_seq, current_seq, lag, pin_count } => {
                out.push_str(&format!(
                    "      {{\"type\": \"snapshot_pin_leak\", \"oldest_pin_seq\": {}, \"current_seq\": {}, \"lag\": {}, \"pin_count\": {}}}{}\n",
                    oldest_pin_seq, current_seq, lag, pin_count, comma
                ));
            }
            HealthIssue::AutoCompactFailing { failures, last_error } => {
                out.push_str(&format!(
                    "      {{\"type\": \"auto_compact_failing\", \"failures\": {}, \"last_error\": \"{}\"}}{}\n",
                    failures, last_error.replace('"', "\\\""), comma
                ));
            }
            HealthIssue::VlogFragmentationHigh { live_ratio_permille, dead_bytes } => {
                out.push_str(&format!(
                    "      {{\"type\": \"vlog_fragmentation_high\", \"live_ratio_permille\": {}, \"dead_bytes\": {}}}{}\n",
                    live_ratio_permille, dead_bytes, comma
                ));
            }
            HealthIssue::DiskSpaceLow { available_bytes, threshold_bytes } => {
                out.push_str(&format!(
                    "      {{\"type\": \"disk_space_low\", \"available_bytes\": {}, \"threshold_bytes\": {}}}{}\n",
                    available_bytes, threshold_bytes, comma
                ));
            }
            HealthIssue::DiskSpaceCritical { available_bytes, floor_bytes } => {
                out.push_str(&format!(
                    "      {{\"type\": \"disk_space_critical\", \"available_bytes\": {}, \"floor_bytes\": {}}}{}\n",
                    available_bytes, floor_bytes, comma
                ));
            }
            HealthIssue::CorruptionDetected { events } => {
                out.push_str(&format!(
                    "      {{\"type\": \"corruption_detected\", \"events\": {}}}{}\n",
                    events, comma
                ));
            }
            HealthIssue::BlockCacheThrashing { hit_ratio_permille, total_requests } => {
                out.push_str(&format!(
                    "      {{\"type\": \"block_cache_thrashing\", \"hit_ratio_permille\": {}, \"total_requests\": {}}}{}\n",
                    hit_ratio_permille, total_requests, comma
                ));
            }
            HealthIssue::TableCacheThrashing { hit_ratio_permille, total_requests } => {
                out.push_str(&format!(
                    "      {{\"type\": \"table_cache_thrashing\", \"hit_ratio_permille\": {}, \"total_requests\": {}}}{}\n",
                    hit_ratio_permille, total_requests, comma
                ));
            }
        }
    }
    out.push_str("    ],\n");

    // recommendations list
    out.push_str("    \"recommended_interventions\": [\n");
    for (i, rec) in health.recommendations.iter().enumerate() {
        let is_last = i + 1 == health.recommendations.len();
        let comma = if is_last { "" } else { "," };
        match rec {
            RecommendedIntervention::RunManualCompaction { reason } => {
                out.push_str(&format!(
                    "      {{\"action\": \"run_manual_compaction\", \"reason\": \"{}\"}}{}\n",
                    reason, comma
                ));
            }
            RecommendedIntervention::ReleaseLeakedSnapshots { oldest_seq, lag } => {
                out.push_str(&format!(
                    "      {{\"action\": \"release_leaked_snapshots\", \"oldest_seq\": {}, \"lag\": {}}}{}\n",
                    oldest_seq, lag, comma
                ));
            }
            RecommendedIntervention::RunVlogGc { reclaimable_bytes } => {
                out.push_str(&format!(
                    "      {{\"action\": \"run_vlog_gc\", \"reclaimable_bytes\": {}}}{}\n",
                    reclaimable_bytes, comma
                ));
            }
            RecommendedIntervention::TuneWriteStallThresholds { current_l0 } => {
                out.push_str(&format!(
                    "      {{\"action\": \"tune_write_stall_thresholds\", \"current_l0\": {}}}{}\n",
                    current_l0, comma
                ));
            }
            RecommendedIntervention::IncreaseBlockCacheCapacity { current_hits, current_misses } => {
                out.push_str(&format!(
                    "      {{\"action\": \"increase_block_cache_capacity\", \"current_hits\": {}, \"current_misses\": {}}}{}\n",
                    current_hits, current_misses, comma
                ));
            }
            RecommendedIntervention::CheckDiskAndPermissions { last_error } => {
                out.push_str(&format!(
                    "      {{\"action\": \"check_disk_and_permissions\", \"last_error\": \"{}\"}}{}\n",
                    last_error.replace('"', "\\\""), comma
                ));
            }
            RecommendedIntervention::IsolateAndRestoreFromBackup { events } => {
                out.push_str(&format!(
                    "      {{\"action\": \"isolate_and_restore_from_backup\", \"events\": {}}}{}\n",
                    events, comma
                ));
            }
            RecommendedIntervention::ProvisionMoreDiskSpace { available_bytes } => {
                out.push_str(&format!(
                    "      {{\"action\": \"provision_more_disk_space\", \"available_bytes\": {}}}{}\n",
                    available_bytes, comma
                ));
            }
            RecommendedIntervention::NoActionRequired => {
                out.push_str(&format!(
                    "      {{\"action\": \"no_action_required\"}}{}\n",
                    comma
                ));
            }
        }
    }
    out.push_str("    ]\n");
    out.push_str("  },\n");

    // storage section
    out.push_str("  \"storage\": {\n");
    out.push_str(&format!("    \"total_disk_bytes\": {},\n", stats.total_disk_bytes));
    out.push_str(&format!("    \"sst_bytes\": {},\n", stats.sst_bytes));
    out.push_str(&format!("    \"wal_bytes\": {},\n", stats.wal_bytes));
    out.push_str(&format!("    \"vlog_bytes\": {},\n", stats.vlog_bytes));
    out.push_str(&format!("    \"vlog_live_bytes\": {},\n", stats.vlog_live_bytes));
    out.push_str(&format!("    \"vlog_live_ratio\": {:.4},\n", stats.vlog_live_ratio()));
    out.push_str(&format!("    \"bytes_ingested\": {},\n", stats.bytes_ingested));
    out.push_str(&format!("    \"bytes_written_wal\": {},\n", stats.bytes_written_wal));
    out.push_str(&format!("    \"bytes_written_sst\": {},\n", stats.bytes_written_sst));
    out.push_str(&format!("    \"write_amplification\": {:.4},\n", stats.write_amplification()));
    out.push_str(&format!("    \"space_amplification\": {:.4}\n", stats.space_amplification()));
    out.push_str("  },\n");

    // lsm section
    out.push_str("  \"lsm\": {\n");
    out.push_str(&format!("    \"last_sequence\": {},\n", stats.last_sequence));
    out.push_str(&format!("    \"max_level\": {},\n", stats.max_level));
    out.push_str(&format!("    \"sst_count\": {},\n", stats.sst_count));
    out.push_str(&format!("    \"sst_entries\": {},\n", stats.sst_entries));
    out.push_str(&format!("    \"l0_files\": {},\n", stats.l0_files));
    out.push_str(&format!("    \"mem_entries\": {},\n", stats.mem_entries));
    out.push_str(&format!("    \"mem_approx_bytes\": {},\n", stats.mem_approx_bytes));
    out.push_str(&format!("    \"compact_count\": {},\n", stats.compact_count));
    out.push_str(&format!("    \"auto_compact_failures\": {},\n", stats.auto_compact_failures));
    out.push_str(&format!("    \"write_stall_count\": {},\n", stats.write_stall_count));
    out.push_str(&format!("    \"write_pressure_count\": {},\n", stats.write_pressure_count));
    out.push_str(&format!("    \"is_write_stalled\": {},\n", stats.is_write_stalled));
    out.push_str(&format!("    \"is_write_pressured\": {}\n", stats.is_write_pressured));
    out.push_str("  },\n");

    // mvcc section
    out.push_str("  \"mvcc\": {\n");
    out.push_str(&format!("    \"earliest_readable_seq\": {},\n", stats.earliest_readable_seq));
    out.push_str(&format!("    \"snapshot_pin_count\": {},\n", stats.snapshot_pin_count));
    match stats.oldest_pin_seq {
        Some(seq) => out.push_str(&format!("    \"oldest_pin_seq\": {},\n", seq)),
        None => out.push_str("    \"oldest_pin_seq\": null,\n"),
    }
    out.push_str(&format!("    \"pin_seq_lag\": {}\n", stats.pin_seq_lag));
    out.push_str("  },\n");

    // cache section
    out.push_str("  \"cache\": {\n");
    out.push_str(&format!("    \"block_cache_hits\": {},\n", stats.block_cache_hits));
    out.push_str(&format!("    \"block_cache_misses\": {},\n", stats.block_cache_misses));
    out.push_str(&format!("    \"block_cache_bytes\": {},\n", stats.block_cache_bytes));
    out.push_str(&format!("    \"block_cache_hit_ratio\": {:.4},\n", stats.block_cache_hit_ratio()));
    out.push_str(&format!("    \"table_cache_hits\": {},\n", stats.table_cache_hits));
    out.push_str(&format!("    \"table_cache_misses\": {},\n", stats.table_cache_misses));
    out.push_str(&format!("    \"table_cache_hit_ratio\": {:.4}\n", stats.table_cache_hit_ratio()));
    out.push_str("  }\n");

    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_evaluation_healthy() {
        let stats = DbStats {
            last_sequence: 1000,
            l0_files: 2,
            write_stall_l0: 12,
            write_pressure_l0: 8,
            vlog_bytes: 10_000,
            vlog_live_bytes: 9_000,
            ..Default::default()
        };
        let cfg = HealthConfig::default();
        let report = evaluate_db_health(&stats, Some(DiskPressureAdmit::Ok), 0, &cfg);
        assert_eq!(report.status, HealthStatus::Healthy);
        assert!(report.is_healthy());
        assert_eq!(report.status.status_code(), 0);
        assert!(report.issues.is_empty());
        assert_eq!(
            report.recommendations,
            vec![RecommendedIntervention::NoActionRequired]
        );
    }

    #[test]
    fn test_health_evaluation_write_stalled() {
        let stats = DbStats {
            last_sequence: 1000,
            l0_files: 14,
            write_stall_l0: 12,
            write_pressure_l0: 8,
            is_write_stalled: true,
            ..Default::default()
        };
        let cfg = HealthConfig::default();
        let report = evaluate_db_health(&stats, Some(DiskPressureAdmit::Ok), 0, &cfg);
        assert_eq!(report.status, HealthStatus::ActionRequired);
        assert!(report.is_action_required());
        assert_eq!(report.status.status_code(), 2);
        assert!(report
            .issues
            .iter()
            .any(|i| matches!(i, HealthIssue::WriteStalledL0 { .. })));
        assert!(report
            .recommendations
            .iter()
            .any(|r| matches!(r, RecommendedIntervention::RunManualCompaction { .. })));
    }

    #[test]
    fn test_health_evaluation_pin_leak() {
        let stats = DbStats {
            last_sequence: 30_000,
            oldest_pin_seq: Some(1_000),
            pin_seq_lag: 29_000,
            snapshot_pin_count: 5,
            ..Default::default()
        };
        let cfg = HealthConfig::default();
        let report = evaluate_db_health(&stats, Some(DiskPressureAdmit::Ok), 0, &cfg);
        assert_eq!(report.status, HealthStatus::ActionRequired);
        assert!(report.issues.iter().any(|i| match i {
            HealthIssue::SnapshotPinLeak { lag, .. } => *lag == 29_000,
            _ => false,
        }));
    }

    #[test]
    fn test_health_evaluation_corruption_escalates_highest() {
        let stats = DbStats {
            last_sequence: 500,
            ..Default::default()
        };
        let cfg = HealthConfig::default();
        let report = evaluate_db_health(&stats, Some(DiskPressureAdmit::Ok), 2, &cfg);
        assert_eq!(report.status, HealthStatus::ActionRequired);
        assert!(report
            .recommendations
            .iter()
            .any(|r| matches!(r, RecommendedIntervention::IsolateAndRestoreFromBackup { .. })));
    }

    #[test]
    fn test_prometheus_formatting_contains_core_metrics() {
        let stats = DbStats {
            last_sequence: 42,
            mem_approx_bytes: 1024,
            sst_count: 5,
            sst_bytes: 2048,
            total_disk_bytes: 4096,
            ..Default::default()
        };
        let report = DbHealthReport {
            status: HealthStatus::Healthy,
            issues: vec![],
            recommendations: vec![RecommendedIntervention::NoActionRequired],
            checked_at_seq: 42,
        };
        let prom = format_prometheus_metrics(&stats, &report);
        assert!(prom.contains("pedradb_health_status 0"));
        assert!(prom.contains("pedradb_last_sequence 42"));
        assert!(prom.contains("pedradb_mem_approx_bytes 1024"));
        assert!(prom.contains("pedradb_total_disk_bytes 4096"));
    }

    #[test]
    fn test_json_status_formatting_valid_structure() {
        let stats = DbStats {
            last_sequence: 100,
            mem_approx_bytes: 512,
            sst_count: 2,
            ..Default::default()
        };
        let report = DbHealthReport {
            status: HealthStatus::Degraded,
            issues: vec![HealthIssue::WritePressureL0 {
                l0_files: 9,
                threshold: 8,
            }],
            recommendations: vec![RecommendedIntervention::RunManualCompaction {
                reason: "soft pressure",
            }],
            checked_at_seq: 100,
        };
        let json = format_json_status(&stats, &report);
        assert!(json.contains("\"status\": \"degraded\""));
        assert!(json.contains("\"status_code\": 1"));
        assert!(json.contains("\"type\": \"write_pressure_l0\""));
        assert!(json.contains("\"action\": \"run_manual_compaction\""));
    }

    #[test]
    fn test_live_db_health_and_stats() {
        let dir = std::env::temp_dir().join(format!(
            "pedra_test_health_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        let mut db = crate::Db::open(&dir).expect("Db::open must succeed");
        db.put(b"alpha", b"123").unwrap();
        db.put(b"beta", b"456").unwrap();

        let stats = db.stats();
        assert_eq!(stats.last_sequence, 2);
        assert!(stats.mem_approx_bytes > 0);
        assert_eq!(stats.mem_entries, 2);
        assert_eq!(stats.snapshot_pin_count, 0);
        assert_eq!(stats.pin_seq_lag, 0);
        assert!(!stats.is_write_stalled);
        assert_eq!(stats.corruptions_detected, 0);

        let health = db.health();
        assert_eq!(health.status, HealthStatus::Healthy);
        assert!(health.is_healthy());
        assert_eq!(health.checked_at_seq, 2);

        // Pin a snapshot and verify pin observability
        let pin = db.pin_snapshot();
        db.put(b"gamma", b"789").unwrap();
        let stats_pinned = db.stats();
        assert_eq!(stats_pinned.snapshot_pin_count, 1);
        assert_eq!(stats_pinned.oldest_pin_seq, Some(2));
        assert_eq!(stats_pinned.pin_seq_lag, 1);
        db.release_snapshot_pin(pin);

        let stats_unpinned = db.stats();
        assert_eq!(stats_unpinned.snapshot_pin_count, 0);
        assert_eq!(stats_unpinned.pin_seq_lag, 0);

        // Verify JSON and Prometheus text outputs
        let prom = health.to_prometheus_text(&stats);
        assert!(prom.contains("pedradb_health_status 0"));
        assert!(prom.contains("pedradb_last_sequence 2"));

        let json = health.to_json(&stats);
        assert!(json.contains("\"status\": \"healthy\""));
        assert!(json.contains("\"last_sequence\": 2"));

        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
