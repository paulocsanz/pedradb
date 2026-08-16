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

## P1.1 follow-up (get path)

Shipped: (1) `lookup` returns on the first mem layer that has a point — skip SST (same G2 as `last_under_user_prefix`); (2) `last_prefix_then_get` one mutex; (3) counters `get_inline` / `get_vlog`.

Clean remesure after (1), two-lock split still on: **get_vlog = 0 / get_inline = 2000**. The 1 KB value is **inline**, not vlog. `get_mem_hit` 1568 / 2000 (78%). get p50 **36 µs** (was 38 µs) — skipping SST on the hot path did **not** move p50. Combined one-lock remesure was noisy (3.1k qps); do not read it as a regression vs 9.5k.

**P1.1 did not reach 2×.** Residual on the mem-hit get (~36 µs vs `ycsb_c` 5.7 µs): mutex + CF encode + `get_entry` on 14k mem + 1 KB copy, after a heavy apply. Next cut (P1.2) must explain those 36 µs (not invent vlog).

## P1.2 follow-up (36 µs mem-hit get)

Shipped: (1) MemTable keyed by user key (`BTreeMap<Bytes, Vec<Version>>`) so `get_entry` is a borrowed lookup — no `InternalKey` / `Bytes::copy_from_slice` probe; (2) `parking_lot::Mutex` on compat; (3) stack `encode_with` + no extra `user.to_vec`; (4) intra-lock split `mvcc_ns_{encode,last,get,copy}`.

Clean remesure ([compat.json](rfc0035-p12/compat.json) / [rocks-ff.json](rfc0035-p12/rocks-ff.json), 4096/2000 zipfian 1 KB, `ROCKS_PARITY_FULL_SYNC=1`, same run):

| | Pedra | p50 | Rocks FF | p50 | slower (qps) |
|---|---:|---:|---:|---:|---:|
| `deps_mvcc_latest` | **18 826** | **2.0 µs** | 251 671 | 3.3 µs | **13×** |
| `deps_scan` | 3 603 | 139 µs | 260 906 | 3.6 µs | 72× (p50 ~39×; not the P1.2 target) |

Was (P1.1): MVCC 9 006 qps / 77 µs. p50 of the combined op is now **faster than Rocks**. Encode 57 ns/op, copy 235 ns/op — **not** the 36 µs. Get mean **10.5 µs** (was 36 µs p50). Last mean **42 µs** (79% mem-hit is µs; 21% SST fallback is the mean).

**2× qps still not met** (need ~126k vs this peer). Remaining #1: **SST fallback 21%**, 3 files every miss, p95 265 µs vs Rocks p95 7 µs. That tail is why 2 µs p50 still yields only 19k qps. P1.3 = cut fallback, not another mem-hit micro-opt.

G1–G8: read-path only. Visibility = `get_entry` / `lookup`. Adversarial compat green. No `sync_data`.

## P1.3 follow-up (SST fallback + scan cache)

Shipped: (1) `last_under_user_prefix` / `lookup` walk SST **newest-first** and stop at the first live point (same single-writer invariant as the mem hit; newer tombstone still `before`-retries); (2) `BlockCache` is LRU (was HashMap `keys().next()`) and default 256 → 2048.

Clean remesure ([compat.json](rfc0035-p13/compat.json) / [rocks-ff.json](rfc0035-p13/rocks-ff.json), same knobs, same run):

| | Pedra | p50 | Rocks FF | p50 | slower (qps) |
|---|---:|---:|---:|---:|---:|
| `deps_mvcc_latest` | **45 277** | **1.5 µs** | 187 050 | 3.4 µs | **4.1×** |
| `deps_scan` | 6 212 | 131 µs | 272 937 | 3.4 µs | 44× |

Probes: SST files / fallback **2.08** (was 3.00); `get_sst_fallback` 432 (was 858). Scan block cache **95% hit** (6016/311), **0.16** decodes/op (was 2.57 / 19%). last mean 13 µs, get mean 8 µs.

**2× still not met** (need ~94k MVCC / ~136k scan). MVCC tail is the remaining 21% fallback (p95 192 µs). Scan p50 **did not move** (~131 µs) after decode went away — the leftover is merge/setup of 3 SST streams + mem materialize, not lz4. Next cut is that CPU path (or P2.1 cliff), not another cache bump.

G1–G8: read-path only. `last_under_user_prefix` still MVCC-user-prefix only. Adversarial green.

## P1.3b follow-up (one visible version per user per layer)

Shipped: SST range + mem stream emit only the newest `seq ≤ snapshot` per user key in that layer (older versions are not cloned into the merge). G2 unchanged: a newer tombstone still wins; later users still appear (`scan_skips_older_versions_still_sees_later_users`).

Clean remesure ([compat.json](rfc0035-p13b/compat.json) / [rocks-ff.json](rfc0035-p13b/rocks-ff.json)):

| | Pedra | p50 | Rocks FF | p50 | slower (qps) |
|---|---:|---:|---:|---:|---:|
| `deps_mvcc_latest` | 51 366 | 1.3 µs | 227 934 | 3.5 µs | **4.4×** |
| `deps_scan` | **7 991** | **117 µs** | 229 010 | 3.8 µs | **29×** (was 44× / 131 µs) |

**2× still not met.** Scan p50 dropped only ~14 µs — walking the block (even without cloning every version) + 3-stream setup is still ~117 µs vs Rocks 3.8 µs. That is the measured cliff for this merge shape. Next is a different iterator (or P2.1), not another skip-versions pass.

G1–G8: read-path only. Adversarial green.
