# Benchmarks — protocol, per-run values, and loss registry

This is the scrutiny annex for the tables in the main
[`README.md`](../README.md). The README carries the results; this file
carries how each cell was measured, the per-run values behind every
median, and every named loss. Nothing here is a win claim unless the
README says so.

## The peer

The only peer that counts is **RocksDB default** with
`WriteOptions.sync=false` — the class production Rocks actually runs.
Consequences, stated as law:

- A ratio against `sync=true` is never a win.
- The Pedra engine still `fdatasync`s before `Ok` in its default
  configuration; the async column matches the peer's durability class
  explicitly.
- A cell publishes only when both backends are in band. If the peer
  measures outside its own historical band on a shape, the ratio is
  refused (see 25M `get_loop` below).
- Bold in the README is a win claim. Plain is a tie or parity. `—` is
  not measured or refused.

## Harnesses

| harness | what it measures | where |
|---|---|---|
| `snapshot_backends` | sorted-ingest route-fold: hydrate, settle, point gets, prefix scans, 100-key lookups | `crates/snapshot-bench` (own workspace) |
| `scale-parity-bench` | same key shape, Fjall column, scale ladder | `rocksdb-parity-bench` |
| `rocks-parity-bench` | YCSB / dependents shapes | `rocksdb-parity-bench` |

`snapshot_backends` was ported from slipstream PR 19 (branch
`cursor/pedradb-snapshot-adapter-cb1b` @ `d3bc6a4`, MIT); the workload
code is byte-faithful to upstream. It pins the Rocks peer to
`rust-rocksdb` 0.50 (RocksDB 11.1.1) in its own workspace — the engine
workspace pins `rocksdb` 0.22 for the parity harness, and cargo forbids
two `links = "rocksdb"` crates in one graph.

Knobs: `SLIPSTREAM_BENCH_ENTRIES`, `SLIPSTREAM_BENCH_VALUE_BYTES` (200),
`SLIPSTREAM_BENCH_CACHE_BYTES`, `SLIPSTREAM_BENCH_BACKENDS`
(comma list), `SLIPSTREAM_BENCH_SEQUENTIAL`. Note the trap:
`SLIPSTREAM_BACKENDS` (without `BENCH_`) is **not** read — exporting it
silently runs every enabled backend in one process.

## Official leg protocol

Linux, single guest (4 vCPU on a Threadripper PRO 3975WX host), NVMe via
`TMPDIR`, `vm.dirty_ratio=5`, `vm.dirty_background_ratio=1`. Values are
200 B, batches 1024 entries, block cache 256 MiB, Pedra bulk-stage clamp
64 MiB (`PEDRA_STAGE_MAX_BYTES=67108864`). One backend per process —
`SLIPSTREAM_BENCH_BACKENDS=pedradb`, then `=rocksdb` — Pedra leg first.
A 50k-entry smoke per backend must exit 0 before any official leg. A
published cell is the median of 3 runs (criterion `mid` of
`[lo, mid, hi]`); intra-run pairing is also checked. Host noise on the
25M hydrate is about ±3 s; the Rocks 25M `get_loop` band is
3.86–4.03 ms (see the registry).

## Sorted ingest — per-run values

### 1M (2026-09-02, one official run)

| cell | Pedra | Rocks | ratio |
|---|---:|---:|---:|
| hydrate | 0.6 s (1.69 M/s) | 1.1 s (0.93 M/s) | **1.82×** |
| settle | 0.4 s | 1.0 s | **2.50×** |
| get_hit | 3.016 µs | 4.344 µs | **1.44×** |
| prefix_scan | 202.2 µs | 330.7 µs | **1.64×** |
| get_loop | 312.5 µs | 424.4 µs | **1.36×** |
| multi_get | 336.4 µs | 364.5 µs | **1.08×** |
| probe_hit p50 | 3.8 µs | 12.4 µs | **3.26×** |
| probe_miss p50 | 1.1 µs | 1.2 µs | 1.09× (old engine) |
| disk after settle | 0.24 GiB | 0.21 GiB | — |

### 10M (2026-09-02, one official run)

| cell | Pedra | Rocks | ratio |
|---|---:|---:|---:|
| hydrate | 10.7 s (0.93 M/s) | 11.1 s (0.90 M/s) | **1.03×** |
| settle | 0.6 s | 4.6 s | **7.67×** |
| get_hit | 13.049 µs | 13.045 µs | 1.000× **tie** (CIs overlap) |
| prefix_scan | 261.4 µs | 341.4 µs | **1.31×** |
| get_loop | 1.143 ms | 1.225 ms | **1.07×** |
| multi_get | 1.230 ms | 1.258 ms | **1.02×** |
| probe_hit p50 | 7.1 µs | 16.8 µs | **2.37×** |
| probe_miss p50 | 1.6 µs | 631 ns | **0.39× loss** (old engine) |
| disk after settle | 2.40 GiB | 2.10 GiB | — |

### 25M

Hydrate is a 3-run campaign (2026-09-03, timing diagnostics off, one
backend per process, all exits 0):

| run | Pedra | Rocks | intra | Pedra disk | Rocks disk |
|---|---:|---:|---:|---|---|
| r1 | 29.4 s | 29.9 s | 1.017× | 5.96 GiB | 6.86 GiB |
| r2 | 30.2 s | 34.4 s | 1.139× | 5.96 GiB | 7.97 GiB |
| r3 | 29.5 s | 30.2 s | 1.024× | 5.96 GiB | 6.82 GiB |
| **median** | **29.5 s** | **30.2 s** | **1.02×** | | |

Rocks's own band on this shape spans 27.9–34.4 s across campaigns; with
±3 s of host noise the cell is **parity**, not a win.

Read cells (2026-09-03, one official run):

| cell | Pedra | Rocks | ratio |
|---|---:|---:|---:|
| settle | 0.3 s | 8.1 s | **27×** |
| get_hit | 35.23 µs | 40.29 µs | **1.144×** |
| prefix_scan | 248.5 µs | 333.4 µs | **1.342×** |
| get_loop | 3.649 ms | 4.168 ms | **refused** (Rocks out of band) |
| multi_get | 3.363 ms | 4.286 ms | **1.274×** |
| probe_hit p50 | 39.8 µs | 45.4 µs | **1.14×** |

### 100M (2026-09-05, 3 runs, current engine)

Engine with per-column-family SST key envelopes and a k-way
disjoint-level merge for settled prefix pages; bulk writer with a real
bloom (10 bits/key).

| cell | Pedra (r1/r2/r3) | Rocks (r1/r2/r3) | Pedra med | Rocks med | ratio |
|---|---|---|---:|---:|---:|
| hydrate | 119.4 / 118.5 / 119.5 s | 151.3 / 147.6 / 156.6 s | 119.4 s | 151.3 s | **1.27×** (intra 1.27 / 1.25 / 1.31) |
| settle | 0.8 / 0.7 / 0.6 s | 56.5 / 56.3 / 58.7 s | 0.7 s | 56.5 s | **81×** |
| get_hit | — | — | 63.7 µs | 68.4 µs | **1.07×** (intra 0.96 / 1.09 / 1.39) |
| prefix_scan | 285.8 / 320.8 / 304.8 µs | 323.4 / 320.0 / 309.6 µs | 304.8 µs | 320.0 µs | **1.05×** (intra 1.13 / **1.00 tie** / 1.02) |
| get_loop | 6.710 / 6.154 / 5.987 ms | 6.997 / 6.908 / 8.125 ms | 6.15 ms | 7.00 ms | **1.14×** |
| multi_get | 6.248 / 5.835 / 5.761 ms | 6.699 / 6.701 / 9.177 ms | 5.84 ms | 6.70 ms | **1.15×** |
| probe_miss p50 | 211 / 210 / 230 ns | 551 / 581 / 571 ns | 211 ns | 571 ns | **2.71×** |

probe_miss p99: Pedra 231–311 ns vs Rocks 842 ns–2.4 µs; p999: Pedra
251–461 ns vs Rocks 6.8–10.8 µs. Disk: Pedra 24.08–24.13 GiB
(259 B/entry) at hydrate, 24.16 GiB after settle, constant across runs;
Rocks 25.0–27.4 GiB at hydrate, 20.98 GiB after settle. Settle peak RSS with the 64 MiB bulk
clamp: 1.65 GiB (the unclamped writer OOM-killed at 3.4 GiB on the same
guest before the fix).

A same-day reconfirmation run (whole campaign re-executed after a
restart, all exits 0) landed inside these bands: probe_miss Pedra 230 ns
vs Rocks 531–621 ns, prefix 346 vs 350–360 µs, get_hit 60.7 vs 77–98 µs.

### 100M, prior engine (2026-09-04, 3 runs) — superseded, kept for the record

| cell | Pedra med | Rocks med | ratio |
|---|---:|---:|---:|
| hydrate | 123.7 s | 152.7 s | **1.23×** |
| settle | 0.7 s | 61.4 s | **88×** |
| get_hit | 58.0 µs | 104.4 µs | **1.80×** |
| prefix_scan | 384.8 µs | 385.6 µs | 1.00× tie (run 2 lost 0.75×) |
| get_loop | 5.57 ms | 9.38 ms | **1.68×** |
| multi_get | 5.48 ms | 9.54 ms | **1.74×** |
| probe_miss p50 | 2.3–2.4 µs | 651–692 ns | **0.29× loss** |

The larger read ratios here are the peer having a slow day on the miss
path, not an engine jump — which is why cells publish on medians with
intra-run pairing, and why this table is not the published one.

## Loss registry

Named losses and refusals, in the open, with dates. A loss moves off
this list only by a 3-run of the current engine.

| cell | loss | date | status |
|---|---|---|---|
| 100M probe_miss | **0.27×–0.29×** (2.3–2.5 µs vs 651–701 ns) | 2026-09-04 | fixed by per-CF envelopes: **2.71×** (2026-09-05) |
| 100M prefix_scan | **0.70×** (576.8 vs 404.0 µs) | 2026-09-04 | fixed by k-way disjoint merge: **1.05×** (2026-09-05), middle run tied |
| 10M probe_miss | **0.39×** (1.6 µs vs 631 ns) | 2026-09-02 | old engine (always-true bulk bloom); cell re-measuring |
| 25M get_loop | refused — Rocks 4.17–4.48 ms, out of its 3.86–4.03 ms band, every attempt (2026-09-02/03) | — | no ratio published; Pedra 3.65 ms is a number, not a claim |
| 10M get_hit | tie, CIs overlap (13.05 vs 13.05 µs) | 2026-09-02 | published as tie |
| 25M hydrate | parity 1.02× inside ±3 s noise | 2026-09-03 | published non-bold |
| G1 single-client write-per-op | below 1× by construction (one barrier per op vs peer's zero) | standing | group commit closes them under concurrency (`apply_mc4` 2.79×) |

## What moved the cells (mechanisms, not knobs)

- **Per-column-family SST key envelopes.** The workload parks a key gap
  between the `data` and `meta` families; a single collapsed `[min,max]`
  envelope swallowed it and every absent-key probe walked every table.
  One envelope per family lets the lookup reject the hole before any
  bloom or table probe. This moved 100M probe_miss from 0.27× to 2.71×.
- **K-way disjoint-level merge on settled prefix pages.** Settled bulk
  levels are disjoint; merging them level-by-level through a heap is
  what the prefix scan used to pay. One k-way pass with newest-sequence
  wins moved 100M prefix_scan from 0.70× to 1.05×.
- **Real bulk bloom (10 bits/key)** — the original bulk writer shipped
  `always_true()`; that is the 10M probe_miss loss above.
- **64 MiB bulk-stage clamp** bounds the open-run RSS (100M settle peak
  1.65 GiB vs SIGKILL at 3.4 GiB before).
- **Owned sparse-index boundary keys** fixed the 100M OOM during
  hydrate on the 3.9 GiB guest (index pinned the ingest key pool).

## Fjall column

Third peer (fjall 3.1.10), not the gate. Legs: 2026-09-04, same guest,
`scale-parity-bench` harness, one backend per process, 3 runs ×
{25M, 100M}, 200 B values, 256 MiB cache. That is a different read
harness than the Pedra/Rocks column, so these ratios are cross-harness,
orientation only — no bold, no gate. Ratio = Fjall / Pedra; the Pedra
rows repeat the official `snapshot_backends` numbers above (25M reads:
single run; 100M: 3-run medians).

| 25M | hydrate | settle | get_hit | prefix_scan | get_loop | probe_miss p50 | disk |
|---|---:|---:|---:|---:|---:|---:|---:|
| Fjall | 49.8 s | 0.1 s | 40.2 µs | 272.3 µs | 3.60 ms | 1.1 µs | 5.16 GiB |
| Pedra | 29.5 s | 0.3 s | 35.2 µs | 248.5 µs | 3.65 ms | — | 5.96 GiB |
| ratio | 1.69× | 0.33× | 1.14× | 1.10× | 0.99× | — | |

| 100M | hydrate | settle | get_hit | prefix_scan | get_loop | probe_miss p50 | disk |
|---|---:|---:|---:|---:|---:|---:|---:|
| Fjall | 191.4 s | 0.0 s | 82.0 µs | 266.8 µs | 7.82 ms | 1.1 µs | 20.61 GiB |
| Pedra | 119.4 s | 0.7 s | 63.7 µs | 304.8 µs | 6.15 ms | 211 ns | 24.16 GiB |
| ratio | 1.60× | ≈0× | 1.29× | 0.88× | 1.27× | 5.21× | |

Per-run Fjall (hydrate / get_hit / prefix_scan / get_loop):

- 25M: 44.2 s / 28.7 µs / 260.8 µs / 3.22 ms · 53.7 s / 48.9 µs /
  272.3 µs / 4.48 ms · 49.8 s / 40.2 µs / 273.8 µs / 3.60 ms.
- 100M: 198.8 s / 81.9 µs / 264.0 µs / 7.72 ms · 190.4 s / 83.6 µs /
  268.5 µs / 8.20 ms · 191.4 s / 82.0 µs / 266.8 µs / 7.82 ms.

probe_miss p50 1.1 µs in every run (p99 2.3–3.2 µs, p999 14.5–17.7 µs,
recurring cold-tail max ~8–48 ms). settle ≈ 0 on every leg: fjall
persists during hydrate. An earlier same-day note compared Fjall's
191.4 s against the bloom-era Pedra 141.1 s (1.35×); the table's
official-Pedra baseline supersedes it.

## Reproducing

```sh
# Pedra, 1M smoke (no extra toolchain)
SCALE_ENTRIES=1000000 ./scripts/reproduce-scale.sh pedradb /tmp/scale-pedra

# Official harness, one backend per process
cd crates/snapshot-bench
for b in pedradb rocksdb; do
  SLIPSTREAM_BENCH_BACKENDS=$b SLIPSTREAM_BENCH_ENTRIES=25000000 \
  SLIPSTREAM_BENCH_CACHE_BYTES=268435456 PEDRA_STAGE_MAX_BYTES=67108864 \
  TMPDIR=/data/stores \
    cargo bench --bench snapshot_backends --features fjall,rocksdb,pedradb \
    -- 'get_hit|prefix_scan|lookup_100'
done
```

`TMPDIR` must be real NVMe, not tmpfs, when disk behavior matters.
Criterion's repeated iterations measure warm-cache reads. The Rocks
feature needs a C++ toolchain and libclang; Fjall and Pedra are pure
Rust. YCSB / dependents: `rocks-parity-bench` with `compat` (Pedra),
`rocksdb` (`--features real`), or `fjall` (`--features fjall`,
YCSB-only).

## RFC-0182 — same-boot write path (existing bins only)

After a write-path change, one Darwin boot, **no** new harness. Peer is
RocksDB default (`ROCKS_PARITY_SYNC=0`). Fjall is a third peer:
absolute QPS only, never a win. Quiet-host floor for overwrite_mc4:
Rocks ≳260 kQPS; a collapsed Rocks QPS is not a Pedra win. 3/3 quiet
≥1.0× is P1.1, not this recipe. Do not WARM 100M. Do not bake 4 GiB.

```sh
# Isolated DIAG knobs (same as RFC-0180 overwrite_mc4).
export ROCKS_PARITY_SYNC=0 ROCKS_PARITY_BIG=0 ROCKS_PARITY_MC_FRESH=1
export ROCKS_PARITY_CLIENTS=4 ROCKS_YCSB_OPS=100000
OUT=findings/rfc0182-same-boot-$(date +%s)
mkdir -p "$OUT"

# Pedra + Rocks: overwrite_mc4, ycsb_a_mc4, ycsb_f_mc4, apply_mc4, 1c overwrite.
ONLY=deps_cache_overwrite_mc4,ycsb_a_mc4,ycsb_f_mc4,deps_apply_batch_mc4,deps_cache_overwrite
for eng in compat rocksdb; do
  feat=; [ "$eng" = rocksdb ] && feat="--features real"
  ROCKS_PARITY_SUITE=ycsb,deps ROCKS_PARITY_ONLY=$ONLY \
    cargo run -q --release -p rocksdb-parity-bench $feat \
      --bin rocks-parity-bench -- "$OUT/$eng" "$eng"
done
ROCKS_PARITY_PEER="$OUT/rocksdb/rocks_parity_bench.json" \
  cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-compare -- \
    "$OUT/compat/rocks_parity_bench.json" "$OUT/compare"
# compare must print sync: false (exits 2 otherwise). Named losses stay named.

# Fjall: YCSB-only. deps overwrite is a CF shape — use ycsb_a_mc4 as the
# write mix. Absolute qps; do not compute compat_over_rocksdb as a win.
ROCKS_PARITY_SUITE=ycsb ROCKS_PARITY_ONLY=ycsb_a_mc4 \
  cargo run -q --release -p rocksdb-parity-bench --features fjall \
    --bin rocks-parity-bench -- "$OUT/fjall" fjall

# snapshot-bench 1M — one backend per process.
cd crates/snapshot-bench
for b in pedradb rocksdb fjall; do
  SLIPSTREAM_BENCH_BACKENDS=$b SLIPSTREAM_BENCH_ENTRIES=1000000 \
    cargo bench --bench snapshot_backends --features fjall,rocksdb,pedradb \
      -- 'get_hit|prefix_scan|lookup_100'
done
```

Smoke (P0.2 exit-0): `ROCKS_YCSB_OPS=1000` on the parity loop;
`SLIPSTREAM_BENCH_ENTRIES=1000000` is already the 1M smoke.

## RFC-0184 — diagnose (surgical cut)

Same ns in → same `lever` out. No new harness. Linux is the scoreboard;
Darwin DIAG is not a vs-Rocks win.

```sh
# 1c overwrite (0183): WAL, not skiplist. --rocks-ns 0 skips gap shares.
cargo run -q --release -p rocksdb-parity-bench --bin pedra -- diagnose write \
  --pedra-ns 3300 --rocks-ns 2600 --clients 1 \
  --wal-ns 2460 --mem-ns 140 --publish-ns 70 --prepare-ns 30 --flush-ns 30

# apply_mc4: flush_check, mem/gap < 15% → do not despark 0055.
cargo run -q --release -p rocksdb-parity-bench --bin pedra -- diagnose write \
  --pedra-ns 203170 --rocks-ns 97163 --clients 4 --avg-group 7.13 \
  --wal-ns 10310 --mem-ns 2896 --flush-ns 148310 --lock-ns 560 --prepare-ns 640

# get vs RFC-0176 clock (1B @ 64 GiB). class=as_is_walk ⇒ P = N_files.
# P2.35: work vector × intel_server_4ghz (happy|capacity|cold). Envelope 0176
# fica; composed_* é o lower bound (bloom reject não paga pread).
cargo run -q --release -p rocksdb-parity-bench --bin pedra -- diagnose get \
  --keys 1000000000 --ram 68719476736 --measured-ns 61000
cargo run -q --release -p rocksdb-parity-bench --bin pedra -- diagnose get \
  --keys 50000000 --ram 68719476736 --cache happy
cargo run -q --release -p rocksdb-parity-bench --bin pedra -- diagnose get \
  --keys 50000000 --ram 68719476736 --cache capacity
cargo run -q --release -p rocksdb-parity-bench --bin pedra -- diagnose get \
  --keys 50000000 --ram 68719476736 --cache cold
```

With `PEDRA_WRITE_PHASE_STATS=1` the parity harness prints
`diagnose <shape> dominant=… lever=…` after phasesΔ and writes
`benches[].diagnose.lever` into the bench JSON (overwrite, apply,
ycsb 1c/mc, raftlog 1c/mc, kvrocks 1c + mc50, qs, myrocks + linkbench,
surreal, nebula, streaming, ceph, solana, arango, venice, oxigraph, rocksapi, ycsb_c_big). `rocks-parity-compare` copies it onto each
ratio row (`diagnose: {"lever":…}` or `null`).

Multi-shape **balance** (RFC-0182 / `/otimizar`): never ship an engine
cut from one cell. `BALANCE_SHAPES` =
`overwrite_mc4,ycsb_a_mc4,ycsb_b_mc4,ycsb_c_mc4,ycsb_f_mc4,apply_mc4,1c overwrite,qs_hot_get_mc4,qs_neg_lookup_mc4,qs_batch_write_mc4,rockset_hybrid_mc4,yugabyte_docdb_rmw_mc4,venice_fanout_get_mc4,kvrocks_get_mc4`.

```sh
# ycsb A: get, not WAL
cargo run -q --release -p rocksdb-parity-bench --bin pedra -- diagnose write \
  --pedra-ns 7000 --rocks-ns 2800 --clients 4 --read-pct 50 --wal-ns 400 --mem-ns 50

# Darwin same-boot board must print admits=0 for wal/flush/get_path
cargo run -q --release -p rocksdb-parity-bench --bin pedra -- diagnose balance \
  --cut wal_encode_or_write \
  --cell get_path:diag --cell get_path:diag \
  --cell wal_encode_or_write:diag --cell flush_check:diag

# cost-trace probes vs P_best (walk-all if class=as_is_walk)
# scale harness: PEDRA_COST_TRACE=1 prints
#   diagnose probes probe_miss/<backend> per_get=… p_best=… class=…
#   diagnose probes prefix_scan/<backend> per_scan=… p_best=… class=…
# scale get_hit / lookup_100 / probe_hit always print (no COST_TRACE needed)
#   diagnose get get_hit/<backend> measured_ns=… class=…
#   diagnose get lookup_100/<backend> measured_ns=… (loop/100) class=…
#   diagnose get probe_hit/<backend> measured_ns=… (p50) class=…
# scale hydrate + PEDRA_WRITE_PHASE_STATS=1:
#   diagnose hydrate/<backend> dominant=… lever=…
# scale settle + PEDRA_WRITE_PHASE_STATS=1:
#   diagnose settle/<backend> dominant=… lever=…
# CLI also prints a JSON object (0184 P2.25):
#   diagnose write → {"lever":…} ; get/probes → {"class":…} ; balance → {"admits":…}
cargo run -q --release -p rocksdb-parity-bench --bin pedra -- diagnose probes \
  --per-get 5 --p-best 5
cargo run -q --release -p rocksdb-parity-bench --bin pedra -- diagnose get \
  --keys 1000000 --ram 68719476736 --measured-ns 4400
```
