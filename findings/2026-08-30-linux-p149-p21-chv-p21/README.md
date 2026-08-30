# RFC-0154 P2.1b — CHV, live MemTable per non-default CF **REFUSED**

**When:** 2026-08-30T04:08:17Z–04:16:20Z  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `sync=false`.  
**Tree:** P1.8 intern + P2.1b (`cf_mem: Vec<MemTable>` for non-default CFs;
YCSB/kvrocks stay on single `mem`). P2.1a reverted.  
**Guest:** `linux-gate-p149b` CHV 4 vCPU.

| shape | min | median | r1 / r2 / r3 | >3 |
|---|---:|---:|---|:---:|
| ycsb_a | 3.443 | **3.599** | 3.652 / 3.443 / 3.599 | yes |
| ycsb_b | 2.458 | **3.601** | 3.601 / 4.198 / 2.458 | yes |
| ycsb_c | 2.804 | **3.789** | 3.789 / 3.918 / 2.804 | yes |
| ycsb_d | 2.026 | **3.465** | 3.599 / 3.465 / 2.026 | yes |
| ycsb_e | 13.955 | **14.180** | 14.180 / 14.562 / 13.955 | yes |
| ycsb_f | 2.389 | 2.535 | 2.566 / 2.535 / 2.389 | |
| deps_cache_overwrite | 2.742 | 2.769 | 2.742 / 6.037 / 2.769 | |
| deps_lock_prewrite | 2.197 | 2.454 | 2.197 / 2.454 / 2.557 | |
| deps_mvcc_latest | 2.581 | **4.013** | 4.013 / 2.581 / 4.372 | yes |
| deps_apply_batch | 1.877 | 2.013 | 1.877 / 2.174 / 2.013 | |
| deps_raftlog | 0.934 | 1.012 | 1.012 / 0.934 / 1.765 | |
| deps_scan | 1.818 | **4.875** | 4.875 / 1.818 / 8.089 | yes |
| kvrocks_get | 4.513 | **4.732** | 4.732 / 4.513 / 12.998 | yes |
| kvrocks_set | 2.778 | 2.825 | 2.825 / 2.778 / 3.877 | |
| kvrocks_scan | 53.062 | **57.795** | 53.062 / 57.795 / 60.190 | yes |
| kvrocks_pipelined_set | 4.068 | **4.145** | 4.145 / 4.068 / 4.288 | yes |
| kvrocks_blob_set | 2.337 | 2.387 | 2.480 / 2.387 / 2.337 | |

`RESULT=P149_PASS over_med=10/17 min_ratio=0.934`

**Refuse.** Lost `ycsb_f` 3× (2.54 vs P1.8 3.06) and `deps_lock_prewrite`
(2.45 vs 3.46). `deps_raftlog` min **0.934** is below the substitute floor
(quote, don't hide). Apply 2.01 / cache 2.77 did not close 3×. Routing
inserts into N live tables paid extra layer walks on get/scan without
isolating the 1c default-CF path (cache and YCSB-F still share `mem`).
Code reverted. Official CHV stays
[`../2026-08-30-linux-p149-p21-chv-p18/`](../2026-08-30-linux-p149-p21-chv-p18/)
**12/17**.
