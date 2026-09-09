# kvrocks_pipelined_set_mc4 Darwin DIAG — RFC-0184 P2.64

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false` (`peer_policy=rocks-default`)
**Meter:** `/tmp/pedra-vs-rocks-k104`

## Number

```
number: ratio=0.807 pedra_qps=29874 rocks_qps=36999 shape=kvrocks_pipelined_set_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 29 874 | 36 999 |
| p50_ms | 0.1178 | 0.0786 |
| p99_ms | 0.268 | 0.253 |
| max_ms | 137.48 | 43.10 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| peer_policy | rocks-default | rocks-default |
| wall_s | 6.69 | 5.41 |

Named loss kept. Same-class async. Rocks 37 k batch-ops ×32 ≈ 1.18 M
puts/s is not collapsed (same family as qs_batch ~32 k / oxigraph
36 k). Not Linux cartaz.

## Shape

Kvrocks redis SET pipeline: WriteBatch of `cfg.batch` zipf `k/{i}`
keys, 4 clients, no seed. `run_kvrocks_pipelined_clients` +
`BALANCE_SHAPES`. Test `rfc0184_kvrocks_pipelined_set_mc4_in_compare`.
