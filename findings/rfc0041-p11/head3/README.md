# RFC-0041 — head3: apply_mc4 crosses 2× (c9890b1)

2026-08-18. Official 16-shape, `ROCKS_PARITY_SYNC=0`, `CLIENTS=4`,
median of 3 (`head3/run{1,2,3}`). JSONs only. Peers `sync: false`.

vs head2: materialize now streams the parked table from an `Arc`
(no 4 MiB deep clone under the Db write lock).

## Result

**3/16 ≥ 2.0**: `deps_apply_batch_mc4` **2.788** (Pedra 8 256 — best
official), `deps_mvcc_latest` **2.342**, `ycsb_e` **2.121**.

| shape | med ratio | Pedra qps | Rocks qps |
|---|---:|---:|---:|
| deps_apply_batch_mc4 | **2.788** | 8 256 | 3 812 |
| deps_mvcc_latest | **2.342** | 656 437 | 274 823 |
| ycsb_e | **2.121** | 250 138 | 117 883 |
| deps_scan | 1.790 | 469 204 | 261 789 |
| ycsb_c | 1.796 | 2 542 776 | 1 410 769 |
| deps_raftlog_mc4 | 1.792 | 24 558 | 14 427 |
| deps_apply_batch | 1.297 | 5 893 | 4 664 |
| ycsb_a (1c) | 0.233 | 49 736 | 201 917 |

apply_mc4 run spread 1.30/2.82/2.79 — the Rocks peer qps varies
(3.8 k–4.7 k); Pedra held ~8 k. FLOOR not enabled.

1c write shapes stay `1/fdatasync`-bound (a/f/overwrite 0.23–0.30).
