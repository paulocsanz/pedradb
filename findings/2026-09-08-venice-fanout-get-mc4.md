# venice_fanout_get_mc4 Darwin DIAG — RFC-0184 P2.45

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false`
**Meter:** `/tmp/pedra-vs-rocks-v85`
**Host load at START:** 12.77 10.59 10.32

## Number

```
number: ratio=0.735 pedra_qps=101931 rocks_qps=138642 shape=venice_fanout_get_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 101 931 | 138 642 |
| p50_ms | 0.0318 | 0.0273 |
| p99_ms | 0.1405 | 0.0513 |
| max_ms | 3.04 | 1.18 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| wall_s | 1.96 | 1.44 |

Named loss kept. Each op is 32 point-gets (scaled-down LinkedIn fanout).
Rocks 139 k fanout-ops ≈ 4.4 M gets/s — not collapsed.
p50 32 vs 27 µs (~1 µs/get). Hole is tail (p99 141 vs 51 µs).
Not Linux cartaz.

## Shape

Venice-inspired: 32 point-gets per op, 4 clients. 1c `venice_fanout_get`
was already in COMPARE; mc4 was missing. `run_venice_clients` +
`BALANCE_SHAPES`. Test `rfc0184_venice_fanout_get_mc4_in_compare`.
