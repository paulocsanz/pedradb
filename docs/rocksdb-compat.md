# rocksdb-compat: Pedra under a rust-rocksdb-shaped API

> Status: rust-rocksdb **0.22 API** on Pedra (RFC-0050). Alias-swap compiles
> against this crate. Layout (prefix CFs, Pedra SST) is not Rocks C++; the
> **function names, types, and observable KV contract** match. TiKV `engine_rocks`
> still has a deeper correctness model (Titan, UDT, per-CF block cache) — that
> is engine internals, not a missing method.
>
> **Strict substitute bar** (RFC-0062, 2026-08-25): same `WriteOptions.sync`
> as the host, Linux, min of 3 rounds **>1.0** on every official shape;
> only advantages; never a defect the host can feel. Not there yet:
> `deps_raftlog` Linux min still <1× (p50 tied after `pwrite`).
> `Checkpoint` / `BackupEngine` names **shipped** (RFC-0062 P1.2). Analysis:
> [`reports/2026-08-25-compat-strict-substitute.md`](reports/2026-08-25-compat-strict-substitute.md).
> The 0.001× G1-vs-async table is **not** this crate's default (default is
> async, RFC-0054) and is **not** full-sync.

## What shipped

`crates/rocksdb-compat` — a rust-rocksdb-shaped API on top of
`pedradb-core::ConcurrentDb`:

- **Writes** join the Rocks-style write group (one leader: appends + one
  barrier if any member asked for sync + apply). Drop-in default is
  **async** (RFC-0054). A lone client takes the single-writer fast path
  (`apply_batch_with`, no channel hop). `set_sync(true)` is G1
  (`F_FULLFSYNC` on Darwin).
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
| `flush` / `compact` / `compact_range(_cf)` | ✅ | compact_range runs a full merge (range is a hint) |
| `SstFileWriter` + `ingest_external_file(_cf)` | ✅ | Pedra SST; ingest writes through WAL then flush (G1) |
| `delete_file_in_range` | ✅ | `delete_range` + flush + compact (tombstones; never unlink SSTs) |
| `WriteBatchWithIndex` | ✅ | last-write-wins overlay + `get_from_batch_and_db` |
| compaction filter | ✅ | applied on `compact` / range compact |
| `create_cf` / `drop_cf` / `list_cf` / `destroy` / `repair` | ✅ | prefix CFs + CFREG |
| `multi_get` / `get_opt` / `put_opt` / `merge` / `live_files` / `key_may_exist` / `get_pinned` | ✅ | |
| `Checkpoint` / `backup::BackupEngine` | ✅ | wrap `create_checkpoint` / `pedradb-ops`; `Env` is a stub for `BackupEngine::open` |
| knobs (`set_*`) | ✅ classified | [`KNOB_INVENTORY`](../crates/rocksdb-compat/src/knobs.rs): Wired / Inert / NotSupported (G2) / SaferDivergent. `set_verify_checksums(false)` → `ErrorKind::NotSupported` |

Dependency swap for a consumer (alias, no crates.io patch):

```toml
[dependencies]
rocksdb = { package = "rocksdb-compat", path = "../rocksdb-compat" }
```

Full substitution by crate name (RFC-0059) — for upper databases that
depend on the crates.io `rocksdb` by name (SurrealDB `kv-rocksdb`
requires `rocksdb = "0.21.0"`), use the shim package
`crates/rocksdb` (package name `rocksdb`, version `0.21.0`, reexports
`rocksdb-compat` plus a `Transaction<'a, D>` alias parameterized on the
DB type). Zero source changes on the consumer:

```toml
[patch.crates-io]
rocksdb = { path = "crates/rocksdb" }
```

Proof harness: `bench/sub-surreal/` — SurrealDB v1.5.4 (vendored) built
twice, storage swapped only by the patch; see
`findings/2026-08-24-sub-surreal-mac/`.

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

## API vs engine internals

The rust-rocksdb **0.22 method/type surface is present and callable**. Remaining
differences are **how** Pedra stores bytes, not missing names:

| Topic | How it works here |
|---|---|
| Column families | Prefix `cf\0key` + CFREG; `create_cf`/`drop_cf`/`list_cf` work. Per-CF block cache / Titan knobs are accepted no-ops. |
| `ingest_external_file` | `SstFileWriter` emits a Pedra SST; ingest applies it then flush. |
| Compaction filter | Runs on live keys at `compact`. |
| `delete_file_in_range` | Tombstone + compact (Rocks unlinks files; we refuse that silent-wrong). Keys in range are gone. |
| `WriteBatchWithIndex` | Overlay + `get_from_batch_and_db`. |
| Iterators | Windowed (64) with `seek`/`seek_for_prev`/`seek_to_last`; `Iterator` impl. |
| Properties | `property_int_value` / `property_value` map `DbStats`. |
| Concurrency | `ConcurrentDb` group-commit. |

TiKV-as-a-product still needs a raftstore integration (not an extra `pub fn`). **Do not** claim a running TiKV cluster from the API table alone.

## Kernel vs drop-in (two APIs, two defaults)

| | Kernel `pedradb_core::Db` | Drop-in `rocksdb-compat::DB` |
|---|---|---|
| Type | `&mut self` single-writer | **`ConcurrentDb` always** (group commit + lone fast path) |
| `sync` default | **true** (G1) | **false** (Rocks-shaped, RFC-0054) |
| Darwin barrier when `sync=true` | `F_FULLFSYNC` (`wal_full_fsync=true`) | same — the flag is on; it only fires if the host opts into G1 |
| WAL recovery | FailClosed | PointInTime |
| Version GC | 24 h horizon + archive | `auto_reclaim=true` (Rocks compact-GC) |
| Memtable flush | 4 MiB | 4 MiB (see below) |

A program that `use rocksdb::…` through the alias hits the **drop-in** column.
`Db::open` / Montanha / CLI hit the **kernel**. Mixing the two defaults is how
"why is my put 4 ms on a Mac" happens: that process called `set_sync(true)`
(or used the kernel) and paid `F_FULLFSYNC`. The drop-in default does not.

### Memtable: 4 MiB vs 64 MiB vs 256 MiB

Three numbers, three jobs — they do **not** leak into each other:

| Where | Size | Why |
|---|---|---|
| Drop-in / kernel **product default** | **4 MiB** | Isolated apply (2000× pre+com): 4 MiB + drain **2251 qps** vs Rocks-shaped 64 MiB drain **1228** — one 64 MiB SST at the end lost apply (RFC-0041, `apply_flush_probe`). |
| Rocks **engine default** | 64 MiB | Their knob. |
| **Official parity bench** | **256 MiB** (`CompatEngine::open`, override `ROCKS_PARITY_COMPAT_MEMTABLE`) | Bigger than any timed suite's write volume so auto-flush does **not** run in the measured window. 4 MiB flushed ~25× during `set_mc50` and poisoned the ratio. |

Official numbers are therefore **flush-free**. Shipping 4 MiB in the drop-in is
a production apply win, not a silent bench cheat. A host that wants Rocks'
64 MiB calls `set_write_buffer_size(64 << 20)`.

## Knob map: RocksDB → compat behavior (RFC-0047 P2.1)

The drop-in contract is not just the API table above — the *operational*
knobs need a stated counterpart. Kernel stays fail-closed; these knobs are
the compat face. Divergences are deliberate and listed, not accidental.

| RocksDB | Compat | Behavior / divergence |
|---|---|---|
| `WriteOptions::sync` (default `false`) | `Options::sync` (default **`false`**, RFC-0054) | **Same class as Rocks default.** `set_sync(true)` is G1 (barrier before Ok; Darwin `F_FULLFSYNC` via `wal_full_fsync=true`). Kernel `OpenOptions.sync` stays `true` — Pedra-the-engine is still durable-by-default. |
| Compaction GCs unpinned obsolete versions | `Options::auto_reclaim` (default **`true`**, RFC-0047 P0.3) | Same storage profile: disk ≈ live set + pins. `false` opts into Pedra F20 full history (PITR) — an option Rocks does not have. |
| `WalRecoveryMode::PointInTimeRecovery` (default) | `Options::wal_recovery = PointInTime` (default) | Serves the clean prefix; the discarded suffix is **reported** (`DB::last_recovery_report`), never guessed. Kernel default stays `FailClosed`. |
| `kAbsoluteConsistency` / `kSkipAnyCorruptedRecords` | `FailClosed` / — | `FailClosed` refuses the open on mid-WAL damage. Skip-any is **absent on purpose**: silent-wrong is banned (G2). |
| `DB::Resume()` after a background error | `DB::resume()` (RFC-0047 P1.1) | Close+replay+reopen, typed outcome on `DB::last_fence_recovery` (`uncertain_from..=uncertain_through`, `lost_writes`). `Ok(())` when nothing was fenced (defensive resume). |
| Rocks auto-retries soft/retryable bg errors | `Options::auto_resume_transient` (default **`true`**, P1.2) | Auto-resume **only** for `FenceClass::Transient` (ENOSPC-like); `Persistent`/`Unknown` stay manual — never an untyped retry flag. |
| `EventListener::on_background_error(reason)` | `Options::set_background_error_listener` (P2.1) | Fired once per fence within one worker poll tick, before auto-resume. Payload `BackgroundError { kind: Fenced, class, message }`; `reason` severity maps to `class` (Transient ≈ retryable/soft, rest ≈ hard). Default off. |
| `flush_wal(true)` | `DB::sync` when `sync=true` | With drop-in default async, `flush_wal(true)` **is** the durability barrier (F193). With `set_sync(true)` the WAL already synced at Ok. |
| `enable_blob_files` + `min_blob_size` (Rocks default **off**) | `Options::set_enable_blob_files` / `set_min_blob_size` | Wired to `OpenOptions.large_value_threshold` (`VALUES.vlog`). Default **off** on the drop-in (Rocks). The parity harness enables 4 KiB so `kvrocks_blob_set` (16 KiB) spills; 1 KiB SET stays inline. `set_blob_file_size` is numbered-blob rotate (Titan); default off. G1 fsyncs the vlog **once per commit** before the WAL pointer is durable; async `write()`s at 64 KiB, no `fdatasync` (same class as WAL). |

Every other `set_*` builder (`set_use_fsync`, `increase_parallelism`,
`set_max_background_jobs`, level-tuning, compression, bloom, …) is accepted
and **inert**: single-node engine, no level structure to tune — a Rocks
program compiles unchanged, it just does not buy anything there.

## Bench parity vs real RocksDB (YCSB A–F + dependent shapes)

`crates/rocksdb-parity-bench` runs the same six YCSB shapes as the Montanha
FDB suite (plus the `deps` suite above) through **one generic runner** with
two engine adapters, so the op schedule (rng seed, zipf CDF,
read/insert/scan/RMW mix) cannot drift between engines:

- `compat` — `rocksdb-compat` on pedradb-core (`ConcurrentDb`). Drop-in
  default is async WAL (RFC-0054). Official batteries pin `PEDRA_PARITY_ASYNC=1`
  / `set_write_sync(false)` so the column is same-class vs the peer.
- `rocksdb` — real RocksDB via the `rocksdb` crate (feature `real`, matching
  the `pedradb-oracle` pin 0.22 / librocksdb-sys 8.10). **Official peer =
  Rocks default** (`WriteOptions.sync=false`, `ROCKS_PARITY_SYNC=0`).
  `ROCKS_PARITY_SYNC=1` is an extra same-class-with-G1 column, never the
  win condition. Kernel Pedra (`Db` / `OpenOptions.sync=true`) is still
  durable-by-default — that is a different API than this drop-in.

**Retention pin (RFC-0047 P0.3):** the compat drop-in now defaults to the
Rocks storage profile (`auto_reclaim=true` — auto-compact GCs unpinned
obsolete versions). The bench pins what each column measures so the flip
never silently changes official numbers: `ROCKS_PARITY_RETENTION=product`
(default) forces Pedra product retention (keep all versions, RFC-0009 F20 —
what every official column has measured); `ROCKS_PARITY_RETENTION=rocks`
measures the drop-in profile. Invalid values or mixing with the legacy
`ROCKS_PARITY_AUTO_RECLAIM=1` exit 2 instead of benching an ambiguous
retention.

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
