# RFC-0149 P2.1 — CHV, P1.2 + P1.4 (1c put sem mutex `tail_ord`)

**When:** 2026-08-30T01:21:59Z–01:29:57Z  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `sync=false`.  
**Tree:** `write` BTree (P1.2) + `tail_ord_stale` atomic (P1.4). Empty prefix BTree.

| shape | min | median | r1 / r2 / r3 | >3 |
|---|---:|---:|---|:---:|
| ycsb_a | 2.355 | 2.697 | 2.355 / 2.697 / 2.783 | |
| ycsb_b | 2.340 | 2.535 | 2.535 / 2.340 / 2.814 | |
| ycsb_c | 2.960 | **3.043** | 3.043 / 2.960 / 3.394 | yes |
| ycsb_d | 2.631 | 2.860 | 2.860 / 2.631 / 3.057 | |
| ycsb_e | 13.631 | **13.845** | 13.845 / 13.631 / 14.961 | yes |
| ycsb_f | 2.181 | 2.349 | 2.349 / 2.181 / 2.413 | |
| deps_cache_overwrite | 1.945 | 2.332 | 1.945 / 2.332 / 3.282 | |
| deps_lock_prewrite | 1.774 | 2.433 | 1.774 / 2.433 / 2.505 | |
| deps_mvcc_latest | 3.699 | **4.096** | 4.096 / 8.228 / 3.699 | yes |
| deps_apply_batch | 2.059 | 2.092 | 2.151 / 2.059 / 2.092 | |
| deps_raftlog | 1.087 | 1.156 | 1.156 / 1.087 / 1.217 | |
| deps_scan | 4.582 | **5.011** | 5.011 / 7.216 / 4.582 | yes |
| kvrocks_get | 3.387 | **4.922** | 4.922 / 3.387 / 5.543 | yes |
| kvrocks_set | 1.284 | 2.616 | 2.694 / 1.284 / 2.616 | |
| kvrocks_scan | 47.774 | **48.962** | 48.962 / 47.774 / 53.832 | yes |
| kvrocks_pipelined_set | 2.973 | **3.675** | 3.675 / 3.709 / 2.973 | yes |
| kvrocks_blob_set | 2.463 | 2.505 | 2.505 / 2.763 / 2.463 | |

`RESULT=P149_FAIL over_med=7/17 min_ratio=1.087`

Same 7/17 as P1.2. Mutex-off did **not** buy d/a/cache 3× (1c write still WAL+BTree). Apply min **2.059** (P1.2 had 1.625 — 2× holds on this cut). mvcc **4.10**. P1.4 stays: correct (no mutex on 1c put). Majority still needs ≥2 more; those are 1c put / raftlog, not `tail_ord`.
