# rockstore_widecol_rw_mc4 Darwin DIAG — RFC-0184 P2.65

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false` (`peer_policy=rocks-default`)
**Meter:** `/tmp/pedra-vs-rocks-r105`

## Number

```
number: ratio=0.566 pedra_qps=199894 rocks_qps=353258 shape=rockstore_widecol_rw_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 199 894 | 353 258 |
| p50_ms | 0.0152 | 0.0055 |
| p99_ms | 0.0825 | 0.0673 |
| max_ms | 7.88 | 13.93 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| peer_policy | rocks-default | rocks-default |
| wall_s | 1.00 | 0.57 |

Named loss kept. Same-class async. p50 Pedra loses 15.2 vs 5.5 µs
(QPS hole is p50, not tail — Pedra max is shorter). Rocks 353 k
mix-ops is not collapsed. Not Linux cartaz. Load at start 29.

## Shape

Pinterest Rockstore wide-column: 50% `put` of `c/{row}/{col}/{ts}` +
50% 8-key col-prefix scan, 4 clients. Seed 100k rows × 4 cols.
`run_rockstore_clients` + `BALANCE_SHAPES`. Test
`rfc0184_rockstore_widecol_rw_mc4_in_compare`. 1c seed gated so
ONLY=mc4 does not pay 500k untimed venice puts.
