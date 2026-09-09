# oxigraph_triple_put_mc4 Darwin DIAG — RFC-0184 P2.60

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false` (`peer_policy=rocks-default`)
**Meter:** `/tmp/pedra-vs-rocks-o100`

## Number

```
number: ratio=0.898 pedra_qps=32211 rocks_qps=35875 shape=oxigraph_triple_put_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 32 211 | 35 875 |
| p50_ms | 0.1227 | 0.0981 |
| p99_ms | 0.253 | 0.184 |
| max_ms | 28.41 | 21.49 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| peer_policy | rocks-default | rocks-default |
| wall_s | 6.21 | 5.57 |

Named loss kept. Same-class async. Rocks 36 k batch-ops ×32 ≈ 1.15 M
puts/s is not collapsed (same family as qs_batch ~32 k / kafka 57 k).
Not Linux cartaz.

## Shape

Oxigraph triple insert: WriteBatch of `cfg.batch` SPO keys
`t/{s}/{p}/{o}`, zipf s/o, 4 clients, no seed.
`run_oxigraph_put_clients` + `BALANCE_SHAPES`. Test
`rfc0184_oxigraph_triple_put_mc4_in_compare`.
