# P1.3 hydrate regression: two mechanisms, separated by counters (2026-09-01)

Run #27 (v25 = threshold fix `aa52dd0`, 25M guest): 23×256 MiB chunks as
designed, but hydrate 116.0 s vs v24 75.6 s / v23 72.5–73.6 s (+54%).
Run #27b (repeat, same image, same loaded host): 120.5 s — regression real
and reproducible.

WRITEPHASE #27: `commits=24415 prepare_ms=1813 wal_ms=6467 mem_ms=9130
publish_ms=10 flush_check_ms=21474 lock_wait_ms=0` vs v24
`flush_check_ms=18.7`.

## Mechanism (a): `MemTable::take_family` reinsert loop — 21.5 s, measured

`maybe_auto_flush` (commit tail, Db write lock held, counted by
`flush_check_ns`) parks a CF over its per-CF limit via
`mem.take_family(&fam)`. Pre-P1.3 the data CF never reached its 256 MiB
limit in the active mem (worker staged at the 64 MiB global first), so the
reinsert loop never ran for data — hence v24's 18.7 ms.

`take_family` (memtable.rs, pre-v27) partitioned by iterating every key
and **reinserting** it into a fresh `BTreeMap` (new node allocations),
then `recount()` on both sides. At 256 MiB ≈ 2.6 M keys ≈ 0.9 s per chunk
at guest-core speed × 23 chunks ≈ the measured 21.5 s.

v27 fix: CF keys are `cf\0user`, so a NUL-free prefixed family is ONE
contiguous BTreeMap range `[F\0, F\x01)`. Partition with two `split_off` +
one `append` (whole-node moves, O(log n)), exact stats for the taken table
in one pass (single cf prefix — no per-version `cf_bytes` map lookups),
keeper keeps its incremental counters minus that pass. `tail_max_seq` of
the keeper may stay at the pre-take max — it only gates iteration strategy
and the stale value errs toward the generic iterator. `default` family
(raw keys interleave with prefixed keys) keeps the loop.
Test: `take_family_contiguous_partition_exact` (membership, byte/version/
tombstone conservation, boundary keys `route` / `route\0` / `route2\0…`,
post-take incremental counters).

## Mechanism (b): flush-debt sleep ping-pong — ≈33 s dead wall

With `flush_debt_cap` = one 256 MiB chunk, every park lands debt AT the
cap → the next submit's `await_flush_debt` sleeps in 2 ms polls until the
worker's 20 ms tick picks the table up and materializes it (1.46 s at
guest speed). Writer sleep ≈ 23 × 1.46 s ≈ 33 s of dead wall (unattributed
by any counter — `await_flush_debt` is not instrumented; visible only as
hydrate wall − CPU sums). On one effective core the sleep is zero-sum CPU
but serializes fill and materialize at the wrong boundary (no pipelining:
cap = exactly one chunk).

v26 fix: `ConcurrentDb::assist_flush_debt` — at submit entry, if a flush
worker is attached and debt ≥ cap, the writer materializes one parked
table inline (`materialize_parked_once`) before submitting. Called at all
five `WriteGroup::submit*` sites (put / delete / delete_range /
apply_batch_vec / apply_batch_occ_with). `await_flush_debt` stays as the
bounded fallback. Tests: `submit_flush_debt_assists_with_worker_attached`
(unattached: straight through AND no assist; attached: parked drops to 0
with no worker thread — the sleep path would wait out
`PEDRA_FLUSH_DEBT_MAX_MS` and leave the debt parked),
`submit_flush_debt_releases_on_materialize` (writer-assist vs worker race,
single-flight flush lock, no deadlock).

Deliberately NOT done in v26: raising the cap to 2× threshold
(pipelining). One lever at a time; it is the follow-up if v26+v27 leave
hydrate above the v23 band.

## Attribution model (updated)

hydrate ≈ materialize (worker/writer CPU) + commit (wal+mem+prepare) +
take_family (flush_check counter) + sleeps/overhead. v25: 33.6 + 17.4 +
21.5 + ~44 = 116. Expected v26+v27: ~33.6 + 17.4 + ~0–3 + sleep≈0 →
55–75 s band (guest core speed decides where in the band).

## Reads: NOT a chunk-count effect

v25 reads did not recover with 23 files vs v24's 88 (both degraded vs the
v23 family). Local paired A/B (6M and 25M, v23 `9698caf` vs v24 `0ecd79e`
stage clones, ×2 each, warm): get_hit 4.04–4.22 µs both, prefix_scan
188–193 µs both — no code regression in P1.1. Gate host was saturated
during v24/v25 (load 34, six qemu ~200% each, caixote-api 249%): guest
read legs are untrustworthy until it quiets. Clean-host re-baseline of
read legs is required before any read claim.

## Run #28 (v26+v27, guest 25M, same loaded host): sleep theory REFUTED

hydrate 110.5 s (v25 116.0/120.5), settle 3.5 s, 23 chunks, exit 0.
WRITEPHASE: `prepare 1828 / wal 6591 / mem 9418 / flush_check 11921 /
publish 13 ms` — v27 removed the reinsert loop (21474 → 11921). But
hydrate moved by EXACTLY the flush_check delta and nothing more:
v26's assist-drain removed the sleeps and the wall did not follow.
Accounting: commit 29.7 + materialize (FLUSHSTAGES) 35.0 = 64.7 s
counted vs 110.5 s wall → **45.8 s unattributed, ≈ unchanged vs v25**.
v24 (64 MiB chunks) had only ~19 s unattributed → the residual scales
with CHUNK SIZE, not with sleep/scheduling. Mechanism (b) above is dead;
the ~26 s delta between eras is something else (candidates: memory
footprint/page reclaim on the 3.9 GB guest — a 256 MiB chunk is a ~2.4
M-entry BTree ≈ 600+ MB real RSS; retire-cache lifetimes; file-size
effects). Local mac 25M RSS sampling queued to bound the footprint.

The remaining 11.9 s flush_check is `spill_tail()` inside take_family:
each entry must land in a BTree exactly once; the tail defers that cost
to take time, and the SST write would pay the same inserts if the tail
were extracted instead — a shard-extraction "fix" only RELOCATES the
cost (v24 paid it inside `enc_ms`, invisible to flush_check). Not worth
code.

Local 6M cross-check (v26+v27, quiet mac): hydrate 11.2 s, settle 2.6 s,
all read legs ≥ rocks (get_hit 4.17 vs 5.57 µs, scan 191 vs 199 µs,
lookup_100 415 vs 628 µs), flush_check 913 ms / 6 chunks = 152 ms/chunk
(≈ the guest's 11.9/23 = 0.52 s × guest-core factor). No local
regression from v26+v27.

## v28: `PEDRA_STAGE_MAX_BYTES` (commit `d1a0130`)

Chunk-size sweep knob: clamps `auto_flush_threshold` down (never up;
0/unparseable/unset = byte-identical default). A 64 MiB clamp also moves
parking back to whole-memtable staging (host worker `try_stage_if_full`)
below any per-CF `take_family` limit — the v24-era shape with v26+v27
code, without touching the vendored bench's CF buffers. Sweep plan:
256 (run #28 baseline) vs 128 vs 64 on the guest; read legs judged only
on a quiet host.

## v28 local 6M on-arm sanity (same mac as the 11.2 s cross-check)

`PEDRA_STAGE_MAX_BYTES=67108864`: hydrate **9.8 s** (vs 11.2 s no-cap
v26+v27, −12.5%), settle **0.5 s** (vs 2.6), 22 chunks × ~60 MiB all
direct-to-L3 (`l0=0`; 20 `install_parked` + 1 `install_flush`),
WRITEPHASE `flush_check 0.5 ms` vs **913.5 ms** — the writer-side
`spill_tail` cost is gone (worker stages below the take_family limit),
confirming the knob reproduces the v24-era shape. Local hydrate improving
with smaller chunks is consistent with the chunk-scaled-residual
hypothesis; the guest 25M run (#29) is the deciding measurement.

## Run #29 (v28 = v26+v27 + 64 MiB cap, guest 25M, loaded host): chunk-size
## hypothesis REFUTED

hydrate **112.5 s** (#28: 110.5; v24: 75.6), settle **0.7 s** (best yet),
exit 0, sst_n=93. The knob worked mechanically: **92 data chunks**
(~60 MiB each, = 4× #28's 23), **92 BULKDIAG installs**, WRITEPHASE
`prepare 1234.6 / wal 6892.5 / mem 9575.2 / publish 17.2 /
flush_check 14.9 ms` — the writer-side spill is fully gone. FLUSHSTAGES
sums (92 chunks): enc 16.8 + lz4 7.6 + bloom 3.4 + crc 1.5 + write 4.0 =
**33.3 s** (flat vs #28's 35.0 — same bytes, same codec work).

Accounting: commit **17.7** + materialize **33.3** = **51.0 s counted vs
112.5 s wall → 61.5 s unattributed — WORSE than #28's 45.8**. Halving
(and quartering) the chunk size did not shrink the residual, and the
memory-footprint-per-chunk theory predicts the opposite direction.
Combined with the local 6M pair (11.2 uncapped → 9.8 capped, cap FASTER
on the mac), the residual is not a property of chunk size at all.

What survives: v24 measured 75.6 s wall / ~19 s unattributed **with this
exact 64 MiB whole-memtable staging shape** (run26-v24 captures) — so the
v24→#29 delta (57 s wall, ~42 s residual) at IDENTICAL shape is code
(v25 threshold fix, v26 assist, v27 take_family fast path — all nominally
inert here) or environment (host load, guest page-cache state; MEMDIAGD
shows the 3.9 GB guest at ~1.9 GB cached during reads). Per-chunk
unattributed: #28 2.0 s/chunk, #29 0.67 s/chunk — neither linear in
chunks nor in entries. Uncounted candidate sinks common to both shapes:
`install_parked` (manifest + level links + fd, 92×), the worker-side
park/swap (outside FLUSHSTAGES timers in #29's shape), retire-cache
bookkeeping, and dirty-page writeback throttle — none have timers yet.

Next instrument: v29 diag timers around park/install/retire/await inside
the hydrate span, or a local sampling profile (`sample` on macOS) during
a local 25M hydrate — the local 15M A/B (capped vs not, RSS-sampled) is
running to see whether the wall anomaly reproduces on a quiet mac with
abundant RAM.

probe_hit p50 54.8 µs / probe_miss p50 3.2 µs — host loaded (six qemu
VMs); read legs remain unjudgeable this run, consistent with #27b/#28.

Operational note (local): two "FjallError: Poisoned" panics at the first
fjall apply (`snapshot_backends.rs:107`) were NOT a bad build — the data
volume was at 100% (299 MiB free): timeout-killed bench runs never drop
their `TempDir`s and had leaked 39 GB into `$TMPDIR`. fjall's background
flusher panics on ENOSPC and poisons its locks; the error surfaces at the
next `apply`, long after the write that filled the disk. Check `df -h`
before blaming the binary; clean `${TMPDIR}/.tmp*` after kills. The
same panic on a fresh `TempDir` is the signature.

## Residual attributed: macOS `sample` of a local 15M uncapped hydrate

Artifact: `run29b-local15m-profile-symbolicated.txt` (25 s window,
18,820 samples, `debug = 1` build; `sample` printed `???` for most
frames — symbolicated after the fact with
`atos -o <bin> -l <load-base> <addrs>`; on-disk libsystem_kernel dyld
stubs mislabel offsets, syscalls inferred from context: 0x45ec =
`__psynch_cvwait` under a pthread wrapper, 0x4510 = blocking write).

Wall was 33.5 s; counted spans (commit 14.4 = wal 8.65 / mem 2.91 /
flush_check 2.68; materialize 13.2) left ~5.9 s. The writer thread was
~99% busy and the missing time is:

1. **~8.0 s (32% of the window) in `parking_lot::RawMutex::lock_slow` →
   `__psynch_cvwait` inside `materialize_parked_once`** — the v26 assist
   parks the WRITER on `flush_lock`, which the flush worker holds across
   the whole `Db::write_imm_l0_files`. This sits outside every timer
   span, which is exactly why the residual never appeared in accounting.
   v24 had no assist → no writer queueing → smaller residual. This is
   the mechanism behind the v24→v26 hydrate regression.
2. **~4.1 s in `commit_async_ops → Wal::write_pending_frame →
   reserve_space → preallocate`** — blocking `F_PREALLOCATE` per 8 MiB
   chunk across ~3.3 GB of WAL.
3. ~2 s in `MemTable::insert_many` / `maybe_auto_flush_best_effort`
   bookkeeping outside the timed sub-spans.

Flush worker 42% busy in `write_imm_l0_sst`; compact worker idle.

## v29 (writer critical path): assist never queues; debt gets runway

Three commits, all answers to the profile above:

- **v29a** (`concurrent.rs`, c2105f7): `materialize_parked_once_try` —
  the assist try-locks `flush_lock`; if the worker holds it, return
  false immediately (skip, don't queue). Bounded `await_flush_debt`
  still resolves when the worker's install drops the debt; anti-wedge
  fallback (writer materializes when the lock is free) preserved.
  Blocking `materialize_parked_once` (worker path) unchanged, body
  extracted to private `materialize_parked_holding_flush`.
- **v29c** (`db.rs`, c2105f7): `flush_debt_cap` = `2 ×
  auto_flush_threshold` (was cap == threshold = stop-and-wait per park;
  run #29 stalled 92×). One parked chunk of runway so fill(N+1)
  overlaps materialize(N); memory bound now 2 parked chunks.
- **v29b** (`wal/mod.rs`, 238e27a): `WAL_PREALLOC_CHUNK` 8 → 64 MiB,
  8× fewer blocking preallocate calls. NOT injected for run #30: guest
  WAL total was only 6.9 s of the 112.5 s wall (#29), it saves little
  there, and 64 MiB `fallocate` per call is untested on the guest's
  Linux/ext4-ublk path. Keep the commit; revisit with its own guest run.

Tests added: `materialize_parked_once_try_skips_when_lock_held`,
`flush_debt_cap_is_two_thresholds`; suite 684 passed / 2 known
pre-existing flakes.

## Local 15M interleaved A/B (v28 d1a0130 vs v29ac c2105f7 vs v29acb
## 238e27a, cap64 shape, 2 reps) — drift-limited, direction favors v29ac

Mac was drifted the whole time (load 7.5/12, `caixote-api` at 210%;
fjall hydrate drifted 17.8–23.1 s vs 12.6 s quiet — the drift meter
swung 30% between slots, so arm deltas below ~3 s are not resolvable).

| arm | r1 hyd | r2 hyd | r1 settle | r2 settle | wal_ms r1/r2 |
|---|---|---|---|---|---|
| v28    | 39.9 | 59.3 (drift spike) | 16.5 | 28.6 | 28.6 / 48.0 |
| v29ac  | 38.7 | **32.1** | 16.8 | **9.7** | 27.2 / 20.4 |
| v29acb | 39.7 | 40.9 | 21.6 | 11.2 | 27.8 / 28.8 |

Readable signals only: v29ac never lost to v28 and produced the two
best runs of the set; v29acb added nothing over v29ac (and r1 settle
21.6 s was the worst of the rep). wal_ms differences are fsync-latency
noise under load. Verdict: **guest run #30 = v29ac only** (concurrent.rs
+ db.rs; wal stays v21p). Binaries preserved: /tmp/bench-v28,
/tmp/bench-v29ac, /tmp/bench-v29acb.

## Run #30 (v29ac, guest 25M, host load 33/64 ≈ #29's): regression
## RECOVERED — residual back to v4/v24 level

hydrate **76.9 s** (#29: 112.5, #28: 110.5, v24: 75.6), settle **1.4 s**
(best family), on disk 11.05 → 5.15 GiB, exit 0, sst_n=90, **88 chunks /
88 BULKDIAG installs**. WRITEPHASE commits=24415 prepare 1314.1 /
wal 7526.8 / mem 10233.1 / publish 22.4 / flush_check 23.3 ms
(= 19.1 s commit counted). FLUSHSTAGES sums (89 chunks): enc 19.4 +
lz4 9.1 + bloom 3.7 + crc 1.6 + write 4.0 = **37.7 s**.

Accounting: 19.1 + 37.7 = 56.8 s counted vs 76.9 s wall →
**residual ~20.1 s ≈ v24's ~19 s**. The v26→v28 regression
(~41 s of writer-side unattributed time) is gone; the `sample`-based
attribution (writer queueing on `flush_lock` inside the assist) is
confirmed on the guest, not just the mac. `parked_n` never exceeded 1 —
with the assist skipping instead of queueing, debt rarely reached two
chunks; the 2× cap is headroom, not a code path we exercised.

Read legs under the same loaded host as #29 (probe_hit p50 61.6 µs,
probe_miss 3.8 µs — unjudgeable, consistent): get_hit 54.5 µs mid,
prefix_scan 543.7 µs, lookup_100 get_loop 5.93 ms / multi_get 5.15 ms.

Hydrate vs rocks default (~34 s extrapolated from the 15M leg): still
~0.44× — the remaining gap is now the OLD v4-era ~19 s residual plus
per-byte write-path CPU (encode-bound at guest-core speed), not the
assist regression. Next single-variable candidates: v29b (WAL 64 MiB
prealloc, addresses the 4.1 s/25 s local preallocate share) and diag
timers around `install_parked`/retire for the last ~20 s.

Operational notes: (1) the injection hit `RACE_SUPERVISOR_RESTARTED_VM`
during teardown — the supervisor auto-restarted the VM ~65 s BEFORE the
mv, so that boot ran the old inode (wasted run); a manual stop/start
after the mv booted the new image cleanly. Budget a stop/start after
every supervisor race. (2) The guest build log's `Compiling` lines are
ANSI-colored — `grep "Compiling pedradb-core"` silently matches nothing;
grep for `Compiling` bare or de-ANSI first (this false-negative nearly
mis-attributed a good run to a stale image). (3) The bench binary
relinked as `/data/target/.../snapshot_backends-658f4d3d34281331` —
the target dir lives on the persistent data volume, so build hashes
persist across container restarts.

## Run #31 (v30 = v29ac + PARKDIAG timers, guest 25M): residual FULLY
## attributed — the wall is serial writer+worker CPU

Reproduces #30 almost exactly: hydrate **76.8 s** (#30: 76.9), settle
1.6 s, commit counted 19.2 s (wal 7465.0 / mem 10347.6 / prepare
1335.1 ms), 88 chunks. PARKDIAG sums (88 chunks; local `awk` prints
decimal commas):

- prep (nums+ctx under write lock) **≈ 0**
- files (`write_imm_l0_files` total) **44.8 s** — FLUSHSTAGES sums
  38.1 s (enc 19.7 + lz4 9.3 + bloom 3.6 + crc 1.6 + write 3.9) →
  **intra-write remainder 6.7 s** (~76 ms/chunk outside the stage
  timers: fd/create/truncate/buffer work)
- install (`bulk_span_level` + `apply_sst_installs` manifest/level/fd
  + parked pop) **5.4 s** (~61 ms/chunk — the per-chunk MANIFEST
  persist is the obvious suspect)
- retire (Arc unwrap + retire-cache/drop) **4.0 s** (~45 ms/chunk)
- AWAITDIAG lines: **0** — the bounded debt wait never slept; v29a
  removed the waits entirely.

New accounting: 19.2 (writer) + 54.2 (worker) = 73.4 s counted vs
76.8 s wall → unattributed **~3.2 s** (memtable rotate/misc). Every
block of the #29-era 61.5 s residual is now named: ~41 s assist lock
queueing (v29a), ~6.7 s intra-write, 5.4 s install, 4.0 s retire.

**The pipeline does not overlap**: writer total 19.2 s, worker total
54.2 s, wall 76.8 ≈ their SUM — on ≥2 free cores a filled-chunk/
materialize pipeline would bound the wall near max(19.2, 54.2) ≈ 55 s.
This matches run #18's "one effective core" conclusion (the guest is
`smp 4` but CPU-quota'd; MEMDIAGD's `cgcur=` field prints empty, so
the quota itself is unverified — the sum-arithmetic is the evidence).
Consequence: on this guest, hydrate wall ≈ total write-path CPU, and
parallelism is not a lever — only per-cycle cuts are. Ranked levers
with ceilings: enc+lz4 29.0 s (algorithmic), mem 10.3 s, wal 7.5 s
(v29b prealloc + P0.3 WAL ring), intra-write 6.7 s (inspect
`write_imm_l0_files`), install 5.4 s (P1.2 batched manifest), retire
4.0 s. Reachable near-term floor without encode work: ~61 s (~0.56×
vs rocks); hydrate ≥1× needs the encode/lz4 block.
