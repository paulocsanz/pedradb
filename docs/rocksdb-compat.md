# rocksdb-compat: Pedra under a rust-rocksdb-shaped API

> Status: **lab foundation shipped** (2026-08-15). Not a TiKV swap. Not drop-in.
> The ask was "replace rocksdb by pedradb in TiKV and run our adversarial tests".
> This document records what shipped, what the adversarial suite says, and the
> concrete gap list between this layer and a real TiKV `engine_rocks` swap.

## What shipped

`crates/rocksdb-compat` — a rust-rocksdb-shaped API on top of
`pedradb-core::ConcurrentDb`:

- **Writes** join the Rocks-style write group (one leader: appends + one
  `fdatasync` + apply). A lone client takes the single-writer fast path
  (`apply_batch_with`, no channel hop) so sequential benches stay on the
  same fsync-before-Ok cost as `Db::put`.
- **Reads** take `RwLock` read guards (point get, prefix latest, count,
  iterator refill). Composite reads (`last_prefix_then_get`, `count_cf`)
  stay under one guard.
- **Host compact worker** (StdEnv `open_cf` only) drains imm via
  `ConcurrentDb::drain_imm_once` and runs L0→L1 off the write lock.
  `open_cf_with_env` stays worker-free so `FailingEnv` campaigns remain
  single-threaded / deterministic.

| rust-rocksdb API | Compat | Notes |
|---|---|---|
| `DB::open_default` / `DB::open` | ✅ | `Options::create_if_missing` respected |
| `DB::open_cf` / `cf_handle` | ✅ (emulated) | CFs are key prefixes (`cf\x00key`); `default` is raw **only** when no named CF exists, otherwise prefixed too, so full-CF scans never leak other CFs' keys |
| `put`/`get`/`delete` (± CF) | ✅ | |
| `delete_range_cf` | ✅ | Maps to Pedra range-delete |
| `write(WriteBatch)` | ✅ atomic | One Pedra `apply_batch` = one WAL group; failed batch applies nothing (tested under fault) |
| `snapshot()` + `get`/`get_cf`/`iterator(_cf)` | ✅ | Sequence-pinned; isolation tested |
| `iterator(IteratorMode)` / `iterator_cf` | ✅ (janela 64) | `Start`/`End`/`From`; forward refill via `range_at_limited` (RFC-0032 P0.1) |
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
| 9 | Concurrency: TiKV writes from many threads | Compat still serializes puts on a mutex over single-writer `Db`. L0 compact is drained by a host thread (`pedra-compat-compact`, RFC-0037 P2.1) after Ok; FailingEnv opens stay single-threaded. `ConcurrentDb` group-commit is not wired here | M |
| 10 | `WAL tail` recovery policy / `manual_wal_flush`, raft-log WAL sync class | Partial (Pedra WAL semantics differ; SyncFail edge tested) | S–M |

Honest verdict: a full TiKV build-and-run on this layer is a multi-session
project (ingest + compaction filters + WriteBatchWithIndex are each
individually large, and TiKV's correctness assumptions run deeper than the
API surface). What this ships: the API-shaped substrate, the alias-swap
mechanism, and the adversarial gates that any future swap work can run
unchanged. **Do not** claim "TiKV on Pedra" from this.

## Bench parity vs real RocksDB (YCSB A–F + dependent shapes)

`crates/rocksdb-parity-bench` runs the same six YCSB shapes as the Montanha
FDB suite (plus the `deps` suite above) through **one generic runner** with
two engine adapters, so the op schedule (rng seed, zipf CDF,
read/insert/scan/RMW mix) cannot drift between engines:

- `compat` — `rocksdb-compat` on pedradb-core (single node, single client,
  WAL fsync before Ok).
- `rocksdb` — real RocksDB via the `rocksdb` crate (feature `real`, matching
  the `pedradb-oracle` pin 0.22 / librocksdb-sys 8.10, no compression codecs
  — payload is random bytes). Durability is labeled: default
  `ROCKS_PARITY_SYNC=1` (sync per write, matched to Pedra's contract);
  `ROCKS_PARITY_SYNC=0` runs RocksDB's async-WAL default as a reference.

```bash
# compat side
cargo run -q --release -p rocksdb-parity-bench -- findings/rocks-parity-local/compat compat
# real side (sync-per-write)
ROCKS_PARITY_SYNC=1 scripts/rocks_side_ycsb.sh findings/rocks-parity-local/rocks_side
# compare + optional gate (ROCKS_PARITY_RATIO_FLOOR; "none"/unset = report-only)
ROCKS_PARITY_PEER=findings/rocks-parity-local/rocks_side/rocks_shaped_peer.json \
  ROCKS_PARITY_RATIO_FLOOR=0.5 \
  cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-compare -- \
    findings/rocks-parity-local/compat/rocks_parity_bench.json findings/rocks-parity-local/compare
# or everything at once:
scripts/rocksdb_parity_v0.sh findings/rocks-parity-local
```

Report: `compat_over_rocksdb` per shape + `parity` block
(`floor`/`shapes_with_peer`/`min_ratio`/`pass`); exit 2 when a floor is set
**and** a real peer produced ratios below it. `ROCKS_PARITY_TEMPLATE=1`
skips the real side (CI template mode, `parity.pass: null`).

Honesty: single-node, single-client lab bench through the compat API subset —
not a distributed/field claim, and the compat side always fsyncs (the peer's
`sync`/`durability` labels are carried in every report).

### Lab numbers (2026-08-15, worktree @ e5c6b80, records=1024 ops=200 payload=100 uniform)

| shape | compat qps | rocksdb sync-per-write | ratio | rocksdb async-WAL | ratio |
|---|---:|---:|---:|---:|---:|
| ycsb_a 50/50 | 94 | 37,728 | **0.002** | 343,643 | 0.000 |
| ycsb_b 95/5 | 913 | 85,674 | 0.011 | 1,145,036 | 0.001 |
| ycsb_c 100r | 156,103 | 1,556,420 | 0.100 | 218,510 | 0.714 |
| ycsb_d 95r/5i | 817 | 88,484 | 0.009 | 1,148,053 | 0.001 |
| ycsb_e scan+5i | 1,073 | 147,289 | 0.007 | 234,707 | 0.005 |
| ycsb_f RMW | 110 | 47,001 | **0.002** | 293,327 | 0.000 |

**Diagnosed write cliff (mechanism, not hand-wave):** every synced write in
`pedradb-core` `commit_ops_with` re-stores the whole CHANGELOG
(`change_feed.rs::store_on`: encode all entries → tmp write → `sync_all` →
rename → dir fsync) — three durability barriers per put plus a body that
grows with every write (quadratic; small runs score relatively better, which
matches: 96-record smoke ratios 0.002–0.47 vs 1024-record 0.002–0.01).
RocksDB's synced path is one WAL fsync (~26 µs here — APFS `fsync` is not
F_FULLFSYNC, so RocksDB's sync-per-write is cheap on this lab box).
RFC-0019 already allows batching: the on-disk CHANGELOG is a *cache* rebuilt
from WAL and must never gate commit success — periodic/batched store is the
sanctioned fix. **Shipped (RFC-0031 P0.1):** `PEDRA_CHANGELOG_INTERVAL`
(default 64) debounces the cache store; flush / close / WAL rotate /
checkpoint still persist. Seed 1024 puts 39.7 s → 6.5 s; ycsb_a 94 → 390
qps. Residual with `interval=0` is the same (~3.8 ms p50) — one WAL
`File::sync_all` (`F_FULLFSYNC` on macOS). RocksDB `sync=true` is `fsync`,
not `F_FULLFSYNC`; we do **not** downgrade `sync_all` to win the bench (G1).
`ROCKS_PARITY_RATIO_FLOOR` against the **fdatasync** peer stays report-only
(mixing `F_FULLFSYNC` ~4.8 ms with `fdatasync` ~50 µs is not an engine
measurement). Official 2× gate is vs `ROCKS_PARITY_FULL_SYNC=1`. Writes
already meet it; `ycsb_c` / `deps_mvcc_latest` / `deps_scan` wait on P1
iterators. See [RFC-0031](rfc/0031-rocks-parity-10x-budget.md).

### Lab numbers after P0.1 debounce (2026-08-15, interval=64, same records/ops)

| shape | compat qps | vs pré-P0.1 |
|---|---:|---:|
| ycsb_a | 390 | 4.1× |
| ycsb_b | 4,245 | 4.6× |
| ycsb_c | 206,629 | 1.3× (já era read-bound) |
| ycsb_f | 352 | 3.2× |
| deps_apply_batch | 78 | 2.9× |
| deps_cache_overwrite | 215 | 4.1× |

## Dependent-shaped suite (`deps`, TiKV as the reference dependent)

Beyond generic YCSB, the bench models the access patterns real RocksDB
dependents issue (TiKV is the canonical one — and the reason this compat
layer exists). CF layout mirrors TiKV's store: `default` (MVCC values),
`write` (commit records), `lock`, plus `raftlog` for the raftdb instance.
Shape provenance:

| shape | Dependent pattern (TiKV source) | Op per iteration |
|---|---|---|
| `deps_apply_batch` | raftstore apply path: one ready = prewrite batch (lock+default) then commit batch (write+lock-del), across CFs (`raftstore::apply`) | 2 atomic multi-CF WriteBatches × `ROCKS_DEPS_BATCH` (default 32) txns |
| `deps_mvcc_latest` | MVCC point read: `SeekForPrev(user_key_MAX)` on `write` CF, then value fetch in `default` (`txn::store`) | 1 reverse-seek + 1 point get |
| `deps_scan` | coprocessor / MVCC-GC range scan over user keys in `write` | 1 scan ≤ 25 keys |
| `deps_raftlog` | raftdb append: batched sequential log entries + trailing read (`raftstore::store::RaftApplyStorage`) | 1 batch × 16 entries + every 8th op 1 read |
| `deps_cache_overwrite` | cache-style dependent: unbatched zipf overwrite of a fixed keyspace (worst case for per-write costs) | 1 unbatched put |

The runner seeds 2 versions/record (batched prewrite+commit rounds), then
runs the five shapes; op counters match exactly across engines (verified in
smoke runs — the schedule cannot drift). Suite selection:
`ROCKS_PARITY_SUITE=ycsb,deps` (default both; `deps` alone is fine).

### Lab numbers, deps suite (2026-08-15, @863df07, records=1024 ops=200 batch=32, uniform)

| shape | compat qps | rocksdb sync-per-write | ratio | rocksdb async-WAL |
|---|---:|---:|---:|---:|
| deps_apply_batch | 27 | 1,720 | **0.016** | 2,649 |
| deps_mvcc_latest | 260 | 84,367 | **0.003** | 125,173 |
| deps_scan | 508 | 104,943 | 0.005 | 81,699 |
| deps_raftlog | 32 | 4,744 | 0.007 | 35,916 |
| deps_cache_overwrite | 52 | 8,976 | 0.006 | 142,607 |

**Second mechanism quantified:** `deps_mvcc_latest`/`deps_scan` sit ~600×
below compat's own point-get throughput (ycsb_c: 156k qps) because the
compat iterator is *eager* — every call materializes the whole CF snapshot
(`Vec<(k,v)>`), so a latest-read pays a full keyspace copy plus reverse
walk. That is gap #6 in the TiKV table below, now with a number attached.
Batching helps where dependents batch (apply: 0.016 vs unbatched overwrite
0.006 — the CHANGELOG store amortizes per batch), but per-write durability
cost and eager iterators dominate. Same conclusion as the YCSB table:
CHANGELOG batched store + lazy iterators are the two highest-leverage
compat fixes.

## Reproduce

```bash
cargo test -p rocksdb-compat                # API + adversarial suite
cargo test -p rocksdb-compat --test adversarial -- --nocapture
cargo test -p rocksdb-parity-bench          # harness determinism + deps suite on compat
```
