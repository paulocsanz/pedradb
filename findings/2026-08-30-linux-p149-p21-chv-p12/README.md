# RFC-0149 P2.1 — CHV 4 vCPU, RFC-0154 P1.2 (`write` BTree)

**When:** 2026-08-30T00:48:02Z–00:55:57Z  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `WriteOptions.sync=false`.
Not a win vs `sync=true`. Not G1.  
**Pass:** ≥9/17 median > 3.0.  
**Tree:** live `tail_idx` + **`write` ordered BTree** (not HashMap).
`lock`/`default` stay HashMap.  
**Harness:** split suites, 3 rounds, ops=2000 zipf, CHV 4 vCPU
`linux-gate-p149b`, kernel 6.12 virt. `RESULT=BUILD_OK 00:54:08`.

JSON: `r{1,2,3}/…`. Gate: `serial.log`.

| shape | min | median | r1 / r2 / r3 | >3 |
|---|---:|---:|---|:---:|
| ycsb_a | 2.516 | 2.528 | 2.516 / 2.528 / 2.783 | |
| ycsb_b | 1.434 | 2.510 | 1.434 / 4.314 / 2.510 | |
| ycsb_c | 3.044 | **3.226** | 3.044 / 5.883 / 3.226 | yes |
| ycsb_d | 2.868 | 2.939 | 2.868 / 6.152 / 2.939 | |
| ycsb_e | 13.489 | **15.478** | 13.489 / 15.978 / 15.478 | yes |
| ycsb_f | 2.136 | 2.209 | 2.136 / 2.209 / 2.520 | |
| deps_cache_overwrite | 2.086 | 2.749 | 3.599 / 2.086 / 2.749 | |
| deps_lock_prewrite | 2.463 | 2.601 | 2.463 / 4.317 / 2.601 | |
| deps_mvcc_latest | 3.283 | **3.480** | 4.175 / 3.283 / 3.480 | yes |
| deps_apply_batch | 1.625 | 2.201 | 2.201 / 2.635 / 1.625 | |
| deps_raftlog | 1.171 | 1.285 | 1.285 / 2.305 / 1.171 | |
| deps_scan | 3.703 | **4.491** | 4.491 / 3.703 / 4.590 | yes |
| kvrocks_get | 3.595 | **4.671** | 10.252 / 3.595 / 4.671 | yes |
| kvrocks_set | 1.935 | 2.445 | 2.516 / 1.935 / 2.445 | |
| kvrocks_scan | 37.135 | **52.983** | 37.135 / 53.766 / 52.983 | yes |
| kvrocks_pipelined_set | 4.273 | **5.054** | 5.054 / 7.283 / 4.273 | yes |
| kvrocks_blob_set | 2.525 | 2.650 | 2.525 / 2.650 / 2.834 | |

`RESULT=P149_FAIL over_med=7/17 min_ratio=1.171`

Versus HashMap-`write` CHV ([`../2026-08-30-linux-p149-p21-chv/`](../2026-08-30-linux-p149-p21-chv/), 6/17 min 0.859):

- **mvcc 1.02 → 3.48×** (P1.2: reverse-seek on BTree, not copy+sort).
  Metal BTree split was 3.45× — recovered.
- apply min 2.23 → **1.625** (BTree insert of always-new write keys).
  2× no longer guaranteed on this CHV cut.
- scan/e/get/pipelined still >3. raftlog still ~1.2.

Need ≥9. Closest: d 2.94, cache 2.75, blob 2.65, lock 2.60, a 2.53.
r2 is noisy (b 1.43 / 4.31). Metal 11/17 is not CHV.

RFC-0149 P2.1 stays `doing`.
