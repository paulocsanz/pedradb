# RFC-0041 — 64 MiB write buffer + idle-only L0 compact

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`implementer/idle64/run{1,2,3}`). JSONs only.

## Result

**3/16 ≥ 2.0** (`ycsb_c` 2.43, `ycsb_e` 4.26, `deps_scan` 17.98).
Do **not** enable `ROCKS_PARITY_RATIO_FLOOR=2.0` (the binary gates all 16).

| shape | l0drain med | idle64 med | Pedra qps (med) | Rocks qps (med) |
|---|---:|---:|---:|---:|
| ycsb_a | 0.057 | 0.162 | 52 922 | 259 183 |
| ycsb_b | 0.282 | 0.403 | 224 947 | 716 867 |
| ycsb_c | 1.030 | **2.426** | 1 991 288 | 671 460 |
| ycsb_d | 0.258 | 0.609 | 410 998 | 683 070 |
| ycsb_e | 1.947 | **4.258** | 297 645 | 62 051 |
| ycsb_f | 0.219 | 0.144 | 40 667 | 308 356 |
| deps_apply_batch | 1.155 | 0.785 | 2 065 | 3 284 |
| deps_mvcc_latest | 1.059 | 1.900 | 354 003 | 221 049 |
| deps_scan | 0.252 | **17.981** | 354 058 | 33 100 |
| deps_raftlog | 0.270 | 0.192 | 6 795 | 35 321 |
| deps_cache_overwrite | 0.119 | 0.172 | 19 487 | 113 002 |
| ycsb_a_mc4 | 0.086 | 0.265 | 14 713 | 55 461 |
| ycsb_f_mc4 | 0.099 | 0.249 | 18 532 | 74 515 |
| deps_cache_overwrite_mc4 | 0.126 | 0.173 | 11 059 | 63 914 |
| deps_apply_batch_mc4 | 1.033 | 0.935 | 1 960 | 2 115 |
| deps_raftlog_mc4 | 0.389 | 0.334 | 4 978 | 14 804 |

YCSB A 1c is now **qps-consistent with p50**: Pedra ~53 k, p50 26.4 µs,
max 0.2–0.3 ms (tails gone). 2× Rocks A (~260–400 k) is still above
`1/t_fd`. apply p50 is the isolated floor; qps still dies on one
64 MiB SST `fsync` mid-apply (max 43–138 ms). Next: do not drain imm
while writers are active (cap 512 MiB so a nonstop writer still flushes).
