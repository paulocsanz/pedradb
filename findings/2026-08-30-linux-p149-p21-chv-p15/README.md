# RFC-0149 P2.1 — CHV, P1.5 (1c put TLS per-key, not epoch wipe)

**When:** 2026-08-30T01:58:19Z–02:06:19Z  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `sync=false`.  
**Tree:** P1.2 write BTree + P1.4 `tail_ord_stale` + P1.5 `KeyGenMap` /
`point_tls_epoch`. Guest `linux-gate-p149b` CHV 4 vCPU.

| shape | min | median | r1 / r2 / r3 | >3 |
|---|---:|---:|---|:---:|
| ycsb_a | 2.918 | 2.930 | 2.930 / 2.918 / 3.007 | |
| ycsb_b | 3.039 | **3.328** | 3.328 / 3.039 / 3.460 | yes |
| ycsb_c | 3.488 | **3.690** | 3.690 / 3.488 / 3.697 | yes |
| ycsb_d | 3.127 | **3.130** | 3.130 / 3.127 / 3.371 | yes |
| ycsb_e | 13.654 | **13.691** | 30.673 / 13.691 / 13.654 | yes |
| ycsb_f | 2.452 | 2.606 | 2.843 / 2.452 / 2.606 | |
| deps_cache_overwrite | 2.775 | **3.082** | 3.082 / 3.504 / 2.775 | yes |
| deps_lock_prewrite | 2.478 | 2.497 | 2.574 / 2.497 / 2.478 | |
| deps_mvcc_latest | 2.440 | **3.687** | 4.322 / 2.440 / 3.687 | yes |
| deps_apply_batch | 2.056 | 2.204 | 2.056 / 2.204 / 2.289 | |
| deps_raftlog | 1.137 | 1.163 | 1.261 / 1.163 / 1.137 | |
| deps_scan | 3.803 | **4.810** | 5.056 / 3.803 / 4.810 | yes |
| kvrocks_get | 4.386 | **4.728** | 4.386 / 5.104 / 4.728 | yes |
| kvrocks_set | 2.457 | 2.620 | 2.620 / 2.657 / 2.457 | |
| kvrocks_scan | 38.792 | **43.180** | 43.180 / 38.792 / 57.121 | yes |
| kvrocks_pipelined_set | 4.066 | **4.199** | 4.199 / 4.066 / 4.340 | yes |
| kvrocks_blob_set | 2.595 | 2.613 | 2.613 / 2.595 / 2.999 | |

`RESULT=P149_PASS over_med=10/17 min_ratio=1.137`

Gate = median of 3 rounds >3.0 on ≥9/17. Official virt shape (CHV 4 vCPU),
not metal. Not a win vs `sync=true`.

P1.4 was **7/17 FAIL**. This cut added **b**, **d**, **cache_overwrite**.
Mechanism: a 1-key put used to bump `read_cache_epoch` and drop all 4096
TLS last-get slots; zipf A/B/D then paid HashMap get after every update.
P1.5 bumps one `KeyGenMap` bucket (and still bumps count-epoch). Other
hot keys stay cached. Fat apply / range still epoch-bumps.

`ycsb_a` 2.93 is the closest remaining miss. raftlog 1.16 is sequential
SkipList-vs-BTree (p50 tied; 2× on 1c raftlog remains refused). Apply 2.20
holds the 2× floor.
