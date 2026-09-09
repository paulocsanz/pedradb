# deps_lock_prewrite_mc4 Darwin DIAG — RFC-0184 P2.67

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false` (`peer_policy=rocks-default`)
**Meter:** `/tmp/pedra-vs-rocks-l107`

## Number

```
number: ratio=1.006 pedra_qps=20113 rocks_qps=20001 shape=deps_lock_prewrite_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 20 113 | 20 001 |
| p50_ms | 0.1211 | 0.1687 |
| p99_ms | 0.2703 | 0.4923 |
| max_ms | 3597.7 | 55.5 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| peer_policy | rocks-default | rocks-default |
| wall_s | 9.94 | 10.00 |

Tied QPS. Pedra wins p50 (121 vs 169 µs) but Pedra max is 3.6 s vs
Rocks 55 ms — not a quiet win. Same-class async. Rocks 20 k batch-ops
×32 ×2 CF ≈ 1.28 M puts/s is not collapsed. Not Linux cartaz. Load at
start 9.

## Shape

TiKV prewrite-only: WriteBatch of `batch` (lock CF put + default CF
mvcc put), 4 clients, unique ts via AtomicU64. `run_lock_prewrite_clients`
+ `BALANCE_SHAPES`. Test `rfc0184_deps_lock_prewrite_mc4_in_compare`.
