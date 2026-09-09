# flink_window_state_mc4 Darwin DIAG — RFC-0184 P2.54

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false` (`peer_policy=rocks-default`)
**Meter:** `/tmp/pedra-vs-rocks-f94`

## Number

```
number: ratio=0.522 pedra_qps=110048 rocks_qps=210776 shape=flink_window_state_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 110 048 | 210 776 |
| p50_ms | 0.0323 | 0.0180 |
| p99_ms | 0.1075 | 0.0585 |
| max_ms | 12.31 | 1.39 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| wall_s | 1.817 | 0.949 |

Named loss kept. Same-class async. Rocks 211 k put+25-key-scan ops is
not collapsed. Not Linux cartaz.

## Shape

Flink window state: one put into a window plus a 25-key prefix scan,
4 clients. 1c `flink_window_state` was already in COMPARE; mc4 was
missing. `run_flink_clients` + `BALANCE_SHAPES`. Test
`rfc0184_flink_window_state_mc4_in_compare`.
