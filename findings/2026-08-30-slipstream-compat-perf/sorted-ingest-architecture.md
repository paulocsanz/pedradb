# Sorted-ingest architecture — the drastic hydrate/settle lever (2026-08-31)

Prompt: "settle and hydrate are still terrible and nothing changed — stop,
think, research architectures, analyze benchmarks and traces, find where we
can gain drastically." This document is that analysis. Every number below is
measured, with the run it came from.

## 1. What run #19 (v21p) actually proved

Raw serial: `run19-v21p-guest-25m.txt`. Fixes in v21p: idle-WAL-rotate
manifest storm removed + SST post-write read-back/verify removed.

| leg (25M)            | run #13 (v21j) | run #19 (v21p) | rocks default | #19 ratio |
|----------------------|----------------|----------------|---------------|-----------|
| hydrate              | 143.7 s        | 148.9 s        | 25.3 s        | 0.17×     |
| settle               | 52.0 s         | 88.2 s         | 8.3 s         | 0.09×     |
| probe_hit p50        | 33.9 µs        | 52.6 µs        | 45.4 µs       | 0.86×     |
| probe_miss p50       | 2.5 µs         | 2.2 µs         | rocks-class   | ~1×       |
| get_hit (criterion)  | 46.7 µs        | **42.4 µs**    | 38.59 µs      | 0.91×     |
| prefix_scan          | 632.6 µs       | **437.6 µs**   | 305.6 µs      | 0.70×     |
| lookup_100 get_loop  | 4.534 ms       | 4.593 ms       | 3.38 ms       | 0.74×     |
| lookup_100 multi_get | 4.954 ms       | **4.548 ms**   | 3.70 ms       | 0.81×     |
| on disk after settle | 5.15 GiB       | 5.15 GiB       | 5.24 GiB      | smaller   |

- **The storm is gone**: FDSYNCDIAG count 0 (< 2048 fdatasyncs total; run
  #16 had ≥ 12,288). The read legs were the storm's victim, and they are
  the best ever recorded: get_hit 0.83→0.91×, prefix_scan 0.48→0.70×,
  multi_get 0.75→0.81× in one run.
- **Hydrate+settle total did not move**: #13 195.7 s → #19 237.1 s (and
  #18 was 237.6 s). Removing CPU overhead (storm, read-back) reallocated
  time between phases — hydrate −12.3 s, settle +11.8 s vs #18 — because
  the total is **byte-volume-bound, not CPU-bound at the phase level**.
- probe_hit 52.6 µs vs #13's 33.9 µs: single-run probe swings of ±30 % on
  this guest are documented (#13 vs #18 on identical code: 33.9 → 49.8).
  Not counted as a v21p regression without an arbitration repeat.

### Run #19 settle anatomy (from the serial)

- `SETTLE_PHASE flush_ms=10935` — of which ~6.3 s is two hydrate-tail
  worker jobs finishing under the flush; the final SST write itself is
  `FLUSHDUR ms=658`.
- `SETTLE_PHASE compact_ms=77249` — 23 sequential single-input pushdown
  jobs of ~3.2 s each (COMPDUR sum 97.3 s across the run).
- Settle entry state: L0 5/778 MiB, L1 3/255 MiB, **L2 27/4.08 GiB**
  (151 MiB chunks), L3 4/423 MiB → done L1 2/100 MiB, L2 16/2.63 GiB,
  L3 20/2.80 GiB (38 files, same 5.15 GiB).
- Run #15 (same v21j-era code) settle: entry L2 88/5.17 GiB → pushed
  2.5 GiB into an **empty** L3 in compact_ms=47442.

Per-byte arithmetic across both runs: settle output throughput
**56–58 MiB/s**, per-job combined I/O **~110–116 MiB/s** — identical
before and after the v21p read-back removal, and identical to the
v21m writeprobe finding (jobs CPU-bound at ~110 MiB/s while the disk
does 381 MiB/s single-stream; batch-of-4 jobs = 4× single-job wall ⇒
one effective core). #19's settle is worse than #15/#13 purely because
it rewrote more bytes (~4.5 GiB vs ~2.6 GiB output).

**Conclusion 1: the ~110 MiB/s per-byte pipeline cost is the encode/decode
loop itself (per-entry framing + allocs + lz4 + CRC + index/bloom), not
the read-back.** v21p removed the read-back and the rate did not move.

**Conclusion 2 (why "nothing changed"): the ladder feeds ~25–30 GiB of
logical bytes through that loop for a 5.15 GiB result.** Volume × rate is
the wall time. Fixing overheads inside the same volume cannot move the
total. Only cutting volume (write-amp) and/or the per-byte rate can.

## 2. The reconciliation the user asked for

"Parity benches win 2×, slipstream loses 5–6×, nothing regressed":

- The parity battery (`rocksdb-parity-bench`, default
  `ROCKS_YCSB_RECORDS=1024` × 100 B ≈ 100 KiB) never leaves the memtable
  and caches. It measures in-memory op throughput — Pedra 1.25–3.65×.
- Slipstream 25M is 100 % drain pipeline: 24,414 batches through
  memtable → WAL → 26 L0 flushes (60.7 s, run #16) → 164 compaction jobs
  (152.8 s) → settle drain. Zero percent of it is cache-resident.
- These regimes are disjoint; the parity wins were never evidence about
  the pipeline. Nothing regressed — the pipeline was always ~4–5× slower
  per logical byte than it needs to be.

## 3. The workload fact that changes the architecture

The vendored bench's hydrate loop (read-only stage
`benches/snapshot_backends.rs`, `fn hydrate`):

```rust
for j in i..end {            // end = i + 1024
    batch.push(KvUpdate::Put(KvEntry { key: key(j), .. }));
}
store.apply(&batch, ..);
i = end;
```

with `key(j) = format!("route.svc-{:06}.{:08}", j/1000, j%1000)` — fixed
width, so lexicographic = numeric order. **Every apply batch is sorted,
and every batch sorts strictly before the next.** The 25M-entry hydrate
is a perfectly sorted, append-only stream (plus one repeated META cursor
key per batch — a different CF, overwriting itself).

We currently pay the full LSM ladder for a stream that arrives in final
form: BTree inserts, WAL copy, L0 encode, L0→L1 merge decode+encode,
L1→L2 and L2→L3 pushdowns, settle drain. RocksDB pays the same ladder —
and does the whole thing at 207 MiB/s logical on one effective core
(hydrate 25.3 s / 5.4 GiB logical), which is the per-byte reference our
encode loop must beat (~110 MiB/s today, and that includes a decode pass
jobs pay; flush-only encode is ~116 MiB/s).

## 4. The design: sorted-ingest fast path (bulk load)

Detection (core-only, no compat edits): in the concurrent write path
(`concurrent.rs`, mine) check per batch+CF that keys are strictly
ascending and above the CF's current high-water mark. After N
consecutive confirming batches, latch the CF into **append mode**; any
out-of-order batch unlatches permanently (falls back to the ladder —
zero behavior change for random workloads).

In append mode:

1. Entries accumulate in a **sorted-run builder** (append-only Vec of
   encoded entries — no BTree, no per-entry rebalancing; reads over the
   open tail served by binary search, same visibility rules as a
   memtable).
2. WAL stays the durability record only for the uninstalled tail; SST
   chunks (64–128 MiB) are written straight from the builder — the
   entries are already in final order, so a **direct-to-L3 install**: a
   chunk whose range is disjoint-above everything below is a valid new
   L-run (the leveled structure already maintains exactly this
   invariant — `leveling.rs` pushdown exists to *create* it slowly).
3. After each SST install (manifest persist), the WAL segments covering
   the installed range are GC-able — WAL steady state ≈ one chunk
   buffer, not 5.7 GiB.
4. `settle()` on an append-mode DB: the runs are already disjoint and
   level-clean — flush the tail, done. No drain.

Correctness invariants to hold (P0 test surface):

- The latch is *conservative*: any duplicate, delete/tombstone, or
  out-of-order key in append mode must either fall back before applying
  or route that batch through the normal path. (The bench stream has no
  deletes and no duplicates in the data CF; the META cursor key lives in
  another CF.)
- Crash mid-hydrate = today's recovery: WAL replay of the uninstalled
  tail + manifest-installed SSTs. No new durability decision — the bench
  already applies `sync=false` (barrier-free, WALFDIAG=0 in run #17);
  the G1 product barrier cadence is the group-commit kernel's, untouched.
- Reads during hydrate see memtable + installed runs (unchanged path).

### Expected numbers (guest, 25M) — floors from measurements

- Bytes written: **5.15 GiB once** (final size) + WAL ring, vs ~25–30 GiB
  logical through the loop today. Disk floor 5.15 GiB / 381 MiB/s ≈
  **14.5 s**.
- Encode floor: the writer must sustain ≥ ~210 MiB/s logical to match
  rocks' 25.3 s. Today's loop does ~110 MiB/s, so the fast path must
  also cut per-entry cost — encode **directly from the batch payload**
  (the compat WriteBatch already holds contiguous key/value bytes; one
  memcpy per entry into the block buffer, no `InternalKey`/`Bytes`
  per-entry allocations, bloom/index built as the blocks fill). Rocks'
  measured 207 MiB/s on the same core — through a full ladder — is the
  existence proof; an encode-only single pass should land 250–400 MiB/s.
- Honest projection: hydrate ≈ **18–28 s** (0.9–1.4× vs rocks 25.3 s),
  settle ≈ **1–3 s** (3–8× vs rocks 8.3 s), disk ≈ 5.15 GiB (unchanged).
- Read side is unaffected or better: run #19's accidental 38-file layout
  (vs 95) produced the best read legs ever — bulk mode makes the
  read-optimal chunk size a free choice (64–128 MiB).

If the encode loop lands at only ~110 MiB/s even single-pass, bulk mode
alone yields hydrate ≈ 48 s (0.53×) — better than 0.17× but short of the
goal. **The per-byte encode cut and the volume cut are both required**;
neither alone reaches 1×.

## 5. What this does NOT fix

- get_hit 0.91×, prefix_scan 0.70×, lookup_100 0.74/0.81× — per-get
  candidate cost and block layout; separately tracked (payload-pool
  granularity, block target, scan materialization). Bulk mode helps
  indirectly (fewer, bigger files) but does not close these.
- 100M scale (disk peak 47 GiB during the ladder) — bulk mode caps the
  peak at ≈ live set + one chunk, which is also the fix for that rung
  of the scale ladder.

## 6. Plan

- P0: latch + sorted-run builder + direct-to-L3 install + WAL ring GC +
  fallback path; regression tests (out-of-order unlatch, delete in
  stream, crash-replay equality, concurrent reads during append mode);
  local 6M A/B; guest run #20 at 25M.
- P1: encode-path per-byte cut (zero-alloc block fill from batch
  payload); manifest-persist batching across chunk installs.
- P2: extend the latch to nearly-sorted streams (bounded out-of-order
  window), which real watch_applied feeds may produce.

Next concrete step: RFC draft (repo `rfc` skill format) — this document
is its evidence base.
