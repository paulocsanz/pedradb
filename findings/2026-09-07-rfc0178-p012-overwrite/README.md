# RFC-0178 P0.12 — adaptive async write group vs overwrite_mc4

**When:** 2026-09-07. **Host:** Darwin. **Not** 4 GiB. **Not** G1.
**Peer:** RocksDB default `ROCKS_PARITY_SYNC=0`. Same-class drop-in.

Isolated `deps_cache_overwrite_mc4`, `ROCKS_YCSB_OPS=100000`, 4 clients,
`MC_FRESH=1`. 3 paired runs.

## Numbers

| run | Pedra qps | Rocks qps | ratio | Pedra p50/p95 | Rocks p50/p95 |
|---|---:|---:|---:|---:|---:|
| 1 | 161 790 | 274 171 | 0.590 | 20.0 / 45.8 | 11.5 / 27.6 |
| 2 | 161 261 | 227 049 | 0.710 | 20.3 / 44.4 | 11.9 / 30.1 |
| 3 | 159 856 | 263 137 | 0.608 | 20.5 / 45.4 | 11.8 / 31.6 |
| **median** | **161 261** | — | **0.608×** | | |

P0.11 isolated 1-run was **0.454×** (133 k vs 293 k), p95 107 µs.

## What moved

Merge async writers when `2 ≤ active ≤ 8` (50-thread bypass stays —
RFC-0044). Catch-up wait applies to async groups too.

Pedra qps 133 k → 161 k. p95 107 → 45 µs. Still **<1×** vs Rocks
default. Named loss. Do not quote as a win.
