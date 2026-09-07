# RFC-0178 P2.1 / P0.5 — 100M Darwin DIAG

**When:** 2026-09-07. **Host:** Darwin ~96 GiB. **Not** 4 GiB box. **Not** vs Rocks.
**Harness:** pub `scale-parity-bench` release, `SCALE_ENTRIES=100000000`,
`SCALE_CACHE_BYTES=256 MiB`, `PEDRA_COST_TRACE=1`, Pedra only.

Logs: `r1.log` (`e0a517c`, cache 256), `r2.log` (`c081d05`, cache 1024 +
prune), `r3.log` (`3780bb3`, P0.5 re-warm on explicit compact).

Compare: P0.2 100M (`findings/2026-09-06-rfc0178-p02-100m/`) and
50M WARM (`findings/2026-09-06-rfc0168-p22-50m/`).

## Numbers (1-run)

| célula | 50M (WARM) | r1 cache 256 | r2 cache 1024 | **r3 re-warm** |
|---|---:|---:|---:|---:|
| hydrate | 18.6 s | 47.0 s | 35.8 s | **40.6 s** |
| disco | 10.20 GiB | 22.82 GiB / 245 B/e | 22.82 / 245 | **22.82 / 245** |
| settle wall | 20.8 s (WARM) | 87.7 s | 85.0 s | **87.4 s** |
| settle_parts warm | — | 0 | 0 | **1.737 s / 24.51 GiB** |
| flush_warm | — | (invisível) | 1.77 s / 24.51 GiB | **1.77 s / 24.51 GiB** |
| prefix (1000 keys) | 40.8 µs | 63.0 µs | 63.4 µs | **66.0 µs** |
| get_loop /100 | 432 µs | 5462 µs | 2650 µs | **2763 µs** |
| get_hit | 4.3 µs | 52.6 µs | 26.1 µs | **30.7 µs** |
| pread/file | — | 41.0 µs | 18.1 µs | **22.4 µs** |

`cost/get_hit` r3: probes/op=**1.00**, n_sst=**347**, file/op=1.05,
resident=97 / file=10526. Point path is O(1) SST.

## What closed

- P2.1: `mode=hot` prints. Cap 77 GiB > 24.5 GiB, `warm_skipped=0`.
- Handle cache 1024 (r1→r2): get_loop 5.46 → 2.65 ms. Real tax (347 > 256).
- P0.5 (r3): settle `warm_bytes=24.51 GiB` in 1.737 s immediately before
  probes. Path-skip after flush is no longer why get is cold.

## What did not close

**get_loop cliff remains** (432 µs @50M → 2.76 ms @100M r3). Immediate
24 GiB WARM **refutes** “forgot to WARM before get.” Random 4 KiB is
still ~22 µs file pread, not 4.3 µs.

Prefix 66 µs, not the 125 µs skip-WARM cell. Not vs Rocks 0.70×. Not 4 GiB.

**settle 87 s** with compact=0.001 + warm=1.737: 85 s is `compact_gate`
wait. Worker `while compat_compact_once` re-takes the mutex between jobs
faster than `lock()` wakes (`try_lock` did not break the tight loop).

## Mechanism (r3)

- Point path is O(1) SST (`probes/op=1.00`). P0.3 holds at 100M.
- Handle cache 1024 halved r1. Real.
- Immediate 24 GiB WARM at 14 GiB/s (RAM reread) does **not** make
  random get 4 µs. 50M’s 20.8 s WARM of 10 GiB is a different regime
  (working set fits Darwin file cache; 24 GiB sequential fill does not
  leave 347-file random 4 KiB hot).
- Darwin `posix_fadvise` is a no-op.

Not a 4 GiB claim. Not vs Rocks.

## Named

- 245 B/e is the campaign number (P0.2 219 was v7-compressible). Fjall 222
  stays named (0168 P2.1).
- 4 GiB box 100M stays bounded-cache. This DIAG does not delete that cliff.
