# RFC-0154 P2.4 — CHV, put_owned interned SET/blob **REFUSED**

**When:** 2026-08-30T05:42:00Z–05:43:49Z (build 05:34-ish–05:42)  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `sync=false`.  
**Tree:** P1.8 intern + P2.4 (`put_owned` + `intern_put_key` so interned
Bytes skip a second memcmp). WAL not skipped. P2.3 reverted.  
**Guest:** `linux-gate-p149b` CHV 4 vCPU.

| shape | min | median | r1 / r2 / r3 | >3 |
|---|---:|---:|---|:---:|
| ycsb_a | 2.577 | **3.308** | 6.388 / 2.577 / 3.308 | yes |
| ycsb_b | 2.984 | **6.989** | 6.989 / 2.984 / 7.285 | yes |
| ycsb_c | 3.226 | **7.108** | 7.108 / 3.226 / 17.420 | yes |
| ycsb_d | 2.501 | **6.963** | 6.963 / 2.501 / 7.905 | yes |
| ycsb_e | 9.696 | **16.979** | 16.979 / 9.696 / 28.672 | yes |
| ycsb_f | 1.770 | 2.865 | 2.865 / 1.770 / 4.383 | |
| deps_cache_overwrite | 1.663 | 1.978 | 2.275 / 1.978 / 1.663 | |
| deps_lock_prewrite | 1.836 | 1.873 | 1.873 / 1.836 / 2.356 | |
| deps_mvcc_latest | 1.571 | **3.277** | 5.123 / 3.277 / 1.571 | yes |
| deps_apply_batch | 1.889 | 2.284 | 2.354 / 2.284 / 1.889 | |
| deps_raftlog | 0.962 | 0.992 | 1.116 / 0.992 / 0.962 | |
| deps_scan | 1.554 | **4.944** | 5.881 / 4.944 / 1.554 | yes |
| kvrocks_get | 3.279 | **3.441** | 3.441 / 3.279 / 4.431 | yes |
| kvrocks_set | 1.628 | 1.868 | 1.868 / 1.914 / 1.628 | |
| kvrocks_scan | 37.082 | **47.412** | 37.082 / 47.412 / 55.614 | yes |
| kvrocks_pipelined_set | 2.568 | 2.675 | 2.568 / 2.675 / 4.060 | |
| kvrocks_blob_set | 2.255 | 2.546 | 2.546 / 2.888 / 2.255 | |

`RESULT=P149_PASS over_med=9/17 min_ratio=0.962`

**Refuse.** Lost `ycsb_f` 3× (2.87 vs P1.8 3.06), `deps_lock_prewrite`
(1.87 vs 3.46), and `kvrocks_pipelined_set` (2.68 vs 3.75). SET 2.46→1.87
did not close; cache 2.82→1.98 worse; blob 2.25→2.55 still not 3×.
`deps_raftlog` min 0.962 is below 1.0 (quote, don't hide). Code reverted.
Official CHV stays
[`../2026-08-30-linux-p149-p21-chv-p18/`](../2026-08-30-linux-p149-p21-chv-p18/)
**12/17**.
