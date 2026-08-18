# RFC-0041 — HEAD remesura pós-tail/idx-fix (3fe7d38)

2026-08-18. Official 16-shape, `ROCKS_PARITY_SYNC=0`, `CLIENTS=4`,
median of 3 (`head2/run{1,2,3}`). JSONs only. All peers `sync: false`.

Include: memtable tail (O(1) apply insert), tail_idx point/range walks,
uncompressed L0, one-resize `encode_ops`, raftlog 16-op catch-up,
point cache 8192 + per-key inval, fat-apply dirty-skip.

## Result

**2/16 ≥ 2.0** (`ycsb_e` **2.551**, `deps_mvcc_latest` **2.527**).
FLOOR not enabled.

Write MC shapes moved to the 2× border (best official numbers):

| shape | med ratio | Pedra qps | Rocks qps |
|---|---:|---:|---:|
| deps_apply_batch_mc4 | **1.771** | 7 462 | 4 603 |
| deps_raftlog_mc4 | **1.962** | 29 730 | 15 156 |
| deps_apply_batch | 0.890 | 5 680 | 6 481 |
| deps_raftlog | 0.917 | 10 372 | 11 879 |
| deps_scan | 1.597 | 426 644 | 271 453 |
| ycsb_c | 1.685 | 2 362 204 | 1 367 560 |
| ycsb_e | **2.551** | 291 458 | 116 058 |
| deps_mvcc_latest | **2.527** | 680 455 | 280 556 |
| ycsb_a (1c) | 0.223 | 52 450 | 225 560 |
| deps_cache_overwrite (1c) | 0.211 | 26 339 | 124 457 |

Also in `head2` (pre-fix, on 2521579): `deps_scan` med **0.087** —
parked 4 MiB tails made each count walk ~20k tail entries (223 µs/op).
Fixed in `3fe7d38`: count/scan merge the map cursor with a `tail_idx`
range when `snapshot ≥ tail_max_seq`; deps-only rerun **437 597 qps**
(p95 3.6 µs).

1c `ycsb_a`/`f`/`overwrite` remain one WAL `fdatasync` per Ok (~20 µs
p50) — 2× Rocks (~225 k) needs ~450 k, above `1/t_fd`. Target and peer
unchanged; FLOOR stays off.
