# kafka_changelog_flush_mc4 Darwin DIAG — RFC-0184 P2.55

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false` (`peer_policy=rocks-default`)
**Meter:** `/tmp/pedra-vs-rocks-k95b`

## Number

```
number: ratio=0.897 pedra_qps=50844 rocks_qps=56660 shape=kafka_changelog_flush_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 50 844 | 56 660 |
| p50_ms | 0.0715 | 0.0580 |
| p99_ms | 0.176 | 0.132 |
| max_ms | 10.09 | 18.08 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| wall_s | 3.93 | 3.53 |

Named loss kept. Same-class async. Rocks 57 k WriteBatch ops
(×32 puts ≈ 1.8 M puts/s) is not collapsed. Not Linux cartaz.

1c `kafka_changelog_flush` still flushes memtable every changelog.
mc4 flush-per-op was an L0 storm (flush_warm 1→10+ ms, 200k SST
rebuilds). Concurrent ingest is batch-only.

## Shape

Kafka Streams changelog: WriteBatch of `cfg.batch` unique keys, 4
clients. `run_kafka_clients` + `BALANCE_SHAPES`. Test
`rfc0184_kafka_changelog_flush_mc4_in_compare`.
