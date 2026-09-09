# qs_neg_lookup_mc4 Darwin DIAG — RFC-0184 P2.41

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false`
**Meter:** `/tmp/pedra-vs-rocks-n81`

## Number

```
number: ratio=0.808 pedra_qps=4339226 rocks_qps=5368023 shape=qs_neg_lookup_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 4 339 226 | 5 368 023 |
| p50_ms | 0.0009 | 0.0006 |
| p99_ms | 0.0015 | 0.001 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |

Named loss kept. Rocks 5.37 M miss-get is quiet (not collapsed).
p50 0.9 vs 0.6 µs — miss path. Same family as probe_miss W 0.27×.

## Shape

Quicksilver negative lookup: 100% get past the keyspace, 4 clients.
1c `qs_neg_lookup` was already in COMPARE; mc4 was missing.
`run_qs_clients` + `BALANCE_SHAPES`. Test `rfc0184_qs_neg_lookup_mc4_in_compare`.
