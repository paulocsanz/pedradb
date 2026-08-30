# RFC-0149 P2.1 — Linux 4-CPU retry after idle admission skip

**When:** 2026-08-29T17:51Z  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `WriteOptions.sync=false`.  
**Pass:** ≥9/17 median > 3.0.  
**Host:** AMD Ryzen Threadripper PRO 3975WX, `CPUQuota=400%` `CPUAffinity=0-3`,
NVMe bind `/var/lib/caixote-nvme/p149-root` (same box as
[`../2026-08-29-linux-p149/`](../2026-08-29-linux-p149/)).

**Code:** skip `batch_families` / `ensure_write_admitted_for` when stall knobs
are off (parity default). Previous 1c put still `to_string()`’d the CF family
on every apply even though stall returned immediately.

JSON: `nvme-cgroup4/`. Gate log: `nvme-cgroup4/gate.log`.

| shape | min | median | >3 |
|---|---:|---:|:---:|
| ycsb_a | 2.698 | 2.732 | |
| ycsb_b | 2.311 | 2.372 | |
| ycsb_c | 2.743 | 2.870 | |
| ycsb_d | 2.566 | 2.573 | |
| ycsb_e | 11.609 | 11.850 | yes |
| ycsb_f | 2.153 | 2.303 | |
| deps_cache_overwrite | 3.123 | 3.179 | yes |
| deps_lock_prewrite | 1.934 | 2.008 | |
| deps_mvcc_latest | 3.388 | 3.570 | yes |
| deps_apply_batch | 1.720 | 1.747 | |
| deps_raftlog | 1.234 | 1.236 | |
| deps_scan | 3.364 | 3.458 | yes |
| kvrocks_get | 3.765 | 4.905 | yes |
| kvrocks_set | 2.756 | 3.158 | yes |
| kvrocks_scan | 34.028 | 37.902 | yes |
| kvrocks_pipelined_set | 3.193 | 3.356 | yes |
| kvrocks_blob_set | 1.974 | 2.026 | |

**8/17 FAIL** (was 7/17). Write 1c shapes moved: `kvrocks_set` 2.974→**3.158**,
`deps_cache_overwrite` 2.889→**3.179**. `ycsb_c` 3.022→2.870 (read path;
2000-op noise, r3 still 3.022). Closest miss: `ycsb_c` 2.870, `ycsb_a` 2.732.

`RESULT=P149_FAIL over_med=8/17 min_ratio=1.234`

This is coluna A (async vs Rocks default). Not a G1 claim. Not a win vs `sync=true`.
