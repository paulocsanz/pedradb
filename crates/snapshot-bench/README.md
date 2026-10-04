# snapshot-bench

The `snapshot_backends` comparative benchmark harness — the sorted-ingest
route-fold workload behind the published Pedra vs RocksDB vs fjall tables:
clustered `route.svc-NNNNNN.NNNNNNNN` keys, 200 B values, 1024-entry batched
applies, point gets (hit and miss), per-service prefix scans, 100-key lookup
batches.

Ported from [beyondoss/slipstream](https://github.com/beyondoss/slipstream)
(MIT), PR #19 / branch `cursor/pedradb-snapshot-adapter-cb1b` @ `d3bc6a4`,
trimmed to the bench surface: the `SnapshotStore` trait, the shared value
codec, and the three on-disk backends (`fjall`, RocksDB via `rust-rocksdb`
0.50, Pedra via the in-tree `rocksdb-compat`). The append-log backend, NATS
watch machinery, and artifact transport are omitted. Everything the benchmark
measures — key/value generators, RNG, batch sizes, criterion groups, tuning
constants — is byte-faithful to upstream.

## Why its own workspace

The official protocol pins the RocksDB peer to `rust-rocksdb 0.50`
(RocksDB 11.1.1). The engine workspace's parity harness pins `rocksdb 0.22`
— the baseline behind the published parity gate — and cargo forbids two
`-sys` crates with the same `links` value in one dependency graph. This crate
is therefore its own workspace, with the engine linked by path
(`../rocksdb-compat`), mirroring upstream where the bench also lives outside
the engine tree.

## Running

```bash
cd crates/snapshot-bench
cargo bench --bench snapshot_backends --features fjall,rocksdb,pedradb -- 'get_hit|prefix_scan|lookup_100'
```

Env knobs (same names as upstream):

| knob | default | meaning |
|---|---|---|
| `SLIPSTREAM_BENCH_ENTRIES` | `1000000` | fold size `n` |
| `SLIPSTREAM_BENCH_VALUE_BYTES` | `200` | value length |
| `SLIPSTREAM_BENCH_CACHE_BYTES` | 256 MiB (official default) | block-cache budget override, same for every backend |
| `SLIPSTREAM_BENCH_BACKENDS` | all | comma list: `fjall,rocksdb,pedradb` |
| `SLIPSTREAM_BENCH_SEQUENTIAL` | auto at ≥ 50M | one backend on disk at a time |

`SLIPSTREAM_BACKENDS` is **not** read by this harness — exporting it
silently runs every enabled backend in one process. Gate one backend per
process with `SLIPSTREAM_BENCH_BACKENDS`.

## Official leg protocol (published-table cells)

One backend per process, Pedra leg then Rocks leg, three runs per cell,
criterion medians (`mid` of `[lo, mid, hi]`); 200 B values, 256 MiB cache,
`TMPDIR` on real NVMe. **Zero-config**: the harness defaults ARE the
protocol — 200 B values, 256 MiB cache for every backend, and Pedra's
bulk chunks self-tune to a 64 MiB ceiling in-engine (RFC-0306, no knob;
the historical `PEDRA_STAGE_MAX_BYTES` pin was a downward-only no-op at
that value). The only envs left are shape selectors and `TMPDIR`:

```bash
export TMPDIR=/path/on/nvme
for r in 1 2 3; do
  for b in pedradb rocksdb; do
    SLIPSTREAM_BENCH_BACKENDS=$b SLIPSTREAM_BENCH_ENTRIES=25000000 \
      cargo bench --bench snapshot_backends --features fjall,rocksdb,pedradb \
      -- 'get_hit|prefix_scan|lookup_100'
  done
done
```

The RocksDB peer is RocksDB **default** with `WriteOptions.sync = false` —
the adapter's default `sync: false`. `hydrate/`, `settle/`, `probe_hit/` and
`probe_miss/` percentiles print to stderr; criterion groups are `get_hit`,
`prefix_scan`, `lookup_100/{name}_get_loop`, `lookup_100/{name}_multi_get`.

Caveats for honest numbers: `TempDir` honors `TMPDIR` — point it at real
NVMe, not tmpfs, when disk behavior matters. Criterion's repeated iterations
measure warm-cache reads; cold-cache latency needs a manual run against a
freshly opened store. Builds need a C++ toolchain and libclang for the
`rocksdb` feature.
