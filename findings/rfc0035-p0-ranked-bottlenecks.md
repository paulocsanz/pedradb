# RFC-0035 P0.4 — ranked bottlenecks (MVCC + deps_scan)

**Date:** 2026-08-16  
**Commit of probe:** this PR  
**Harness:** `ROCKS_PARITY_SUITE=deps` 4096/2000 zipfian 1 KB, after `deps_apply_batch` (64k txns). Compat only for splits; Rocks FF peer from the same-class lab (`586ec6e` / `eaa4adf`): MVCC 160k qps / 5.4 µs, scan 165k / 5.7 µs.  
**Raw:** [rfc0035-p0/compat-deps.json](rfc0035-p0/compat-deps.json)

LSM at read time: **3 SST** (L0=2, L1=1), **14 400** mem entries.

## MVCC latest (`deps_mvcc_latest`)

2000 ops. Pedra 9 042 qps / p50 **77.6 µs** / p95 310 µs → **~18×** Rocks (need ~11 µs for 2×).

| split | p50 | p95 |
|---|---:|---:|
| `latest_cf` (write prefix) | **38.9 µs** | 228 µs |
| `get_cf` (default, 1 KB) | **37.5 µs** | 82.5 µs |

Counters:

| | |
|---|---:|
| `latest_mem_hit` | 1 574 (**79%**) |
| `latest_sst_fallback` | 426 (**21%**) |
| SST probed / fallback | 1 278 / 426 = **3.00** (every file, every fallback) |
| block cache | 1 208 hit / 1 067 miss (all misses decoded) |

**Ranking (MVCC)**

1. **`get_cf` of the 1 KB default value — 37.5 µs p50.** Already **7×** Rocks' *entire* latest+get (5.4 µs). Even if `latest_cf` were free, 37.5 µs ⇒ max ~27k qps ≪ 80k floor. This **blocks 2× by itself**. Suspects (not yet split): vlog resolve (`large_value_threshold` default 512), CF-prefix encode, lookup over 3 SST + 14k mem. `ycsb_c` on a lighter LSM was 5.7 µs for the same 1 KB — same get class, different shape.
2. **`latest_cf` p50 39 µs on the 79% mem-hit path.** Mem BTree + mutex should be ~µs; 39 µs is still ~7× a Rocks SeekForPrev. Suspects: `Mutex<Db>` + encode CF + `get_entry` on 14k mem.
3. **SST fallback 21% at p95 latest 228 µs.** 3 files probed every miss. Pulls the average (wall 111 µs/op) but is **not** the p50.

## deps_scan

2000 ops. Pedra 6 164 qps / p50 **160 µs** → **~27×** Rocks (need ~12 µs for 2×).

| | |
|---|---:|
| scan ops | 2 000 |
| SST probed | 6 000 = **3.00 / op** (every file, every scan) |
| block cache | 1 193 hit / **5 134 miss** (hit rate **19%**) |
| blocks decoded | 5 134 = **2.57 / op** |

**Ranking (scan)**

1. **Almost-cold block cache: 2.57 lz4 decodes per 25-key scan.** Zipfian windows hit different blocks in the same L0; capacity 256 does not hold the working set. Each decode is on the order of tens of µs → this alone can explain ~160 µs.
2. **Always 3 SST streams** (2 L0 + 1 L1). Merge setup is paid even when `limit=25` and KeyOnly.

## P1.1 target (this finding)

**Cut `get_cf` of the 1 KB default value on the MVCC path down toward `ycsb_c` (~6 µs).** Until that number moves, 2× on `deps_mvcc_latest` is arithmetically impossible.

Next measurement inside that cut (do not skip): count how many of those gets resolve a vlog pointer vs an inline SST/mem value.

Scan 2× is a separate #1 (decode miss rate / L0 overlap). Do not start it until MVCC #1 has a number after P1.1.

G1–G8: this finding is read-path only. No `sync_data`. No per-layer visibility cap.
