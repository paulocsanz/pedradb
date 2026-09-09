# mixgraph_like_mc4 Darwin DIAG — RFC-0184 P2.59

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false` (`peer_policy=rocks-default`)
**Meter:** `/tmp/pedra-vs-rocks-m99`

## Number

```
number: ratio=1.154 pedra_qps=108216 rocks_qps=93757 shape=mixgraph_like_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 108 216 | 93 757 |
| p50_ms | 0.0327 | 0.0217 |
| p99_ms | 0.147 | 0.135 |
| max_ms | 4.93 | 37.35 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| peer_policy | rocks-default | rocks-default |
| wall_s | 1.85 | 2.13 |

Same-class async. Rocks 94 k mixgraph-ops (put+2get+seek) is not
collapsed — p50 22 µs beats Pedra 33 µs. Pedra QPS is a shorter tail
(max 4.9 vs 37 ms). Do not quote 1.154 as Linux cartaz or as a
published win vs Rocks default.

## Shape

db_bench mixgraph-like: zipf `ycsb/` put + get + neighbor get + 8-key
seek, 4 clients, no seed. `run_mixgraph_clients` + `BALANCE_SHAPES`.
Test `rfc0184_mixgraph_like_mc4_in_compare`. Compact-filter 1c stays
gated so `ONLY=mc4` does not flush-per-op.
