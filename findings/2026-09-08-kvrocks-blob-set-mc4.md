# kvrocks_blob_set_mc4 Darwin DIAG — RFC-0184 P2.66

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false` (`peer_policy=rocks-default`)
**Meter:** `/tmp/pedra-vs-rocks-b106`

## Number

```
number: ratio=0.766 pedra_qps=46302 rocks_qps=60460 shape=kvrocks_blob_set_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 46 302 | 60 460 |
| p50_ms | 0.0000 | 0.0411 |
| p99_ms | 0.0000 | 0.1833 |
| max_ms | 3.33 | 36.19 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| peer_policy | rocks-default | rocks-default |
| wall_s | 4.32 | 3.31 |

Named loss kept. Same-class async. Pedra p50 printed 0.0000 ms is
timer granularity; the unpaid number is wall/QPS (4.32 s vs 3.31 s).
Rocks 60 k 16 KiB SETs is not collapsed. Not Linux cartaz. Load at
start 28.

## Shape

Kvrocks BlobDB-sized SET: 16 KiB values on `b/{i}`, zipf over
`min(records,256)` recency window, 4 clients. Seed 256 keys.
`run_kvrocks_blob_clients` + `BALANCE_SHAPES`. Test
`rfc0184_kvrocks_blob_set_mc4_in_compare`.
