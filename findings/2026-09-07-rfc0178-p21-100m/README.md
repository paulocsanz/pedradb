# RFC-0178 P2.1 / P0.5–P0.9 — 100M Darwin DIAG

**When:** 2026-09-07. **Host:** Darwin ~96 GiB. **Not** 4 GiB box. **Not** vs Rocks.
**Harness:** pub `scale-parity-bench` release, `SCALE_ENTRIES=100000000`,
`SCALE_CACHE_BYTES=256 MiB`, `PEDRA_COST_TRACE=1`, Pedra only.

Logs: r1–r8 as before; `r9.log` (`8f952c0`, P0.9 skip SST walk in `stats()`).

Compare: P0.2 100M (`findings/2026-09-06-rfc0178-p02-100m/`) and
50M WARM (`findings/2026-09-06-rfc0168-p22-50m/`).

## Numbers (1-run)

| célula | 50M | r8 | **r9** |
|---|---:|---:|---:|
| settle wall | 20.8 s | 82.4 s | **1.743 s** |
| compact_leveled | — | 1.745 s | **1.734 s** |
| settle_stats | — | ~80 s (7× `stats`) | **0.008 s** |
| get_loop/100 | 432 µs | 2342 µs | **427 µs** |
| get_hit | 4.3 µs | 22.8 µs | **4.4 µs** |
| pread_us/file | — | 16.0 | **1.4** |
| prefix | 40.8 µs | ~64 µs | **65.5 µs** |

`cost/get_hit` r9: probes/op=**1.00**, n_sst=**347**, file/op=1.05,
resident=97 / file=10526. Point path is O(1) SST.

## What closed (r9 / P0.9)

- **settle 82 s → 1.74 s.** r8 proved compact is 1.745 s. Leftover was
  `property_int_value` → `ConcurrentDb::stats` → `vlog_size_stats` →
  `entries_cloned` of every live SST (100M inline values, 7 times).
  No vlog on this path. Skip the walk: `settle_stats_wall=0.008s`.
- **get_loop cliff closed on this host.** 427 µs vs 50M 432 µs. get_hit
  4.4 µs vs 50M 4.3 µs. r8 pread 16.0 µs/file → r9 1.4 µs/file: the
  stats materialize was evicting Darwin page cache before probes.
- P2.1: `mode=hot`. Cap 77 GiB > 24.5 GiB, `warm_skipped=0`.
- Handle cache 1024 (r1→r2): real tax (347 > 256).

## What did not close

Prefix **65.5 µs** vs 50M 40.8 µs (still ~1.6×; not the 125 µs skip-WARM
cell). Not vs Rocks 0.70×. Not 4 GiB.

4 GiB box 100M stays bounded-cache. This DIAG does not delete that cliff.

## Mechanism (r9)

- Point path is O(1) SST (`probes/op=1.00`). P0.3 holds at 100M.
- `stats()` must not decode SST bodies for observability on inline DBs.
- Sequential 24 GiB WARM at 14 GiB/s **does** leave random 4 KiB hot
  once settle stops allocating a 100M-entry clone storm.

Not a 4 GiB claim. Not vs Rocks.

## Named

- 245 B/e is the campaign number (P0.2 219 was v7-compressible). Fjall 222
  stays named (0168 P2.1).
- 4 GiB box 100M stays bounded-cache.
