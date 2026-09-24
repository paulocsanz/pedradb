# Snapshot Backends

Three [`SnapshotStore`] backends ship with slipstream. Pick one based on fold size
and read pattern.

## Quick reference

| | `AppendLogSnapshot` | `FjallSnapshot` | `RocksDbSnapshot` |
|---|---|---|---|
| Feature flag | (default) | `fjall` | `rocksdb` |
| Build deps | none | none | C++ toolchain, libclang |
| Fold size | fits in RAM | any | any |
| Cold get p50 (500M routes) | n/a | 542 µs | 292 µs |
| Cold get p999 (500M routes) | n/a | 3.7 ms | 898 µs |
| Hydrate + settle (500M) | n/a | 40 min | 43 min |
| Disk per entry (500M) | n/a | 226 B | 245 B |
| `settle()` cost (500M) | n/a | ~19 min, 2x disk | ~40 s |

## Choosing

**`AppendLogSnapshot`**: fold fits in RAM. No configuration, no dependencies.

**`FjallSnapshot`**: fold is too large for RAM; build environment is pure Rust;
write throughput matters more than cold-read tail latency.

**`RocksDbSnapshot`**: fold is too large for RAM; cold-read tail latency or settle
time matters; operational tooling (`ldb`, `sst_dump`) is useful.

Total time-to-serving-ready at 500M routes is a wash (2602 s fjall, 2593 s
rocksdb). The divergence is tail latency and settle cost.

## Benchmark data

All numbers: 500M routes, ~60 B keys, ~200 B incompressible values, 1 GiB block
cache, NVMe ext4, settled trees. Source: `benches/snapshot_backends.rs`.

### Hydration (`apply` path, 1024-update batches)

| | fjall | rocksdb |
|---|---|---|
| 50M routes | 47 s (1.06 M/s) | 121 s (0.41 M/s) |
| 100M routes | 110 s (0.91 M/s) | 344 s (0.29 M/s) |
| 250M routes | 480 s (0.52 M/s) | 1165 s (0.21 M/s) |
| 500M routes | 1475 s (0.34 M/s) | 2552 s (0.20 M/s) |

fjall throughput decays with scale as compaction debt accumulates during
hydration; rocksdb drains that debt concurrently.

### `settle()` after 500M hydration

| | fjall | rocksdb |
|---|---|---|
| Duration | 1127 s | 41 s |
| Peak disk | 203 GiB (2x: old + new generations) | 105 GiB (shrinks: zstd reaches bottom) |
| Mechanism | Full tree rewrite (major compaction) | Drain queued compactions |

**Call `settle()` before serving.** Both engines accumulate compaction debt during
bulk hydration. Without it, cold point-gets are 8-10x slower (measured: rocksdb
~0.9 ms mean unsettled vs 248 µs mean settled at 500M).

### Cold point-gets (settled, uniform random, 10k probes)

| | fjall | rocksdb |
|---|---|---|
| p50 | 542 µs | 292 µs |
| p90 | 757 µs | 485 µs |
| p99 | 1.9 ms | 686 µs |
| p999 | 3.7 ms | 898 µs |
| max | 39.9 ms | 2.5 ms |

### Absent-key lookups (settled, filters reject in RAM)

| | fjall | rocksdb |
|---|---|---|
| p50 | 421 ns | 321 ns |

Both engines build bottom-level filters. `expect_point_read_hits` (fjall) and
`optimize_filters_for_hits` (rocksdb) are disabled in both backends: without
bottom-level filters, absent-key lookups become guaranteed disk probes, and
cold-get latency on unsettled trees spikes to double digits of milliseconds.

### Prefix scans (hot prefix, 1000 entries)

| | fjall | rocksdb |
|---|---|---|
| 50M routes | 189 µs | 122 µs |
| 500M routes | 176 µs | 129 µs |

Scan latency is flat with scale for both engines. This measures a single
service's routes, resident in cache.

### `RocksDbReader::multi_get` vs get-loop (100 cold keys)

| | settled | unsettled |
|---|---|---|
| get-loop | 23.7 ms | 103 ms |
| multi_get | 19.5 ms | 18.5 ms |

`multi_get` overlaps cold block reads the loop pays sequentially. Its edge
grows with compaction debt and cache-miss rate. Against a hot working set,
the loop is faster (marshaling overhead, nothing to coalesce).

## Usage

```rust
use slipstream::{RocksDbConfig, RocksDbSnapshot, SnapshotStore};

// open or resume
let (cursor, mut store) = RocksDbSnapshot::open(path, RocksDbConfig::default())?;

// after bulk hydration, before serving
store.settle()?;

// concurrent read handle (safe to clone across threads)
let reader = store.reader();
```

```rust
use slipstream::{FjallConfig, FjallSnapshot, SnapshotStore};

let (cursor, mut store) = FjallSnapshot::open(path, FjallConfig::default())?;
store.settle()?; // ~19 min at 500M routes; budget 2x disk headroom
let reader = store.reader();
```

Both backends implement [`SnapshotStore`], so the `watch_applied` integration
is identical regardless of which you choose.
