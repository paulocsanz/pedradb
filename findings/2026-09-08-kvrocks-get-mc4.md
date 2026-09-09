# kvrocks_get_mc4 Darwin DIAG — RFC-0184 P2.46

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false`
**Meter:** `/tmp/pedra-vs-rocks-k86`
**Host load at START:** 6.36 8.31 9.40

## Number

```
number: ratio=1.205 pedra_qps=5321691 rocks_qps=4415507 shape=kvrocks_get_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 5 321 691 | 4 415 507 |
| p50_ms | 0.0004 | 0.0007 |
| p99_ms | 0.0022 | 0.0022 |
| max_ms | 0.235 | 0.102 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| wall_s | 0.038 | 0.045 |

DIAG, not Linux cartaz. Rocks 4.42 M GET is in the same family as
ycsb_c_mc4 Rocks 5.18 M — not collapsed. p50 0.4 vs 0.7 µs.

## Shape

Kvrocks/redis-benchmark GET, 4 clients. 1c `kvrocks_get` was already in
COMPARE; mc4 was missing (`kvrocks_set_mc50` exists). `run_kvrocks_get_clients`
+ `BALANCE_SHAPES`. Test `rfc0184_kvrocks_get_mc4_in_compare`.
