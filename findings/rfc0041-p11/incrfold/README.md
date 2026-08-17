# RFC-0041 — incremental fold every worker tick (rejected)

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`incrfold/run{1,2,3}`). JSONs only. Peers `sync: false`.

Hypothesis: foldidle's 5 ms idle fold absorbed ~16 parked BTrees in the
MVCC/scan window (MVCC 3.08 → 1.31). Fold **every 5 ms tick** off the
write lock, even during a write burst.

## Result

**1/16 ≥ 2.0** (`deps_mvcc_latest` **2.044**). **Rejected** as a write
path: apply_mc4 **1.25 → 0.667** (Pedra 1.7 k). Fold during apply
contends for the write lock on install.

| shape | foldidle | incrfold med | Pedra qps |
|---|---:|---:|---:|
| deps_mvcc_latest | 1.31 | **2.044** | 319 687 |
| deps_scan | 0.68 | 0.939 | 185 573 |
| ycsb_c | 1.47 | 1.725 | 1 109 852 |
| ycsb_e | **2.29** | 1.636 | 104 057 |
| apply_mc4 | **1.25** | 0.667 | 1 709 |
| apply 1c | 0.45 | 0.271 | 1 307 |
| ycsb_a | 0.09 | 0.087 | 26 328 |

G1 unchanged. FLOOR not enabled. Next: park imm **without** SST write
during the burst (apply must not pay lz4); scan uses L0 SSTs, not the
retired BTree chain.
