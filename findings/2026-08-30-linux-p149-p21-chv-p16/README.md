# RFC-0149 P2.1 — CHV, P1.6 (1c async put sem Vec + sem clock no submit)

**When:** 2026-08-30T02:23:26Z–02:31:47Z  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `sync=false`.  
**Tree:** P1.5 TLS per-key + `commit_async_one` / `submit_one` (no
`Vec<BatchOp>` on lone async; no `SystemTime` on submit entry). Guest
`linux-gate-p149b` CHV 4 vCPU.

| shape | min | median | r1 / r2 / r3 | >3 |
|---|---:|---:|---|:---:|
| ycsb_a | 2.959 | **3.191** | 2.959 / 3.191 / 3.265 | yes |
| ycsb_b | 3.099 | **3.299** | 3.099 / 3.510 / 3.299 | yes |
| ycsb_c | 3.197 | **3.517** | 3.197 / 3.517 / 3.744 | yes |
| ycsb_d | 3.031 | **3.474** | 3.031 / 3.474 / 3.597 | yes |
| ycsb_e | 13.551 | **15.023** | 13.551 / 15.023 / 16.977 | yes |
| ycsb_f | 2.416 | 2.729 | 2.729 / 2.416 / 2.989 | |
| deps_cache_overwrite | 2.007 | 2.410 | 2.410 / 2.416 / 2.007 | |
| deps_lock_prewrite | 2.504 | 2.609 | 2.609 / 2.504 / 2.610 | |
| deps_mvcc_latest | 2.723 | **4.213** | 4.343 / 4.213 / 2.723 | yes |
| deps_apply_batch | 2.192 | 2.199 | 2.462 / 2.192 / 2.199 | |
| deps_raftlog | 1.118 | 1.182 | 1.200 / 1.182 / 1.118 | |
| deps_scan | 4.108 | **4.873** | 5.224 / 4.873 / 4.108 | yes |
| kvrocks_get | 4.520 | **4.558** | 4.883 / 4.558 / 4.520 | yes |
| kvrocks_set | 2.743 | 2.759 | 2.872 / 2.759 / 2.743 | |
| kvrocks_scan | 54.246 | **54.647** | 54.246 / 54.647 / 61.587 | yes |
| kvrocks_pipelined_set | 3.774 | **3.819** | 4.170 / 3.819 / 3.774 | yes |
| kvrocks_blob_set | 2.429 | 2.523 | 2.523 / 2.616 / 2.429 | |

`RESULT=P149_PASS over_med=10/17 min_ratio=1.118`

P1.5 was also 10/17, with `ycsb_a` 2.93 and `cache_overwrite` 3.08. This
cut **closed A** (3.19). `cache_overwrite` fell back under 3× (noisy
write-only; P1.4 already swung 1.95–3.28). Same 10 shapes over 3×, A
in, cache out.

Not a win vs `sync=true`. Metal is not this virt gate. raftlog 1.18 still
the p50-tied 1c log (2× recusado). Apply 2.20 holds 2×. `ycsb_f` 2.73 is
the closest remaining miss.
