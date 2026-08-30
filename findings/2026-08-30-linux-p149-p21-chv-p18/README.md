# RFC-0149 P2.1 — CHV, P1.8 (put TLS intern, no second memcpy)

**When:** 2026-08-30T02:59:42Z–03:07:42Z  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `sync=false`.  
**Tree:** P1.6 `commit_async_one` + P1.8 `intern_put_value` (LAST_GET/LAST_CF
write-through is a refcount). P1.7 RMW reverted. Guest `linux-gate-p149b`
CHV 4 vCPU.

| shape | min | median | r1 / r2 / r3 | >3 |
|---|---:|---:|---|:---:|
| ycsb_a | 3.246 | **3.410** | 3.410 / 3.246 / 3.883 | yes |
| ycsb_b | 3.337 | **3.820** | 3.820 / 3.337 / 4.786 | yes |
| ycsb_c | 3.678 | **4.150** | 4.150 / 3.678 / 5.193 | yes |
| ycsb_d | 2.944 | **3.591** | 3.591 / 2.944 / 4.734 | yes |
| ycsb_e | 13.738 | **15.647** | 15.647 / 23.355 / 13.738 | yes |
| ycsb_f | 2.673 | **3.055** | 2.673 / 3.055 / 3.420 | yes |
| deps_cache_overwrite | 1.471 | 2.819 | 3.280 / 2.819 / 1.471 | |
| deps_lock_prewrite | 2.516 | **3.461** | 3.461 / 2.516 / 3.818 | yes |
| deps_mvcc_latest | 2.763 | **3.959** | 4.456 / 3.959 / 2.763 | yes |
| deps_apply_batch | 1.772 | 1.878 | 1.772 / 1.878 / 2.348 | |
| deps_raftlog | 1.054 | 1.079 | 1.212 / 1.079 / 1.054 | |
| deps_scan | 4.048 | **4.434** | 5.042 / 4.434 / 4.048 | yes |
| kvrocks_get | 4.145 | **4.823** | 4.145 / 4.823 / 5.046 | yes |
| kvrocks_set | 2.200 | 2.457 | 2.457 / 2.200 / 3.024 | |
| kvrocks_scan | 37.838 | **43.832** | 43.832 / 37.838 / 60.810 | yes |
| kvrocks_pipelined_set | 3.485 | **3.750** | 3.485 / 3.750 / 4.167 | yes |
| kvrocks_blob_set | 2.201 | 2.251 | 2.201 / 2.251 / 2.606 | |

`RESULT=P149_PASS over_med=12/17 min_ratio=1.054`

P1.6 was **10/17**. This cut added **ycsb_f** (2.73→3.06) and
**deps_lock_prewrite** (2.61→3.46). Repeat payloads (YCSB `yval`, lock
prewrite) no longer `copy_from_slice` into TLS on every put.

`kvrocks_set` 2.46 did **not** close (one round 3.02). Apply 1.88 is
below the 2× floor of earlier cuts — quote it, don't hide it. raftlog
1.08 still the sequential p50 tie. Not a win vs `sync=true`.
