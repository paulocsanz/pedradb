# Guest v34 read-leg baseline (image v33, both backends, 25M)

Date: 2026-09-01. Injection v34 = entrypoint-only change (one variable):
`SLIPSTREAM_BACKENDS=pedradb` → `rocksdb,pedradb`; all 8 core-file md5s
verified unchanged from run #34's v33 image. Cache 256 MiB both backends
(`SLIPSTREAM_BENCH_CACHE_BYTES=268435456`), entries 25M, filter
`get_hit|prefix_scan|lookup_100`, one process. Gate load 15.0–16.1
(uptime captured in `guest-v34-readlegs-baseline-25m.txt`). Capture:
`guest-v34-readlegs-baseline-25m.txt`.

| leg                   | pedra     | rocks     | ratio | prior baseline |
|-----------------------|-----------|-----------|-------|----------------|
| get_hit               | 39.80 µs  | 37.22 µs  | 0.935 | 0.80           |
| prefix_scan           | 506.61 µs | 353.29 µs | 0.697 | 0.67           |
| lookup_100 get_loop   | 3.8025 ms | 3.7752 ms | 0.993 | 0.75           |
| lookup_100 multi_get  | 4.0477 ms | 3.7046 ms | 0.915 | 0.79           |

Write-path sanity in the same capture: hydrate/pedra 51.9 s, **settle/pedra
0.9 s** (bulk install intact, ≤3 s bar), disk 5.15 GiB; rocks hydrate 30.5 s,
settle 8.8 s, 5.24 GiB. `BENCH_EXIT_readlegs=0`, no read-verification errors.

Probes (guest): probe_hit pedra 42.7 µs p50 vs rocks 52.1 (pedra ahead);
probe_miss pedra 2.7 µs vs rocks 0.66 µs (linear 86-SST walk vs level binary
search — not a goal leg, but confirms the walk cost).

Analysis (regime split, local `local-25m-readlegs-256mib.log` + 1 GiB runs):
- Point legs: both backends pay ~1 pread/op at 5 % residency; pedra's deficit
  is per-op CPU — per-access CRC+lz4 on resident payloads, linear SST walk,
  per-get copies. Levers (core files): verified-block bitmap (CRC once per
  residency epoch), per-level binary search, copy trims.
- prefix_scan: one service ≈ 250 rows ≈ 56 KiB ≈ 14 pedra blocks (4 KiB) vs
  ~4 rocks blocks (16 KiB + readahead) → ~10 extra preads/scan ≈ the observed
  150 µs gap. Local runs are page-cache-warm (pedra 1.09–1.18× locally) — the
  gap is guest-I/O-specific. Lever (core files): sequential block
  batching/readahead in the core scan cursor. Compat iterator per-row decode
  remains a second lever (concurrent-session files — coordinate).
- multi_get additionally pays compat loop overhead (+3.2 µs/get vs rocks'
  coalesced MultiGet — rocksdb-compat files).

## v35 scan-diag decomposition (same v33 code + `PEDRA_SCAN_DIAG=1`)

Second guest run, identical point-path code (capture
`guest-v35-scandiag-25m.txt`, load ~15): get_hit **1.066×** (36.9 vs 39.4 µs),
prefix_scan 0.614× (585 vs 359 µs), get_loop 0.912×, multi_get 0.975×. The
get_hit flip 0.935 → 1.066 across identical-code runs puts guest leg variance
at ±10 % at load 15 — margin is mandatory, two-run protocol stands.

Steady-state `SCANDIAG` on the pedra prefix_scan leg: **cache_misses/op =
0.00** (scan fully block-cache-hot), cache_hits/op = 22.3, streams/op = 2.0,
setup_ns/op ≈ 6.1k, **row_ns/row ≈ 0.24 µs** over 333 rows/op. Core scan cost
≈ 6.1 µs + 333 × 0.24 µs ≈ 88 µs of the 506–585 µs op (~15–17 %). The
remaining ~83 % is compat-iterator page decode (`codec.decode + to_vec` ×2 per
row in `page_forward`) + stage `decode_entry` (backend-common). The
block-size/readahead theory for scans is dead: misses are zero. Closing
prefix_scan requires compat-side per-row cuts (concurrent-session files);
core-side zeroing cannot recover the 150–230 µs gap.

Local all-resident regime (8 GiB caches, `local-25m-readlegs-8gib-resident.log`):
pedra get_hit pure CPU 5.83 µs (rocks' 8 GiB cache-fill pathology made its arm
unusable — 225 µs — so rocks' CPU reference stays ~1.5 µs est.); prefix_scan
CPU-near-parity 273 vs 258 µs (0.94×). Guest point-leg deficit is therefore
CPU-stack (walk + CRC/lz4 + compat per-get) exposed once both backends pay
guest preads.

## get_hit CPU decomposition (macOS `sample`, leg warmup, 8 GiB resident)

`local-8g-gethit-cpu-sample.txt` (3×5 s samples; sample 1 landed in the
pedradb get_hit warmup, 3868 in-loop samples of 4000):

| layer                                              | samples | share of loop |
|----------------------------------------------------|---------|---------------|
| `Db::lookup` total                                 | 2987    | 77 %          |
| — `SstTable::point_at_seeking` (bloom + CRC+lz4)   | 1711    | 44 %          |
| — walk/arbitration outside the probe               | 1276    | 33 %          |
| compat `get_cached` + `ConcurrentDb::get` wrappers | ~514    | 13 %          |
| bench kv decode                                    | ~286    | 7 %           |

The linear 86-table walk plus per-table tombstone checks are a full third of
the pure-CPU op — the largest core-side, non-probe chunk. Levers ranked:
L2 per-level bisect (the 33 %), then in-probe costs (44 %: bloom, CRC,
per-access 4 KiB lz4 — payload-cache residency question), compat wrappers
(their files).

## L2 run bisect (db.rs, this session)

`SstRun { level, tables_newest_first, disjoint_by_lo }` built in
`rebuild_sst_order` (single funnel, 14 call sites + recovery). `lookup()`
walks runs level-ascending (same order as the flat walk); per run: linear
range-tombstone pass over all tables (superset of the old up-to-winner
collection; inert for older tables since `range_deleted` hides only strictly
newer points), then `partition_point` on `disjoint_by_lo` (last table with
`lo <= key` is the only candidate; probe-side bounds re-check kept as
fail-safe). Non-disjoint/unbounded runs keep the linear walk. Oracle test
`lookup_bisect_disjoint_run_matches_linear_walk` compares the bisect against
a verbatim copy of the pre-L2 flat walk + a ground-truth model over chunk
keys, boundary/gap keys, and range-tombstone spans, live and post-reopen.

Local A/B (interleaved stage-binary vs l2ab build, 25M, 256 MiB, pedra
medians µs; `ab-l2-*.log` in SCRATCH): get_hit old 5.69/5.29/6.27 vs new
5.42/230.53(anom)/5.73; get_loop old 571/531/540 vs new 666/9782(anom)/532;
multi_get old 579/545/566 vs new 610/9381(anom)/544. The new-2 arm collapsed
for pedra only (rocks normal in-process) — OS page-cache loss on the
cacheless pedra point path under machine pressure, not L2: the bisect probes
the same single candidate table and the tombstone pass reads nothing.
Controlled pair new-3/old-3 (post-anomaly): new better on every pedra point
median. Same-process ratios ≥ 1.0 in all healthy arms.

**CPU cut quantified (8 GiB all-resident, get_hit-only): pedra 5.83 µs →
4.40 µs = −24.5 % per get** (`local-8g-gethit-l2.log` vs
`local-25m-readlegs-8gib-resident.log`; ~1.4 µs of walk removed, matching
the 33 % sample attribution). Suite: 688 pass / 2 known flakes. Guest
injection v36 (db.rs v32 = `a16b50b7c2cfc0b6a20f0c31a5e03b76`, all other
files + entrypoint v35 verified unchanged, swap + start 2026-09-01 18:48 UTC
at gate load 15.7).

## v36 guest runs (L2 live, same v35 entrypoint)

| leg        | v34    | v35    | v36 r1 (load 15.7) | v36 r2 (15.05) | v36 r3 (16.4) |
|------------|--------|--------|--------------------|----------------|---------------|
| get_hit    | 0.935  | 1.066  | 0.749 (52.3/39.1)  | 0.846 (44.5/37.7) | 1.027 (42.6/43.8) |
| prefix_scan| 0.697  | 0.614  | 0.639              | 0.644          | 0.710         |
| get_loop   | 0.993  | 0.912  | 0.891 (4527/4034)  | 0.984 (3769/3707) | 1.074 (3782/4063) |
| multi_get  | 0.915  | 0.975  | 0.923 (4799/4428)  | 1.117 (3587/4009) | 1.150 (3994/4592) |

(pedra µs / rocks µs in parens; captures `guest-v36-l2-25m-run{1,2,3}.txt`.)
Hydrate 51–56 s, settle 1.1 s (bulk intact), BULKDIAG normal, no read-verification
errors. **probe_miss pedra 2.7–2.8 → 1.9 µs (−30 %)** — the walk cut is real
on-guest; multi_get medians 3.59–3.99 ms vs 4.05–4.23 (v34/v35) — improved;
get_loop medians 3.77–3.78 (r2/r3) vs 3.80–4.19 — improved. get_hit medians
42.6–52.3 vs 36.9–39.8: dominated by a churny host window (rocks' own get_hit
swung 37.2→43.8 across the same runs; probe p999 2.1 ms vs v35's 258 µs;
gate load drifted 15.0→16.6 during the series). Run 3 passed all three point
legs simultaneously. prefix_scan unchanged (0.61–0.71 band) — compat-bound
per the v35 SCANDIAG decomposition.
