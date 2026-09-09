# surreal_tx_get_mc4 Darwin DIAG — RFC-0184 P2.50

**Date:** 2026-09-08
**Peer:** suite tag host-default (Surreal `sync-on-commit`). Timed window is
snapshot GET. Not a published win vs Rocks default `sync=false`.
**Meter:** `/tmp/pedra-vs-rocks-s90`

## Number

```
number: ratio=1.364 pedra_qps=364142 rocks_qps=266957 shape=surreal_tx_get_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 364 142 | 266 957 |
| p50_ms | 0.0101 | 0.0120 |
| p99_ms | 0.0436 | 0.0627 |
| max_ms | 11.25 | 1.03 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync (JSON) | false | true |
| peer_policy | host-default | host-default |
| wall_s | 0.549 | 0.749 |

JSON `peer_sync=true` is the suite tag (`write_sync_for_suite("surreal")`),
not the timed path. Seed + window used `set_write_sync(false)`. Rocks p50
12 µs is not an fsync. Still: do not quote 1.364 as beating Rocks default.
Not Linux cartaz.

Rocks 267 k OCC snapshot-get+commit is not collapsed (raw GET mc4 was
millions). OCC txn is the shape.

## Shape

SurrealDB `kv-rocksdb` / crud-bench: read-only optimistic txn (snapshot
get + commit), 4 clients. 1c `surreal_tx_get` and `surreal_tx_rmw_mc8`
were already in COMPARE; concurrent GET was missing.
`run_surreal_get_clients` + `BALANCE_SHAPES`. Test
`rfc0184_surreal_tx_get_mc4_in_compare`.
