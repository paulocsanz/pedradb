# qs_hot_get_mc4 Darwin DIAG — RFC-0184 P2.40

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false`
**Meter:** `/tmp/pedra-vs-rocks-q80`

## Number

```
number: ratio=0.719 pedra_qps=2085866 rocks_qps=2899808 shape=qs_hot_get_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 2 085 866 | 2 899 808 |
| p50_ms | 0.0007 | 0.0008 |
| p99_ms | 0.0334 | 0.024 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |

Named loss kept. Rocks 2.90 M is quiet for 99% hot-get (not collapsed).
p50 Pedra slightly better; 28% QPS hole is tail.

## Shape

Quicksilver-inspired: 99% get on hot 10% of the keyspace, 1% WriteBatch
on that set, 4 clients. 1c `qs_hot_get` was already in COMPARE; mc4 was
missing. `run_qs_clients` + `BALANCE_SHAPES`. Test
`rfc0184_qs_hot_get_mc4_in_compare`.
