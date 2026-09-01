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

## v36+ guest read-leg series (ratio = rocks/pedra, ≥1.0 = pedra wins)

Runs v34–run6 code/identity verified by md5 at injection and, from v36 on,
by the probe_miss signature (L2 bisect ≈ 1.8–2.0 µs p50; pre-L2 linear walk
2.7–2.8 µs). v37c control is pre-L2 v31 db.rs (`0381bf328ca7385a86293f694baf1a6d`)
run 2026-09-01 in the same day's window, forced-rebuild verified.

| run                     | code | load | get_hit              | prefix_scan | get_loop             | multi_get            |
|-------------------------|------|------|----------------------|-------------|----------------------|----------------------|
| v34 baseline            | old  | 15.0–16.1 | 0.935 (39.8/37.2) | 0.697       | 0.993 (3802/3775)    | 0.915 (4048/3705)    |
| v35 scandiag            | old  | ~15  | 1.066 (36.9/39.4)    | 0.614       | 0.912                | 0.975                |
| v36 r1                  | L2   | 15.7 | 0.749 (52.3/39.1)    | 0.639       | 0.891 (4527/4034)    | 0.923 (4799/4428)    |
| v36 r2                  | L2   | 15.05| 0.846 (44.5/37.7)    | 0.644       | 0.984 (3769/3707)    | 1.117 (3587/4009)    |
| v36 r3                  | L2   | 16.4 | 1.027 (42.6/43.8)    | 0.710       | 1.074 (3782/4063)    | 1.150 (3994/4592)    |
| v36 r4                  | L2   | 16.6 | 1.028 (37.9/38.9)    | 0.593       | 0.984 (3823/3760)    | 0.866 (4116/3566)    |
| v36 r5                  | L2   | ~17  | 0.969 (38.4/37.2)    | 0.665       | 0.956 (4261/4072)    | 0.835 (4203/3510)    |
| v37 "control" (mislabeled) | L2 | ~17  | 0.850 (43.4/36.8)    | 0.673       | 0.892 (4001/3571)    | 0.922 (3958/3651)    |
| v37c true control       | old  | 15.3 | 1.028 (45.1/46.4)    | 0.654       | 1.008 (4389/4425)    | 0.927 (4625/4288)    |
| v38 r1 (L2 restored)    | L2   | 15.5 | 1.041 (43.5/45.3)    | 0.676       | 0.835 (4856/4053)    | 1.066 (4179/4455)    |
| v38 r2 (reboot-only)    | L2   | 16.2 | 0.996 (45.7/45.5)    | 0.672       | 1.069 (3754/4011)    | 1.012 (4399/4451)    |
| v38 r3 (reboot-only)    | L2   | 16.0 | 1.119 (41.2/46.1)    | 0.593       | 1.110 (4081/4528)    | 0.966 (4519/4365)    |

(pedra µs / rocks µs in parens; captures `guest-v36-l2-25m-run{1..5}.txt`,
`guest-v37-mislabeled-l2-run6.txt`, `guest-v37c-true-v31-run1.txt`.)
Hydrate 51–56 s, settle 1.1 s (bulk intact), BULKDIAG normal, no read-verification
errors in any run. **probe_miss pedra 2.7–2.8 → 1.9–2.0 µs (−30 %) in every
L2-code run** — the walk cut is real on-guest; multi_get medians 3.59–3.99 ms
vs 4.05–4.23 (v34/v35) — improved; get_loop medians 3.77–3.82 (r2–r4) vs
3.80–4.19 — improved. prefix_scan unchanged (0.59–0.71 band) — compat-bound
per the v35 SCANDIAG decomposition.

### v37 failed control → v37c hardening (injection-methods postmortem)

The first v37 control attempt produced numbers but was **not** a control —
two compounding failures: (1) the gate clock runs ~3 h behind the guest, so a
plain `cp` gives the swapped file an mtime older than the guest-built cargo
artifacts and cargo skips the rebuild ("Finished in 0.38 s"); (2) the nbd
writeback was lost at `qemu-nbd -d` disconnect under host load 17 — the
in-mount md5 verified from page cache but the qcow2 never got the blocks
(image still held v32; probe_miss 1.8 µs in that run = L2 signature, proving
old code never ran). Fixes, now standing procedure for every injection:
future-date the swapped mtime (`touch -d "$(date -u -d '+1 day' …)"`),
`sync`, disconnect, **reconnect and re-verify md5 persisted** before boot, and
require both a `Compiling pedradb-core` serial line and the probe_miss
signature before trusting any run. The mislabeled capture is kept as a 6th
L2-code sample (`guest-v37-mislabeled-l2-run6.txt`).

### v37c true-control verdict: the get_hit ratio is window-dominated

v31 old-walk code, forced rebuild (Compiling pedradb-core, 26.6 s), forced
persistence, run 2026-09-01 19:27 UTC at gate load 15.3:
**get_hit 1.028 / get_loop 1.008 / multi_get 0.927** (probe_miss 2.8 µs —
old-walk signature). The old code passes get_hit/get_loop in a window where
L2 code had failed 1 h earlier (0.850 at load ~17), and it does so with the
slowest rocks get_hit of the whole series (46.4 µs vs its own 36.8–43.8 band)
while pedra sits in its usual 43–45 µs band. Attribution: **the point-leg
ratios swing ±15–25 % with the host window on both code versions** (rocks'
own get_hit moved 37.2→46.4 across the series; pedra 36.9→52.3; bands for old
vs L2 code overlap almost completely). L2's −1 µs/op walk cut (~2–3 % of the
op) is real but invisible at this noise level. Consequences: (a) run 3's
all-three-pass is genuine but not separable from window luck — the two-run
protocol needs a second pass and honest margin reporting, not attribution to
L2; (b) a quiet window is a prerequisite for any decisive acceptance run;
(c) the fat probe p999 tails (2.1–2.2 ms) in the v36-era runs vs 12 µs in
the v37c control are a window artifact of that churny hour, not an L2
regression (probe_miss p999 14–20 µs appears in both code versions' calmer
runs).

## Session verdict (2026-09-01): point legs at parity-within-noise; goal NOT closed

Nine L2-code runs (v36 r1–r5, mislabeled run6, v38 r1–r3): **all three
point legs passed simultaneously exactly once (v36 r3: 1.027/1.074/1.150)**.
Every leg passes individually in 4–5 of 9 runs and the failing leg rotates
(r4 multi_get, v38 r1 get_loop, v38 r2 get_hit −0.4 %, v38 r3 multi_get).
L2-run medians:
get_hit 0.996, get_loop 0.984, multi_get 0.966 — each within the ±15–25 %
window band of 1.0, none ≥ 1.0 on median. Old-code medians (3 runs):
get_hit 1.028, get_loop 0.993, multi_get 0.927 — overlapping bands, and
multi_get is the one leg where L2 runs hold the only ≥1.0 passes (1.117,
1.150, 1.066, 1.012 vs old-code max 0.975).

What is settled and shipped (commit e3c5750): L2 run bisect is correct
(oracle live+reopen, suite 688 pass / 2 known flakes), cuts pure-CPU get_hit
−24.5 % locally (5.83 → 4.40 µs) and probe_miss −30 % on guest
(2.7–2.8 → 1.6–2.0 µs), bulk route intact (BULKDIAG level=3, ssts 29–86,
settle 1.1–1.3 s), zero read-verification errors across every capture.

Why the goal is not marked done: the two-run all-four acceptance is unmet —
prefix_scan is 0.59–0.71 and blocked on compat per-row decode (v35 SCANDIAG:
~83 % of the op; concurrent-session files), and the point legs' single-run
pass flips with the host window (v37c control attribution). Per the plan's
own bar, a scrape-by 1.01× that flips on noise fails; farming reboots for a
second lucky pass would be recording window luck, not performance.

Residual and next ranked levers (all core-owned unless noted): (1) in-probe
costs — 44 % of get_hit CPU per the `sample` decomposition: bloom filter,
per-access CRC + 4 KiB lz4 on resident payloads (verified-block residency
bitmap must preserve the CRC fail-closed contract — audit before shipping);
(2) compat wrappers 13 % (concurrent-session files); (3) a genuinely quiet
gate window (< load 12) for a decisive two-run capture — the entire day ran
load 15–19 with rocks' own legs swinging 36.8–46.4 µs.

## prefix_scan: zero-copy scan pages (compat lib.rs, 2026-09-01 evening)

User unlocked the compat territory (concurrent session idle 14 h, only a
cosmetic uncommitted api.rs edit — untouched). Local macOS `sample` of the
25 M/256 MiB prefix_scan arm (`sample-scan-pedra-2.txt` in SCRATCH,
893 in-arm samples): **~24 % pure allocator churn** — `page_forward`'s
`codec.decode(...).to_vec()` + `row.value.to_vec()` = 2 mallocs + 2 copies
per row × 333 rows/op, all under nanov2/malloc_zone; ~40 % core merge
machinery (BinaryHeap of owned keys, per-advance `Bytes` clones, `skip_user`
clone per emitted row); remainder stage-common (utf8 + `decode_entry`,
shared with rocks and cancels). Guest amplifies this: guest pedra scan
585 µs vs local 251 µs (2.3×) while rocks only 1.6× — allocation-heavy code
scales worst on the slow shared guest core.

The cut (one variable, compat `lib.rs` only): `WindowKv` was already
`(Bytes, Bytes)` — so refill pages now hand out **`Bytes` handles sliced
from the cached blocks** (`KeyCodec::decode_bytes` = the same suffix slice
`decode_cf_key` does, on a handle; short-key parity with `unwrap_or(&[])`
kept + unit test `key_codec_decode_bytes_matches_decode`).
`DBIterator::items: Vec<(Bytes, Bytes)>`, `Iterator::Item =
Result<(Bytes, Bytes)>` (the stage's only consumer derefs — `starts_with`,
`from_utf8`, `decode_entry` — compiles unchanged), `collect_rest` keeps its
owned-`Vec` signature. Per row: 2 refcount bumps instead of 2 mallocs +
2 memcpys. `page_last_n` ring likewise.

Local 25 M/256 MiB A/B: **pedra prefix_scan 250.78 → 135.72 µs (−46 %),
ratio 0.919× → 1.497×** (rocks 230.4 → 203.0 µs same-machine drift band;
`local-256m-scan-bytes-new1.log` vs the sampled run, SCRATCH). Compat suite:
84 pass / 1 pre-existing env failure (`block_cache_usage_is_bytes_not_hits`
fails identically at HEAD with my lib.rs reverted — recorded, not mine).


## get_hit: verified-block residency bitmap (core, 2026-09-01 night)

Root cause of per-probe CRC cost: every point probe on a resident payload
re-ran crc32c over the whole ~4 KiB block body (sample `sample-gethit-pedra-2.txt`:
crc32c 579 of 3017 in-arm samples ≈ 19 % of the op; `seek_point_in_block_image`
subtree ≈ 887). RocksDB's block cache carries the same contract structurally:
verify once on the cache-miss path, trust afterwards.

The cut (one logical variable, core files): `PayloadSlot` now holds
`ResidentBody { img: Arc<[u8]>, verified: Option<Box<[u64]>> }` — one lazy
bitmap per resident image; `point_at_seeking`'s resident branch serves a
marked block via `seek_point_in_block_body` (lz4 + walk, no CRC), and the
first probe verifies + marks under the write guard. Any payload write
installs fresh empty marks, so invalidation is structural (no epochs, no
ABA) and the CRC fail-closed contract is preserved — `from_eager_entries`,
rot tests, and eviction all re-verify. File-path reads (non-resident
blocks) still verify every time, fail-closed. New test
`point_seek_verified_marks_skip_crc_and_invalidate_on_swap` covers
first-probe-marks / repeat-skips / swap-invalidates / rotten-swap-fails.

Local 4 M / 8 GiB get_hit A/B, strictly alternating N C N C N C (calm load
4.6–6.4; `ab2-bitmap-*.log`, SCRATCH): pedra medians **new 2.647 / 2.666 /
2.704 µs vs ctl 2.881 / 2.925 / 3.027 µs → −9.2 % mean, clean in 3/3
pairs** (every new run beats every ctl run). Rocks same runs 2.49–2.60 µs
(local ratio ≈ 0.85 → ≈ 0.96). An earlier 4-run batch under load 7–12 was
noise-dominated (one new run at 3.215) — alternating pairs under calm load
is the reliable local protocol. Suite: 689 pass / 2 documented flakes
(`catchup_wait_bounded_by_half_fd`, `maybe_auto_flush_physical_cf_is_not_linear_in_keys`
— the latter load-sensitive). Counter exposed as `ReadProbeSnap.blocks_crc_skipped`.

## prefix_scan: index-heap borrowed-head merge (core merge.rs, 2026-09-01 night)

Old merge: `BinaryHeap<HeapItem>` sifted an owned `{InternalKey, Bytes,
usize}` struct through every level per row (push+pop each ~log2(streams)
copies of two refcounted handles + drop glue), on top of the per-advance
owned handoff from each stream. Rewrite: the heap now sifts **plain
`usize` stream indices**; owned head rows sit in a side `heads` vec and
move only when emitted. Ordering replicates `InternalKey` Ord exactly
(user_key asc, newest seq first, kind desc); `heapify` once at start,
sift-up on refill. Public API unchanged (merge 27 + scan 35 tests green,
incl. `multi_level_large_scan_matches_model`; suite 689 pass + the two
documented flakes).

Local 25 M/256 MiB prefix_scan A/B, alternating N C ×3 (calm load 5.5–6.4;
`scanheap-*.log`, SCRATCH): pedra medians **new 122.38 / 124.66 / 116.73 µs
vs ctl (d8125f8) 139.33 / 136.69 / 137.14 µs → −12.0 % mean, clean 3/3**
(rocks 199–216 µs tight; local ratio ≈ 1.44–1.58 → ≈ 1.65–1.74).

Guest v40 (bitmap) 6-run series same night, gate load 15–18 (worst window
yet; v37c applies to point legs): get_hit median ratio **0.923** (pedra
45.9 vs rocks 42.8 µs), get_loop 0.958, multi_get 0.953, prefix_scan
**0.784** (pedra 461.3 vs rocks 356.8 µs, tight 0.759–0.817 — scan is
compute-bound and code-responsive). Bitmap verdict on the guest: ~−2 %
get_hit — expected: 1 GiB payload budget / 3.5 GB data ⇒ ~29 % resident
share of a ~1–2 % CRC slice of the 46 µs guest op; the −9.2 % local win
is the fully-resident (8 GiB) regime. Keep: correct, fail-closed, free.
probe_miss p50 1.9 µs = L2-class signature on all runs. Write path
unchanged (hydrate 51.8 s, settle 1.1 s).
