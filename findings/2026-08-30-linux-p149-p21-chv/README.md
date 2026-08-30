# RFC-0149 P2.1 — CHV 4 vCPU, RFC-0154 live idx

**When:** 2026-08-30T00:19:40Z–00:27:37Z  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `WriteOptions.sync=false`
(`ROCKS_PARITY_SYNC=0`). Not a win vs `sync=true`. Not G1.  
**Pass:** ≥9/17 median > 3.0.  
**Guest:** `linux-gate-p149b`, CHV 4 vCPU, ~4 GiB, kernel `6.12.94-0-virt`,
image `p149a` overlay with RFC-0154 (`tail_idx` live; no `idx_stale` /
`write_log`).  
**Harness:** split `ycsb` / `deps` / `kvrocks`, 3 rounds, `ops=2000` zipf,
cargo `--offline` in-guest (`RESULT=BUILD_OK 00:25:48`). Guest `load1` 0.41–0.76.

JSON: `r{1,2,3}/{async,rocks,deps,rocks-deps,kvr,rocks-kvr}/`. Gate log:
`serial.log`.

| shape | min | median | r1 / r2 / r3 | >3 |
|---|---:|---:|---|:---:|
| ycsb_a | 2.283 | 2.734 | 2.734 / 2.749 / 2.283 | |
| ycsb_b | 2.590 | 2.607 | 2.590 / 2.811 / 2.607 | |
| ycsb_c | 2.976 | **3.093** | 3.093 / 4.723 / 2.976 | yes |
| ycsb_d | 2.786 | 2.937 | 2.937 / 4.498 / 2.786 | |
| ycsb_e | 14.323 | **15.659** | 15.659 / 15.963 / 14.323 | yes |
| ycsb_f | 1.333 | 2.205 | 2.285 / 1.333 / 2.205 | |
| deps_cache_overwrite | 2.576 | 2.646 | 2.910 / 2.576 / 2.646 | |
| deps_lock_prewrite | 2.461 | 2.674 | 3.097 / 2.674 / 2.461 | |
| deps_mvcc_latest | 0.859 | 1.021 | 1.021 / 0.859 / 1.089 | |
| deps_apply_batch | 2.228 | 2.261 | 3.080 / 2.261 / 2.228 | |
| deps_raftlog | 1.092 | 1.146 | 1.239 / 1.092 / 1.146 | |
| deps_scan | 4.614 | **6.391** | 6.391 / 4.614 / 6.887 | yes |
| kvrocks_get | 4.984 | **5.431** | 5.431 / 4.984 / 6.310 | yes |
| kvrocks_set | 2.616 | 2.753 | 2.616 / 2.753 / 2.963 | |
| kvrocks_scan | 50.421 | **59.221** | 61.177 / 50.421 / 59.221 | yes |
| kvrocks_pipelined_set | 3.626 | **3.925** | 4.258 / 3.925 / 3.626 | yes |
| kvrocks_blob_set | 2.067 | 2.478 | 2.478 / 2.067 / 2.542 | |

`RESULT=P149_FAIL over_med=6/17 min_ratio=0.859`

Versus the lazy-idx CHV cut (`findings/2026-08-29-linux-p149-p21-chv/`, 5/17
min 0.070):

- `kvrocks_scan` 0.070 → **59×** (Pedra ~1.7–2.1M qps vs Rocks ~28–35k).
- `deps_mvcc_latest` 0.208 → **1.02×** (Pedra ~213–227k vs Rocks ~195–250k).
  The O(n) rebuild is gone; it is now same-class with Rocks, not 3×.
- apply still ≥2× (min **2.228**, was 2.963 with skipped idx).

Need ≥9. Closest below 3×: d 2.94, set 2.75, a 2.73, lock 2.67, cache 2.65.
Metal 11/17 and Mac 12/17 are **not** this virt gate.

RFC-0149 P2.1 stays `doing`.

Per-shape (p50/wall/probes vs Rocks SkipList, Hash* wiki, TiKV raftdb /
Raft Engine): [`ARCHITECTURE.md`](ARCHITECTURE.md).
