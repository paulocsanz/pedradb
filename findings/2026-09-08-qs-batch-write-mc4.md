# qs_batch_write_mc4 Darwin DIAG — RFC-0184 P2.42

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false`
**Meter:** `/tmp/pedra-vs-rocks-b82`
**Host load at START:** 12.40 15.24 12.61 — not quiet.

## Number

```
number: ratio=1.021 pedra_qps=32661 rocks_qps=32004 shape=qs_batch_write_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 32 661 | 32 004 |
| p50_ms | 0.108 | 0.1147 |
| p99_ms | 0.2589 | 0.2366 |
| p999_ms | 2.8875 | 3.7151 |
| max_ms | 147.14 | 17.67 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| wall_s | 6.12 | 6.25 |

Tied. Not a quiet win (load 12–15). Pedra p50 slightly better; Pedra
max 147 ms vs Rocks 17 ms is the tail. Each op is one WriteBatch of 32
puts → ~1.05 M puts/s; not a collapsed-Rocks 17 k single-put artifact.
Not Linux cartaz.

## Shape

Quicksilver root write: every op one batched put over the full
keyspace, 4 clients. 1c `qs_batch_write` was already in COMPARE; mc4
was missing. `run_qs_clients` + `BALANCE_SHAPES`. Test
`rfc0184_qs_batch_write_mc4_in_compare`.
