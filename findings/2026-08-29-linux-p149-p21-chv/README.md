# RFC-0149 P2.1 — CHV 4 vCPU, current tree, split suites

**When:** 2026-08-29T23:33:47Z–23:41:45Z  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `WriteOptions.sync=false`
(`ROCKS_PARITY_SYNC=0`). Not a win vs `sync=true`. Not G1.  
**Pass:** ≥9/17 median > 3.0.  
**Guest:** `linux-gate-p149b` / `cnt_0293db0846dc46e1b1d7d1f080e61473`,
CHV 4 vCPU, ~4 GiB, kernel `6.12.94-0-virt`, image `p149a` overlay with
this tree injected (`write_log`, skip live idx on apply `hint≥16`,
RFC-0153 P0 cache bytes).  
**Harness:** split `ycsb` / `deps` / `kvrocks` (extra CFs only when the
suite needs them), 3 rounds, `ops=2000` zipf, cargo `--offline` in-guest
(`RESULT=BUILD_OK 23:39:54`). Guest `load1` 0.51–1.19.

JSON: `r{1,2,3}/{async,rocks,deps,rocks-deps,kvr,rocks-kvr}/`. Gate log:
`serial.log`.

| shape | min | median | r1 / r2 / r3 | >3 |
|---|---:|---:|---|:---:|
| ycsb_a | 2.413 | 2.683 | 2.413 / 2.779 / 2.683 | |
| ycsb_b | 1.572 | 2.328 | 2.328 / 1.572 / 2.976 | |
| ycsb_c | 1.660 | 2.816 | 2.816 / 1.660 / 3.941 | |
| ycsb_d | 1.666 | 2.628 | 2.628 / 1.666 / 3.298 | |
| ycsb_e | 9.632 | **11.026** | 11.026 / 9.632 / 11.512 | yes |
| ycsb_f | 1.983 | 2.009 | 2.199 / 1.983 / 2.009 | |
| deps_cache_overwrite | 1.716 | 1.844 | 1.844 / 3.103 / 1.716 | |
| deps_lock_prewrite | 3.818 | **3.907** | 3.918 / 3.818 / 3.907 | yes |
| deps_mvcc_latest | 0.196 | 0.208 | 0.208 / 0.196 / 0.213 | |
| deps_apply_batch | 2.963 | 2.987 | 2.987 / 2.963 / 3.021 | |
| deps_raftlog | 1.606 | 1.688 | 1.721 / 1.688 / 1.606 | |
| deps_scan | 4.764 | **4.821** | 4.764 / 4.821 / 5.162 | yes |
| kvrocks_get | 3.724 | **4.477** | 4.477 / 5.076 / 3.724 | yes |
| kvrocks_set | 1.432 | 1.666 | 2.702 / 1.432 / 1.666 | |
| kvrocks_scan | 0.070 | 0.076 | 0.070 / 0.079 / 0.076 | |
| kvrocks_pipelined_set | 3.528 | **4.619** | 6.024 / 4.619 / 3.528 | yes |
| kvrocks_blob_set | 2.259 | 2.463 | 2.259 / 2.654 / 2.463 | |

`RESULT=P149_FAIL over_med=5/17 min_ratio=0.070`

Apply 2× **closed on this CHV cut** (min 2.963, 3/3 ≥2.96). Lock and
`deps_scan` are over 3×. The holes vs the previous CHV quiet 5/17
(`min=1.051`, old tree, no `write_log`) are **default-CF scan / mvcc**:

- `kvrocks_scan` Pedra ~2.5k qps vs Rocks ~34k (was 38–48× on the old tree).
- `deps_mvcc_latest` Pedra ~45k vs Rocks ~220k (was ~2.3–2.5×).

That is the apply-path deferred idx: `insert_many` hint≥16 skips live
`tail_idx` on `default`/`lock`; write CF stays on `write_log` so
`deps_scan` is a range count. Default-CF iterators / latest-visible
still pay the stale idx. Metal 11/17 split and Mac 12/17 are **not**
this virt gate.

RFC-0149 P2.1 stays `doing`.
