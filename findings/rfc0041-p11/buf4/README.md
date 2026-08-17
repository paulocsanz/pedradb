# RFC-0041 — compat write buffer 4 MiB (no compact-while-busy)

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3.

4 MiB is the isolated apply winner (2251 vs 64 MiB drain 1228). This
remesura is **4 MiB + idle-only L0 compact** (no compact while writers
are active).

## Result

**1/16 ≥ 2.0** (MVCC 2.082). Do **not** enable FLOOR.

| shape | idlerot2 | buf4 med | Pedra qps | notes |
|---|---:|---:|---:|---|
| apply 1c | 0.461 | **1.245** | 2 733 | p50 162–174 µs |
| apply_mc4 | 0.676 | **1.167** | 3 125 | Pedra 1.6 k → 3.1 k |
| ycsb_e | 2.926 | 1.916 | 154 005 | lost 2× |
| ycsb_c | 1.791 | 1.108 | 1 108 852 | |
| deps_scan | 0.952 | 0.504 | 63 437 | |
| ycsb_a | 0.123 | 0.184 | 40 681 | still 1/fd |

Run1 MVCC probe: **L0=23**, L1=0. Idle compact never finishes — MVCC
wall is ~5 ms. Next: one L0→L1 job per worker tick when `L0 ≥ 4`
(not drain-to-0 during apply).
