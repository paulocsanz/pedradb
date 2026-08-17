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
3. **`changelog_interval=0` no longer appends `ChangeEntry` on every write.**
   Watchers rebuild from WAL (full history) or last-per-key after rotate.
   Flush/close still persist. This was growing a million-entry `Vec` on apply.

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

`ROCKS_PARITY_SUITE=deps ROCKS_PARITY_CLIENTS=4 ROCKS_PARITY_SYNC=0`.

| | apply MC4 Pedra | vs Rocks | raftlog MC4 Pedra | vs Rocks |
|---|---:|---:|---:|---:|
| P0.2 official (YCSB prefix) | 3 120 | 0.89 | 9 499 | 0.53 |
| probe1 (MANIFEST + skip catch-up) | 3 695 | 0.40* | 10 682 | 0.40* |
| **probe2 (+ skip ChangeEntry)** | **5 486** | **0.88** | **22 198** | **1.17** |

\*probe1 Rocks apply_mc4 was an outlier 9.3 k; probe2 Rocks 6.2 k / 19 k.

Skipping the in-memory feed vec (interval=0) is the qps move: apply_mc4
+48 % vs probe1, raftlog_mc4 **>2×** our P0.2 self. Still **< 2.0** vs
this Rocks. apply_mc4 max still ~400 ms.

P1.1 stays `doing`. Official 16-shape remesura is next; then cut the
remaining serialized CPU toward the 110 µs floor.

## Tests

`large_batch_skips_catchup_and_flush_reopens`; ConcurrentDb flush/group;
rocksdb-compat adversarial **unedited**.
