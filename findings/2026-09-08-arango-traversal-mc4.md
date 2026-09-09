# arango_traversal_mc4 Darwin DIAG — RFC-0184 P2.49

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false` (`peer_policy=rocks-default`)
**Meter:** `/tmp/pedra-vs-rocks-a89c` (repeat of `/tmp/pedra-vs-rocks-a89`)

## Number

```
number: ratio=0.003 pedra_qps=3437 rocks_qps=1048571 shape=arango_traversal_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 3 437 | 1 048 571 |
| p50_ms | 0.0023 | 0.0035 |
| p99_ms | 4.859 | 0.0065 |
| max_ms | 22.7 | 0.301 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| wall_s | 58.20 | 0.191 |

Named loss kept. Same-class async. Rocks 1.05 M 2-hop scans is not
collapsed. p50 Pedra better (2.3 vs 3.5 µs); QPS hole is tail (p99
4.86 vs 0.007 ms). Repeat under load ~6 gave the same 0.003 as the
load-9 first meter — not a one-off. Not Linux cartaz.

Nebula 1-hop mc4 on the same keys was DIAG 0.776 (1.56 M / 2.01 M,
wall 0.13 s). Two hops of the same prefix scan should not be 450×
slower; the unpaid lever is the concurrent 2-hop tail, not the hop
count.

## Shape

ArangoDB 2-hop: prefix scan of outgoing edges of `u` then `u+1`, 4
clients. 1c `arango_traversal` was already in COMPARE; mc4 was missing.
`run_arango_clients` + `BALANCE_SHAPES`. Test
`rfc0184_arango_traversal_mc4_in_compare`.
