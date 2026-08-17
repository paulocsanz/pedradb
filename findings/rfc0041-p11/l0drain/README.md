# RFC-0041 — L0 drain-to-0 remesura (after `bd54f16`)

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`implementer/l0drain/run{1,2,3}`). JSONs only in this dir (no SST blobs).

## Result

**0/16 ≥ 2.0.** Do not enable `ROCKS_PARITY_RATIO_FLOOR=2.0`.

Drain-to-0 after `L0_COMPACTION_TRIGGER` **hurt apply**: Pedra
`deps_apply_batch_mc4` fell from ~3.5 k (P1.1 off-lock remesura) to
median **1 015** qps. Scan/MVCC stayed a race (run2 scan 2.5 k / max
379 ms; run3 scan 133 k / max 0.8 ms). After apply, probes still showed
`l0_files=7` — the single host worker cannot empty L0 during the write
burst, and trying to drain in the 5 ms poll gaps contends the write lock.

| shape | median ratio | Pedra qps (med) | Rocks qps (med) |
|---|---:|---:|---:|
| ycsb_a | 0.057 | 14 268 | 248 627 |
| ycsb_b | 0.282 | 183 279 | 756 931 |
| ycsb_c | 1.030 | 975 431 | 566 786 |
| ycsb_d | 0.258 | 213 276 | 309 270 |
| ycsb_e | 1.947 | 113 469 | 58 270 |
| ycsb_f | 0.219 | 22 659 | 150 426 |
| deps_apply_batch | 1.155 | 1 047 | 1 516 |
| deps_mvcc_latest | 1.059 | 138 158 | 163 313 |
| deps_scan | 0.252 | 37 490 | 217 437 |
| deps_raftlog | 0.270 | 6 810 | 25 296 |
| deps_cache_overwrite | 0.119 | 9 953 | 162 322 |
| ycsb_a_mc4 | 0.086 | 10 582 | 87 991 |
| ycsb_f_mc4 | 0.099 | 11 609 | 115 791 |
| deps_cache_overwrite_mc4 | 0.126 | 5 827 | 103 220 |
| deps_apply_batch_mc4 | 1.033 | 1 015 | 1 079 |
| deps_raftlog_mc4 | 0.389 | 4 117 | 8 284 |

YCSB A 1c p50 stays **25.5 µs** (one `fdatasync`) while qps is 9–20 k
because p99/max is 0.5–55 ms (flush/compact). 2× Rocks A (~250–350 k)
is still above `1/t_fd`. apply 1c p50 **192–222 µs** already matches
the isolated pre+com floor; qps dies on max **150–242 ms**.

## Next cut (this change)

1. Compat default `write_buffer_size = 64 MiB` (Rocks default), not 4 MiB.
2. Host L0 compact only when `writes_idle_for(5 ms)` — still drain imm
   and still drain-to-0 **after** the burst.
3. Lone-writer `fdatasync` off the write lock (same as the group leader).
