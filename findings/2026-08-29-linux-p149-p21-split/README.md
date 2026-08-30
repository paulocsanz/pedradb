# RFC-0149 P2.1 — Linux 4-CPU coluna A maioria >3× **PASS**

**When:** 2026-08-29T18:00Z  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `WriteOptions.sync=false`.  
**Pass:** ≥9/17 median > 3.0.  
**Host:** AMD Ryzen Threadripper PRO 3975WX, `CPUQuota=400%` `CPUAffinity=0-3`,
NVMe chroot `/var/lib/caixote-nvme/p149-root`.

**Two cuts vs the 7/17 FAIL** ([`../2026-08-29-linux-p149/`](../2026-08-29-linux-p149/)):

1. Idle admission skip (1c put does not `to_string()` the CF when stall is off).
2. Split suites: `ycsb` / `deps` / `kvrocks` each open their own DB.
   `CompatEngine` only registers extra CFs when the suite needs them
   (`engines.rs`). Combined `ycsb,deps` forced `default\0` on every YCSB key;
   Rocks default CF does not. Same 17 shapes, same peer class.

JSON: `nvme-cgroup4/`. Gate log: `nvme-cgroup4/gate.log`.

| shape | min | median | >3 |
|---|---:|---:|:---:|
| ycsb_a | 3.082 | **3.231** | yes |
| ycsb_b | 2.617 | 2.756 | |
| ycsb_c | 3.012 | **3.059** | yes |
| ycsb_d | 2.971 | **3.069** | yes |
| ycsb_e | 12.834 | **13.058** | yes |
| ycsb_f | 2.344 | 2.444 | |
| deps_cache_overwrite | 2.856 | **3.060** | yes |
| deps_lock_prewrite | 1.883 | 1.935 | |
| deps_mvcc_latest | 3.415 | **3.452** | yes |
| deps_apply_batch | 1.690 | 1.729 | |
| deps_raftlog | 1.130 | 1.130 | |
| deps_scan | 3.134 | **3.144** | yes |
| kvrocks_get | 4.564 | **4.768** | yes |
| kvrocks_set | 2.907 | **3.032** | yes |
| kvrocks_scan | 37.009 | **37.875** | yes |
| kvrocks_pipelined_set | 3.052 | **3.221** | yes |
| kvrocks_blob_set | 1.952 | 2.084 | |

**11/17 PASS** (need ≥9). Still below 3×: b, f, lock, apply, raftlog, blob.

`RESULT=P149_PASS over_med=11/17 min_ratio=1.130`

Coluna A only (async vs Rocks default). Not a win vs `sync=true`. Not G1.
