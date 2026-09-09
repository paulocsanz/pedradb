# nebula_insert_edge_mc4 Darwin DIAG — RFC-0184 P2.61

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false` (`peer_policy=rocks-default`)
**Meter:** `/tmp/pedra-vs-rocks-n101`

## Number

```
number: ratio=1.091 pedra_qps=33186 rocks_qps=30412 shape=nebula_insert_edge_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 33 186 | 30 412 |
| p50_ms | 0.1206 | 0.0999 |
| p99_ms | 0.248 | 0.298 |
| max_ms | 13.91 | 146.90 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| peer_policy | rocks-default | rocks-default |
| wall_s | 6.03 | 6.58 |

Same-class async. Rocks 30 k batch-ops ×32 ≈ 0.97 M puts/s is not
collapsed. Pedra loses p50 (121 vs 100 µs); QPS is a shorter tail
(max 14 vs 147 ms). Do not quote 1.091 as Linux cartaz or as a
published win vs Rocks default.

## Shape

NebulaGraph edge insert: WriteBatch of `cfg.batch` `e/{src}/{dst}`
keys, zipf src/dst, 4 clients, no seed. `run_nebula_insert_clients` +
`BALANCE_SHAPES`. Test `rfc0184_nebula_insert_edge_mc4_in_compare`.
