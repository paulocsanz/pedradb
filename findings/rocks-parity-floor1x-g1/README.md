# Parity battery floor 1× — G1 product column (2026-08-24)

The official product column: **Pedra fdatasyncs before Ok** (G1,
`PEDRA_PARITY_G1=1` → `opts.set_sync(true)`; F_FULLFSYNC on Darwin) vs the
official peer **RocksDB default** (`WriteOptions.sync=false`,
`ROCKS_PARITY_SYNC=0`, real side built and run by `rocks_side_ycsb.sh`).
More durability on the Pedra side by construction; the table below is what
that costs and buys per shape. Gate run at `ROCKS_PARITY_RATIO_FLOOR=1.0`.

Protocol: `scripts/rocksdb_parity_v0.sh`, `records=1024 ops=200 payload=100
dist=uniform batch=32`, single client, single run, quiet box. Smoke-scale
protocol — same protocol as the gated same-class sibling
[`rocks-parity-floor1x/`](../rocks-parity-floor1x/README.md), so the two
tables are directly comparable. The untimed `ycsb_c_big` seed (2^20 puts)
is seeded async, symmetric with the peer's own async setup; the measured
loop of that shape is pure point reads (bench change, same date).

| shape | Pedra G1 | RocksDB default | ratio | ≥1× |
|---|---:|---:|---:|---|
| ycsb_c_unif | 3609066.0 | 1817487.9 | **1.986** | ✓ |
| deps_mvcc_latest | 450111.9 | 249778.6 | **1.802** | ✓ |
| deps_scan | 467699.5 | 308920.4 | **1.514** | ✓ |
| ycsb_c | 2760219.7 | 2271643.1 | **1.215** | ✓ |
| ycsb_c_big | 257303.6 | 228104.3 | **1.128** | ✓ |
| ycsb_e | 6259.7 | 283604.3 | 0.022 | ✗ |
| deps_lock_prewrite | 229.1 | 16212.3 | 0.014 | ✗ |
| deps_apply_batch | 117.7 | 12994.1 | 0.009 | ✗ |
| ycsb_b_unif | 7781.1 | 1842604.7 | 0.004 | ✗ |
| ycsb_b | 4860.6 | 1500003.8 | 0.003 | ✗ |
| ycsb_d | 3750.6 | 1490690.6 | 0.003 | ✗ |
| deps_raftlog | 213.9 | 110964.7 | 0.002 | ✗ |
| deps_cache_overwrite | 238.5 | 141347.0 | 0.002 | ✗ |
| ycsb_a | 339.5 | 488549.6 | 0.001 | ✗ |
| ycsb_f | 504.0 | 412194.4 | 0.001 | ✗ |

**min_ratio 0.001 — gate fails on every write-containing shape.**

## Mechanism (named, not hidden)

- **Reads:** every read shape is ≥ 1.128× the default peer while Pedra
  keeps the stronger write path. No fsync in the read path; nothing to
  ceiling.
- **Writes (single client, write-per-op):** each timed op commits with one
  full barrier — F_FULLFSYNC p50 ≈ **3.8 ms** measured in-band
  (`ycsb_a` p50 3.82 ms ⇒ ~339 qps ≈ 1/p50) — while the peer's default
  writer performs **zero** syncs. A per-op-sync single client is
  fd-bound below 1× **by construction on any OS**; the repo affirms this
  ceiling in code (`rfc0041_one_fdatasync_cannot_hit_2x_rocks_default_ycsb_a`,
  test family; Linux fdatasync p50 25.7 µs moved the same shapes to only
  0.056×, RFC-0041 P0.2). This is physics, not a pathology: no fix on the
  engine path changes the number of barriers the G1 contract requires.
- **The closers:** (a) group commit — one barrier per *group*, not per op;
  under concurrency the G1 column reaches 2.788× on `apply_mc4` (Linux
  head3, RFC-0041 P1.1), which is exactly what the proved group-commit
  kernel (RFC-0057 P2.1) now guards; (b) the drop-in async column — same
  engine, WAL write without the barrier — which is the gated official
  floor today (15/15 ≥ 1.254, sibling finding).

## Where the floor lives now (2026-08-24 decision)

The official gate (`scripts/rocksdb_parity_v0.sh` default) is
`ROCKS_PARITY_RATIO_FLOOR=1.0` on the **drop-in same-class column**
(async Pedra vs default Rocks, RFC-0054) — every shape gated, 15/15 pass,
min 1.254. The G1 product column above is the **published claim table**:
reads and group-commit shapes beat default Rocks with more durability;
single-client per-op-sync writes carry the fd ceiling with the mechanism
named. Never quote the write rows as wins; never hide them either.

Files: `compat/rocks_parity_bench.json` (Pedra G1 side),
`rocks_side/rocks_shaped_peer.json` (real Rocks side),
`compare/compare_report.json` (ratios; `parity.floor=1, pass=false` — the
honest outcome this finding documents).
