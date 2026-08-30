# RFC-0154 P2.1a — CHV, per-CF `TailShard.versions` **REFUSED**

**When:** 2026-08-30T03:42:58Z–03:50:58Z  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `sync=false`.  
**Tree:** P1.8 intern + P2.1a (`TailShard.versions` per CF prefix, not shared
`MemTable::tail`). P1.9 reverted.  
**Guest:** `linux-gate-p149b` CHV 4 vCPU.

| shape | min | median | r1 / r2 / r3 | >3 |
|---|---:|---:|---|:---:|
| ycsb_a | 3.203 | **3.228** | 3.203 / 3.242 / 3.228 | yes |
| ycsb_b | 3.107 | **3.511** | 3.107 / 3.511 / 3.636 | yes |
| ycsb_c | 3.500 | **3.616** | 3.500 / 3.616 / 3.660 | yes |
| ycsb_d | 3.403 | **3.435** | 3.403 / 3.509 / 3.435 | yes |
| ycsb_e | 14.587 | **15.311** | 15.395 / 15.311 / 14.587 | yes |
| ycsb_f | 2.636 | 2.874 | 2.874 / 2.876 / 2.636 | |
| deps_cache_overwrite | 2.370 | 2.542 | 3.616 / 2.542 / 2.370 | |
| deps_lock_prewrite | 1.857 | 1.885 | 2.541 / 1.857 / 1.885 | |
| deps_mvcc_latest | 4.612 | **4.765** | 4.765 / 4.819 / 4.612 | yes |
| deps_apply_batch | 1.529 | 2.122 | 2.122 / 2.127 / 1.529 | |
| deps_raftlog | 1.017 | 1.070 | 1.070 / 1.070 / 1.017 | |
| deps_scan | 4.815 | **5.291** | 5.351 / 5.291 / 4.815 | yes |
| kvrocks_get | 4.222 | **5.119** | 4.222 / 5.119 / 34.152 | yes |
| kvrocks_set | 2.408 | 2.885 | 2.408 / 2.885 / 3.087 | |
| kvrocks_scan | 44.030 | **52.248** | 44.030 / 52.248 / 58.064 | yes |
| kvrocks_pipelined_set | 3.806 | **4.043** | 3.806 / 4.043 / 4.191 | yes |
| kvrocks_blob_set | 2.434 | 2.458 | 2.458 / 2.434 / 2.619 | |

`RESULT=P149_PASS over_med=10/17 min_ratio=1.017`

**Refuse.** Lost `ycsb_f` 3× (2.87 vs P1.8 3.06) and `deps_lock_prewrite`
(1.89 vs 3.46 — three rounds, not one virt spike). Cache 2.54 did **not**
close; apply median 2.12 vs P1.8 1.88 is not a 3× close either. Isolation
of apply's 128 inserts from cache's empty-prefix vec did not pay. Code
reverted. Official CHV stays
[`../2026-08-30-linux-p149-p21-chv-p18/`](../2026-08-30-linux-p149-p21-chv-p18/)
**12/17**.
