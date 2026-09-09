# rockset_hybrid_mc4 Darwin DIAG — RFC-0184 P2.43

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false`
**Meter:** `/tmp/pedra-vs-rocks-r83`
**Host load at START:** 9.71 9.83 10.72

## Number

```
number: ratio=0.543 pedra_qps=28967 rocks_qps=53312 shape=rockset_hybrid_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 28 967 | 53 312 |
| p50_ms | 0.0604 | 0.038 |
| p99_ms | 1.2405 | 0.5947 |
| p999_ms | 9.7935 | 4.3831 |
| max_ms | 91.05 | 15.00 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| wall_s | 6.90 | 3.75 |

Named loss kept. 1c `rockset_hybrid` Darwin DIAG was 0.811 (115 k / 142 k).
Concurrent ingest+get is worse. Rocks 53 k is not collapsed.
Not Linux cartaz.

## Shape

Rockset-inspired: every op one ingest WriteBatch (≤8 puts) + one point
get, 4 clients. 1c `rockset_hybrid` was already in COMPARE; mc4 was
missing. `run_rockset_clients` + `BALANCE_SHAPES`. Test
`rfc0184_rockset_hybrid_mc4_in_compare`.
