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
4. **Late-join:** after the first WAL append, absorb writers already in the
   queue and share one `fdatasync` (no extra wait). Isolated avg_group 1.93.

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

## Official 16-shape remesura (YCSB prefix + MC4, 3 runs)

Same harness as P0.2. Median of `run{1,2,3}/compare`. **0/16 ≥ 2.0.**

| shape | P0.2 | P1.1 | Pedra qps | Rocks qps |
|---|---:|---:|---:|---:|
| ycsb_e | 0.91 | **1.56** | 190 624 | 123 121 |
| deps_mvcc_latest | 0.79 | **1.44** | 438 100 | 297 794 |
| ycsb_c | 0.78 | 0.92 | 1 313 881 | 1 390 458 |
| apply 1c | 0.39 | 0.64 | 4 885 | 7 616 |
| **apply_mc4** | **0.89** | **0.58** | 3 584 | 7 793 |
| raftlog_mc4 | 0.53 | 0.49 | 11 320 | 23 143 |
| ycsb_a | 0.056 | 0.13 | 53 323 | 400 464 |

apply_mc4 Pedra qps 3.1 k → 3.6 k; Rocks this set is 7.8 k (P0.2 Rocks was 3.7 k — noisy peer). Ratio down, absolute Pedra up a little. raftlog_mc4 Pedra 9.5 k → 11.3 k.

## Late-join (same slice)

After `group_start` appends the first drain, the leader **absorbs anyone
already queued** and shares **one** fsync — no extra 50 µs wait. Isolated
`apply_profile` MC4: `avg_group` 1.21 → **1.93**.

Official remesura `late{1,2,3}` (YCSB prefix, 3 runs) is **noisy** (both
engines slower than `run{1,2,3}`). Median apply_mc4 **1.01** vs a Rocks that
fell to 2.9 k qps; Pedra 2.8 k (below the quieter 3.6 k). raftlog_mc4 0.41.
**0/16 ≥ 2.0.** Do not read the 1.01 as a product win.

## Off-lock WAL `fdatasync` (same slice)

Group leader appends under the write lock, then `fdatasync`s via a shared
`Mutex<Wal>` **without** that lock; mem apply waits for the fd (G1). WAL
rotate is blocked while `commit_inflight > 0`. Test:
`off_lock_group_fsync_is_durable_on_reopen`.

Official remesura after this (`implementer/run{1,2,3}`): **0/16 ≥ 2.0**.
Closest: MVCC 1.71, E 1.51, apply_mc4 0.81. YCSB A 0.051 — 1/fd p50
**22.2 µs** ≈ 45 k qps; Rocks A ~200–370 k; 2× A is above one fd/Ok.

## L0 drain-to-0 (same slice, rejected as a write-path win)

After trigger, keep compacting until L0==0. Official remesura
[`l0drain/`](l0drain/README.md): **0/16 ≥ 2.0**. apply_mc4 Pedra **3.5 k →
1.0 k** (worker rewrite in apply gaps).

## 64 MiB buffer + idle-only L0 compact

[`idle64/`](idle64/README.md): **3/16 ≥ 2.0** (C 2.43, E 4.26, scan 18).
YCSB A 1c 14 k → **53 k** (max 0.2 ms). apply_mc4 still ~2 k — one 64 MiB
SST `fsync` mid-apply. Next: skip imm drain while writers are active.

## Tests

`large_batch_skips_catchup_and_flush_reopens`; ConcurrentDb flush/group;
rocksdb-compat adversarial **unedited**.
