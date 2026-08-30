# 2026-08-30 — rocksdb-compat per-op overhead cut (slipstream PR 19 shapes)

**Trigger:** beyondoss/slipstream PR 19 wires `rocksdb-compat` into their
snapshot-store bench (route-fold workload: `route.svc-{svc:06}.{route:08}`
keys, ~200 B values, 1000 routes/service, 1024-op apply batches, point
gets, per-service prefix scans, multi_get batches). Their card showed Pedra
losing badly on get_hit / prefix_scan / lookup_100 / settle.

**Diagnosis (compat layer only — core untouched):**
- `cf_handle` took the CF-registry **Mutex** + linear scan + a fresh
  `String` alloc per call; the adapter resolves the CF handle per get.
- `get_cf` → `get_named` re-validated the CF name (`check_cf`: mutex +
  scan) on **every** get, although handles are registry-validated at
  creation.
- `DBIterator::next()` copied every key+value out of the window slot
  (`to_vec().into_boxed_slice()`) — 2 extra allocs per entry on top of the
  2 the refill page already paid.
- `ReadOptions.lower/.upper` were **stored and never consumed** by
  `iterator_cf_opt`/`iterator_opt` (doc claimed "honoured"); the
  slipstream adapter worked around it with a manual `starts_with` break,
  so prefix scans over-read one window past the prefix.

**Fix (this commit):**
1. CF registry `Mutex<Vec<String>>` → `RwLock<Vec<Arc<str>>>`;
   `ColumnFamily.name: Arc<str>` — `cf_handle` is now a read lock + Arc
   clone, no alloc; `check_cf` a read lock.
2. `get_cf` fast path skips `check_cf` (handle trusted; stale
   use-after-`drop_cf` reads the dropped prefix as empty — caller-owned,
   like rocks). `get_named` (arbitrary strings) keeps the check.
3. `DBIterator::next`/`collect_rest` move the slot out (`mem::take`) —
   refill pages decode into exact-capacity Vecs, so the boxed handoff is
   pointer-only. Refill resume keys moved from `items[len-1]` to explicit
   `resume_fwd`/`resume_rev` fields (consumed slots are moved out, so the
   boundary key can no longer be read back from `items`).
4. `ReadOptions` iterate bounds consumed in `iterator_cf_opt` /
   `iterator_opt`: lower clamps the seek + start bound, upper tightens the
   encoded end bound (exclusive, rocks semantics), so refills and the core
   scan stop at the bound. Regression tests added (named-CF + default-CF +
   multi-window refill).
5. New tests: `iterator_cf_opt_honours_iterate_bounds`,
   `iterator_opt_bounds_and_multiwindow_refill`.

**Tests:** `cargo test -p rocksdb-compat` green in both trees
(monorepo: 82 lib + 11 integration; pub: same).

## Measurements

Guest: CHV `linux-gate-p149b` (quiet 64-core host, kernel-backed bench,
rustc 1.98.0), 1M entries (`SLIPSTREAM_BENCH_ENTRIES=1000000`), patched
to the local `rocksdb-compat` (this commit) via `[patch]` path. Their
bench binary unchanged. Local Mac was under load average 10–12 during all
local attempts; absolute local numbers were unstable and are not quoted.
Their PR card ran on their Linux box against the **published** compat —
ratios below are from our guest run, card ratios quoted for reference.

| shape | fjall | rocksdb | pedra (this fix) | pedra/rocks | was (their card) |
|---|---|---|---|---|---|
| get_hit (criterion) | 3.93 µs | 4.71 µs | 4.77 µs | 1.01× (tied) | 1.66× slower |
| probe_hit p50 | 7.3 µs | 11.0 µs | **3.3 µs** | **0.30×** | — |
| probe_hit p999 | 36.4 µs | 252.6 µs | **29.0 µs** | **0.11×** | card flagged 832 µs tails |
| probe_miss p50 | 491 ns | 1.1 µs | 1.6 µs | 1.45× | — |
| prefix_scan (1000 keys) | 329 µs | 333 µs | 1.009 ms | 3.03× | ~8× |
| lookup_100 get-loop | — | 429.5 µs | **367.7 µs** | **0.86×** | 1.57× slower |
| lookup_100 multi_get | — | 459.1 µs | 507.4 µs | 1.10× | 1.93× slower |
| hydrate 1M (write) | 0.84 M/s | 1.01 M/s | 0.90 M/s | 0.89× | — |
| settle (flush+compact) | 0.0 s | 0.7 s | **0.4 s** | **0.57×** | card: 4.8 s vs 0.6 s (8×) |
| on disk | 355 B/e | 239 B/e | 256 B/e | 1.07× | — |

(fjall has no lookup_100 benches in their harness.)

**Reads are the win:** point-get p50 is 3.3× faster than rocks and p999
is 8.7× tighter (29 µs vs 253 µs — the tail their card called out);
get-loop over 100 keys is faster than both rocks paths; criterion get_hit
is a tie (their 1.66× deficit is gone). Settle is now faster than rocks
(0.4 s vs 0.7 s).

**Remaining gaps (core-side, out of compat scope):**
- `prefix_scan` 3.03×: per-entry cost is core scan materialization (each
  entry decoded into fresh `Vec`s inside the core page fetch before the
  compat window can move it out). Closing it needs a zero-copy/cursor
  core scan API — a core slice with a full 17-shape gate run.
- `lookup_100 multi_get` 1.10×: compat `multi_get` loops the single-get
  path; rocks batches block reads across keys. Our get-loop (367.7 µs)
  beats our multi (507.4 µs) — a batched multi could aim at ≤ get-loop.
- `probe_hit` max 1.2 ms single outlier (p999 is clean at 29 µs).
