# Metrics & Health Observability in PedraDB

PedraDB provides built-in, low-overhead observability designed for both embedded programmatic control and cloud-native telemetry pipelines (Kubernetes, Prometheus, alert webhooks).

Observability in PedraDB adheres to core architectural principles:
1. **Zero Lock Contention**: Collecting metrics or evaluating engine health never acquires exclusive write locks or stalls the commit/WAL pipeline.
2. **Zero Heavy Telemetry Dependencies**: The kernel (`pedradb-core`) does not pull in external telemetry SDKs or serialization runtimes (`opentelemetry`, `serde`). All metrics and health reports format through allocation-conscious, pure string buffers.
3. **Actionable Diagnostics Over Raw Counters**: Beyond simple counters, the engine synthesizes symptoms into actionable diagnoses (`HealthIssue`) and concrete remediations (`RecommendedIntervention`).
4. **Cloud-Native & Machine-Readable**: Native OpenMetrics/Prometheus exposition (`/metrics`), structured JSON status (`/health`, `/status`), and automated push webhooks.

---

## 1. Tri-State Health Model

The engine health is continuously evaluated against operational thresholds and categorized into three states:

```rust
pub enum HealthStatus {
    /// All subsystems within nominal operating thresholds.
    Healthy = 0,
    /// Non-fatal degradation (e.g. write pressure, cache thrashing).
    Degraded = 1,
    /// Severe condition requiring operator or automated intervention (e.g. stalled writes, corruption, pin leak).
    ActionRequired = 2,
}
```

### Health Diagnostics (`HealthIssue`)

| Issue | Severity | Condition | Meaning |
|---|---|---|---|
| `WriteStalledL0` | `ActionRequired` | L0 files $\ge$ `write_stall_l0_files` | L0 compaction is overwhelmed; writes are actively blocked. |
| `WriteStalledMem` | `ActionRequired` | Active + imm mem $\ge$ `write_stall_mem_bytes` | Memtable flush is lagging; writes are actively blocked. |
| `WritePressureL0` | `Degraded` | L0 files approaching stall threshold | L0 backlog building up; compaction falling behind ingestion rate. |
| `SnapshotPinLeak` | `ActionRequired` | Oldest pinned seq lag > `max_pin_seq_lag` | Long-lived snapshot pin is preventing MVCC GC (analogous to Postgres transaction holding back `datfrozenxid`). |
| `CorruptionDetected` | `ActionRequired` | Non-zero events in `CORRUPTLOG` journal | Physical media corruption or torn block detected on disk. |
| `DiskSpaceCritical` | `ActionRequired` | Available disk bytes < 500 MB | Filesystem is near full; writes risk hitting `ENOSPC`. |
| `DiskSpaceLow` | `Degraded` | Available disk bytes < 5 GB | Storage headroom is shrinking. |
| `BlockCacheThrashing` | `Degraded` | Block cache hit ratio < 40% with $\ge$ 1,000 lookups | Cache capacity insufficient for current active working set. |
| `TableCacheThrashing` | `Degraded` | Table cache hit ratio < 50% with $\ge$ 1,000 lookups | SST file descriptor cache is thrashing. |
| `AutoCompactFailing` | `ActionRequired` | Consecutive compaction failures > 0 | Background compactions are erroring (I/O, permissions, or corrupt SST). |
| `VlogFragmentationHigh` | `Degraded` | Reclaimable garbage bytes in vlog > 50% | Value log requires garbage collection. |

### Recommended Interventions (`RecommendedIntervention`)

When `ActionRequired` or `Degraded` status is reached, `DbHealthReport::remediations` provides structured remedies:
- `RunManualCompaction`: Trigger explicit compaction to drain L0.
- `ReleaseLeakedSnapshots`: Drop unreleased client snapshot handles that hold back MVCC garbage collection.
- `RunVlogGc`: Trigger value log sweep to reclaim disk space.
- `TuneWriteStallThresholds`: Adjust L0 or memory stall limits for heavy ingestion bursts.
- `IncreaseBlockCacheCapacity`: Expand block cache size.
- `IsolateAndRestoreFromBackup`: Isolate corrupt file and restore via point-in-time recovery.
- `ProvisionMoreDiskSpace`: Free or expand disk volume.
- `CheckDiskAndPermissions`: Validate filesystem permissions and underlying hardware health.

---

## 2. Programmatic API

Applications embedding `pedradb-core` can inspect metrics and health directly in Rust:

```rust
use pedradb_core::{ConcurrentDb, HealthConfig, HealthStatus};

let db = ConcurrentDb::open("/path/to/db")?;

// 1. Snapshot engine statistics
let stats = db.stats();
println!("SSTs: {}, L0 files: {}", stats.sst_count, stats.l0_file_count);
println!("Block Cache Hit Ratio: {:.2}%", stats.block_cache_hit_ratio() * 100.0);
println!("Write Amplification: {:.2}x", stats.write_amplification());
println!("Space Amplification: {:.2}x", stats.space_amplification());

// 2. Evaluate system health
let health = db.health();
if health.status == HealthStatus::ActionRequired {
    eprintln!("Alert! Issues detected: {:?}", health.issues);
    eprintln!("Recommended remediations: {:?}", health.remediations);
    
    // Example automated remediation
    if health.issues.iter().any(|i| matches!(i, pedradb_core::HealthIssue::WriteStalledL0)) {
        eprintln!("Triggering manual compaction to resolve L0 stall...");
        db.compact_range(None, None)?;
    }
}

// 3. Custom health configuration
let custom_cfg = HealthConfig {
    max_pin_seq_lag: 500_000, // Alert if snapshot pin lags by 500k sequences
    min_disk_space_bytes: 10 * 1024 * 1024 * 1024, // 10 GB warning
    ..HealthConfig::default()
};
let custom_report = db.health_with_config(&custom_cfg);
```

### Zero-Dependency Expositions

`pedradb-core` provides pure formatting functions out of the box:

```rust
use pedradb_core::{format_prometheus_metrics, format_json_status};

let stats = db.stats();
let health = db.health();

// Prometheus / OpenMetrics text exposition
let prometheus_text: String = format_prometheus_metrics(&stats, &health);

// Structured JSON status
let json_status: String = format_json_status(&stats, &health);
```

---

## 3. Cloud-Native HTTP Endpoints (`pedradb-http`)

When serving via `pedradb-http`, standard observability endpoints are exposed:

### `GET /metrics`
Returns OpenMetrics / Prometheus formatted plain text (`text/plain; version=0.0.4; charset=utf-8`):

```text
# HELP pedra_healthy Overall health status (0=Healthy, 1=Degraded, 2=ActionRequired)
# TYPE pedra_healthy gauge
pedra_healthy 0
# HELP pedra_write_stalled Whether writes are actively stalled (1=true, 0=false)
# TYPE pedra_write_stalled gauge
pedra_write_stalled 0
# HELP pedra_write_pressured Whether writes are under pressure (1=true, 0=false)
# TYPE pedra_write_pressured gauge
pedra_write_pressured 0
# HELP pedra_sst_files Total number of SST files
# TYPE pedra_sst_files gauge
pedra_sst_files 12
# HELP pedra_l0_files Number of L0 SST files
# TYPE pedra_l0_files gauge
pedra_l0_files 3
# HELP pedra_pin_seq_lag Sequence distance between oldest snapshot pin and current seq
# TYPE pedra_pin_seq_lag gauge
pedra_pin_seq_lag 0
# HELP pedra_block_cache_hit_ratio Hit ratio of block cache (0.0 - 1.0)
# TYPE pedra_block_cache_hit_ratio gauge
pedra_block_cache_hit_ratio 0.942000
# HELP pedra_write_amplification Estimated write amplification factor
# TYPE pedra_write_amplification gauge
pedra_write_amplification 1.850000
# HELP pedra_space_amplification Ratio of disk storage bytes to logical key-value bytes
# TYPE pedra_space_amplification gauge
pedra_space_amplification 1.210000
# HELP pedra_corruptions_detected Number of corruption events recorded in CORRUPTLOG
# TYPE pedra_corruptions_detected counter
pedra_corruptions_detected 0
```

### `GET /health` and `GET /status`
Returns JSON representation of engine health.

**HTTP Status Codes (Cloud-Native Fail-Closed Semantics):**
- **`200 OK`**: Status is `Healthy` or `Degraded`. Traffic is permitted.
- **`503 Service Unavailable`**: Status is `ActionRequired`. Kubernetes readiness/liveness probes automatically evict the pod or fail over traffic.

**Example Response:**
```json
{
  "status": "Healthy",
  "status_code": 0,
  "issues": [],
  "remediations": [],
  "stats": {
    "mem_table_bytes": 1048576,
    "sst_count": 8,
    "l0_file_count": 2,
    "total_disk_bytes": 16777216,
    "pin_seq_lag": 0,
    "is_write_stalled": false,
    "is_write_pressured": false,
    "block_cache_hit_ratio": 0.942,
    "table_cache_hit_ratio": 0.985,
    "write_amplification": 1.45,
    "space_amplification": 1.15,
    "corruptions_detected": 0
  }
}
```

---

## 4. Push Alerting Webhooks

For environments without active pull-based Prometheus scraping, `pedradb-http` supports automated alert webhooks:

```rust
use pedradb_http::dispatch_health_webhook;

let report = db.health();
let stats = db.stats();

if report.status == pedradb_core::HealthStatus::ActionRequired {
    let webhook_addr = "127.0.0.1:9090";
    let endpoint = "/api/v1/alerts";
    if let Err(e) = dispatch_health_webhook(webhook_addr, endpoint, &report, &stats) {
        eprintln!("Failed to push alert webhook: {e}");
    }
}
```

---

## 5. Architectural References

- **RFC-0269**: [Sistema Unificado de Métricas, Saúde, Intervenção e Telemetria](rfc/0269-sistema-unificado-metricas-saude-intervencao-telemetria.md)
- **Comparative Research Report**: [Comparative Study of Database Internals & Health Observability](reports/2026-09-24-internal-metrics-and-health-observability.md)
- **Zero-Twin Verification Policy**: [RFC-0270](rfc/0270-zero-twin-verification.md)
