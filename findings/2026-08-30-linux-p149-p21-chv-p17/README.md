# RFC-0154 P1.7 — CHV, RMW last-byte **REFUSED**

**When:** 2026-08-30T02:40:52Z–02:48:51Z  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `sync=false`.  
**Tree:** P1.6 + `rmw_bump_last` (Engine::rmw skip `Vec` of old value).  
**Guest:** `linux-gate-p149b` CHV 4 vCPU. Restart before this run was
messy (ACPI poweroff during two failed starts).

| shape | min | median | r1 / r2 / r3 | >3 |
|---|---:|---:|---|:---:|
| ycsb_a | 2.791 | 2.853 | 3.079 / 2.791 / 2.853 | |
| ycsb_b | 2.798 | **3.257** | 3.435 / 2.798 / 3.257 | yes |
| ycsb_c | 3.587 | **3.730** | 3.730 / 3.587 / 4.487 | yes |
| ycsb_d | 2.722 | **3.339** | 3.519 / 2.722 / 3.339 | yes |
| ycsb_e | 4.523 | **14.994** | 14.994 / 15.858 / 4.523 | yes |
| ycsb_f | 0.791 | 2.741 | 2.741 / 2.962 / 0.791 | |
| deps_cache_overwrite | 1.784 | 2.794 | 3.225 / 1.784 / 2.794 | |
| deps_lock_prewrite | 2.207 | 2.298 | 2.725 / 2.298 / 2.207 | |
| deps_mvcc_latest | 2.839 | **3.233** | 4.366 / 2.839 / 3.233 | yes |
| deps_apply_batch | 2.032 | 2.192 | 2.195 / 2.192 / 2.032 | |
| deps_raftlog | 1.070 | 1.075 | 1.114 / 1.070 / 1.075 | |
| deps_scan | 2.051 | **4.263** | 5.200 / 2.051 / 4.263 | yes |
| kvrocks_get | 5.400 | **5.499** | 5.400 / 5.573 / 5.499 | yes |
| kvrocks_set | 2.072 | 2.891 | 2.891 / 2.072 / 2.893 | |
| kvrocks_scan | 53.548 | **59.912** | 53.548 / 71.901 / 59.912 | yes |
| kvrocks_pipelined_set | 1.811 | **3.867** | 1.811 / 4.426 / 3.867 | yes |
| kvrocks_blob_set | 2.566 | 2.668 | 2.566 / 2.772 / 2.668 | |

`RESULT=P149_PASS over_med=9/17 min_ratio=0.791`

**Refuse.** F median 2.74 (P1.6 was 2.73) — the last-byte cut did not
close 3×. r3 F 0.791 and A dropping under 3× (2.85 vs P1.6 3.19) make
this remedir worse than the P1.6 tree. Code reverted. Official CHV
number stays
[`../2026-08-30-linux-p149-p21-chv-p16/`](../2026-08-30-linux-p149-p21-chv-p16/)
**10/17**.
