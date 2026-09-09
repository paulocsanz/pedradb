# RFC-0178 P0.10 — `pedra scale` 50M + 100M Darwin

**When:** 2026-09-07. **Host:** Darwin ~96 GiB. **Not** 4 GiB box. **Not** vs Rocks.
**CLI:** pub `pedra scale --entries N` (`211e844`), cache default 256 MiB,
`PEDRA_COST_TRACE=1`. One process per n.

Logs: `50m.log`, `100m.log` (both rc=0).

## Numbers (1-run, same-boot, same CLI)

| célula | **50M** | **100M** |
|---|---:|---:|
| hydrate | 17.9 s | 54.3 s |
| settle | **0.871 s** | **1.740 s** |
| settle_stats | 0.004 s | 0.008 s |
| get_hit | **3.8 µs** | **3.9 µs** |
| get_loop/100 | **403 µs** | **405 µs** |
| prefix | **65.5 µs** | **65.1 µs** |
| n_sst | 174 | 347 |
| mode | hot | hot |
| B/e | 245 | 245 |

`probes/op=1.00` both. Point path O(1) SST.

## What this is

P0.9 closed the 82 s `stats()` walk. P0.10 is the same harness through
`pedra scale` (not `scale-parity-bench`). 50M→100M get_loop and prefix
are **flat** on this host. Old 50M prefix 40.8 µs / get_loop 432 µs was
a different settle (20.8 s WARM+walk); same-CLI 50M is 65.5 / 403.

Not vs Rocks. Not 4 GiB. 4 GiB 100M stays bounded-cache.

## Named

- 245 B/e campaign (Fjall 222 named, 0168 P2.1).
- Prefix 0.70× on the 4 GiB box is P1.2, not this DIAG.
