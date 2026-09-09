# RFC-0180 P0.78 — write_buffer_for_ram 64 MiB below 8 GiB

**When:** 2026-09-08
**Pub:** (this fire)
**Peer:** same-class async Pedra vs Rocks `sync=false`. Always-on.

## Why

Linux overwrite_mc4 25M **0.557×** is the 4 GiB caixa: Pedra bench mem
256 MiB + parked leftover + WAL vs Rocks default 64 MiB. Prefix 100M
@ 4 GiB **0.70×** is bounded-cache DONTNEED-all vs Rocks kernel LRU.

`write_buffer_for_ram` caps configured memtable at 64 MiB when
`ram_ceiling < 8 GiB` (never raises 4 MiB product default). 8 GiB+
keeps 256 MiB (kvrocks_set_mc50 timed window). `page_cache_keep_bytes`
keeps ram/4 newest SST pages in bounded-cache.

## Number

Leftover+L0 2M with `PEDRA_RAM_BUDGET_BYTES=4GiB` (4 GiB-shaped DIAG):

```
pedra_qps=227279 p50=16.1µs  (P0.75 baseline 208925 / 17.4µs)
```

Rocks that run 140 k — **collapsed** (quiet leftover ≳263 k). Do not
quote ratio=1.62. Darwin 100k (96 GiB, 256 MiB stays): ratio=0.716
pedra=163935 rocks=228913 p50 21.3 vs 13.2 — Rocks 229 k just under
the 244 k quiet band; named loss, not Linux cartaz.

`sync: false`.
