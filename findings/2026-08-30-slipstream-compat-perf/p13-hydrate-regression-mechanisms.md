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
