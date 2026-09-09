# solana_shred_append_mc4 Darwin DIAG — RFC-0184 P2.62

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false` (`peer_policy=rocks-default`)
**Meter:** `/tmp/pedra-vs-rocks-s102`

## Number

```
number: ratio=1.296 pedra_qps=84858 rocks_qps=65485 shape=solana_shred_append_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 84 858 | 65 485 |
| p50_ms | 0.0439 | 0.0357 |
| p99_ms | 0.098 | 0.153 |
| max_ms | 5.84 | 112.38 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| peer_policy | rocks-default | rocks-default |
| wall_s | 2.36 | 3.05 |

Same-class async. Rocks 65 k batch-ops ×16 ≈ 1.05 M puts/s is not
collapsed. Pedra loses p50 (44 vs 36 µs); QPS is a shorter tail
(max 5.8 vs 112 ms). Do not quote 1.296 as Linux cartaz or as a
published win vs Rocks default.

## Shape

Solana/Agave shred append: WriteBatch of 16 unique `sh/{n:08}` keys,
4 clients, no seed. `run_solana_append_clients` + `BALANCE_SHAPES`.
Test `rfc0184_solana_shred_append_mc4_in_compare`.
