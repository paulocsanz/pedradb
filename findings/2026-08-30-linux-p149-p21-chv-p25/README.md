# RFC-0154 P2.5 — CHV, 1c WAL encode / skip vlog mutex **REFUSED**

**When:** 2026-08-30T06:08:41Z–06:16:46Z  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `sync=false`.  
**Tree:** P1.8 intern + P2.5 (async 1c skips vlog mutex; one-shot v1 emit
for a 1-op Full WAL fragment). WAL not skipped. P2.3/P2.4 reverted.  
**Guest:** `linux-gate-p149b` CHV 4 vCPU.

| shape | min | median | r1 / r2 / r3 | >3 |
|---|---:|---:|---|:---:|
| ycsb_a | 3.392 | **3.504** | 3.504 / 3.892 / 3.392 | yes |
| ycsb_b | 3.435 | **3.642** | 3.642 / 3.709 / 3.435 | yes |
| ycsb_c | 3.727 | **3.891** | 3.727 / 3.891 / 5.262 | yes |
| ycsb_d | 3.468 | **3.588** | 3.468 / 3.588 / 4.780 | yes |
| ycsb_e | 13.984 | **14.395** | 13.984 / 16.016 / 14.395 | yes |
| ycsb_f | 2.772 | 2.837 | 2.837 / 3.040 / 2.772 | |
| deps_cache_overwrite | 1.983 | 2.648 | 2.648 / 2.775 / 1.983 | |
| deps_lock_prewrite | 2.330 | 2.353 | 2.353 / 2.330 / 2.391 | |
| deps_mvcc_latest | 3.519 | **4.200** | 4.200 / 3.519 / 4.696 | yes |
| deps_apply_batch | 1.557 | 1.881 | 1.557 / 2.681 / 1.881 | |
| deps_raftlog | 1.048 | 1.088 | 1.487 / 1.048 / 1.088 | |
| deps_scan | 4.833 | **4.902** | 5.374 / 4.833 / 4.902 | yes |
| kvrocks_get | 4.583 | **5.095** | 5.298 / 5.095 / 4.583 | yes |
| kvrocks_set | 2.403 | 2.762 | 2.762 / 2.403 / 2.913 | |
| kvrocks_scan | 41.257 | **55.804** | 68.418 / 55.804 / 41.257 | yes |
| kvrocks_pipelined_set | 3.464 | **3.890** | 3.890 / 4.184 / 3.464 | yes |
| kvrocks_blob_set | 2.129 | 2.219 | 2.219 / 2.511 / 2.129 | |

`RESULT=P149_PASS over_med=10/17 min_ratio=1.048`

**Refuse.** Lost `ycsb_f` 3× (2.84 vs P1.8 3.06) and `deps_lock_prewrite`
(2.35 vs 3.46). SET 2.46→2.76 did **not** close 3× (quote, don't hide).
Blob 2.25→2.22. Code reverted. Official CHV stays
[`../2026-08-30-linux-p149-p21-chv-p18/`](../2026-08-30-linux-p149-p21-chv-p18/)
**12/17**.
