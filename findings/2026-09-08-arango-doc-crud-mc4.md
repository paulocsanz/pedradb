# arango_doc_crud_mc4 Darwin DIAG — RFC-0184 P2.63

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false` (`peer_policy=rocks-default`)
**Meter:** `/tmp/pedra-vs-rocks-a103`

## Number

```
number: ratio=0.445 pedra_qps=239020 rocks_qps=536968 shape=arango_doc_crud_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 239 020 | 536 968 |
| p50_ms | 0.0051 | 0.0021 |
| p99_ms | 0.090 | 0.063 |
| max_ms | 2.65 | 0.86 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| peer_policy | rocks-default | rocks-default |
| wall_s | 0.837 | 0.373 |

Named loss kept. Same-class async. Rocks 537 k mix-ops is not
collapsed. Different shape from `arango_traversal_mc4` 0.003 (2-hop
scan). Not Linux cartaz.

## Shape

ArangoDB document CRUD: 50% get / 30% put / 20% 5-key scan on `d/{i}`,
zipf, 4 clients, 100k doc seed. `run_arango_crud_clients` +
`BALANCE_SHAPES`. Test `rfc0184_arango_doc_crud_mc4_in_compare`.
