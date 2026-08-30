# RFC-0154 P2.3 — CHV, apply WAL/prepare 2×64 **REFUSED**

**When:** 2026-08-30T05:13:58Z–05:22:37Z  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `sync=false`.  
**Tree:** P1.8 intern + P2.3 (range-seq, recycle `Vec<WriteOp>`, one-pass
`encoded_meta`, occupied-shard `get_mut`). WAL not skipped.  
**Guest:** `linux-gate-p149b` CHV 4 vCPU.

| shape | min | median | r1 / r2 / r3 | >3 |
|---|---:|---:|---|:---:|
| ycsb_a | 2.362 | **3.399** | 3.399 / 3.634 / 2.362 | yes |
| ycsb_b | 0.286 | 2.688 | 3.423 / 0.286 / 2.688 | |
| ycsb_c | 3.660 | **3.872** | 3.872 / 3.660 / 3.894 | yes |
| ycsb_d | 1.623 | **3.521** | 3.665 / 3.521 / 1.623 | yes |
| ycsb_e | 11.183 | **14.621** | 14.621 / 15.334 / 11.183 | yes |
| ycsb_f | 2.253 | 2.880 | 2.880 / 2.950 / 2.253 | |
| deps_cache_overwrite | 1.625 | 1.854 | 1.625 / 6.378 / 1.854 | |
| deps_lock_prewrite | 2.278 | 2.675 | 2.675 / 2.745 / 2.278 | |
| deps_mvcc_latest | 4.369 | **4.830** | 4.830 / 4.369 / 5.081 | yes |
| deps_apply_batch | 1.683 | 1.751 | 1.683 / 1.751 / 1.906 | |
| deps_raftlog | 1.302 | 1.514 | 1.302 / 1.514 / 1.625 | |
| deps_scan | 5.260 | **5.650** | 12.453 / 5.260 / 5.650 | yes |
| kvrocks_get | 3.029 | **4.668** | 4.668 / 4.998 / 3.029 | yes |
| kvrocks_set | 1.242 | 1.691 | 1.242 / 2.863 / 1.691 | |
| kvrocks_scan | 46.490 | **64.601** | 101.844 / 46.490 / 64.601 | yes |
| kvrocks_pipelined_set | 3.848 | **3.996** | 3.996 / 3.848 / 4.061 | yes |
| kvrocks_blob_set | 2.347 | 2.353 | 2.347 / 2.353 / 2.547 | |

`RESULT=P149_PASS over_med=9/17 min_ratio=0.286`

**Refuse.** Lost `ycsb_f` 3× (2.88 vs P1.8 3.06) and `deps_lock_prewrite`
(2.68 vs 3.46). Apply 1.88→1.75 did not close 3×. `ycsb_b` min 0.286 is
a real round (quote, don't hide). Code reverted. Official CHV stays
[`../2026-08-30-linux-p149-p21-chv-p18/`](../2026-08-30-linux-p149-p21-chv-p18/)
**12/17**.
