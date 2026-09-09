# bluestore_omap_read_mc4 Darwin DIAG — RFC-0184 P2.56

**Date:** 2026-09-08
**Peer:** suite tag host-default (Ceph omap sync-on-commit). Timed window is
GET + 8-key scan. Not a published win vs Rocks default `sync=false`.
**Meter:** `/tmp/pedra-vs-rocks-c96`

## Number

```
number: ratio=0.828 pedra_qps=1118438 rocks_qps=1350550 shape=bluestore_omap_read_mc4 (DIAG)
```

| | Pedra | Rocks |
|---|---:|---:|
| QPS | 1 118 438 | 1 350 550 |
| p50_ms | 0.0009 | 0.0027 |
| p99_ms | 0.0384 | 0.0049 |
| max_ms | 2.74 | 0.194 |
| n | 200 000 | 200 000 |
| clients | 4 | 4 |
| errors | 0 | 0 |
| sync (JSON) | false | true |
| peer_policy | host-default | host-default |
| wall_s | 0.179 | 0.148 |

JSON `peer_sync=true` is the suite tag, not the timed path. Seed +
window used `set_write_sync(false)`. Rocks p50 2.7 µs is not an fsync.
Still: do not quote 0.828 as beating Rocks default. Not Linux cartaz.

Rocks 1.35 M get+8-scan is not collapsed.

## Shape

Ceph BlueStore omap read: point get plus 8-key scan, 4 clients. 1c
`bluestore_omap_read` was already in COMPARE; mc4 was missing.
`run_ceph_read_clients` + `BALANCE_SHAPES`. Test
`rfc0184_bluestore_omap_read_mc4_in_compare`.
