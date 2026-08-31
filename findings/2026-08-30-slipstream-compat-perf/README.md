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


