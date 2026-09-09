# RFC-0180 P0.70 — 1c checkpoint fat WAL

**When:** 2026-09-08  
**Pub:** `a1dc8adb5e279d8a5c5a526690912db9f5a676b5`  
**Peer:** same-class async Pedra vs Rocks `sync=false`. Always-on; no Cargo feature.

## Why

`rotate_wal_if_writers_idle` waits 200 ms idle. 1c seed never sits that
long, and `wal_rotate_decision` refuses while live mem is non-empty, so
WAL grows to the whole seed (25M ≈ 3 GiB next to SST + mem on a 4 GiB
box). mc4 must stay stage/park (P0.14).

## Cut

`ConcurrentDb::checkpoint_wal_if_lone_and_fat`: if not `recently_multi(2ms)`
and WAL ≥ 2× write-buffer, `flush()` live mem then rotate. Host worker
calls it on the opportunistic tick and the non-idle poll. Tests
`rfc0180_checkpoint_wal_if_lone_and_fat`,
`rfc0180_checkpoint_wal_skips_when_recently_multi`.
`drain_imm_does_not_rotate_wal` still green.

## Numbers (this fire)

Leftover+L0 DIAG 2M×50k×4 overwrite_mc4:

| | pedra | rocks | ratio |
|---|---:|---:|---:|
| before P0.70 (load ~20) | 135 994 | 106 414 | 1.278 |
| after P0.70 (load ~10) | 165 501 | 178 175 | **0.929** |

Pedra 136 k→165 k. Rocks recovered from host stall; do not quote 1.278
as a win. `sync: false`. Not a Linux 0.557 cartaz win.

Linux overwrite_mc4 0.557× 25M still unpaid.
