# RFC-0041 — idle WAL rotate + lz4 L0 (uncompressed reverted)

2026-08-17. `e3daddd` (revert of uncompressed L0; idle-only rotate kept).
Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`idlerot2/run{1,2,3}`). JSONs only.

Idle-only WAL rotate stays: `drain_imm_once` installs L0 in memory and
does **not** rotate (empty active mem after a stage used to `fdatasync`
the new 64 MiB SST mid-apply). L0 write is streamed **lz4 v4** —
uncompressed v3 was tried in `idlerot/` and **rejected** (apply_mc4
0.74 → 0.46).

## Result

**2/16 ≥ 2.0** (`ycsb_e` 2.926, `deps_mvcc_latest` 2.394).
Do **not** enable `ROCKS_PARITY_RATIO_FLOOR=2.0` (the binary gates all 16).

| shape | streamsst | idlerot2 med | Pedra qps (med) | Rocks qps (med) |
|---|---:|---:|---:|---:|
| ycsb_a | 0.11 | 0.123 | 30 103 | 244 305 |
| ycsb_b | — | 0.380 | 272 274 | 717 103 |
| ycsb_c | 2.46 | 1.791 | 1 836 266 | 1 026 958 |
| ycsb_d | — | 0.395 | 321 543 | 740 181 |
| ycsb_e | 2.40 | **2.926** | 207 940 | 85 440 |
| ycsb_f | — | 0.127 | 33 753 | 250 506 |
| deps_apply_batch | — | 0.461 | 1 746 | 4 313 |
| deps_mvcc_latest | 2.26 | **2.394** | 382 254 | 185 203 |
| deps_scan | — | 0.952 | 114 081 | 207 940 |
| deps_raftlog | — | 0.381 | 7 709 | 20 798 |
| deps_cache_overwrite | — | 0.177 | 22 440 | 127 051 |
| ycsb_a_mc4 | — | 0.346 | 21 905 | 63 261 |
| ycsb_f_mc4 | — | 0.162 | 19 033 | 67 866 |
| deps_cache_overwrite_mc4 | — | 0.178 | 10 732 | 60 314 |
| deps_apply_batch_mc4 | 0.74 | 0.676 | 1 649 | 2 420 |
| deps_raftlog_mc4 | — | 0.531 | 6 960 | 16 434 |

YCSB A 1c p50 is one WAL `fdatasync` (run1 33.5 µs). 2× Rocks A
(~244–400 k) is still above `1/t_fd`. apply_mc4 Pedra 1.6 k — isolated
probe2 was 5.5 k; official prefix + 64 MiB L0 write during apply still
show up as tail. C 1.79 (Pedra still ~1.8 M; Rocks this set is faster
than idle64's 671 k). scan 0.95 — Pedra 114 k vs idle64 354 k; leftover
mem + unsynced L0 into the read shapes.

FLOOR not enabled.

## Rejected next cut: WAL-archive rotate

Isolated POSIX `fdatasync` (same syscall as `pedradb-posix`) after a
4 KiB append, prefilled file:

| prefill | p50 | p95 |
|---|---:|---:|
| 4 KiB | 60 µs | 306 µs |
| 1 MiB | 55 µs | 208 µs |
| 16 MiB | 54 µs | 101 µs |
| 64 MiB | 60 µs | 169 µs |

p50 is **flat**. Splitting `CURRENT.log` would not cut 1c A/F or apply fd.

## Next cut (measured)

Isolated apply 2000×(pre+com), `defer_auto_compact`, drain every 16 iters:

| policy | qps | last100 |
|---|---:|---:|
| 4 MiB + drain | **2251** | 430 µs |
| 64 MiB stage only | 1613 | 180 µs |
| no flush | 1498 | 845 µs |
| 64 MiB + drain | 1228 | **4162 µs** |

Compat default buffer returns to **4 MiB**. Unsynced L0 persist off the
write lock when idle.
