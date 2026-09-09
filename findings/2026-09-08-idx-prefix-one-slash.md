# RFC-0180 P0.66 — one-slash tail idx (`c/` vs `ycsb/`)

**When:** 2026-09-08  
**Peer:** Pedra async vs Rocks `sync=false`. Darwin DIAG. Not Linux cartaz.

## Why

Linux overwrite_mc4 **0.557×** at 25M is unique-key: seed is `ycsb/{i}`, timed
puts are `c/{u}`. Both lacked NUL, so they shared the empty-prefix BTree.
Darwin 10k×400k overfits P0.62 (40 versions/key). HashMap on that empty
prefix (P0.64) is forbidden.

## Cut

`idx_prefix`: NUL CFs unchanged; raw keys with **exactly one** `/` shard on
that component (`c/`, `ycsb/`, `u/`). Multi-slash stays empty so F220
`["d/m/", "d/m0")` cannot pin a single slash shard. `cf_prefix` (family /
bytes / flush) unchanged. Not a HashMap.

## Numbers (DIAG, host load ~18–25)

Unique-key overwrite (`RECORDS=100000` `OPS=50000` `CLIENTS=4`, 200k ops):

| | qps | p50 | p999 |
|---|---:|---:|---:|
| Pedra | 182 106 | 20.4 µs | 105 µs |
| Rocks | 132 770 | 19.7 µs | 3.40 ms |
| **ratio** | **1.372×** | | |

`sync: false` both. Rocks 132 k is **not** quiet-10k ≳244 k (different
keyspace). Do not quote as a quiet 10k win. Pedra p999 0.105 vs Rocks 3.4 ms.

ycsb_e anti-overfit (same Rocks JSON, 10k records × 100k ops, 1c):

| | qps | p50 | p99 | vs Rocks |
|---|---:|---:|---:|---:|
| P0.66 | 47 600 | 0.6 µs | 550 µs | 0.170 |
| HEAD `54c5f59` | 45 901 | 0.6 µs | 559 µs | 0.164 |

No smash. Darwin 1c ycsb_e DIAG is already ~0.16 on HEAD (p99 stall); Linux
floor1x ycsb_e stays **S**. P0.64 HashMap 0.165 was this Darwin cell, not a
new scan tax from slash shards.

## Tests

`rfc0180_idx_prefix_one_slash_splits_c_and_ycsb`,
`rfc0180_idx_prefix_f220_bounds_disagree`, F220 `range_window_regression`.

Linux 0.557× 25M still unpaid (caixa / SST-after-seed).
