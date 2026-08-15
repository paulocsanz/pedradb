# rocksdb-compat: Pedra under a rust-rocksdb-shaped API

> Status: **lab foundation shipped** (2026-08-15). Not a TiKV swap. Not drop-in.
> The ask was "replace rocksdb by pedradb in TiKV and run our adversarial tests".
> This document records what shipped, what the adversarial suite says, and the
> concrete gap list between this layer and a real TiKV `engine_rocks` swap.

## What shipped

`crates/rocksdb-compat` — a rust-rocksdb-shaped API on top of `pedradb-core`:

| rust-rocksdb API | Compat | Notes |
|---|---|---|
| `DB::open_default` / `DB::open` | ✅ | `Options::create_if_missing` respected |
| `DB::open_cf` / `cf_handle` | ✅ (emulated) | CFs are key prefixes (`cf\x00key`); `default` is raw **only** when no named CF exists, otherwise prefixed too, so full-CF scans never leak other CFs' keys |
| `put`/`get`/`delete` (± CF) | ✅ | |
| `delete_range_cf` | ✅ | Maps to Pedra range-delete |
| `write(WriteBatch)` | ✅ atomic | One Pedra `apply_batch` = one WAL group; failed batch applies nothing (tested under fault) |
| `snapshot()` + `get`/`get_cf`/`iterator(_cf)` | ✅ | Sequence-pinned; isolation tested |
| `iterator(IteratorMode)` / `iterator_cf` | ✅ (eager) | `Start`/`End`/`From(k, Forward\|Reverse)`; materialized — documented perf scope |
| `flush` / `compact` | ✅ | |

Dependency swap for a consumer (alias, no crates.io patch):

```toml
[dependencies]
rocksdb = { package = "rocksdb-compat", path = "../rocksdb-compat" }
```

## Adversarial results (our suite style, on the compat layer)

`crates/rocksdb-compat/tests/adversarial.rs`:

- **Model-checked random campaigns** (32 seeds × 96 ops): put/delete/atomic
  batch/range-delete/scan-compare vs a `BTreeMap`. Zero silent-wrong.
- **Post-crash exact contract** (all kinds): for every key, the durable value
  must be in the *acceptable set* derived from the op history — an **Ok**
  (synced) write pins exactly its outcome (and must survive reopen); an
  **Err** write unions its outcome with the prior acceptable set, because
  whether bytes landed depends on which fallible op (write vs sync) the fault
  hit. Never-written keys must be absent.
- **Dead-disk (`FailingEnv` IoError)** armed after a healthy open: every
  refused op is a visible error; across all 32 seeds no Ok'd synced write was
  lost and no wrong value appeared.
- **Sync-fail edge** (write lands, fsync errors): an `Err` write may still be
  durable — its value is admitted by the union rule above; intermediate or
  torn values are not.
- **Short-write (torn WAL record)**: reopen **fails closed** with a WAL CRC
  error instead of silently dropping/keeping data. This matches Pedra's
  intended integrity contract (`explode_choose_crc_fail_stops_reopen` in
  pedradb-sim): the operator repairs; recovery does not guess. The compat
  harness encodes this as the expected outcome for torn records.
- **Batch all-or-nothing under fault** (16 seeds): key count after a mid-flight
  disk death is exactly `base + 2×applied_batches`.
- **Iterator positioning** (seek-from, forward) equals the model range.

Two harness-visible design constraints surfaced during this work (both now
handled in the crate, recorded here so they are not re-discovered):

1. **CF scan leakage**: a naive raw-keyspace `default` CF lets other CFs'
   encoded keys leak into full scans. Fixed by prefixing `default` whenever
   named CFs exist and bounding each CF scan to `[prefix, prefix\x01)`.
2. **Fault-arm timing**: arming `fail_after(n)` before `Db::open` burns the
   budget on open's own I/O. The harness opens healthy, then arms via the
   shared-`Rc` env clone (same pattern as pedradb-sim).

## TiKV swap: gap list (why this is not "done")

Swapping TiKV's storage engine means replacing its `engine_rocks` (wrapping
`tikv/rust-rocksdb`) end to end. Concrete blockers, in rough order of size:

| # | TiKV need | Status in compat | Size |
|---|---|---|---|
| 1 | Column families with per-CF options (block cache, compaction settings, `TitanBlobRunMode`) | Prefix emulation only; no per-CF knobs | M |
| 2 | `ingest_external_file` (BR backup/restore, PITR) | ❌ | L |
| 3 | Compaction filters (raft GC, MVCC GC in `raftstore`) | ❌ (Pedra GC is operator/explicit) | L |
| 4 | `delete_files_in_range` (fast region drop) | ❌ and **unsafe to fake**: drops files without tombstones | M |
| 5 | `WriteBatchWithIndex` / read-your-writes inside batch (raftstore apply path) | ❌ (plain atomic batch only) | M |
| 6 | Iterator: lazy streaming with `seek_to_last`, upper/lower bounds, `next` on pinned SST iters | Eager `Vec` today; correctness equivalent, memory profile not | M |
| 7 | Properties / statistics / tickers (`get_property_int_cf`, `RocksStatistics`) | ❌ (Pedra `DbStats` exists; no mapping layer) | M |
| 8 | Manual compaction shapes (`compact_range_cf` with levels, bottommost) | Whole-merge only | M |
| 9 | Concurrency: TiKV writes from many threads | Compat serializes on a mutex over single-writer `Db`; `ConcurrentDb` exists in core but is not wired here | M |
| 10 | `WAL tail` recovery policy / `manual_wal_flush`, raft-log WAL sync class | Partial (Pedra WAL semantics differ; SyncFail edge tested) | S–M |

Honest verdict: a full TiKV build-and-run on this layer is a multi-session
project (ingest + compaction filters + WriteBatchWithIndex are each
individually large, and TiKV's correctness assumptions run deeper than the
API surface). What this ships: the API-shaped substrate, the alias-swap
mechanism, and the adversarial gates that any future swap work can run
unchanged. **Do not** claim "TiKV on Pedra" from this.

## Reproduce

```bash
cargo test -p rocksdb-compat                # API + adversarial suite
cargo test -p rocksdb-compat --test adversarial -- --nocapture
```
