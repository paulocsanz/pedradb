# RFC-0180 P0.75 — park remainder leftover + skip materialize while multi

**When:** 2026-09-08
**Pub:** (this fire)
**Peer:** same-class async Pedra vs Rocks `sync=false`. Always-on; no Cargo feature.
**Meter:** `/tmp/pedra-vs-rocks-p077` leftover+L0 2M×50k×4

## Why

P0.68 parked leftover only at ≥ write-buffer/2. After seed auto-flush
the remainder is often ~44 MiB and mixed with timed `c/`. P0.74 dropped
the gate **alone** — leftover+L0 Pedra 177k→175k because the flush
worker materialized parked leftover during mc4 (`db.write()` + SST
rebuild). P0.75 parks any non-empty foreign one-slash leftover **and**
skips flush/compact-worker `with_read`+materialize while `recently_multi`.
1c L0 drain stays.

## Number

```
number: ratio=0.795 pedra_qps=208925 rocks_qps=262885 shape=overwrite_mc4 leftover+L0 2M (DIAG)
```

| | Pedra | Rocks | p50 |
|---|---:|---:|---|
| Fire 74 (noisy, Rocks 195 k) | 177 124 | 194 715 | 20.6 vs 14.9 µs |
| P0.74 park-alone (reverted) | 174 633 | 182 256 | 20.6 vs 17.4 µs |
| **P0.75 quiet** | **208 925** | **262 885** | **17.4 vs 11.5 µs** |

`sync: false`. Rocks 263 k is quiet (≳244–260 k overwrite band). Named
loss kept. Previous 0.91 was vs a slow peer, not a quieter Pedra.

Pedra QPS 177k→209k, p50 20.6→17.4 µs. Hole remains p50 vs Rocks 11.5 µs
(WAL/apply). Linux 0.557× 25M unpaid.

Unique-key 100k after P0.75 (parks 12 MiB leftover): `ratio=1.259`
pedra=200072 rocks=158891 p50 18.6 vs 18.3 µs. Did **not** regress P0.71 1.13.

## Tests

`rfc0180_park_foreign_idx_decision` (12 MiB parks),
`rfc0180_park_foreign_idx_remainder_below_half_buffer`,
`rfc0180_flush_worker_skips_materialize_when_recently_multi`,
`rfc0180_skip_l0_compact_while_recently_multi`,
`host_worker_drains_l0` (1c drain stays).
