# RFC-0154 P1.9 — CHV, 1c put sem `dirty_points` **REFUSED**

**When:** 2026-08-30T03:16:51Z–03:24:54Z  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `sync=false`.  
**Tree:** P1.8 intern + `publish_one_key` (no dirty_points mutex on 1c).  
**Guest:** `linux-gate-p149b` CHV 4 vCPU.

| shape | min | median | r1 / r2 / r3 | >3 |
|---|---:|---:|---|:---:|
| ycsb_a | 3.397 | **3.445** | 4.349 / 3.445 / 3.397 | yes |
| ycsb_b | 3.387 | **3.448** | 3.387 / 3.763 / 3.448 | yes |
| ycsb_c | 3.546 | **3.768** | 3.546 / 3.768 / 3.779 | yes |
| ycsb_d | 3.308 | **3.662** | 3.308 / 3.960 / 3.662 | yes |
| ycsb_e | 9.116 | **14.291** | 14.291 / 9.116 / 15.367 | yes |
| ycsb_f | 2.927 | 2.927 | 3.625 / 2.927 / 2.927 | |
| deps_cache_overwrite | 0.940 | 2.736 | 2.736 / 2.932 / 0.940 | |
| deps_lock_prewrite | 1.761 | 2.177 | 1.761 / 2.547 / 2.177 | |
| deps_mvcc_latest | 3.582 | **4.115** | 3.582 / 4.194 / 4.115 | yes |
| deps_apply_batch | 1.765 | 2.180 | 1.765 / 2.718 / 2.180 | |
| deps_raftlog | 1.122 | 1.139 | 1.122 / 1.139 / 1.207 | |
| deps_scan | 4.836 | **5.453** | 4.836 / 5.453 / 8.271 | yes |
| kvrocks_get | 4.478 | **4.719** | 4.719 / 5.569 / 4.478 | yes |
| kvrocks_set | 2.866 | 2.889 | 2.866 / 3.202 / 2.889 | |
| kvrocks_scan | 50.381 | **53.538** | 50.381 / 61.170 / 53.538 | yes |
| kvrocks_pipelined_set | 2.537 | **4.263** | 2.537 / 4.456 / 4.263 | yes |
| kvrocks_blob_set | 2.473 | 2.634 | 2.634 / 2.646 / 2.473 | |

`RESULT=P149_PASS over_med=10/17 min_ratio=0.940`

**Refuse.** Lost `ycsb_f` 3× (2.93 vs P1.8 3.06) and `deps_lock_prewrite`
(2.18 vs 3.46 — that path is `write_cf_owned`, so likely virt noise, but
the remedir is worse). Cache min 0.940. Code reverted. Official CHV stays
[`../2026-08-30-linux-p149-p21-chv-p18/`](../2026-08-30-linux-p149-p21-chv-p18/)
**12/17**.
