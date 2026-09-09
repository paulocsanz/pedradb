# kvrocks_scan_mc4 Darwin DIAG — RFC-0184 P2.53

**Date:** 2026-09-08
**Peer:** RocksDB default `WriteOptions.sync=false` (`peer_policy=rocks-default`)
**Meter:** `/tmp/pedra-vs-rocks-k93`

## Number

```
number: ratio=1.687 pedra_qps=1747372 rocks_qps=1036044 shape=kvrocks_scan_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 1 747 372 | 1 036 044 |
| p50_ms | 0.0004 | 0.0036 |
| p99_ms | 0.0239 | 0.0057 |
| max_ms | 0.482 | 0.096 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync | false | false |
| wall_s | 0.114 | 0.193 |

Same-class async. Rocks 1.04 M 25-key SCAN is not collapsed (solana
trailing 25-key was 968 k). Host load ~18 — DIAG only. Not Linux cartaz.
Do not quote as a published win.

## Shape

Kvrocks Redis SCAN COUNT=25 over a window, 4 clients. 1c `kvrocks_scan`
and `kvrocks_get_mc4` were already in COMPARE; concurrent SCAN was
missing. `run_kvrocks_scan_clients` + `BALANCE_SHAPES`. Test
`rfc0184_kvrocks_scan_mc4_in_compare`.
