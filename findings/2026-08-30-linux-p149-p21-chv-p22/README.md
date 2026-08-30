# RFC-0065 / RFC-0154 P2.1 — CHV, raftdb second DB **REFUSED**

**When:** 2026-08-30T04:33:13Z–04:41:17Z  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `sync=false`.  
**Tree:** P1.8 intern + P2.1 (`raftlog` → `{path}/raftdb` second
`ConcurrentDb`; mixed lock+raftlog batch refused). P2.1a/P2.1b reverted.  
**Guest:** `linux-gate-p149b` CHV 4 vCPU.

| shape | min | median | r1 / r2 / r3 | >3 |
|---|---:|---:|---|:---:|
| ycsb_a | 3.301 | **3.362** | 3.301 / 3.391 / 3.362 | yes |
| ycsb_b | 3.050 | **3.385** | 3.385 / 3.449 / 3.050 | yes |
| ycsb_c | 3.492 | **3.560** | 3.560 / 3.696 / 3.492 | yes |
| ycsb_d | 3.446 | **3.457** | 3.446 / 3.523 / 3.457 | yes |
| ycsb_e | 13.705 | **14.561** | 15.613 / 14.561 / 13.705 | yes |
| ycsb_f | 2.632 | 2.676 | 4.075 / 2.676 / 2.632 | |
| deps_cache_overwrite | 2.537 | 2.917 | 2.917 / 2.537 / 3.421 | |
| deps_lock_prewrite | 1.977 | 2.663 | 2.663 / 2.985 / 1.977 | |
| deps_mvcc_latest | 4.029 | **4.204** | 4.029 / 4.204 / 7.553 | yes |
| deps_apply_batch | 1.601 | 2.003 | 2.003 / 2.304 / 1.601 | |
| deps_raftlog | 1.234 | 1.244 | 1.234 / 1.244 / 1.266 | |
| deps_scan | 3.568 | **4.918** | 4.918 / 5.077 / 3.568 | yes |
| kvrocks_get | 4.442 | **4.873** | 4.873 / 4.918 / 4.442 | yes |
| kvrocks_set | 2.599 | 2.873 | 3.030 / 2.873 / 2.599 | |
| kvrocks_scan | 40.246 | **56.974** | 40.246 / 63.763 / 56.974 | yes |
| kvrocks_pipelined_set | 3.384 | **4.270** | 3.384 / 4.270 / 4.968 | yes |
| kvrocks_blob_set | 2.136 | 2.203 | 2.136 / 2.203 / 2.389 | |

`RESULT=P149_PASS over_med=10/17 min_ratio=1.234`

**Refuse.** Lost `ycsb_f` 3× (2.68 vs P1.8 3.06) and `deps_lock_prewrite`
(2.66 vs 3.46). `deps_raftlog` 1.08→1.24 is not a 3× close (p50 still
tied with Rocks' own raftlog CF). Cache 2.92 / apply 2.00 did not close.
Second DB on a 4 vCPU guest did not pay. Code reverted. Official CHV stays
[`../2026-08-30-linux-p149-p21-chv-p18/`](../2026-08-30-linux-p149-p21-chv-p18/)
**12/17**.
