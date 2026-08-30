# RFC-0154 P2.6 — CHV refuse: write-only publish skips empty read-cache mutexes

**Verdict: REFUSED.** `RESULT=P149_PASS over_med=9/17 min_ratio=0.951`
(2026-08-30 12:42:58Z, guest `linux-gate-p149b`, 3 rounds, ops=2000 zipf,
split suites, official peer RocksDB default `sync=false`).

Cut under test: `AnswerCache::used: AtomicBool` — `clear()` /
`invalidate_many()` early-return while the cache was never filled, so a
write-only 1c publish skips the `point_cache` / `last_prefix_cache` mutexes.
No WAL skip, no `dirty_points` skip, `CountCache` still locks (F204).

## 17 shapes vs P1.8 hold (12/17 min 1.054)

| shape | P1.8 min/med | P2.6 min/med | rounds | verdict |
|---|---|---|---|---|
| ycsb_a | 3.246 / **3.410** | 3.190 / **3.292** | 3.342 3.292 3.190 | kept 3× |
| ycsb_b | 3.337 / **3.820** | 3.230 / **3.322** | 3.322 3.322 3.230 | kept 3× |
| ycsb_c | 3.678 / **4.150** | 3.310 / **3.393** | 3.594 3.310 3.393 | kept 3× |
| ycsb_d | 2.944 / **3.591** | 3.126 / **3.381** | 3.381 3.409 3.126 | kept 3× |
| ycsb_e | 13.738 / **15.647** | 12.653 / **13.874** | 13.874 14.572 12.653 | kept 3× |
| ycsb_f | 2.673 / **3.055** | 0.992 / 2.341 | 0.992 2.802 2.341 | **LOST 3×** |
| deps_cache_overwrite | 1.471 / 2.819 | 1.426 / 2.711 | 2.711 2.871 1.426 | target, flat/worse |
| deps_lock_prewrite | 2.516 / **3.461** | 2.013 / 2.337 | 2.368 2.337 2.013 | **LOST 3×** |
| deps_mvcc_latest | 2.763 / **3.959** | 1.681 / 2.357 | 4.290 2.357 1.681 | **LOST 3×** |
| deps_apply_batch | 1.772 / 1.878 | 1.287 / 1.998 | 2.018 1.998 1.287 | still <3× |
| deps_raftlog | 1.054 / 1.079 | 0.951 / 1.005 | 1.005 0.951 1.376 | **min < 1.0 floor** |
| deps_scan | 4.048 / **4.434** | 3.010 / **3.587** | 5.450 3.587 3.010 | kept 3× |
| kvrocks_get | 4.145 / **4.823** | 3.836 / **4.352** | 4.888 4.352 3.836 | kept 3× |
| kvrocks_set | 2.200 / 2.457 | 1.345 / 1.916 | 3.160 1.345 1.916 | target, **worse** |
| kvrocks_scan | 37.838 / **43.832** | 39.059 / **50.506** | 60.438 39.059 50.506 | kept 3× |
| kvrocks_pipelined_set | 3.485 / **3.750** | 2.465 / **3.705** | 4.233 2.465 3.705 | kept 3× |
| kvrocks_blob_set | 2.201 / 2.251 | 1.714 / 2.527 | 2.527 2.617 1.714 | target, flat |

## Why refused

- Gate regressed **12/17 → 9/17**: `ycsb_f`, `deps_lock_prewrite`,
  `deps_mvcc_latest` all lost majority 3×.
- `deps_raftlog` min 0.951 is **below the RFC-0041 1.0 floor**.
- The three target shapes did not close: `kvrocks_set` got **worse**
  (2.46→1.92 med), `kvrocks_blob_set` flat (2.25→2.53), and
  `deps_cache_overwrite` flat (2.82→2.71).
- Same refusal signature as P1.7/P1.9/P2.1a–P2.5: a 1c write-path
  publish change costs the read-modify-write / lock shapes their 3× even
  when the write-only shapes are the intended beneficiaries. Once any
  read fills the cache (`used=true`), ycsb_f/lock/mvcc publishes pay the
  same mutex as before **plus** two extra atomic round-trips per publish,
  and their round spread (mvcc 4.29→1.68) widened.

Code reverted to the P1.8 baseline (`cache.rs` no `used` flag; both
acceptance tests removed). Evidence: `serial.log`, `gate.txt` in this dir.
Not a win vs `sync=true`; single-client fd shapes stay fd-ceiling below 1×
by construction.
