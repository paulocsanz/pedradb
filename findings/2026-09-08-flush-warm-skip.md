# RFC-0180 P0.77 — skip flush_warm mid-burst

**When:** 2026-09-08
**Pub:** (this fire)
**Peer:** same-class async Pedra vs Rocks `sync=false`. Always-on; no Cargo feature.

## Why

P0.70 `checkpoint_wal_if_lone_and_fat` calls `ConcurrentDb::flush` with
inflight=0 between 1c seed puts. Each 256 MiB L0 is under the 3 GiB warm
cap, so `take_warm_plan` streamed it. On a 4 GiB box, WAL + mem + warmed
SSTs is the overwrite_mc4 25M Linux 0.557× cache hole.

Predicate `flush_warm_allowed(inflight, recently_multi, recently_ok)` is
snapshotted at flush **entry** (SST I/O expires the 200 µs hold). Idle
settle still warms. `group_admit` skips the two `level_file_count(0)`
walks when stall knobs are off.

## Number

Quiet 100k zipf overwrite (Rocks 267 k ≳244 k):

```
number: ratio=0.500 pedra_qps=133455 rocks_qps=266957 shape=deps_cache_overwrite_mc4 (DIAG)
```

Named loss. p50 20.9 vs 12.9 µs (baseline ow108 19.7 vs 12.5, ratio 0.742
Pedra 191 k / Rocks 258 k). 100k does not flush in the timed window —
this cut is the 4 GiB seed/checkpoint hole, not a 100k p50 win. Do not
quote as Linux cartaz. Do not quote collapsed 1.15× / 5.94× / 1.375×
from disk-full / load-20 runs.

Leftover+L0 2M Pedra 190 k p50 19.7 µs (baseline P0.75 209 k / 17.4 µs);
Rocks 31 k collapsed that run — not a ratio.

`sync: false`.
