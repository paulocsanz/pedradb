# wbwi_read_your_writes_mc4 Darwin DIAG — RFC-0184 P2.58

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false` (`peer_policy=rocks-default`)
**Meter:** `/tmp/pedra-vs-rocks-w98`

## Number

```
number: ratio=0.410 pedra_qps=6539812 rocks_qps=15951189 shape=wbwi_read_your_writes_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 6 539 812 | 15 951 189 |
| p50_ms | 0.0004 | 0.0002 |
| p99_ms | 0.0012 | 0.0004 |
| max_ms | 0.0572 | 0.0164 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| peer_policy | rocks-default | rocks-default |
| wall_s | 0.0306 | 0.0125 |

Named loss kept. Same-class async. Rocks 16.0 M overlay-ops is not
collapsed. Pedra pays real `WriteBatchWithIndex::get_from_batch_and_db`;
the Rocks adapter (rust-rocksdb 0.22 has no WBWI type) is last-write-wins
overlay then `db.get`. Not Linux cartaz.

## Shape

RocksAPI `WriteBatchWithIndex` read-your-writes: unique `wbwi/{n:08}`
keys, overlay put+get, 4 clients, no seed. `run_wbwi_clients` +
`BALANCE_SHAPES`. Test `rfc0184_wbwi_read_your_writes_mc4_in_compare`.
1c `mixgraph_like` / `compaction_filter_drop` / `ingest_sst` gated so
`ONLY=mc4` does not leak compact-filter flush-per-op.
