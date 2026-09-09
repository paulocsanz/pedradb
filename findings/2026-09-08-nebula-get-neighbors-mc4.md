# nebula_get_neighbors_mc4 Darwin DIAG — RFC-0184 P2.48

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false` (`peer_policy=rocks-default`)
**Meter:** `/tmp/pedra-vs-rocks-g88`

## Number

```
number: ratio=0.776 pedra_qps=1556647 rocks_qps=2005694 shape=nebula_get_neighbors_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 1 556 647 | 2 005 694 |
| p50_ms | 0.0005 | 0.0018 |
| p99_ms | 0.0315 | 0.0038 |
| max_ms | 1.25 | 0.140 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| wall_s | 0.129 | 0.100 |

Named loss kept. Same-class async. Rocks 2.01 M 1-hop scans is not
collapsed. p50 Pedra better (0.5 vs 1.8 µs); 22% QPS hole is tail.
Not Linux cartaz.

## Shape

NebulaGraph GO 1-hop: prefix scan of outgoing edges, 4 clients. 1c
`nebula_get_neighbors` was already in COMPARE; mc4 was missing.
`run_nebula_clients` + `BALANCE_SHAPES`. Test
`rfc0184_nebula_get_neighbors_mc4_in_compare`.
