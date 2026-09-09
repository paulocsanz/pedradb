# solana_trailing_read_mc4 Darwin DIAG — RFC-0184 P2.52

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false` (`peer_policy=rocks-default`)
**Meter:** `/tmp/pedra-vs-rocks-s92`

## Number

```
number: ratio=2.414 pedra_qps=2335732 rocks_qps=967606 shape=solana_trailing_read_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 2 335 732 | 967 606 |
| p50_ms | 0.0003 | 0.0038 |
| p99_ms | 0.0172 | 0.0056 |
| max_ms | 0.503 | 0.115 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| wall_s | 0.0856 | 0.2067 |

Same-class async. Rocks 968 k 25-key trailing scans is not collapsed
(nebula 1-hop was 2.01 M; this range is 25 keys). Host load ~14 — DIAG
only. Not Linux cartaz. Do not quote as a published win.

## Shape

Solana/Agave blockstore trailing slot read: `scan_count` of 25 shreds
ending at a zipf slot, 4 clients. 1c `solana_trailing_read` was already
in COMPARE; mc4 was missing. `run_solana_clients` + `BALANCE_SHAPES`.
Test `rfc0184_solana_trailing_read_mc4_in_compare`.
