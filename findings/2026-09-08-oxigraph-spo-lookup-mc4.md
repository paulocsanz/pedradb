# oxigraph_spo_lookup_mc4 Darwin DIAG — RFC-0184 P2.51

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false` (`peer_policy=rocks-default`)
**Meter:** `/tmp/pedra-vs-rocks-o91`

## Number

```
number: ratio=0.985 pedra_qps=4278033 rocks_qps=4341471 shape=oxigraph_spo_lookup_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 4 278 033 | 4 341 471 |
| p50_ms | 0.0006 | 0.0008 |
| p99_ms | 0.0027 | 0.0019 |
| max_ms | 0.317 | 0.039 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| wall_s | 0.0468 | 0.0461 |

Named loss kept. Same-class async. Rocks 4.34 M SPO gets is not
collapsed. p50 Pedra better (0.6 vs 0.8 µs); 1.5% QPS hole is tail.
Not Linux cartaz.

## Shape

Oxigraph SPO point lookup (not SPARQL), 4 clients. 1c
`oxigraph_spo_lookup` was already in COMPARE; mc4 was missing.
`run_oxigraph_clients` + `BALANCE_SHAPES`. Test
`rfc0184_oxigraph_spo_lookup_mc4_in_compare`.
