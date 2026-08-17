# RFC-0041 P1.1 — MANIFEST off the write lock + catch-up skip on fat batches

2026-08-17. Goal of the slice is still `deps_apply_batch_mc4` and
`deps_raftlog_mc4` ≥ **2.0** vs Rocks default. This commit is the first
mechanism, **not** the 2×.

## What changed

1. **MANIFEST/`CURRENT` `fsync` no longer holds the Db write lock** on the
   host flush/compact path (`drain_imm_once`, `install_prepared_l0_off_lock`).
   In-memory SST install stays under the write lock; file `fsync`s run behind
   `persist_lock`. WAL rotate waits until persist succeeds. G1 unchanged.
2. **Catch-up wait skipped when the drained group already has ≥ 16 user ops**
   (apply = 64, raftlog = 16). The 50 µs window is longer than one `fdatasync`
   on this box (25.7 µs) and was serializing more CPU. Small YCSB puts still wait.

## Isolated split (`apply_profile`, 400 apply-ops, quiet run)

| slice | µs / apply-op (pre+com) |
|---|---:|
| BTree 64 unique keys | 17 |
| MemTable put | 22 |
| `encode_ops` | 0.9 |
| ChangeEntry ×64 | 0.5 |
| build `BatchOp`s | 16 |
| `Db` apply **no** sync | **110** |
| `Db` apply + 2× fd | 210 |
| ConcurrentDb MC4 catch-up 50 | 244 |
| ConcurrentDb MC4 catch-up 0 | 217 |

`1/110 µs ≈ 9 k` qps is the CPU floor of one serialized apply. 2× official
Rocks apply_mc4 (~3.7 k) is **7.3 k** — only reachable if MC service stays
near that floor. A 338 µs p50 is a ~3 k serialized ceiling.

## Probe (deps suite only, not the official 16-shape prefix)

`ROCKS_PARITY_SUITE=deps ROCKS_PARITY_CLIENTS=4 ROCKS_PARITY_SYNC=0`
→ `findings/rfc0041-p11/probe1/`.

| shape | Pedra qps | Rocks default | ratio | Pedra p50 |
|---|---:|---:|---:|---:|
| apply 1c | 3 377 | 6 983 | 0.48 | 171 µs |
| **apply MC4** | 3 695 | 9 283 | **0.40** | 338 µs |
| raftlog MC4 | 10 682 | 26 676 | 0.40 | 108 µs |

avg_group 1.21 (catch-up skipped). apply_mc4 max 332 ms — one tail still
eats the qps. Rocks on this clean deps-only run is much faster than the
official P0.2 Rocks (3.7 k) because P0.2 runs YCSB first on the same DB.

P1.1 stays `doing`. Next: cut group-commit service toward the 110 µs floor
(WAL fd off the write lock with a flush barrier; keep LSM from blocking Ok).

## Tests

`large_batch_skips_catchup_and_flush_reopens`; ConcurrentDb flush/group;
rocksdb-compat adversarial **unedited**.
