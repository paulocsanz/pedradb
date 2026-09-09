# myrocks_point_select_mc4 Darwin DIAG — RFC-0184 P2.47

**Date:** 2026-09-08
**Peer:** timed window is 100% GET. JSON suite tag is MyRocks
`peer_policy=host-default` (1c writes sync-on-commit). Not a win vs
`sync=true`. Seed for mc4 is async (GET-only timed).
**Meter:** `/tmp/pedra-vs-rocks-m87`
**Host load at START:** 7.50 7.88 8.32

## Number

```
number: ratio=0.430 pedra_qps=2122782 rocks_qps=4938698 shape=myrocks_point_select_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 2 122 782 | 4 938 698 |
| p50_ms | 0.0005 | 0.0007 |
| p99_ms | 0.0282 | 0.0018 |
| max_ms | 2.65 | 0.077 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| JSON sync | false | true (suite tag) |
| wall_s | 0.094 | 0.041 |

Named loss kept. Rocks 4.94 M GET is the same family as kvrocks_get_mc4
Rocks 4.42 M — not collapsed. p50 Pedra slightly better; 57% QPS hole
is tail. Not Linux cartaz. Not an official Rocks-default write win.

## Shape

sysbench `oltp_point_select`, 4 clients. 1c `myrocks_point_select` was
already in COMPARE; mc4 was missing. `run_myrocks_clients` +
`BALANCE_SHAPES`. Test `rfc0184_myrocks_point_select_mc4_in_compare`.
