# RFC-0149 P0 — coluna A maioria >3× (Mac, load ~6)

Peer Rocks frozen from [`../2026-08-29-3x-baseline/rocks/`](../2026-08-29-3x-baseline/rocks/)
(`sync=false`). Three Pedra rounds after the auto-flush O(CFs) cut.

**12/17 official shapes median >3×.** (before the cut: 5/17, `ycsb_a` 0.93×)

| shape | r1 | r2 | r3 | median | >3× |
|---|---:|---:|---:|---:|:---:|
| ycsb_a | 3.824 | 3.750 | 3.975 | **3.824** | yes |
| ycsb_b | 3.266 | 3.149 | 3.277 | **3.266** | yes |
| ycsb_c | 3.475 | 3.149 | 3.532 | **3.475** | yes |
| ycsb_d | 3.616 | 3.459 | 3.558 | **3.558** | yes |
| ycsb_e | 15.149 | 15.728 | 15.811 | **15.728** | yes |
| ycsb_f | 3.257 | 3.296 | 3.327 | **3.296** | yes |
| deps_cache_overwrite | 3.564 | 3.738 | 3.893 | **3.738** | yes |
| deps_lock_prewrite | 1.860 | 1.804 | 1.849 | 1.849 | |
| deps_mvcc_latest | 4.016 | 4.009 | 3.926 | **4.009** | yes |
| deps_apply_batch | 1.933 | 1.859 | 1.914 | 1.914 | |
| deps_raftlog | 1.271 | 1.159 | 1.159 | 1.159 | |
| deps_scan | 2.840 | 2.885 | 2.690 | 2.840 | |
| kvrocks_get | 8.380 | 8.418 | 7.062 | **8.380** | yes |
| kvrocks_set | 3.349 | 3.675 | 3.394 | **3.394** | yes |
| kvrocks_scan | 61.488 | 68.620 | 58.020 | **61.488** | yes |
| kvrocks_pipelined_set | 4.013 | 4.107 | 3.983 | **4.013** | yes |
| kvrocks_blob_set | 1.222 | 1.153 | 1.088 | 1.153 | |

Still <3×: lock / apply / raftlog / scan / blob. Linux 4 vCPU = RFC-0149 P2.1.
