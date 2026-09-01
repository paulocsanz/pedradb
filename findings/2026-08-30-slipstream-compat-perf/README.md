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

## 2026-08-30/31 — the honest-scale campaign (2M → 25M, CHV 4 GiB guest)

Target: PR 19 shapes at 25M entries in the CHV guest (4 vCPU, 3892 MiB
RAM, `linux-gate-p149b`) — the largest scale that fits the box honestly.
Getting there took three memory fixes and one read-path bug hunt, all
measured in-guest at 2M first.

**Harness (guest-only patch, upstream file unchanged in spirit):** each
backend runs in its own process (`SLIPSTREAM_BACKENDS=fjall|rocksdb|pedradb`
selects per-backend blocks; store dirs are block-scoped TempDirs). This
fixes three fairness failures of the single-process layout: disk
accumulation across legs (old ENOSPC at 25M), cross-backend allocator
carry-over (~800 MB RSS floor inherited into the pedra leg), and one
backend's OOM kill ending the remaining legs.

**Core fixes that made 25M survivable (mono `95aab80` / pub `cf6d990`
era):** bounded CHANGELOG rebuild (Fix 3), parked-memtable bound at 1×
write buffer with a dedicated flush worker (Fix 4), chunked compaction
output at 256 MiB target (Fix 5, mono `10c2e0e` / pub `5712e4e`). With
those, the 2M pedra leg completes in-guest: hydrate 3.7–5.4 s, settle
7.0–8.1 s, store 0.66→0.41 GiB, peak RSS ~2.7 GB.

**v9d 2M results (first fully clean guest run, per-backend isolation):**

| shape | fjall | rocksdb | pedra |
|---|---|---|---|
| hydrate | 3.0 s (0.67 M/s) | 2.2 s (0.92 M/s) | 5.4 s (0.37 M/s) |
| settle | 1.0 s → 1.05 GiB | 0.9 s → 0.42 GiB | 8.1 s → 0.41 GiB |
| probe_hit p50 | 7.0 µs | 11.7 µs | 7.8 µs |
| get_hit (criterion) | 5.90 µs | 6.70 µs | **409 µs** |
| prefix_scan | 294 µs | 376.8 µs | 644 µs |
| lookup_100 get-loop | — | 650 µs | 42.6 ms |

The 409 µs get_hit (replicated: 438 µs second run, p=0.26) against a
7.8 µs probe p50 in the same leg was a stable pathology, not noise.

### Anomaly hunt (v9e → v9f → v9g)

- **v9e A/B** (same boot, pedra-only 2M, only env differs): no
  `MALLOC_*` knobs → 424 µs; exact v9b knob set → 427 µs. Knobs
  exonerated for reads (they only taxed the write path: hydrate 3.8 s
  vs 5.0 s). Peak RSS 2.75 GB without knobs vs 2.70 GB with — no
  memory reason to keep them either.
- **v9f hammer** (10×50k uniform gets, per-chunk percentiles, plus
  `/proc/self/io` before/after): chunk 0 fast (p50 7.5 µs); knee
  between 50k and 100k gets; chunks 2–9 steady p50 ~517 µs. After a
  5 s idle, fresh probes pay 523 µs (permanent). `read_bytes`
  **identical** before/after (zero disk reads) — pure CPU. A repeated
  hot key stays 351 ns (TLS path fine).
- **Root cause:** `BlockCache::evict_one` scanned the whole map
  (`min_by_key(tick)`) to pick the LRU victim, and the insert path
  calls it on every insert once the byte budget fills. 256 MiB ÷
  ~5 KiB-per-get ≈ 50k gets to fill — matching the knee; ~65k entries
  × ~8 ns ≈ the 517 µs steady cost. The RFC-0035 comment in the same
  struct records the hit path was already fixed for O(capacity); the
  eviction path kept the scan.
- **Fix (mono `695d7c6` / pub `49648f2`):** lazy-LRU — a `VecDeque`
  recency queue in push order; a hit re-pushes its key with a fresh
  epoch (slot epoch bumped, older queue entries become ghosts);
  eviction pops from the front skipping ghosts (the `AnswerCache` F178
  pattern). Hit and evict both O(1) amortized; re-push bounded at
  4×live+64 so hit-heavy caches cannot grow the queue unboundedly.
  Cache tests 28/28 both trees; core lib 622/3 (same three known
  pre-existing flakes); compat 84/0.
- **v9g verification (same guest, same hammer instrument):** all 10
  chunks flat p50 7.0–9.4 µs (no knee); probe-after 9.0 µs; criterion
  `get_hit` **14.7 µs** (was 420.6 µs; −96.6%, p=0.00, measured after
  500k hammer gets — honest post-fill steady state); `lookup_100`
  **1.7–1.8 ms/100** (was 42 ms). `BENCH_EXIT=0`.

### 25M harvest (v11)

Per-backend isolation (`SLIPSTREAM_BACKENDS`), pristine bench source,
patched cache, no `MALLOC_*` env, 5400 s/leg cap.

- **fjall (exit 0):** hydrate 39.8 s (0.63 M/s) → 5.46 GiB (235 B/e);
  settle 27.4 s → 9.92 GiB; probe_hit p50 67.3 µs / p99 226.9 µs /
  p999 3.1 ms; probe_miss p50 592 ns; `get_hit` 35.97 µs;
  `prefix_scan` 305.4 µs.
- **rocksdb default peer (exit 0, `WriteOptions.sync=false`):**
  hydrate 25.3 s (0.99 M/s) → 7.78 GiB (334 B/e); settle 8.3 s →
  5.24 GiB; probe_hit p50 45.4 µs / p99 83.9 µs / p999 1.1 ms;
  probe_miss p50 521 ns; `get_hit` 38.59 µs; `prefix_scan` 305.6 µs;
  `lookup_100` 3.38/3.70 ms per 100.
- **pedra: OOM-killed during hydrate** (cargo exit 101 wrapping
  SIGKILL). Guest RSS 486 MB → 1.80 GB → 2.91 GB → 3.30 GB within ~8 s
  of hydrate start (~4 M entries in) on the 3892 MB guest; no kernel
  OOM lines on serial, kill signature from cargo's `signal: 9` and the
  RSS trajectory. Disk fine (rocks peaked 7.78 GiB on a 40 G /data).

### 25M pedra OOM diagnosis (v12 → v15)

`PEDRA_FLUSH_DIAG=1` (env-gated, committed) makes the compat flush
worker print one layer breakdown per second:
`parked_n/parked_b/active_b/imm/retired_b/sst_n/rss_kb`.

- **v12 (no knobs):** every tracked layer *bounded* while RSS died —
  parked ≤ 1×256 MiB table (oscillating with materialize), active
  ≤ 204 MB (CF cap 256 MiB), retired capped ~202–269 MB, `imm` never
  set. Real:approx drift grew 2.2× → 3.6× → 5.2× over the run.
- **v13 (`MALLOC_ARENA_MAX=2` + `MALLOC_TRIM_THRESHOLD_=64MiB`):**
  same SIGKILL at RSS 3.49 GB — knobs are *not* the fix; the growth is
  live footprint plus table churn, not arena hoarding alone.
- Root causes found in code after the series pointed there:
  1. the compact worker folded parked pairs whenever
     `writes_active() <= 1` — **true during single-writer bulk
     ingest**, exactly when materialization lags and two 256 MiB
     parked tables pile up; the fold deep-clones both (~3 tables live,
     >1.5 GiB real transient) at the worst moment;
  2. `materialize` retired every parked table into the point/MVCC
     read cache — one ~256 MiB BTree (~2× real) per installed L0,
     pure waste while ingest serves zero reads;
  3. `approx_memory_usage` counts payload only (`key + value + 8`) —
     real BTree/`Bytes`/malloc overhead is ~2×+, so every
     approx-denominated bound underestimates real footprint.
- **Fix (mono `c992c4c` / pub `69bdfaf`):** fold gated to the 200 ms
  write-idle window; retire gate drops materialized tables unless
  reads arrived since the last decision (`reads_served` bumped on
  `get`/`get_at`/`scan_collect*`). Policy tests:
  drop-without-reads, retire-after-reads, bounded-retire backstop;
  compat 84/0/3 ignored + 11 integration green.
- **v14 (fix in, no knobs):** survived ~2× longer than v12/v13; RSS
  fell 2.9 → 2.3 GB when the first materialized table was *dropped*
  instead of retired (the fix working), then regrew to 3.22 GB and
  died. The flush worker's diag went silent after t+7 s right after
  parking a full table — materialization is not keeping up with the
  ~185 MB/s ingest (parked debt grows ~1 table/2 s until the box
  dies).
- **v15 (worker heartbeats):** `tick_s` = last flush-tick seconds,
  `mat_n` = materialize count, `COMPACTDIAG` = compact-worker level
  counts. Series (t≈0…10 s): t2 `mat_n=1` parked 68 MB; t5 parked
  **268.6 MB (one full table)**, `sst_n=5`, `tick_s=0 mat_n=2`,
  retired flat 67.8 MB (the c992c4c retire gate holding); t4
  `MEMDIAGD rss=1.39 GB dirty_kb=0` — disk idle with a full table
  parked; t7 RSS **3.42 GB (+1.68 GB in 1 s)**; t10 3.57 GB → kill.
  Verdict: ticks are *fast* (`tick_s=0` always) — the flush thread is
  not doing slow I/O, it stops printing because the whole tick is
  stuck inside one materialize sharing the disk with a compaction.
  `COMPACTDIAG` printed exactly once (t0, `l0=0`): the compact worker
  entered its L0→L1 merge at trigger=4 and never returned to the poll
  loop. `prepare_l0_compact` takes **all** L0 files of the family —
  at the 256 MiB bench buffer that is ≥1 GiB of inputs per job, a
  multi-second merge that owns the disk while ingest keeps parking
  256 MiB tables (~512 MiB real each). And nothing anywhere bounds
  `parked_unflushed`: the concurrent apply path has **no admission
  control at all** (`ensure_write_admitted` is single-writer-Db
  only; the compat open path never enables the L0/mem stall knobs).
- **v16 (fix in):** three scheduling cuts, all worker/apply policy —
  per-CF `write_buffer_size` untouched:
  1. **flush-debt writer throttle** — `WriteGroup::await_flush_debt`
     (core `concurrent.rs`): when the compat flush worker is attached
     and parked bytes ≥ one table's worth (`Db::flush_debt_cap` =
     max of auto-flush and per-CF buffer), a submit sleeps (2 ms poll,
     30 s hard cap, `PEDRA_FLUSH_DEBT_MAX_MS` override) instead of
     parking another table. Rocks-shaped: every LSM blocks writers on
     flush debt. Attached-only so core tests without a flusher never
     wait.
  2. **bounded merge inputs** — `CompactOptions::max_input_files`
     (core `db.rs`): `prepare_l0_compact` truncates to the oldest N;
     the compat worker passes `Some(2)` (`COMPACT_MAX_L0_INPUTS`) so
     one L0→L1 job holds ≤2×256 MiB inputs and the trigger loop
     drains L0 in bounded slices.
  3. **fold stays off debt** — the parked fold also requires
     `parked_unflushed_bytes < flush_debt_cap`: a debt-throttled
     writer *looks* idle to `writes_idle_for(200 ms)` and would
     otherwise re-trigger the c992c4c fold transient.
  Tests: `prepare_l0_compact_respects_max_input_files`,
  `submit_flush_debt_waits_only_with_worker_attached`,
  `submit_flush_debt_releases_on_materialize`; park/materialize/fold/
  worker subset 21/21; compat lib 84/0/3 ignored + 11 integration.
  Commit mono `ff133eb` / pub `008cf49`.
- **v16 guest result: still OOM, but the shape changed (proves the cuts
  work).** SIGKILL at RSS **2.74 GB** (v15: 3.57) after ~10 s. The debt
  throttle holds: parked oscillates full→**0** each materialize (v15
  froze at one full table), `retired_b` stays 0, `mat_n` climbs (2→4),
  and `COMPACTDIAG` keeps printing — the compact worker now returns to
  its poll loop (`l0=0 l1=4` after the first bounded merge) instead of
  vanishing for the whole run. First merge cycle peaked 2.42 GB then
  **freed 630 MB** (1.79 GB trough). The remaining growth: each
  park/materialize cycle re-peaks ~250–400 MB higher and never returns
  (2.62 → 2.74 at kill with parked=1 full, active=57 KB, retired=0 —
  no merge in flight): allocator fragmentation from dropped BTree
  tables, plus an unknown container ceiling (killed at 2.74 GB RSS
  with ~1.3 GB of clean page cache outstanding ⇒ suspect a cgroup
  limit below the 3892 MB VM). Bench exonerated as the missing memory:
  it regenerates keys from RNG state and reuses one 1 MiB value pool.
- **v17 (dead end, reverted):** `malloc_trim(0)` in the flush worker after
  each materialize drop and in `compat_compact_once` after each install +
  budget forensics in the sampler. **Compile-refused in the guest**:
  `pedradb-core` is `#![forbid(unsafe_code)]` and `malloc_trim` needs an
  `unsafe` FFI call — invisible locally because the trim call sat behind a
  linux-only cfg and macOS never compiled it. Fully reverted (guest md5s
  restored). What the leg still bought: the forensics answer. **There is
  no cgroup** (`CG_LIMIT` empty the whole run; no `memory.max`); the
  ceiling is the VM itself (MemTotal 3985596 kB ≈ 3.87 GiB, at-rest
  MemAvailable 3639 MB). So v16's kill at 2.74 GB RSS was not a hidden
  container cap.
- **Root cause (re-diagnosis after v16/v17):** every `SstTable` keeps its
  whole CRC-stripped file body resident (`payload: Arc<[u8]>`) for lazy
  block decode. At 25M entries that is **~5.9 GB of payloads alone** on a
  3.87 GiB box — v16's re-peaking cycles and the 2.74 GB kill were the
  allocator's view of a structurally unbounded data set, not scheduling.
  v16 arithmetic with the real number: parked ≤ 3×256 MiB + active
  256 MiB + merge I/O is bounded fine; payloads are not. RocksDB does
  not do this — it reads 4 KiB blocks on demand through its block cache.
- **v18 (fix in): bounded resident-payload pool.** `SstTable.payload` is
  now an evictable shared slot (`Arc<RwLock<Arc<[u8]>>>`, empty =
  evicted) and evicted blocks are re-read from file, fail-closed:
  v5 files (compact output, per-block CRC32C) read exactly one block
  range + verify its CRC; ≤v4 files (evicted L0, no per-block CRC)
  reload the whole body and re-verify the **file-level** CRC before any
  decode — bitrot refuses, never decodes garbage. A free-standing table
  with no file source errors loudly. `SstPayloadPool` (budget, FIFO by
  install order, Weak self-cleaning) is armed **before recovery** at
  open (`Db::open_with_env_bounded`, every compat entry point the bench
  uses; adversarial Rc-env opens stay unbounded by design), and each of
  the four install choke points adopts the new table. The compat cache
  knob maps to the pool budget (`SLIPSTREAM_BENCH_CACHE_BYTES` →
  payload pool; decoded-block cache capped at 32 MiB), so total
  cache-ish memory tracks the knob like Rocks' block cache. Fairness
  unchanged: per-CF `write_buffer_size` 256 MiB for every backend.
  Steady-state RSS estimate ~1.6–1.9 GB (pool 256 MiB + parked
  ≤3×256 MiB + active 256 MiB + small caches) vs the 3.87 GiB ceiling.
  Tests: identical reads after budget-0 eviction (per-block equality +
  streaming iter), fail-closed no-source, v3 whole-body reload +
  bitrot refusal, pool FIFO-to-budget, bounded reopen serves reads
  within budget. Core 632/635 (3 known flakes), compat green.

- **v19 (`beb3709`): L0 flushes write compressed SST v5 (lz4 +
  per-block CRC32C).** The v18 25M SIGKILL root cause was uncompressed
  v3 L0 bodies; compressed bodies keep both the writer's in-RAM file
  body and the pooled payload several times smaller, and streaming
  merges read one block range instead of reloading whole ≤v4 bodies
  per input. Compressed flushes are ~2.5× faster, so flush debt clears
  and the compat compact worker (write-idle gated, `l0 ≥
  L0_COMPACTION_TRIGGER`, `COMPACT_MAX_L0_INPUTS=2`) starts racing
  hydrate (COMPACTDIAG l0↔l1 churn mid-hydrate). Two test regressions
  fixed in-commit: `compact_horizon_reclaims_aged_versions` (constant
  fill let lz4 crush every round — the byte-ratio measured
  compression, not reclamation; payloads now LCG-filled) and
  `remote_sidecar_prunes_segment_fetch` (compressed L0s sit just under
  the dead-weight doubling threshold → different remote segment set →
  hole key proven-absent instead of coverage-gap). **Guest runs #1/#2
  both still OOMed** — compression alone does not close the gap.
- **v20 diag (`a1335a8`) + guest run #3 (tunables on) — the decisive
  attribution.** FLUSHDIAG/COMPACTDIAG now print `pool_n= pool_b=
  ent_e=` (`SstPayloadPool::tracked_tables()/.resident_bytes()`, new
  `Db::sst_cached_entries()`; `SstTable::cached_entries_count()`).
  Run #3: `COMPACTDIAG l0=2 l1=6 parked_n=1 pool_n=2 pool_b=256688310
  ent_e=1154308`; FLUSHDIAG at mat_n=6 sst_n=9: `parked_b=268609536
  active_b=143370 retired_b=0 rss_kb=2961916 pool_b=256688032
  ent_e=1154308`; later `sst_n=8 rss_kb=2413960 pool_b=242153747
  ent_e=0`; then OOM `BENCH_EXIT_pedradb_diag=101`. Conclusions:
  (1) the v18 payload pool is bounded exactly at its 256 MiB budget —
  no registration gap; (2) `ent_e=1,154,308` unbounded per-table
  decoded-entries caches (≈130–250 MB) are a real layer; (3) 550 MB
  DID return when the caches freed (mmap'd ≥16 KB allocs respect the
  tunables) — the residual ~2.4 GB floor is arena-pinned sub-16 KB
  churn (per-entry InternalKey/Bytes allocations, memtable BTree
  nodes, decoded-block Vecs ~13 KB, just under the 16 KB mmap
  threshold) interleaved with long-lived allocations.
- **v21 (`8890425`): compaction GC streams — the ent_e layer is gone.**
  A PEDRA_MAT_TRACE backtrace named the hydrate-time `materialize_entries`
  caller: the compat compact worker (`spawn_compact_worker →
  compat_compact_once → PreparedL0Compact::write → write_merged_tables
  → entries_cloned → materialize_entries`). With `auto_reclaim`,
  `compat_compact_once` passes `CompactGcOptions::for_oldest_snapshot`,
  the old GC branch cloned **every input table** into the persistent
  per-table entries caches, then `gc_compact_entries` built a full
  BTreeMap copy (~3× input bytes per L0→L1 job) — during write-only
  hydrate. `GcMergeSource` now wraps the k-way internal merge and
  applies the same per-run decisions (oldest-snapshot GC with
  `point_version_fate`, keep-only-latest with range-deletion coverage
  and the F177 bottommost tombstone drop, lone-tombstone collapse,
  `min_sequence` filter) while buffering one user-key run at a time;
  `write_merged_tables` drives every GC request through it, so no
  input table is ever materialized. Property test: streaming equals
  the old `gc_compact_entries` across an 8-variant option matrix on
  interleaved streams. 6M local leg: `ent_e` 1,087,488 → **0 on every
  tick**, same flush cadence and disk footprint (2.46 GiB), hydrate
  11.0 s → 10.5 s.
- **v21b (`9077d4a`): SST entry-count floor only bounds uncompressed
  bodies (compat hang fix).** v19 exposed a latent bug:
  `check_sst_entry_count` rejected any header count above
  `file_len/8+1` — an uncompressed-file heuristic compressed files
  legitimately violate (1001 hot-key versions in 7,756 bytes). The
  freshly written `.sst.tmp` failed its own write-verify decode,
  `write_imm_l0_files` errored, and `materialize_parked_once`'s
  `Err(_) => return false` swallowed it — the flush worker retried the
  SAME parked memtable forever (observed 14,637 retries, zero
  completions), so `auto_reclaim_worker_gcs_versions` livelocked at
  the 30 s flush-debt ceiling per put. The byte floor now applies only
  to uncompressed bodies (v1's eager-allocate guard kept); every
  header still faces `MAX_SST_ENTRIES`, and compressed counts are
  verified against the decoded block stream. Regression test writes a
  repetitive two-hot-key flush that packs below the old floor (fails
  the old check by construction) and reopens whole. Suites post-fix:
  compat lib **84/0**/3 ignored (previously hung at 82 on this test),
  core 636/3 known flakes. The 25M bench never tripped it —
  pseudo-random payloads compress weakly — so this is a correctness
  repair, not a bench mover.
- **Guest run #4 (v21+v21b, entrypoint unchanged from run #3 — tunables
  present, so the pedradb tree is the single variable).** First start
  was a **false start**: the injection script died at `rm /tmp/p149.raw`
  (EPERM — loop still referenced) BEFORE its `chown`/`mv` installed the
  new image, so that boot silently ran the v20-diag tree again — an
  accidental control run: same OOM point (mat_n=6, RSS 3.16 GB) with
  `ent_e` 1,087,488 vs run #3's 1,154,308 (same code, race-timing
  variance), no rebuild. The v21 image (built but stranded in
  `/tmp/p149-new.qcow2`) was then md5-verified standalone (merge.rs
  `5cc85f3e…`, db.rs `90b0a43e…`, table.rs `dcfb8d85…`, compat lib.rs
  still `d9f472bd…`), installed, and the real run #4 booted.
- **Guest run #4 result — the memory war is won; a v5 eviction-read
  integrity bug is the new blocker.** Real run (rebuild confirmed:
  `Compiling pedradb-core … Finished in 20.75s`): **hydrate completed
  all 25,000,000 entries in 104.5 s (0.24M entries/s), 11.05 GiB on
  disk (475 B/entry), RSS flat ~1.8 GB, `ent_e=0` on EVERY
  FLUSHDIAG/COMPACTDIAG tick** — no OOM, vs run #3's kill at ~3.3M
  entries / RSS 3.1 GB. Pool stayed at budget (pool_n=9, pool_b≈243
  MB). Two follow-ups it exposed: (1) the compact worker is write-idle
  gated, so sustained hydrate left **l0=87 undrained** (l1=8 pinned;
  settle's own compact pass moved almost nothing: 86 ms); (2) **settle
  failed closed**: `SST block CRC mismatch in
  /data/stores/.tmpnjeecn/store/000018.sst` — an early v5 SST read
  back long after its payload was evicted from the 256 MiB pool (only
  9 of 93 tables resident). The v18 eviction tests covered ≤v4
  whole-body reload; **the v5 evicted-block single-range re-read +
  per-block CRC path has no test** and is the prime suspect
  (write-verify passed at flush time, so the bytes were valid once;
  either the re-read computes the CRC over the wrong slice or the
  block range is off). Fail-closed behavior is correct (refused to
  decode); the read path is wrong.
- **v21c (`18669e4`): the run #4 CRC panic was a file-number TOCTOU,
  not a v5 eviction-read bug.** Re-reading the code paths killed the
  eviction-read theory: `prepare_l0_compact` reserved exactly **one**
  file number (`alloc_file_num()`), but the v21 chunked writer emits
  `file_num, file_num+1, …` — numbers past the reservation. The
  off-lock `write()` runs between `prepare` and `install`; a
  concurrent L0 flush (allocating under the same Db write lock) can be
  handed the same numbers, and its tmp→rename clobbers a chunk path
  (or vice versa). The surviving file no longer matches the other
  table's in-memory index, so v5 per-block re-reads at stale offsets
  fail the block CRC — exactly the observed error, on `000018.sst`,
  an early file from the compaction-churn-against-flush window of
  run #4 (93 tables, l0=87). The local 6M leg never hit it because
  settle has no concurrent writer. **Fix:** splitting is advisory
  sizing, so `prepare` now reserves `chunk_budget = Σ input on-disk
  bytes / compact_target + 2` numbers and the writer is **capped**
  to that many chunks — past the budget the last chunk runs long
  (correct, less even). Collision is structurally impossible; no
  compat-visible signature changed (the budget rides inside
  `PreparedL0Compact`). `rewrite_ssts` keeps unlimited chunks: it
  runs under the `&mut self` write lock and advances `next_file_num`
  only after the write, so nothing can interleave. A static writer
  bound was ruled out first — the writer splits on *uncompressed*
  logical entry bytes while inputs shrink ~35× under lz4, so no
  compressed-size formula bounds the chunk count (a reservation-only
  attempt failed its own regression test 3 numbers short before the
  cap existed). Regression test
  `prepare_l0_compact_reserves_whole_chunk_range`: a compressible
  multi-flush fixture must produce ≥2 chunks whose file numbers all
  sit inside the reservation and the next `alloc_file_num()` must
  exceed them. Suites: compat 84/0/3 ignored, core 637/3 known
  flakes. Forensic confirmation from the run #4 data disk was
  attempted (stop → convert 42 G data.qcow2 → mount) but abandoned:
  the convert filled the host disk (100%), wedged a `losetup` in
  D-state, and was not worth the box — the root cause stands on the
  allocation-path reading above.
- **Guest run #5 (v21c) — hydrate won at last, settle hit a NEW
  OOM.** Rebuild confirmed (`Compiling pedradb-core`, 20.84s). Leg
  `=== start 15:18:32 UTC ===`: **hydrate 25,000,000 entries in
  105.3 s (0.24 M/s), 11.09 GiB (476 B/e), RSS flat 1.8–2.3 GB,
  `ent_e=0` on every tick** — and, unlike run #4, **l0 stayed
  drained** (`COMPACTDIAG l0=0 l1=32→48`): with install no longer
  corrupted by collisions the compat worker really keeps up. Then
  `SETTLE_PHASE flush_ms=3654` (1.68 GB after flush) and the compact
  phase climbed RSS 1.9 → 3.06 GB in ~15 s → SIGKILL (`signal: 9`),
  no settle line, no panic. The v21c fix held (no CRC mismatch);
  settle itself now exhausts memory.
- **v21d (`915ef89`): rewrite chunks were invisible to the payload
  pool.** Settle's `compact()` runs `compact_with_ssts_only →
  compact_levels(0→1) → rewrite_ssts` — a whole-levels merge of all
  48 L1 files (~11 GB) through `write_merged_tables`. Every finished
  chunk is opened by `SstTable::open_on` with its **whole file body
  resident**, and the writer accumulates all chunks of the job in its
  out-Vec before returning. `adopt_sst` existed precisely for
  "every point a table enters `self.ssts`" but was only called at
  the four install choke points — never for `rewrite_ssts` output —
  so those payloads were unregistered: the pool could never evict
  them, ~230 MB per chunk, kernel kill at ~6 chunks (+1.4 GB — the
  observed climb). Hydrate never saw it because the compat worker
  compacts through the prepare/install path (adopted; `pool_n=2`
  all run long). Fix: `write_merged_tables` takes the payload kit
  and registers each chunk the moment it is opened, so the pool's
  FIFO evicts older chunks to budget *while the job is still
  writing*; both callers build the kit (idempotent with the
  install-time adopt; legacy unbounded opens unchanged). Regression
  test `compact_rewrite_chunks_are_pool_evictable` (bounded open at
  budget 1 → whole-levels compact must leave zero resident payloads,
  identical reads through the evicted re-read path), negative
  verified: fails with the registration disabled. Suites: core
  638/3 known flakes, compat 84/0/3 ignored.
- **Guest run #6 (v21c+v21d) — settle rewrite now progresses but
  still peaks out.** Hydrate passed again (103.9 s, l0 drained,
  `ent_e=0`), flush passed (3.38 s, 1.64 GB after). The whole-levels
  rewrite then ran **~90 s** (v21d visibly working: RSS oscillating
  as chunks registered and evicted, dirty spikes from real writes)
  before a transient peak (~3.1–3.3 GB on a ~2.2 GB floor) hit the
  ceiling and the kernel killed it. Attribution: at the 256 MiB
  logical chunk target the per-chunk transient — chunk body `Vec`
  (with doubling), the 25M-entry-capacity bloom (re-allocated every
  chunk), and the `open_on` whole-file read-back — is ~0.9–1.3 GB,
  layered on the hydrate allocator residue plus the 256 MiB pool.
- **v21e (`77a72e7`): whole-levels rewrites split at 64 MiB logical
  (RocksDB's own L1 target-file-size shape at this scale).**
  `rewrite_ssts` now uses `min(compact_target_file_bytes,
  rewrite_chunk_target_bytes)` — the per-chunk transient drops to a
  few hundred MB; the hydrate-time compact worker path (2-input
  merges, already keeping l0 drained within budget) is untouched.
  The cap is a crate-private field defaulting to the const so the
  mechanism is testable without a 64 MiB fixture:
  `rewrite_caps_chunk_size_for_whole_level_merges` sets the compact
  target to `u64::MAX/2` and the cap to 8 KiB — a whole-levels
  compact must still split (a revert to the raw compact target emits
  exactly one file). Suites: core 639/3 known flakes, compat 84/0/3
  ignored, fmt clean.
- **Guest run #7 (v21e) — transient peaks gone; settle now dies of a
  monotonic floor.** Hydrate passed identically (~104 s, l0 drained,
  `ent_e=0`), flush passed (after_flush 1.68 GB). The compact phase
  then climbed **monotonically 2.73 → 3.36 GB through the rewrite**
  — no oscillation, dirty_kb tiny, page cache squeezed to ~160 MB —
  until the cgroup ceiling (3892 MiB) killed it. v21e removed the
  big per-chunk transients; what remains is slow accumulation.
- **Local 6M repro (macOS, `PEDRA_REWRITE_DIAG=1`): the rewrite loop
  retains nothing — the guest climb is allocator amplification.**
  One `REWRITEDIAG` line per finished chunk (file number, job chunk
  count, pool resident bytes, process RSS, elapsed). The settle
  whole-levels job (24 chunks, ~64 MiB logical each, 17.2 s): RSS
  ramps only while the pool fills to its budget (4.44 → 4.90 GB over
  chunks 1–5, `pool_b` → 254 MB) and is then **flat 4.85–4.96 GB
  through chunk 24** while the pool FIFO-evicts each new chunk
  against budget. Per-chunk retained state (sparse index + first-key
  `Bytes` + per-chunk bloom) is ~1 MB — invisible in the flat
  trajectory. On glibc the same freed small allocations (25M
  entries' decode churn interleaved with the retained per-chunk
  index keys) pin arena pages instead of releasing them, which is
  the monotonic +630 MB seen in the guest. Local run also survived
  end-to-end (exit 0): settle/pedradb 21.5 s at 6M.
- **Per-chunk `malloc_trim(0)` via `pedradb-posix`** (core is
  `#![forbid(unsafe_code)]`, so the glibc FFI lives in the unsafe
  island with the other allocator/syscall shims; advisory rc, no-op
  off Linux): releases free arena pages after every rewrite chunk,
  directly countering the pinning. Run #8 prints post-trim RSS per
  chunk — flat means trim holds it; still-climbing means real
  retention at 25M scale that the 6M repro cannot see.
- **Guest run #8 (v21e + trim) — trim is not the killer's antidote;
  the diag env was missing.** Build passed (posix FFI island
  accepted), hydrate passed (103.0 s, l0 drained, `ent_e=0`), flush
  passed — at a visibly **lower floor** (after_flush 1.30 GB vs
  1.68 GB in run #7: the hydrate-time per-chunk trims pay off). The
  compact phase then climbed monotonically again (RSS → 3.50 GB,
  avail 78 MB) → SIGKILL. Zero `REWRITEDIAG` lines in the serial:
  `PEDRA_REWRITE_DIAG=1` was never added to the guest entrypoint —
  the instrumentation ran blind. Fixed for run #9 (the injection now
  exports it in the entrypoint).
- **Root cause found: every rewrite chunk carried a whole-job-sized
  bloom.** `write_sst_try_sorted_body` allocated
  `BloomFilter::with_capacity(bloom_hint)` UP FRONT, and
  whole-levels rewrites pass `bloom_hint = Σ all input entry counts`
  — 25M keys. Every 64 MiB chunk file then serialized a ~31 MB
  near-zero bloom (10 bits × 25M keys), and `SstTable::open_on`
  rebuilds that bloom **in RAM per opened table as a plain field —
  never payload-evictable**. ~170 chunks × 31 MB ≈ 5 GB of
  retained blooms: exactly the monotonic settle climb that killed
  runs #5–#8. The 6M local repro was flat because 24 × 7.5 MB =
  180 MB hides in noise — found only by matching the guest's
  +2.2 GB against the per-chunk arithmetic and checking the writer.
  Same bug bloats the payload pool's accounting (each 64 MiB chunk
  body counted ~95 MB) and the read path (a `may_contain` probe
  scattered k=7 random reads across 7.5–31 MB of bits).
- **Fix (`write_sst_try_sorted_body`): build the bloom AFTER the
  entry loop from the distinct user keys actually written**
  (transient `Vec<Bytes>`, ~5% of chunk); `bloom_hint` now only
  gates whether the file gets a filter. Regression test
  `write_sst_bloom_is_sized_by_written_keys_not_hint` (100 entries
  with a 100M-key hint must stay a <1 MB file; bloom active; point
  read works). Local 6M post-fix: settle 21.5 → 12.2 s, settle
  output 1.40 → 1.24 GiB (−0.16 GiB = exactly the 24 × 7.5 MB
  predicted bloat), hydrate 13.8 → 10.2 s, REWRITEDIAG trajectory
  flat, and **get_hit 84.2 → 15.1 µs (5.6×)** — the read-path gap
  to RocksDB (6.1 µs local) collapsed from 14× to 2.5× as a side
  effect. Suites: core 640/3 known flakes, compat 84/0/3 (one
  timing-dependent `compact_range_cf_lock_leaves_default` flake
  seen once in 9 runs — 0/8 at baseline — consistent with a latent
  background-auto-compact race; output for its tiny files is
  byte-identical pre/post fix, so no semantic delta).
- **Read-path profile (macOS `sample`, 5 s during get_hit/pedradb at
  6M, post-bloom-fix): 41% of all main-thread samples are
  `std::fs::File::open`.** The chain
  `Db::get → lookup → point_at_with → BlockCache::get_or_insert_with
  → decode_block → SstFileSource::read_range → IoUringEnv::open_read
  → File::open` (1625/3990 samples) shows every 4 KiB block read
  from an evicted table **opens the file before reading it** — a
  full path-lookup + vnode syscall per cold block. RocksDB holds
  open file handles in a file-cache; that is the remaining ~2.5×
  get_hit gap (15.1 µs vs 6.1 µs local) after the bloom fix removed
  the 14× bloat. Next lever: an fd/handle cache in `SstFileSource`
  (LRU-bounded, keyed by path) so `read_range` reuses handles.
- **Guest run #9 (v21f, 25M) — first full completion.** The memory
  war is won: hydrate passed (103.1 s, 11.06 GiB on disk after),
  flush passed, and **settle survived to the end** — `SETTLE_PHASE
  compact_ms=76240`, `SETTLE_RSS after_compact=1230324 kB` (1.23 GB
  under the 3.8 GB ceiling; run #8 died at 3.50 GB), settle wall
  79.9 s, 5.15 GiB on disk after. `BENCH_EXIT_pedradb_diag=0`.
  Read legs vs the RocksDB-default 25M reference:

  | leg (25M)              | pedradb v21f | rocks default | ratio |
  |------------------------|--------------|---------------|-------|
  | probe_hit p50          | 39.5 µs      | 45.4 µs       | **1.15× faster** |
  | probe_miss p50         | 2.8 µs       | —             | — |
  | get_hit (criterion)    | 66.8 µs      | 38.59 µs      | 0.58× (1.73× slower) |
  | prefix_scan            | 730 µs       | 305.6 µs      | 0.42× (2.39× slower) |
  | lookup_100 get_loop    | 6.585 ms     | 3.38 ms       | ~0.51× |
  | lookup_100 multi_get   | 6.655 ms     | 3.70 ms       | ~0.56× |
  | hydrate                | 103.1 s      | 25.3 s        | 0.25× (4.1× slower) |
  | settle                 | 79.9 s       | 8.3 s         | 0.10× (9.6× slower) |
  | on disk after settle   | 5.15 GiB     | 5.24 GiB      | smaller |

  Honest read: **probe_hit (the raw point-read path) is now faster
  than RocksDB default at 25M**; the criterion legs still trail
  (get_hit 1.73×, prefix_scan 2.39×, lookup_100 ~1.9×) and hydrate/
  settle are far behind — settle is intrinsic to our stacked
  overlapping L1 runs + whole-levels rewrite, reported as-is. On-disk
  size is now on par (5.15 vs 5.24 GiB). The remaining read gap
  matches the `File::open` profile above: per-block open on evicted
  tables.
- **Run #9 forensics gap (low priority): 0 `REWRITEDIAG` lines in
  the serial** even though `PEDRA_REWRITE_DIAG=1` was verified
  exported in the guest entrypoint (line 14) before convert-back,
  and the settle summary lines (`SETTLE_PHASE`, `SETTLE_RSS`)
  printed fine. Suspect stderr capture in the bench harness
  (`eprintln!` from the compact thread). Only forensic value now
  that settle survives; not chased further.
- **v21g (`6d49a4a`): LRU file-handle cache for evicted-SST block
  reads — the `File::open` tax is gone.** Bounded opens now build
  `CachedEnvSource` + `FileHandleCache` (default 256 handles,
  `PEDRA_SST_FILE_CACHE` override, `0` disables) instead of
  `EnvSource`: first miss opens through `Env::open_read` (fault
  seam intact) and caches the handle; hits do one pread-class
  positioned read (`EnvFile::positioned_read_exact`, new trait
  method — `FileExt::read_exact_at` for std `File` and
  `IoUringFile`, portable seek+read default for in-memory test
  envs). Deletion routes through `Db::remove_db_file`
  (remove + invalidate): an open fd pins an unlinked inode's
  disk space, and a failed SST write rolls `next_file_num` back
  so a path can be re-allocated with different bytes — both make
  invalidation part of the delete, not an optimization
  (regression test: invalidate → new file at same path → new
  bytes). `open_with_env_bounded` gains `E::File: Send + 'static`
  (handles now live in the shared source; every real env's file
  type already satisfies it).
- **Local 6M A/B, same binary, `PEDRA_SST_FILE_CACHE` 0 vs default
  (256):**

  | leg (6M local)      | cache off | cache on | speedup |
  |---------------------|-----------|----------|---------|
  | probe_hit p50       | 13.1 µs   | 5.2 µs   | **2.52×** |
  | get_hit             | 15.06 µs  | 7.24 µs  | **2.08×** |
  | prefix_scan         | 234.1 µs  | 225.5 µs | 1.04× |
  | lookup_100 get_loop | 1.541 ms  | 721 µs   | **2.14×** |
  | lookup_100 multi_get| 1.618 ms  | 734 µs   | **2.20×** |

  Cache-off reproduces the post-bloom baseline exactly (15.06 vs
  15.1 µs) — clean attribution. Against rocks 6M in the same
  process: get_hit 7.24 vs 5.57 µs (1.30× behind, was 2.5×),
  lookup_100 721 vs 585 µs (1.23×), prefix_scan 225.5 vs 216.5 µs
  (1.04×), probe_hit p50 **5.2 vs 6.2 µs (faster)**. prefix_scan
  barely moves: block-opens are not its bottleneck at 6M. Suites:
  core 644/3 (same 3 known flakes at HEAD, stash-verified), compat
  84/0/3, io-uring 22/22. Next: guest run #10 at 25M.

## v21h — encoded-block point seek + resolved block cache (2026-08-31, `8524353`)

User ask: "make it just as fast at least" — read legs ≥ 1.0× vs
RocksDB default. Two changes, both attributed by profile first:

- **get_hit profile with fd cache ON** (`gethit4.sample`, 4105
  samples in `get_entry`): ~51% of a get was decoded-block-cache
  machinery — the 8192-entry `BlockCache` thrashes at 6M random
  keys (insert + `evict_one` on nearly every get, ~5% hit rate),
  plus `InternalKey::decode`/`Bytes::copy_from_slice` per entry of
  every probed block.
- **Fix (point):** `SstTable::point_at_seeking` — same
  bounds/bloom gate and candidate window as `point_at_with`, then
  per block: CRC32C-verify the raw image (fail-closed), lz4 into a
  caller-owned scratch (`lz4_flex::block::uncompressed_size` +
  `decompress_into`, zero steady-state allocs), walk
  `ikey_len|ikey|val_len|val` comparing raw user-key prefixes
  (no `InternalKey` allocs), copy out only the winning value.
  `Db::lookup` swaps to it; the decoded-block cache stays for
  scans. ≤v4 files (no per-block CRC) keep the whole-body path.
  **Integrity:** the old closure's `decode_block().unwrap_or_default()`
  served a CRC-broken block as a **silent miss**; seek errors now
  `fail_stop_corrupt_block` (F1 sibling of `fail_stop_corrupt_value`).
- **prefix_scan profile** (`v21h-prefix-scan.sample`, macOS
  `sample`, pedra leg): per block load — `path_id` Sip-hashed the
  path **string per fetch** (~4%), a full `Vec<(InternalKey,Bytes)>`
  **deep clone per load** (~4%) to resolve vlog pointers in place,
  then per-load vlog re-resolve (~5%). Criterion re-scans the same
  prefix, so every load redid all three.
- **Fix (scan):** hash the path id once per stream
  (`BlockCache::get_or_insert_with_id`), and cache **value-resolved**
  blocks under a tagged id (`RESOLVED_BLOCK_TAG`) so a full scan
  resolves each block once on miss and later loads are pure `Arc`
  clones. Resolving is **not idempotent** (`INLINE_ESCAPE` byte is
  stripped, F188), so resolved slots must never flow back through a
  resolve — the tag keeps raw (key-only scans) and resolved forms in
  separate slots. Scan loader fails-stop on decode errors (the old
  `unwrap_or_default` silently **skipped keys** of a faulted block).

Local 6M, one process, rocks = default `sync=false`
(`v21h-local6m-ab.log`):

| leg (6M local)        | pedra    | rocks    | ratio |
|-----------------------|----------|----------|-------|
| get_hit               | 4.20 µs  | 6.21 µs  | **1.48×** |
| prefix_scan           | 196.7 µs | 217.3 µs | **1.11×** |
| lookup_100 get_loop   | 444.5 µs | 604.1 µs | **1.36×** |
| lookup_100 multi_get  | 439.1 µs | 634.3 µs | **1.44×** |

All read legs ≥ 1.0× within-run (goal met locally). Motion vs the
fd-cache A/B: get_hit 7.24 → 4.20 µs, prefix_scan 225.5 → 196.7 µs
(268 → 197 within the ab2 process, −27%).

Suites: core 647/650 (3 pre-existing flakes unchanged), clippy at
pre-change state (warnings in touched regions pre-date the change).
New tests: seek parity vs decoded path (resident + evicted payload,
tombstone/snapshot/multi-block/empty keys), seek CRC fail-closed,
resolved-slot verbatim reuse across repeated scans interleaved with a
raw key-only pass (escape-prefixed + vlog-spilled values).

Caveats, stated plainly:
- Resolved slots are larger than raw (real values vs 20 B pointers):
  the entry-capped (8192) block cache's byte occupancy grows; it was
  never byte-budgeted and sits outside the 256 MiB payload knob
  (pre-existing accounting, unchanged by this commit). Guest RSS at
  25M must be checked in run #10.
- Two `decode_block().unwrap_or_default()` swallows remain at
  `last_visible_under_prefix_with` call sites (key-only, errors
  become skipped candidates) — pre-existing, flagged for a later F1
  pass.
- 25M guest numbers pending: injection staged, blocked on sudo.

## Guest run #10 (v21h, 25M) — read legs improve 9–21% (p=0.00), still <1× at 25M

Injection path correction: the script was always for the **gate host**
(`192.168.68.109`, SSH key auth, passwordless sudo there — the earlier
"sudo blocked" was tested on the Mac, wrong host; `losetup`/`md5sum` never
existed locally). All 9 files md5-verified in-image (`MD5_OK`), guest
rebuild confirmed (`Finished bench profile in 24.19s`), the in-image
entrypoint auto-ran the isolated pedra leg. Raw serial:
`run10-25m-serial.txt`.

| leg (25M)              | run #9 (v21f) | run #10 (v21h) | rocks default | run10 ratio |
|------------------------|---------------|----------------|---------------|-------------|
| hydrate                | 103.1 s       | 117.6 s        | 25.3 s        | 0.22×       |
| settle                 | 79.9 s        | 89.6 s         | 8.3 s         | 0.09×       |
| probe_hit p50          | 39.5 µs       | 39.8 µs        | 45.4 µs       | **1.14×**   |
| get_hit (criterion)    | 66.8 µs       | 54.1 µs        | 38.59 µs      | 0.71×       |
| prefix_scan            | 730 µs        | 644.6 µs       | 305.6 µs      | 0.47×       |
| lookup_100 get_loop    | 6.585 ms      | 5.191 ms       | 3.38 ms       | 0.65×       |
| lookup_100 multi_get   | 6.655 ms      | 6.023 ms       | 3.70 ms       | 0.61×       |
| on disk after settle   | 5.15 GiB      | 5.15 GiB       | 5.24 GiB      | smaller     |

Honest read:
- Every criterion read leg improved (get_hit −19%, get_loop −21%,
  multi_get −9.5%, prefix_scan −12%; all p=0.00), but the local 6M
  "all legs ≥ 1.0×" did **not** transfer to 25M. At 25M the settled L1
  is 5.15 GiB in 99 disjoint chunks while the payload pool is 256 MiB
  (~5% resident) — most reads are disk-bound, so the per-get cost is
  block read + CRC + lz4 + raw walk vs Rocks' block read + search.
  probe_hit p50 stays ahead of Rocks (1.14×) but the tails do not
  (p99 2.1 ms / p999 4.8 ms).
- Memory held: RSS flat ~1.2 GB through the read legs, pool at budget,
  `ent_e=0` on every tick, `BENCH_EXIT_pedradb_diag=0`. The
  resolved-slot cache did not blow the box.
- settle regressed slightly (79.9 → 89.6 s; same whole-level shape, gate
  noise). Post-settle L1 is 99 **disjoint** chunks — stacked runs only
  exist during hydrate.
- Compaction structure (from code + the local 6M settle sample): the
  hydrate-time worker jobs take **only L0s** (`prepare_l0_compact`
  never merges into L1 → stacked overlapping L1 runs during hydrate),
  and settle is one single-threaded whole-level rewrite (86% of the
  settle wall in `compact_levels → rewrite_ssts → write_merged_tables`,
  including the output write-verify re-read). The Rocks shape needs
  overlap-based L1 input selection + a bounded L1 size target +
  incremental L1→L2 — that single change attacks hydrate, settle, the
  probe tails, and the 100M disk peak (47 GiB → ~live set) together.

## Guest run #11 (v21i, 25M) — leveled compaction + parallel merge spans

Implemented in core (`leveling.rs` new; `db.rs`, `concurrent.rs`,
`sst/table.rs`, `lib.rs`): leveled selection kernel (L0→L1 jobs absorb
the disjoint L1 overlap slice, hull-closure-capped at 4× the 256 MiB L1
target; over-cap → bounded pushdown L1→L2/L2→L3 of the oldest chunk plus
its overlap), `Db::compact_leveled` settle drain (stacked-level repair +
bounded jobs, 100k safety valve), pushdowns piggyback on every worker
install (≤4/tick), and a type-erased `ParallelMerge` seam so merge jobs
run as Rocks-shaped key-space subcompactions (`std::thread::scope`,
shared file-number atomic) — `E: Send + Sync` holds only on the host
open path (`ConcurrentDb::open_with_env_bounded`), compat stays
untouched (the crate compiles against it with zero edits). Kill
switches: `PEDRA_LEVELED=0`, `PEDRA_MERGE_SPANS=N`. Core suite: 654
pass; the 3 failures (`catchup_wait_bounded_by_half_fd`,
`verified_report_matches_catalog`, `maybe_auto_flush…`) reproduce
**at HEAD in a clean worktree** — pre-existing, not from this change.
Property test `leveling::job_output_keeps_level_disjoint` initially
failed on its own generator (decimal keys invert byte order across
digit-count boundaries — ranges production never produces); zero-padded
keys pass all 200 cases. Raw serial: `run11-25m-leveled.txt`. Local 1M
sanity: hydrate 1.0 s vs rocks 0.9 s, 8 parallel spans visible.

| leg (25M)              | run #10 (v21h) | run #11 (v21i) | rocks default | v21i ratio |
|------------------------|----------------|----------------|---------------|------------|
| hydrate                | 117.6 s        | 149.2 s        | 25.3 s        | 0.17×      |
| settle                 | 89.6 s         | **51.4 s**     | 8.3 s         | 0.16×      |
| probe_hit p50          | 39.8 µs        | 43.1 µs        | 45.4 µs       | 1.05×      |
| get_hit (criterion)    | 54.1 µs        | **49.4 µs**    | 38.59 µs      | 0.78×      |
| prefix_scan            | 644.6 µs       | 616.1 µs       | 305.6 µs      | 0.50×      |
| lookup_100 multi_get   | 6.023 ms       | **5.019 ms**   | 3.70 ms       | 0.74×      |
| on disk after settle   | 5.15 GiB       | 5.15 GiB       | 5.24 GiB      | smaller    |

Honest read:
- settle −43%, get_hit +9.5%, lookup_100 multi_get +17% — but hydrate
  **+27%** (117.6 → 149.2 s): leveled jobs rewrite the L1 slice per tick
  and pushdowns run during ingest, so hydrate pays I/O it used to defer
  to settle. Net hydrate+settle 207.2 → 200.6 s — roughly break-even,
  with a far better read structure as the residual win.
- Post-settle `COMPACTDIAG l0=0 l1=5 sst_n=95` — but run #10's settle
  already produced 99 **disjoint** chunks, so stacking was never the
  settled-read bottleneck: the gap is per-get candidate cost (every
  chunk bloom-checked, ~95 chunks ≈ the 10.8 µs get_hit gap over rocks)
  and per-block scan cost (4 KiB `BLOCK_TARGET` vs rocks 16 KiB).
- Hydrate is **fd-floor-bound**: 24 414 sequential apply batches × one
  fdatasync each (G1 product, fdatasync-before-Ok) on qcow2/virtio vs
  the peer's zero fsyncs (`sync=false`). Even perfect compaction overlap
  cannot reach 1× on this disk class — the registered single-client
  fd-ceiling disclosure applies to this leg (local Mac: 0.9×).

## Guest run #12 (v21j, 25M) — point-lookup range prune (+ LEVELDIAG)

`Db::lookup` now skips the point seek for chunks whose
smallest/largest user key excludes the key (bounds span every entry's
user key, deletion markers included; range tombstones still collected
from every chunk — a tombstone's end key lives in its value, outside
the bounds). Injection also exports `PEDRA_LEVEL_DIAG=1` in
`p04_entrypoint.sh` (which exports `PEDRA_FLUSH_DIAG` only — the run #9
"REWRITEDIAG exported" note does not hold for this image; that is why
run #10/#11 have zero REWRITEDIAG lines). `BENCH_EXIT_pedradb_diag=0`,
raw serial: `run12-25m-prune.txt`. Criterion baselines (the `change:`
lines) are run #11 on the same guest disk.

| leg (25M)              | run #11 (v21i) | run #12 (v21j) | rocks default | v21j ratio |
|------------------------|----------------|----------------|---------------|------------|
| hydrate                | 149.2 s        | 146.8 s        | 25.3 s        | 0.17×      |
| settle                 | 51.4 s         | 51.4 s         | 8.3 s         | 0.16×      |
| probe_hit p50          | 43.1 µs        | **38.9 µs**    | 45.4 µs       | **1.17×**  |
| probe_miss p50         | —              | **2.8 µs**     | rocks-class   | ~1×        |
| get_hit (criterion)    | 49.4 µs        | 54.9 µs ⚠      | 38.59 µs      | 0.70×      |
| prefix_scan            | 616.1 µs       | 699.9 µs ⚠     | 305.6 µs      | 0.44×      |
| lookup_100 get_loop    | ~5.13 ms       | 6.137 ms ⚠     | 3.38 ms       | 0.55×      |
| lookup_100 multi_get   | 5.019 ms       | 5.730 ms ⚠     | 3.70 ms       | 0.65×      |
| on disk after settle   | 5.15 GiB       | 5.15 GiB       | 5.24 GiB      | smaller    |

LEVELDIAG ground truth at `compact_leveled_done` (the run #10 open
question — COMPACTDIAG's `l1=5` never meant 5×256 MiB):

- L0 = 0 files · L1 = 5 files / 225 MiB (at the 256 MiB target)
- L2 = 44 files / 2.46 GiB (just under its fanout-10 2.5 GiB target —
  the drain stopped correctly, not stalled)
- L3 = 46 files / 2.47 GiB (last level, unbounded) · total 5.15 GiB ✓

Honest read:
- The prune wins exactly where predicted: **probe_miss p50 2.8 µs**
  (miss keys sit above every chunk's largest bound → all point seeks
  skipped; the walk still collects range tombstones from all 95 chunks
  and stays rocks-class) and **probe_hit 43.1 → 38.9 µs = 1.17×**.
- The four ⚠ criterion legs all regressed together (+13–20 %, p ≤ 0.03
  vs run #11 baselines) — including `prefix_scan`, whose code path
  v21j does not touch, while settle (pure write throughput) was
  identical to the second. The prune adds two key compares per chunk;
  it cannot cost 5–8 µs per op. Read: this run's criterion legs saw a
  slower guest read path (qcow2 state after the 12th hydrate), not a
  prune regression. Arbitrated by run #13 (next section): **guest
  noise — every ⚠ leg came back at or better than its v21i value.**
- get_hit vs rocks is ~11 µs/get short even on the trusted v21i number.
  Static read of the path kills the tombstone lead:
  `Table::collect_range_tombstones` early-outs on an in-memory
  `range_tombstones.is_empty()` (table.rs:287), so collecting from 95
  zero-tombstone chunks costs ~100 ns/get — not the gap. The walk is
  also thin (`sst_indices_newest_first` returns a cached order slice;
  the bounds prune is two memcmps per chunk). Remaining candidates:
  the per-get block read itself (whole-table payload pool at ~45 MiB
  granularity vs rocks' 16 KiB block cache; positioned read + CRC +
  lz4 + in-block scan per get) and 4 KiB `BLOCK_TARGET` vs rocks'
  16 KiB for the scan leg.

## Guest run #13 (v21j, 25M, same image) — noise arbitration repeat

No re-injection: container stop/start only, entrypoint rebuild + bench
re-ran. `BENCH_EXIT_pedradb_diag=0`, raw serial:
`run13-25m-prune-repeat.txt`.

| leg (25M)              | run #11 (v21i) | run #12 (v21j) | run #13 (v21j) | rocks default | v21j ratio |
|------------------------|----------------|----------------|----------------|---------------|------------|
| hydrate                | 149.2 s        | 146.8 s        | 143.7 s        | 25.3 s        | 0.18×      |
| settle                 | 51.4 s         | 51.4 s         | 52.0 s         | 8.3 s         | 0.16×      |
| probe_hit p50          | 43.1 µs        | 38.9 µs        | **33.9 µs**    | 45.4 µs       | **1.34×**  |
| probe_miss p50         | —              | 2.8 µs         | 2.5 µs         | rocks-class   | ~1×        |
| get_hit (criterion)    | 49.4 µs        | 54.9 µs        | **46.7 µs**    | 38.59 µs      | 0.83×      |
| prefix_scan            | 616.1 µs       | 699.9 µs       | 632.6 µs       | 305.6 µs      | 0.48×      |
| lookup_100 get_loop    | ~5.13 ms       | 6.137 ms       | **4.534 ms**   | 3.38 ms       | 0.75×      |
| lookup_100 multi_get   | 5.019 ms       | 5.730 ms       | 4.954 ms       | 3.70 ms       | 0.75×      |
| on disk after settle   | 5.15 GiB       | 5.15 GiB       | 5.15 GiB       | 5.24 GiB      | smaller    |

Verdict: run #12's four criterion regressions were guest noise — on the
repeat every one came back at or better than its v21i value (get_hit
−5.5 %, get_loop −11.6 %, multi_get −1.3 %, prefix_scan +2.7 %), and the
LEVELDIAG level split reproduced exactly (L1 5/225 MiB, L2 44/2.46 GiB,
L3 46/2.47 GiB). v21j keeps: probe_hit 1.34×, probe_miss rocks-class,
get_hit best-yet 0.83×, lookup_100 both variants 0.75×. Lesson recorded:
single-run criterion deltas on this guest swing ±15 % — never accept or
reject a lever on one run; the probe legs (custom harness, printed
percentiles) were far more stable across #11–#13 than the criterion
legs.




## Local 6M BLOCK_TARGET A/B + PEDRA_BLOCK_TARGET knob (pre-v21k)

Method fix first: running the bench binary **directly without `--bench`
puts criterion in smoke-test mode** ("Testing X / Success", no timings —
criterion lib.rs:963 `(false, _) => true`); the guest entrypoint invokes
via `cargo bench`, which passes `--bench`. All local criterion runs
before this point were invalid single-iteration tests (probe percentiles
were always real — custom harness inside the setup). Also: cargo's bench
binary hash is graph-derived, not content-derived — a rebuild relinks
**in place** (`ls -t` can hand you the old or new build; same filename).
Verify mtime, not name.

Arm A — local 6M, v21j (4 KiB blocks), `--bench`, TMPDIR pinned:

| leg (6M, local)     | pedra    | rocks    | ratio |
|---------------------|----------|----------|-------|
| get_hit             | 5.386 µs | 6.774 µs | 1.26× |
| prefix_scan         | 202.9 µs | 225.7 µs | 1.11× |
| lookup_100 get_loop | 482 µs   | 608 µs   | 1.26× |
| lookup_100 multi_get| 436 µs   | 777 µs   | 1.78× |

Local 6M is **>1× on every criterion leg with the current format** — the
25M guest gaps are guest-regime (5.15 GiB vs a 256 MiB pool on
qcow2/virtio), not format-inherent.

Arm B — same tree, BLOCK_TARGET 16384 (const flip, pre-knob): raw deltas
unusable (the concurrent session's builds moved the **rocks** legs
−13/−18 % between arms on identical rocks code). Normalized
pedra/rocks ratios A→B: get_hit 0.80→1.07, get_loop 0.79→0.99,
multi_get 0.56→1.03, scan 0.90→0.92. Direction is consistent with
theory: 16 KiB blocks lengthen the in-block walk (cache-hot point legs
lose their edge) and amortize per-block decode/read (scan flat-to-
better). On the cache-cold guest the scan should gain ~4× fewer block
reads per scan while point legs stay request-latency-bound (≈neutral).

Knob (v21k, commit d7b53d0): `PEDRA_BLOCK_TARGET` (bytes, clamped
1 KiB–256 KiB, default 4096) read once via `OnceLock`; only the writer
consults it, reads are self-describing per block so mixed-target tables
coexist. Local validation: unset → 203.1 µs scan (reproduces arm A);
16384 → 198.1 µs. Default stays 4096 until the ladder proves 16 KiB at
every scale (1M/10M are cache-warm, where the local point-leg regression
applies). Guest run #14 = v21k: knob + entrypoint
`PEDRA_BLOCK_TARGET=16384` + a hydrate fd-floor probe
(`fdprobe.py`: 200 × 230.4 KiB append + fdatasync on /data/stores →
`FDFSYNC_PROBE per_op_ms=…`; ×24 414 = the structural hydrate floor).

## Guest run #14 (v21k, 25M) — 16 KiB blocks refuted; fd floor measured

v21k = v21j + `PEDRA_BLOCK_TARGET` knob, entrypoint exported to 16384,
+ fd-floor probe. `BENCH_EXIT_pedradb_diag=0`, raw serial:
`run14-25m-block16k.txt`.

| leg (25M)              | run #13 (4 KiB) | run #14 (16 KiB) | rocks default | #14 ratio |
|------------------------|-----------------|------------------|---------------|-----------|
| hydrate                | 143.7 s         | **136.1 s**      | 25.3 s        | 0.19×     |
| settle                 | 52.0 s          | **47.9 s**       | 8.3 s         | 0.17×     |
| probe_hit p50          | 33.9 µs         | 48.0 µs          | 45.4 µs       | 0.95×     |
| probe_miss p50         | 2.5 µs          | 2.6 µs           | rocks-class   | ~1×       |
| get_hit (criterion)    | 46.7 µs         | 56.0 µs          | 38.59 µs      | 0.69×     |
| prefix_scan            | 632.6 µs        | 592.5 µs         | 305.6 µs      | 0.52×     |
| lookup_100 get_loop    | 4.534 ms        | 5.928 ms         | 3.38 ms       | 0.57×     |
| lookup_100 multi_get   | 4.954 ms        | 6.184 ms         | 3.70 ms       | 0.60×     |
| on disk after settle   | 5.15 GiB        | 5.04 GiB         | 5.24 GiB      | smaller    |

Honest read:
- **16 KiB blocks are refuted at 25M guest scale.** Every point leg got
  worse (probe_hit −42 %, get_hit +15 % p=0.04, lookup_100 +25/+31 %
  p=0.00 vs the #13 baselines on the same disk) and prefix_scan only
  moved −6 % (p=0.84, not significant). The scan-is-block-read-bound
  theory is falsified: the guest scan is dominated by per-key iterator
  work, which block size does not reduce, while point lookups pay for
  the wider read and longer in-block walk. Default stays 4096; the knob
  stays (default-off, cheap, self-describing reads); **the guest
  entrypoint's `PEDRA_BLOCK_TARGET=16384` export must be dropped in the
  next injection.**

## Guest run #19 (v21p, 25M) — storm gone, best read legs, settle worse: the pipeline is byte-volume-bound

v21p (`38dc2c2`): idle-WAL-rotate manifest storm fix + SST write-path
single-image rewrite (read-back/verify removal). Entrypoint now exports
only `PEDRA_FLUSH_DIAG PEDRA_LEVEL_DIAG PEDRA_FDSYNC_DIAG` (JOBS=4
dropped). Raw serial: `run19-v21p-guest-25m.txt`; full analysis:
`sorted-ingest-architecture.md`.

| leg (25M)              | run #13 (v21j) | run #19 (v21p) | rocks default | #19 ratio |
|------------------------|----------------|----------------|---------------|-----------|
| hydrate                | 143.7 s        | 148.9 s        | 25.3 s        | 0.17×     |
| settle                 | 52.0 s         | 88.2 s         | 8.3 s         | 0.09×     |
| probe_hit p50          | 33.9 µs        | 52.6 µs ⚠      | 45.4 µs       | 0.86×     |
| probe_miss p50         | 2.5 µs         | 2.2 µs         | rocks-class   | ~1×       |
| get_hit (criterion)    | 46.7 µs        | **42.4 µs**    | 38.59 µs      | 0.91×     |
| prefix_scan            | 632.6 µs       | **437.6 µs**   | 305.6 µs      | 0.70×     |
| lookup_100 get_loop    | 4.534 ms       | 4.593 ms       | 3.38 ms       | 0.74×     |
| lookup_100 multi_get   | 4.954 ms       | **4.548 ms**   | 3.70 ms       | 0.81×     |
| on disk after settle   | 5.15 GiB       | 5.15 GiB       | 5.24 GiB      | smaller   |

- **FDSYNCDIAG count 0** (run #16: ≥ 12,288 syncs): the manifest storm
  is gone. The read legs it was drowning are the best ever: get_hit
  0.83→0.91×, prefix_scan 0.48→0.70×, multi_get 0.75→0.81×.
- probe_hit ⚠: documented ±30 % single-run swing on this guest
  (identical code: 33.9 in #13, 49.8 in #18) — needs an arbitration
  repeat before being called a regression.
- **hydrate+settle total unchanged** (#18 237.6 s → #19 237.1 s):
  removing CPU overhead only moved time between phases. The wall time is
  the ladder's ~25–30 GiB of logical bytes × the ~110 MiB/s per-byte
  encode/decode rate (unchanged by the read-back removal — measured
  again at 56–58 MiB/s output / ~110–116 MiB/s per-job in this run's
  settle). Settle anatomy: flush_ms=10935 (of which ~6.3 s = two
  hydrate-tail worker jobs) + compact_ms=77249 (23 sequential
  single-input pushdowns, ~3.2 s each); settle entry L2 = 4.08 GiB in
  151 MiB chunks vs #15's 5.17 GiB in 59 MiB chunks — same per-byte
  rate, more bytes.
- The 38-file final layout (vs 95 in #13) is why the read legs jumped:
  fewer candidate chunks per probe. The chunk growth is an emergent
  scheduling artifact, not an intended change.
- **Drastic-gain answer (see `sorted-ingest-architecture.md`): the bench
  hydrate is a perfectly sorted append-only stream** (fixed-width
  ascending keys, batch k sorts before batch k+1) that we push through
  the full LSM ladder. A sorted-ingest fast path (detect ascending
  batches → sorted-run builder → direct-to-L3 disjoint install, WAL ring
  only for the uninstalled tail, settle ≈ tail flush) writes 5.15 GiB
  once instead of ~25–30 GiB through a 110 MiB/s loop: projected
  hydrate 18–28 s, settle 1–3 s — provided the encode loop also drops
  per-entry cost (rocks does 207 MiB/s through its whole ladder on the
  same core; that is the per-byte bar).
- Only wins: hydrate 136.1 s and settle 47.9 s (both best-yet — fewer
  block boundaries, slightly less write/verify overhead) and disk after
  settle 5.04 GiB. Not worth the point-leg cost.
- **fd floor measured** (`FDFSYNC_PROBE n=200 … per_op_ms=1.974`,
  python3 present in the image): 24 414 hydrate batches × 1.974 ms ≈
  **48.2 s structural floor** — hydrate at 136.1 s carries ~88 s of
  non-floor cost. Ingest write-amp (leveled L1-slice rewrites during
  hydrate; 10.85 GiB written for a 5.04 GiB settled set ≈ 2.15×) is the
  hydrate lever, not the fd ceiling. Caveat: the probe ran idle; under
  hydrate's concurrent compaction I/O the effective per-op cost is
  higher, so 48.2 s is a lower bound on the floor.
- probe_hit across #11–#14: 43.1 / 38.9 / 33.9 / 48.0 µs — even the
  custom-harness probe swings ±20 % run-to-run on this guest. The 16 KiB
  verdict rests on all four point legs agreeing (incl. criterion's own
  significance tests), not on any single leg.

## v21p follow-up — the caller-side read-back (why #19's rate didn't move) + local 6M A/B

v21p removed the writer's *internal* re-verify (in-place `SstTable`
construction from writer state), but all four write-path callers then
did `drop(table); rename; SstTable::open_on(final)` — and `open_on` →
`decode` does a full pass over the fresh file (whole-file read,
per-block lz4 + CRC, per-entry decode + sortedness check). Run #19's
per-job rate therefore still paid one decode pass per write; that, not
the removed verify, was the visible ~110 MiB/s.

Fix (uncommitted-until-this-entry; `db.rs` + `SstTable::with_path` in
`table.rs`): the four sites (`write_imm_l0_file`,
`write_imm_l0_file_for_family`, whole-Db GC rewrite, and
`finish_merged_chunk_on` — every compaction chunk) keep the writer's
in-place table and only retarget its path past the rename. Recovery /
reopen opens (`db.rs` 4612, 6892, 7122, 10085) still verify fully —
fail-closed recovery is unchanged; `write_all` errors on short write;
rename is atomic.

Local 6M quiet A/B (interleaved old/new/old/new on this host, both
binaries built from explicit states: old = HEAD `a3572f1`, new = HEAD +
fix; `SLIPSTREAM_BENCH_CACHE_BYTES=256MiB`):

| leg   | old r1 | new r1 | old r2 | new r2 | median Δ |
|-------|--------|--------|--------|--------|----------|
| hydrate/pedradb | 24.8 s | 22.0 s | 26.5 s | 16.2 s | 25.65→19.1 s (−26 %) |
| settle/pedradb  | 14.0 s | 8.5 s  | 11.0 s | 10.0 s | 12.5→9.25 s (−26 %) |

All four pairings favor the fix. The rocksdb control arm drifted
−8 %/−22 % between arms (host load), so the drift-net hydrate gain is
conservatively ~10–25 %; the settle direction is consistent in both
rounds. On-disk after settle identical (1.24 GiB). Guest arbitration
(run #20 at 25M, where flush_ms/compact_ms give the phase split) is the
next measurement; expected effect there: settle compact_ms −30–40 %
(one decode pass fewer per rewritten byte), hydrate −10–25 %.

## Guest runs #20/#21 (read-back removal, 25M) — arbitration: NET LOSS; the read-back was an accidental page-cache warmer

Two identical runs (`ba2d73e` = v21p + caller-side read-back removal,
nothing else). Raw serial: `run20-readback-25m.txt`,
`run21-readback-25m.txt`. Every leg reproduced within a few percent
(ratio = rocks/pedra, same convention as the #19 table):

| leg (25M)        | rocks default | #19 (v21p) | #20      | #21      | #20/#21 ratio |
|------------------|---------------|------------|----------|----------|---------------|
| hydrate          | 25.3 s        | 148.9 s    | 129.8 s  | 130.0 s  | 0.19×         |
| settle           | 8.3 s         | 88.2 s     | 128.3 s  | 130.3 s  | 0.065×        |
| probe_hit p50    | 45.4 µs       | 52.6 µs    | 51.1 µs  | 56.5 µs  | 0.80–0.89×    |
| probe_miss p50   | rocks-class   | 2.2 µs     | 2.3 µs   | 2.7 µs   | ~1×           |
| get_hit          | 38.59 µs      | 42.4 µs    | 54.6 µs  | 53.1 µs  | 0.71–0.73×    |
| prefix_scan      | 305.6 µs      | 437.6 µs   | 947.0 µs | 953.8 µs | 0.32×         |
| lookup get_loop  | 3.38 ms       | 4.593 ms   | 4.833 ms | 5.554 ms | 0.61–0.70×    |
| lookup multi_get | 3.70 ms       | 4.548 ms   | 4.763 ms | 5.497 ms | 0.67–0.78×    |
| disk after       | 5.24 GiB      | 5.15 GiB   | 5.15 GiB | 5.15 GiB | smaller       |

- hydrate −12.7 % (148.9 → ~130 s, both runs) — the one real win, same
  direction as the local 6M A/B.
- settle +47 % (88.2 → ~129 s, both runs) **with less entry debt in
  #20** (L2 entry 3.48 GiB vs #19's L0+L1+L2 5.1 GiB): per-GiB pushdown
  roughly halved. #21 entered with more debt (L0=8/1.4 GiB + L1/L2/L3)
  and landed on the same 128–130 s and the identical final layout
  (L1=2, L2=14/2.57 GiB, L3=18/2.91 GiB).
- Every read leg regressed 15–118 %; prefix_scan 2.2×.

Mechanism (hypothesis, consistent with all observations): the removed
caller-side read-back (`open_on` over every freshly written SST) was
doubling as a read-ahead that pushed each fresh chunk through the
guest's page cache (3.9 GiB RAM). Without it, settle's compaction reads
its inputs cold and the read legs start on a cold block/page cache. On
the Mac (6M, big RAM, fast NVMe) the same change measured −26 %/−26 % —
cache warming was worthless there and decode CPU dominated. The guest
inverts that trade.

Decision: `ba2d73e` stays (correctness-neutral, wins on big-RAM hosts),
but the arbitration image line builds on the v21p read-back until an
explicit warm (`fadvise(WILLNEED)` over fresh chunks / settle inputs)
replaces the accidental one. Next guest config (v22) = v21p read-back
+ RFC-0159 P0.2 bulk bottom-level install: bulk chunks are written once
and never re-laddered, so the settle pushdown disappears by construction
and hydrate stops paying the L0→L1 ladder tax.

## Guest run #22 (v22 = v21p read-back + RFC-0159 P0.2, 25M) — bulk path INERT: the bench flushes through `ConcurrentDb`, whose install funnels were unwired

Raw serial: `run22-v22-25m.txt`. v22 = v21p (a3572f1 read-back) +
`bff465c` (P0.2 bulk bottom-level install). Verdict: **the bulk path
never engaged** — BULKDIAG count 0 across the whole run, settle 84.9 s
(in family with #19's 88.2 s, i.e. the ladder did all the work), and
every other leg matches the #19/#20 read-back family:

| leg (25M)        | rocks default | #19 (v21p) | #22 (v22) | ratio #22 |
|------------------|---------------|------------|-----------|-----------|
| hydrate          | 25.3 s        | 148.9 s    | 157.0 s   | 0.16×     |
| settle           | 8.3 s         | 88.2 s     | 84.9 s    | 0.098×    |
| probe_hit p50    | 45.4 µs       | 52.6 µs    | 54.0 µs   | 0.84×     |
| probe_miss p50   | rocks-class   | 2.2 µs     | 2.3 µs    | ~1×       |
| get_hit          | 38.59 µs      | 42.4 µs    | 47.4 µs   | 0.81×     |
| prefix_scan      | 305.6 µs      | 437.6 µs   | 449.0 µs  | 0.68×     |
| lookup get_loop  | 3.38 ms       | 4.593 ms   | 4.468 ms  | 0.76×     |
| lookup multi_get | 3.70 ms       | 4.548 ms   | 4.503 ms  | 0.82×     |

Mechanism (code-traced): the compat bench writes through
`rocksdb_compat::DB::write` → `ConcurrentDb::apply_batch_vec` → group
commit, and settles through `ConcurrentDb::flush` /
`drain_imm_once` / `materialize_parked_once`. P0.2's write-side
observation was already live on that path (`commit_async_ops` /
`commit_async_one` call `observe_bulk_batch`), but the level decision
(`bulk_span_level`) was consulted only in `Db::flush_cf` /
`Db::flush_imm_to_l0` — every `ConcurrentDb` install site called
`install_l0_ssts` / `apply_l0_installs` unconditionally, and
`bulk_diag` printed only from `flush_cf`, hence BULKDIAG=0. Run #22 is
therefore a clean control: v21p read-back reproduced (within run noise)
with P0.2 present-but-inert.

Fix (v23): the three `ConcurrentDb` install sites route through
`bulk_span_level` + `install_ssts_at_levels`/`apply_sst_installs`
exactly like `Db::flush_imm_to_l0`, with their own BULKDIAG tags
(`install_flush` / `install_drain` / `install_parked`); 3 regression
tests in `concurrent::tests` drive the real funnels
(`apply_batch_vec` + `flush`, deferred drain, parked materialize) and
assert bottom-level installs.

### v23 local 6M A/B — bulk engaged, −39…−45 % hydrate / −25…−43 % settle; stale-binary correction

`PEDRA_BULK=0/1` kill switch, same binary, 2 interleaved rounds
(fresh build; commit `9698caf`). **Correction:** the earlier "flat"
local A/B that supported the non-engagement diagnosis ran a stale
binary — `/tmp/slip-inject/target-local` held an Aug 31 pre-P0.2
build (no `BULKDIAG`/`install_*` strings at all), while the stage
rebuilds landed in `stage/target`. The A/B script now resolves the
stage binary and hard-fails on a binary lacking `install_flush`.

| 6M local | rocks default | off r1 | off r2 | on r1 | on r2 |
|----------|---------------|--------|--------|-------|-------|
| hydrate  | 20.3–22.7 s   | 61.5 s | 52.7 s | 33.5 s | 32.2 s |
| settle   | 12.5–13.1 s   | 23.8 s | 18.0 s | 13.6 s | 13.6 s |

On-arm BULKDIAG = 22 both rounds (21 `install_parked` + 1
`install_flush`), off-arm 0. The parked-materialize funnel dominates:
during hydrate the deferred CF auto-flush parks the data family, and
the host worker materializes each parked table straight to the bottom
level — the L0→L1→L2→L3 ladder tax disappears from hydrate, and
settle's explicit flush installs its final span at the bottom too.
Settle is at parity with Rocks locally (13.6 vs ~13 s); hydrate still
behind (32–34 vs ~21 s — WAL/memtable apply CPU, next target).
Guest run #23 (25M, v23 image) arbitrates at scale.

## Guest run #23 (v23 = v22 + ConcurrentDb bulk wiring, 25M) — SETTLE CROSSES: 2.3 s vs Rocks 8.3 s (3.6×); hydrate −53 %

Raw serial: `run23-v23-25m.txt`. v23 = v22 + the three `ConcurrentDb`
install funnels routed through `bulk_span_level` (commit `9698caf`).
**73 BULKDIAG** across the run (72 `install_parked` during hydrate +
1 `install_flush` at settle) — the fast path engaged end to end at
scale, and the ladder pushdown disappeared by construction:

| leg (25M)        | rocks default | #22 (v22) | #23 (v23) | ratio #23 | vs #22   |
|------------------|---------------|-----------|-----------|-----------|----------|
| hydrate          | 25.3 s        | 157.0 s   | 73.6 s    | 0.34×     | −53 %    |
| settle           | 8.3 s         | 84.9 s    | 2.3 s     | **3.61×** | −97 %    |
| probe_hit p50    | 45.4 µs       | 54.0 µs   | 54.9 µs   | 0.83×     | ~flat    |
| probe_miss p50   | rocks-class   | 2.3 µs    | 2.9 µs    | ~1×       | ~flat    |
| get_hit          | 38.59 µs      | 47.4 µs   | 47.0 µs   | 0.82×     | ~flat    |
| prefix_scan      | 305.6 µs      | 449.0 µs  | 457.1 µs  | 0.67×     | ~flat    |
| lookup get_loop  | 3.38 ms       | 4.468 ms  | 4.963 ms  | 0.68×     | +11 %    |
| lookup multi_get | 3.70 ms       | 4.503 ms  | 5.183 ms  | 0.71×     | +15 %    |
| disk after       | 5.24 GiB      | 5.15 GiB  | 5.15 GiB  | smaller   | same     |

- **Settle is the first write-leg crossing at 25M: 2.3 s vs Rocks'
  8.3 s (3.61×), with Pedra still fdatasyncing before Ok (G1).** The
  bulk chunks are written once and never re-laddered, so settle has
  nothing to push down — its 2.3 s is the final memtable flush plus
  manifest work.
- Hydrate halved (157.0 → 73.6 s) but remains 0.34×: the remaining
  gap is per-op commit CPU (WAL encode + memtable apply + payload
  copy), not write volume — write-amp during hydrate collapsed with
  the ladder gone. The bench peer is `sync=false`, so this leg has no
  per-op fd on either side; the RFC-0159 P1.1 encode-path per-byte
  cut is the lever.
- Read legs unchanged within noise (get_hit 47.0 vs 47.4, scan 457 vs
  449); get_loop/multi_get +11–15 % vs #22 (single-run swing — the
  documented ±30 % band covers it; layout identical at 5.15 GiB).
- No failure markers; `BENCH_EXIT_pedradb_diag=0`.

Next: the read legs are now the whole remaining gap at 25M (hydrate
CPU aside): get_hit 0.82×, prefix_scan 0.67×, lookup 0.68–0.71× —
same levers as the read-legs queue (payload-pool granularity, block
layout, compat iterator window).

## Guest prefix_scan gap: local attribution (macOS `sample`, 6M)

Where does a scan op actually go? Local 6M pedra-only prefix_scan,
sampled with macOS `sample` during criterion's Collecting phase (clean
reading 215.68 µs [211.33, 221.12]; local v21j/4 KiB criterion: pedra
202.9 µs vs rocks 225.7 µs). Raw tree: `local-6m-scan-sample.txt`.

Attribution of 8335 `Bencher::iter` samples:

- `scan_prefix` (compat) 2458 samples = 29.5 % of the op, of which
  `page_forward` 1141 (13.7 %): `next_window_kv` ≈ 960 — heap push 268
  (3.2 %), heap pop 79, `SstRangeIter::next` ≈ 450 (memcmp walk),
  block-cache get_or_insert ≈ 45 — and `page_forward` self ≈ 816
  (window materialization: codec decode + Vec push + malloc per item).
- The other ~70 % collapses into frame-pointer-less `Bencher::iter`
  self (inlined closure body: decode_entry, utf8, starts_with,
  black_box) — work the rocks leg also pays in its own shape.

So engine-specific work is ~30 % of the op and **already beats rocks
locally**. The guest multiplies pedra's path ×2.9 (0.53 → 1.56 µs/key)
while rocks only ×1.35 (0.59 → 0.80). Two live hypotheses, untestable
remotely: (a) guest scans are not cache-hot — block-cache misses
re-read blocks through qcow2/virtio; (b) the guest vCPU penalizes
pedra's instruction mix (Bytes atomic refcounts, heap sifts, malloc).

Side-finding: local hydrate is parking-bound — 92 % of main-thread
samples sit in `WriteGroup::await_flush_debt` → nanosleep. Relevant
when the hydrate write-amp lever opens.

## v21l — scan diagnostics instrument (run #15)

Goal: split hypothesis (a) cache misses from (b) CPU cost **on the
guest**, without touching the read-only compat layer.

- `PEDRA_SCAN_DIAG=1` (read once, `OnceLock`): `next_window_kv` (the
  function the compat scan actually drives via `into_window_kvs`) is
  timed with `Instant` around an inner fn; rows + ns go to crate-level
  atomics. `Db::scan_at_raw` records stream count + setup ns and every
  2048 scans prints
  `SCANDIAG ops=N streams/op=… setup_ns/op=… rows/op=… row_ns/row=… cache_hits/op=… cache_misses/op=…`
  (block-cache counters are DB-global — the per-op attribution holds on
  a scan-only leg).
- Settle-start shape: `dump_level_diag("compact_leveled_start")` before
  the leveled drain, under the existing `PEDRA_LEVEL_DIAG` — learn how
  many bytes settle actually moves at 25M.
- Entrypoint: `PEDRA_BLOCK_TARGET=16384` export dropped (refuted,
  run #14); default 4096 rides again.

Local validation (1M, `local-1m-scandiag.txt`): diag ON prints
streams/op=2.0, rows/op=333.3 (the bench prefix size), row_ns/row ≈ 70
ns, cache_misses/op = 0.00, cache_hits/op ≈ 22 — locally the scan is
fully block-cache-hot, as expected at 1M/256 MiB. OFF-path control:
199.81 µs, identical to the pre-instrument level (one relaxed load per
row when disabled). ON-path overhead ≈ +31 % at 1M (two `Instant` calls
per row) — **guest prefix_scan absolutes in run #15 are instrumented,
not comparable to #13/#14**; the point is row_ns/row and
cache_misses/op, not the leg time. Read the instrument against the
local reading with the same overhead included (~70 ns/row local).

## Guest run #15 (v21l, 25M) — scan verdict: cache-hot, gap is per-row CPU

`BENCH_EXIT_pedradb_diag=0`; raw serial `run15-25m-scandiag.txt`
(20 SCANDIAG lines over 40 960 scan ops). Default 4 KiB blocks restored.

| leg (25M)              | run #13 | run #15 (v21l)  | rocks default | #15 ratio |
|------------------------|---------|-----------------|---------------|-----------|
| hydrate                | 143.7 s | 155.7 s         | 25.3 s        | 0.16× (see run #17 correction) |
| settle                 | 52.0 s  | 53.3 s          | 8.3 s         | 0.16×     |
| probe_hit p50          | 33.9 µs | 37.9 µs         | 45.4 µs       | 1.20×     |
| probe_miss p50         | 2.5 µs  | 2.7 µs          | rocks-class   | ~1×       |
| get_hit (criterion)    | 46.7 µs | 60.5 µs (p=0.32 n.s.) | 38.59 µs | 0.64×     |
| prefix_scan            | 632.6 µs | **781.7 µs instrumented** | 305.6 µs | 0.39×* |
| lookup_100 get_loop    | 4.534 ms | 7.263 ms (p=0.17 n.s.) | 3.38 ms | 0.47× |
| lookup_100 multi_get   | 4.954 ms | 5.624 ms        | 3.70 ms       | 0.66×     |
| on disk after settle   | 5.15 GiB | 5.15 GiB       | 5.24 GiB      | smaller   |

\* instrumented: two `Instant` calls per row ≈ +60–90 µs at 333 rows/op;
this boot was also I/O-slow (see fd floor below). The number is for the
diagnosis, not for the ladder.

**SCANDIAG verdict (the run's purpose), stable across all 20 windows:**

- streams/op = 2.0, rows/op = 333.3, setup_ns/op ≈ 5.2–7.5 µs,
  **row_ns/row ≈ 200–229 ns (median ~208)**, cache_hits/op ≈ 22.3,
  **cache_misses/op = 0.00** (first window 0.03 — cold start).
- **The 25M guest scan is fully block-cache-hot.** Hypothesis (a)
  (pool/residency misses re-reading through qcow2) is refuted for this
  leg: the bench's prefix working set (~22 resolved blocks/op) stays in
  the 256 MiB block cache, same as at 1M locally.
- Hypothesis (b) confirmed: the guest vCPU runs pedra's per-row merge
  work at ~208 ns/row vs ~70 ns/row locally (same instrument, ×3.0
  amplification) while the rocks leg only amplifies ×1.35. The scan gap
  is per-row CPU cost, not block I/O.
- **But the core is only ~12 % of the op**: setup 5.9 µs + 333 × 208 ns
  ≈ 75 µs of a ~632 µs (uninstrumented, #13) op. Even zeroing core scan
  cost entirely leaves ~557 µs of compat `page_forward` window
  materialization (codec decode + `to_vec` per row, read-only layer
  today) + shared bench closure vs rocks' 305.6 µs whole op. **The
  prefix_scan leg is not closable to ≥1× from inside `scan_at_raw`
  alone** — it needs the compat iterator path (cheaper per-row
  materialization) or a core API that lets compat fill windows without
  per-row allocation.

Boot-regime caveats (why no point-lever verdicts here):

- fd floor this boot: `FDFSYNC_PROBE per_op_ms=3.371` → ×24 414 ≈
  **82.3 s** vs 48.2 s on run #14's boot. The floor is boot-variable
  ×1.7 on this guest (same probe, same image) — quote it as a range
  (48–82 s), never a single number. This boot's hydrate 155.7 s sits
  ~73 s above its own floor.
- Point legs all read worse than #13 (get_hit +29 %, get_loop +60 %)
  with criterion significance mixed (get_hit p=0.32, get_loop p=0.17,
  multi_get p=0.00) — consistent with a globally slower-I/O boot, not
  with a v21l regression (the scan-path instrument is off the point
  path; OFF-control locally was identical to pre-instrument). Runs #12
  and #14 showed the same ±15–20 % single-run swings.

Settle-start LEVELDIAG (new): at settle entry the shape is
L0 2×53.6 MiB, L1 6×286.7 MiB, **L2 88×4.82 GiB**, L3 empty → settle
moves ~2.4 GiB L2→L3 (settled: L1 5×225.7 MiB, L2 44×2.46 GiB,
L3 46×2.47 GiB). So the 52–53 s settle is a ~2.4 GiB single-writer
rewrite (~46 MiB/s effective) — the settle-2 lever (batch pairwise-
disjoint jobs, parallel write via the ParallelMerge seam, sequential
install) now has its number: **parallelize ~2.4 GiB of L2→L3 pushdown**.

## v21m + Guest run #16 (25M) — hydrate/settle attribution under load

v21m = `PEDRA_FDSYNC_DIAG=1` (posix `fdatasync_file` choke point:
count/ns/max per 2048 barriers), `FLUSHDUR` (per-memtable SST write in
`write_imm_l0_files`), `COMPDUR` (per compaction job in
`PreparedL0Compact::write`). SCAN_DIAG off. Raw serial:
`run16-25m-hydratediag.txt`. hydrate 145.9 s, settle 49.5 s, disk
5.15 GiB — same regime as #13/#15.

| bucket (25M)                | wall       | notes                                   |
|-----------------------------|------------|-----------------------------------------|
| memtable flushes (26)       | 60.7 s     | avg 2.4 s per 256 MiB flush (~110 MiB/s)|
| compaction jobs (164)       | 152.8 s    | top job 1.44 s; avg 0.93 s              |
| fdatasync_file (≥12 288)    | ≥47.4 s    | first 2048 avg 15.8 ms (32.5 s!) — big-file syncs; later ~1.5 ms |
| WAL per-batch barriers      | uncounted  | see bypass below                        |

Sum (260.9 s) exceeds the 195.4 s hydrate+settle wall — flush/compaction
run on the compat worker while the writer runs, and the writer parks
behind the drain (FLUSHDIAG parked_n=1 with a full 256 MiB memtable,
repeatedly).

Two structural findings:

- **The parallel merge seam was never installed.** `set_parallel_merge`
  has no caller — compat or bench — so all 164 jobs and 26 flushes ran
  sequential. The seam (`ParallelMergeEnv` → `write_merged_tables_parallel`,
  key-disjoint spans, ≥96 MiB inputs, ≤8 spans) is complete and tested in
  core; it just never executes. Local 6M A/B (parallel default-on vs
  `PEDRA_PARALLEL_MERGE=0`): **net negative locally** — settle 7.9 vs
  5.9 s, apply leg ~3× slower (span sharding over-smalls files on this
  machine). Shipped as opt-in (`PEDRA_PARALLEL_MERGE=1`, installed
  automatically in `open_with_env_bounded`); guest run #17 is the ON
  arm against run #16's sequential baseline.
- **The WAL per-batch barrier bypasses the posix counter**: `Wal::sync_data`
  → `inner_mut().sync_data()` on concrete `W = std::fs::File` resolves to
  the *inherent* std method (Linux `fdatasync` inside std), not the
  `EnvFile` trait impl that routes to `fdatasync_file`. So the ~24 414
  G1 batch barriers are invisible to the posix choke point; v21n adds
  `WALFDIAG` at `Wal::sync_data` itself. The counted ≥12 288 big syncs
  (15.8 ms avg class) are SST/flush-side — per-file, not per-chunk
  (SSTs sync once per file, table.rs:2371).

## v21n + Guest run #17 (25M) — parallel merge no-go; hydrate is BARRIER-FREE

v21n = `PEDRA_PARALLEL_MERGE=1` exported in the guest entrypoint (the
opt-in seam auto-installs `ParallelMergeEnv` in `open_with_env_bounded`)
+ `WALFDIAG` (counters at `Wal::sync_data` itself, prints every 2048).
Raw serial: `run17-25m-parallel.txt`.

| leg (25M) | run #16 (seq) | run #17 (parallel ON) |
|-----------|---------------|------------------------|
| hydrate   | 145.9 s       | 158.5 s (11.23 GiB, 482 B/entry) |
| settle    | 49.5 s        | 54.7 s (5.15 GiB after) |
| 26 flushes   | 60.7 s (2.4 s avg) | 66.0 s (2.54 s avg) |
| 164 compaction jobs | 152.8 s (0.93 s avg) | 164.8 s (1.005 s avg) |
| posix fdatasync | ≥12 288 = ≥47.4 s | ≥10 240 = 48.1 s (max 515.9 ms) |

**Parallel-within-job merge is neutral on the guest** (hydrate band
143.7–158.5 s across #13–#17, settle band 47.9–54.7 s) and net-negative
locally (settle +34 %, apply leg ~3×). Lever **closed as no-go** —
consistent with the guest drain being disk-throughput-bound, not CPU-
bound. `PEDRA_PARALLEL_MERGE` stays opt-in default-off.

### Major correction — the fd-ceiling hydrate story was wrong for this bench

- `WALFDIAG` printed **zero lines** in run #17. The instrument prints at
  exact multiples of 2048 calls, so zero lines proves **< 2048 calls**
  vs ~24 414 batches (< 8 %); the mechanism says zero outright:
  `need_sync = inflight.needs_sync()` = `any_sync && …`, and no bench
  write requests sync — `snapshot_pedradb.rs` builds `PedraDbConfig`
  with `sync: false` (default, line 89), opens `set_sync(false)`
  (line 120), applies every write with `wo.set_sync(self.config.sync)`
  (line 323); the `sync: true` at line 369 is only the export/verify
  checkpoint. The rocks backend is the same (snapshot_rocksdb.rs:570,
  `WriteOptions::default()`).
- **Slipstream hydrate is async-vs-async and barrier-free.** The
  48–82 s `FDFSYNC_PROBE` "floor" (one fdatasync per batch) is not paid
  by this leg at all — it measures what a sync-mode hydrate would pay.
  RFC-0041's fd-ceiling write-per-op claims apply to G1-default writes
  (kernel `OpenOptions.sync=true`), not to this bench's explicit
  `sync=false`. Never quote the probe floor against this leg again.
- The remaining ≥10 240 posix fdatasyncs (48.1 s) are SST/flush-side
  per-file syncs (table.rs:2371, one per finished SST) plus manifest —
  engine-internal durability, amortizable by overlapping jobs, not a
  per-batch barrier.
- Therefore **the 0.16–0.18× hydrate gap is pure engine work and fully
  attackable**: sequential drain on one compat worker (26 flushes 66 s +
  164 jobs 165 s, sum 278.8 s vs 213.2 s wall — writer parks behind the
  debt, `parked_b=268 609 536`), write-amp 2.15× (10.85 GiB written for
  a 5.15 GiB settled set), at ~110 MiB/s single-writer flush throughput.

Next lever (v21o): **across-job** parallelism inside core
`compact_leveled` — prepare K pairwise key-disjoint jobs, write them
concurrently (`std::thread::scope`), install sequentially (file numbers
pre-allocated atomically); optionally the same machinery for flush
encode in `write_imm_l0_files`. Sizing requires the guest's aggregate
write ceiling (single- vs multi-stream), still unmeasured.

## v21o — across-job disjoint-batch compaction: NO-GO on this guest (run #18)

**Guest write ceiling measured (run #18 boot, `writeprobe.py`, 512 MiB
O_DIRECT per stream on /data/stores):**

| streams | per-stream MiB/s | aggregate MiB/s |
|---------|------------------|-----------------|
| 1       | 381              | 381             |
| 2       | 227 + 227        | **454**         |
| 4       | 113 × 4          | **453**         |

The guest's aggregate write ceiling is ~453 MiB/s — only **1.19×** the
single-stream 381 MiB/s. But the drain never got near either number: run
#16/#17 measured **~110 MiB/s per compaction job** (avg 1.0 s/job) and
~110 MiB/s per flush. A single job uses less than a third of one
stream's raw bandwidth — the per-job bottleneck is merge/encode CPU or
write pattern, not disk. Four concurrent jobs demand ~440 MiB/s, just
under the ceiling: if per-job throughput holds under concurrency, the
164.8 s of compaction could compress toward ~40 s.

Core lever on the barrier-free gap. `Db::compact_leveled` now forms a
**batch of up to K pairwise key-disjoint pushdown jobs** from the first
over-target (level, family), writes them concurrently through the
`ParallelMerge` seam (`merge_jobs`: `std::thread::scope`, one thread per
job, each job merged sequentially), then installs them sequentially —
`apply_prepared_l0_compact` is path-based, so installs of disjoint jobs
commute exactly like today's.

- Knob `PEDRA_PARALLEL_JOBS` (read once, clamp 1..=8, default 1 = off).
  Needs the seam installed: the host open path
  (`open_with_env_bounded`) installs `ParallelMergeEnv` when **either**
  knob is on. `PEDRA_PARALLEL_MERGE` (within-job spans, run #17 no-go)
  is now gated independently and stays off by default — the two
  dimensions are orthogonal.
- Disjointness rule (`prepare_disjoint_pushdown_batch`): walk the source
  view oldest-first; a candidate joins the batch only if its **combined
  input hull** (source ∪ overlapping destination files) stays clear of
  every already-claimed hull, shared boundary counting as overlap (same
  rule as `leveling::is_disjoint`). Hull disjointness gives both
  required properties: no shared input file (a wide destination file
  spanning two sources is absorbed by whichever job claims it first) and
  disjoint output ranges at the destination (the leveled invariant).
  `max_jobs = 1` delegates to the original single-job picker — identical
  behavior with the knob off.
- File numbers: `build_prepared` burns a per-job chunk range in prepare
  order, so concurrent writers own disjoint ranges (the v5 double-install
  clobber cannot happen). L0→L1 jobs stay one-at-a-time (they absorb the
  newest flush and the shared L1 slice).
- Lock discipline unchanged: `ConcurrentDb::compact` holds the write lock
  across `compact_leveled` as before; the K job writes overlap each other
  inside that hold, so the writer's park shrinks from K×job to ~max(job).
- Core test `parallel_jobs_batch_disjoint_and_correct`: batch fills the
  requested width, hulls pairwise disjoint, a full drain through the real
  `ParallelMergeEnv` (scoped threads) keeps every key readable, every
  level a disjoint run set, and the MANIFEST reopens. Core suite after
  v21o: 659 passed / 1 failed (`catchup_wait_bounded_by_half_fd`,
  pre-existing, concurrent-session-flaky).
- Guest run #18 entrypoint: `PEDRA_PARALLEL_JOBS=4` (arm under test),
  `PEDRA_PARALLEL_MERGE` export dropped, diag exports unchanged, plus
  `writeprobe.py` (1/2/4 streams × 512 MiB O_DIRECT on /data/stores) to
  measure the guest's aggregate write ceiling in the same boot as the
  bench — the sizing number the lever was blocked on.

### Run #18 verdict (raw serial `run18-25m-paralleljobs4.txt`)

hydrate 161.2 s (band 143.7–158.5 unchanged — the lever never engaged
during hydrate: the L0-first loop arm keeps ingest compaction single,
and one family cannot batch L0→L1 jobs that share the L1 slice);
**settle 76.4 s** (band 47.9–54.7) — no win, slight loss.

- Batches engaged only at settle: 7 batches (4+3+4+4+4+4+4 = 27
  pushdown jobs), walls 4.24–5.25 s each — **exactly ~4× a single job**
  (sequential pushdowns avg ~1.0–1.2 s). Perfect timesharing: the four
  scoped threads got zero aggregate speedup.
- This boot's settle also carried a 17.7 s whole-L0 monster
  (`COMPDUR ms=17654 inputs=5` — hydrate left L0 6×1.02 GiB, vs
  2×53.6 MiB at run #15's settle entry; slow-boot shape variance).
  Ex-monster settle ≈ 58.6 s — still no better than sequential 54.7 s.
- **Why: the guest compacts on effectively one core.** A single job
  drains ~110 MiB/s of a 381 MiB/s single-stream disk (jobs are CPU-
  bound, not disk-bound); four concurrent CPU-bound jobs timeshare one
  core → 4× wall. Same signature as the writeprobe's CPU-issuing
  threads scaling anyway, and opposite to its I/O-bound scaling.
- **Lever closed as no-go on this guest** (a multi-core guest is the
  seam's real home). `PEDRA_PARALLEL_JOBS` stays default-off; the code,
  disjointness rule, and test stay (core suite 659/1-pre-existing).
- **The sharpened target for hydrate/settle:** per-job throughput
  110 MiB/s vs 381 MiB/s single-stream disk = 3.5× headroom INSIDE one
  job, and flush encode hits the same ~110 MiB/s (26×2.68 s for 256 MiB
  each). The shared bottleneck is the SST write path's per-byte CPU
  (decode/merge/encode/CRC + per-file fsync ~18 % of wall), not
  concurrency and not the disk. Next levers: cheaper encode per byte in
  `table.rs` (mine) and/or write-amp reduction (2.15×: 10.85 GiB written
  for 5.15 GiB settled) via level-target/slice-cap tuning.

## Guest run #24 (v23 REPEAT — post-zombie recovery run, 25M) — run #23 CONFIRMED leg-for-leg; stale gate injectors deleted

The pre-compaction zombie task fired the old v21p injector on the gate after
run #23, clobbering the v23 image (entrypoint lost `PEDRA_BULK_DIAG`).
Recovery re-ran `/tmp/inject-v23-gate.sh` — MD5_OK ×5, entrypoint restored
(BULK_DIAG on, PARALLEL_JOBS dropped) — and the bench that restart launched
doubles as a run #23 repeat. Full capture:
`run24-v23-repeat-25m.txt`.

- **hydrate 72.5 s** (#23: 73.6) = 0.35× rocks 25.3 s; **settle 3.0 s**
  (#23: 2.3) = **2.77× rocks 8.3 s** — the write-leg crossing at 25M is
  stable at 2.8–3.6×, Pedra still fdatasyncing before Ok. 73 BULKDIAG
  (72 parked + 1 flush) — identical funnel mix to #23.
- Read legs, repeat vs #23 (rocks): get_hit 48.47 vs 47.01 µs (38.59) =
  0.80×/0.82×; prefix_scan 454.7 vs 457.1 µs (305.6) = 0.67×/0.67×;
  get_loop 4.4922 vs 4.9627 ms (3.38) = 0.75×/0.68×; multi_get 4.6737 vs
  5.1830 ms (3.70) = 0.79×/0.71×; probe_hit 47.2 vs 54.9 µs (45.4) =
  0.96×/0.83×; probe_miss 2.7 µs ≈ 1×. Ranked remaining gaps at 25M:
  **hydrate 0.35× (commit CPU, RFC-0159 P1.1) > prefix_scan 0.67× >
  get_loop 0.75× > multi_get 0.79× > get_hit 0.80×** > probe_hit ~parity.
- **probe_hit arbitration closed:** #13 33.9 / #19 52.6 / #23 54.9 /
  #24 47.2 µs vs rocks 45.4 — the leg swings up to 60 % run-to-run; v23
  sits at 0.83–0.96×. Treat as parity-within-noise, stop tracking it.
- Gate hygiene: `inject-v21p-gate.sh`, `inject-v22-gate.sh`,
  `inject_v21h/v21k..v21o.sh` deleted from the gate; only
  `inject-v23-gate.sh` + the `.v23` staging files remain.
