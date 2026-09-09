# RFC-0180 P0.71 — `c/` tail idx is a point HashMap

**When:** 2026-09-08  
**Pub:** `1b602c2bf1fba0c60f29c7cab7fc63411e04efe1`  
**Peer:** same-class async Pedra vs Rocks `sync=false`. Always-on; no Cargo feature.

## Why

`deps_cache_overwrite` keys are `c/{u:06}` — unique point inserts, 0% get
in the timed window. The `c/` shard was a BTree (`short`). Empty-prefix
HashMap is forbidden (P0.64 / P1.3 ycsb_e 0.165×). `ycsb/` stays BTree
for zipf range/count.

## Cut

`point_cf` includes `c/` (with `point_reserve` 2^19). `lock`/`default`
unchanged. Tests `rfc0180_c_slash_point_hashmap`,
`rfc0180_idx_prefix_one_slash_splits_c_and_ycsb` (c/ → point, ycsb/ → short).

## Numbers (this fire)

Unique-key DIAG 100k×50k×4 overwrite_mc4:

| | pedra | rocks | ratio |
|---|---:|---:|---:|
| before | 127 055 | 142 838 | 0.889 |
| after P0.71 | **163 303** | 144 568 | **1.13** |

`sync: false`. Rocks stable. Not a Linux 0.557 cartaz win.

Linux overwrite_mc4 0.557× 25M still unpaid.
