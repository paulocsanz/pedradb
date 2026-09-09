# myrocks_read_only_mc4 Darwin DIAG — RFC-0184 P2.57

**Date:** 2026-09-08
**Peer:** suite tag host-default (MyRocks sync-on-commit). Timed window is
25-key PK range scan. Not a published win vs Rocks default `sync=false`.
**Meter:** `/tmp/pedra-vs-rocks-m97`

## Number

```
number: ratio=1.870 pedra_qps=1944039 rocks_qps=1039381 shape=myrocks_read_only_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 1 944 039 | 1 039 381 |
| p50_ms | 0.0003 | 0.0036 |
| p99_ms | 0.0198 | 0.0057 |
| max_ms | 6.17 | 0.220 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync (JSON) | false | true |
| peer_policy | host-default | host-default |
| wall_s | 0.103 | 0.192 |

JSON `peer_sync=true` is the suite tag. Seed + window used
`set_write_sync(false)`. Rocks p50 3.6 µs is not an fsync. Rocks 1.04 M
25-key scans matches kvrocks_scan_mc4. Do not quote 1.870 as beating
Rocks default. Not Linux cartaz.

## Shape

MyRocks sysbench oltp_read_only: PK range scan of 25 keys, 4 clients.
1c `myrocks_read_only` was already in COMPARE; mc4 was missing.
`run_myrocks_range_clients` + `BALANCE_SHAPES`. Test
`rfc0184_myrocks_read_only_mc4_in_compare`.
