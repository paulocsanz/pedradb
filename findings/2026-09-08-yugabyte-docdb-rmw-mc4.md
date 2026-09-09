# yugabyte_docdb_rmw_mc4 Darwin DIAG — RFC-0184 P2.44

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false`
**Meter:** `/tmp/pedra-vs-rocks-y84`
**Host load at START:** 6.16 7.55 9.42

## Number

```
number: ratio=0.952 pedra_qps=371838 rocks_qps=390487 shape=yugabyte_docdb_rmw_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 371 838 | 390 487 |
| p50_ms | 0.0023 | 0.0025 |
| p99_ms | 0.0558 | 0.0772 |
| max_ms | 1.06 | 14.64 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| wall_s | 0.54 | 0.51 |

Named loss kept. 1c `yugabyte_docdb_rmw` Darwin DIAG was 0.732 (404 k /
552 k). Concurrent overlay-get is closer. Rocks 390 k is not collapsed.
p50 tied ~2.4 µs; 5% QPS hole. Not Linux cartaz.

## Shape

Yugabyte DocDB: 70% overlay-get (intent + committed) / 30% dual-put
RMW, 4 clients. 1c was already in COMPARE; mc4 was missing.
`run_yugabyte_clients` + `BALANCE_SHAPES`. Test
`rfc0184_yugabyte_docdb_rmw_mc4_in_compare`.
