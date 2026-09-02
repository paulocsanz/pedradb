# Hydrate sorted-run builder (RFC-0159 P0.3 + P1.5 + P1.1 clone-drop)

**When:** 2026-09-02  
**Peer:** RocksDB default `WriteOptions.sync=false`. Pedra async, same class.

Latched-family puts skip WAL + memtable BTree, accumulate in a sorted vec,
flush straight to `MAX_LSM_LEVEL`. Uninstalled tail is RAM-only (Rocks
`disableWAL` class). Installed chunks land in MANIFEST; async skips SST
fdatasync (write() of SST+MANIFEST).

**P1.5 (1M envelope):** after latch, `write_cf_owned`'s first-CF run skips
`BatchOp` / WriteGroup; intern the value once; `BulkRun::reserve`;
high-water ratchets the last key only.

**P1.1 follow-up (25M encode):** no per-entry `block_last_user` /
`bloom_keys` clones; bloom `insert(&[u8])` during the encode loop;
capacity capped at 2 Mi keys.

## Local Darwin (apply only, no settle in the timer)

`HYDRATE_N=… cargo run --release -p rocksdb-parity-bench --features real --example hydrate_shape`

1024-op batches, 200 B values, `data`+`meta` CFs, 256 MiB write buffer.

Darwin APFS inflates the WAL-skip (local is not the official claim).
Same-run pairs:

| n | pedra | rocks | ratio |
|---:|---:|---:|---:|
| 1M ×3 | 6.05 / 4.89 / 5.12 M/s | 2.26 / 2.72 / 3.09 M/s | **1.66–2.68×** (median **1.80×**) |
| 4M | 3.14 M/s | 1.77 M/s | **1.77×** |
| 10M | 2.89 M/s | 1.49 M/s | **1.94×** |
| 25M | 3.02 M/s | 1.52 M/s | **1.98×** |
| 100M | 3.04 M/s | 0.99 M/s | **3.07×** (completed; no OOM) |

P0.3-only 1M was 2.39 vs 3.13 (**0.76×**). P1.5 closed the 1M envelope.

## Guest CHV (official) — `linux-gate-p149b`

Peer: Rocks default `sync=false`. Pedra `Options.sync=false` + per-apply
`WriteOptions.sync=false` (slipstream `write_opt`, not `write_cf_owned`).
Diag-off, `PEDRA_STAGE_MAX_BYTES=64 MiB`. Load ~15–17.

| n | pedra hydrate | rocks hydrate | ratio | pedra settle | rocks settle |
|---:|---:|---:|---:|---:|---:|
| 1M v55 | **1.67 M/s** (0.6 s) | 0.97 M/s (1.0 s) | **1.72×** | 0.4 s | 1.0 s |
| 1M v56 | **1.69 M/s** (0.6 s) | 0.93 M/s (1.1 s) | **1.82×** | 0.4 s | 1.0 s |
| 10M v56 | **0.93 M/s** (10.7 s) | 0.90 M/s (11.1 s) | **1.03×** | 0.6 s | 4.6 s |
| 25M v56 | **0.75 M/s** (33.3 s) | 0.87 M/s (28.9 s) | **0.86×** | 0.8 s | 9.0 s |
| 25M v61 r1 | **0.84 M/s** (29.7 s) | 0.85 M/s (29.2 s) | **0.98×** | 0.8 s | 7.4 s |
| 25M v61 r2 | **0.87 M/s** (28.6 s) | 0.87 M/s (28.8 s) | **1.01×** | 0.7 s | 7.4 s |
| 25M v61 r3 | **0.84 M/s** (29.8 s) | 0.85 M/s (29.4 s) | **0.99×** | 0.7 s | 6.4 s |
| 25M v62 r1 | **0.87 M/s** (28.8 s) | 0.89 M/s (28.2 s) | **0.98×** | 0.7 s | 7.8 s |
| 25M v62 r2 | **0.80 M/s** (31.2 s) | 0.85 M/s (29.3 s) | **0.94×** | 0.9 s | 7.0 s |
| 25M v63 r1 | **0.87 M/s** (28.7 s) | 0.82 M/s (30.6 s) | **1.06×** | 1.0 s | 6.3 s |
| 25M v63 r2 | **0.89 M/s** (28.1 s) | 0.87 M/s (28.6 s) | **1.02×** | 1.1 s | 8.6 s |
| 25M v63 r3 | **0.87 M/s** (28.8 s) | 0.90 M/s (27.9 s) | **0.97×** | 1.0 s | 6.0 s |
| 25M v64 r1 | **0.84 M/s** (29.7 s) | 0.85 M/s (29.4 s) | **0.99×** | 1.2 s | 8.1 s |
| 25M v67 r1 | **0.88 M/s** (28.6 s) | 0.86 M/s (29.1 s) | **1.02×** | 1.0 s | 7.2 s |
| 25M v44 | 0.44–0.49 M/s | 0.81–0.84 M/s | **0.53–0.59×** | ~2.3 s | 8.3 s |
| 25M v45 | **0.39 M/s** (64.6 s) | 0.83 M/s (30.0 s) | **0.47×** | **0.2 s** | 6.8 s |
| 25M v46 | **0.55 M/s** (45.6 s) | 0.85 M/s (29.4 s) | **0.65×** | 1.3 s | 6.2 s |
| 25M v47 | **0.64 M/s** (39.1 s) | 0.85 M/s (29.4 s) | **0.75×** | 1.2 s | 6.2 s |
| 25M v48 | **0.71 M/s** (35.0 s) | 0.86 M/s (29.1 s) | **0.83×** | 0.8 s | 6.1 s |
| 25M v49 | **0.85 M/s** (29.3 s) | 0.87 M/s (28.8 s) | **0.98×** | 0.9 s | 7.9 s |
| 25M v50 | **0.78 M/s** (32.2 s) | 0.86 M/s (28.9 s) | **0.91×** | 1.1 s | 6.2 s |
| 25M v51 | **0.87 M/s** (28.9 s) | 0.87 M/s (28.9 s) | **1.00×** | 0.8 s | 7.5 s |
| 25M v52a | **0.79 M/s** (31.7 s) | 0.85 M/s (29.4 s) | **0.93×** | 0.9 s | 6.2 s |
| 25M v52b | **0.89 M/s** (28.0 s) | 0.88 M/s (28.5 s) | **1.01×** | 0.8 s | 8.9 s |
| 25M v52c | **0.75 M/s** (33.4 s) | 0.85 M/s (29.3 s) | **0.88×** | 0.8 s | 8.0 s |
| 25M v53a | **0.83 M/s** (30.1 s) | 0.89 M/s (28.2 s) | **0.93×** | 0.7 s | 7.1 s |
| 25M v54a | **0.87 M/s** (28.7 s) | 0.87 M/s (28.9 s) | **1.00×** | 0.7 s | 8.1 s |
| 25M v54b | **0.86 M/s** (29.0 s) | 0.85 M/s (29.4 s) | **1.01×** | 0.6 s | 7.1 s |
| 25M v54c | **0.88 M/s** (28.5 s) | 0.87 M/s (28.8 s) | **1.01×** | 0.6 s | 7.5 s |
| 100M v44 | SIGKILL (WAL-ring OOM) | 0.83 M/s (120.5 s) | — | — | 21.5 s |
| 100M v45 | SIGKILL (RSS 3.63 GiB, no hydrate line) | 0.86 M/s (115.6 s) | — | — | 21.5 s |

v55 1M (SST v6 4 KiB + block CRC, empty payload): hydrate **1.72×**,
settle 2.50×, prefix_scan 1.50×. Point gets lose: get_hit **0.79×**
(5.67 vs 4.46 µs), get_loop 0.85×, multi_get 0.73×. Streaming writer
left payload empty and `attach_payload_kit` ghost-registered
`payload_len` into the 256 MiB pool, so every probe was pread+CRC.
v44 1M get_hit was 1.375× with resident payload. v56 promotes on first
seek when the file fits (`guest-v55-1m.txt`).

v54 25M (stream SST in 4 MiB write batches, no 256 MiB image) ×3, load ~16–17:
a 1.00× (28.7 vs 28.9) / b 1.01× (29.0 vs 29.4) / c 1.01× (28.5 vs 28.8).
Median **1.01×**. All three walls Pedra < Rocks. Not a 3 s cut — the
memcpy floor is ~28.5 s. v53 (per-block 256 KiB writes) was 0.93×
(syscall overhead).

v52 25M (latched 1-put meta cursor skips WAL) ×3 same image, load 16–17:
a 0.93× / b **1.01×** / c 0.88×. Median **0.93×**. One run >1× is not
a published win. Host noise on this box is ~3 s Pedra (~10 %).

v51 25M (MANIFEST persist off the write lock): 28.9 vs 28.9 s = **1.00×**.
Tied, not >1×. Settle 0.8 vs 7.5 s. Single run; see v52 repeats.

v50 25M (P1.2 MANIFEST every 4, still under write lock): 32.2 vs 28.9 s =
**0.91×**, REGRESSED vs v49. Batching a larger persist while holding the
writer lock is the wrong shape.

v49 25M (256 KiB blocks + CRC-while-hot + seq range): 29.3 vs 28.8 s =
**0.98×**. Gap 0.5 s.

v48 25M (P1.7 park + in-place 64 KiB SST): 35.0 vs 29.1 s = **0.83×**.
Saved 4.1 s vs v47, not ≥1×. 1-effective-core guest: park does not hide
encode CPU.

v45 25M: compile of new core 24.82 s (md5-verified BulkRun on the image).
Settle still wins (bottom-level install). Hydrate stays SST-materialize
bound on the 1-effective-core guest — P1.5 (`write_cf_owned`) does not
fire on this bench (`WriteBatch` + `write_opt`). P0.3 skip-WAL should
be live via `commit_async_ops`; wall did not drop below the v44 band.

v45 100M: Rocks completed (0.86 M/s, 23.62 GiB). Pedra RSS climbed
~0.8 → 3.63 GiB during hydrate (avail 29 MB) then SIGKILL. Same 3.9 GiB
guest ceiling as v44. Linear RSS means the open tail is still accumulating
in RAM (WAL ring and/or an unflushed BulkRun), not bound to the 64 MiB
stage cap. Local Darwin 100M (96 GiB) completed at 3.04 vs 0.99 M/s.

## Crash contract

Process crash loses the open run. `flush()` / `settle()` persists it.
Same as Rocks bulk load with WAL disabled. G1 (`sync=true`) still
fsyncs SST+MANIFEST per chunk.
