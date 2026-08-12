# RFC-0011: Env filesystem seam + FailingEnv fault injection

**Status:** done (P0–P2 media models shipped; seed DST in [RFC-0012](0012-next-significant-steps.md) / `pedradb-dst`)  
**Updated:** 2026-08-11  
**Parent:** [RFC-0001](0001-pedradb-high-level-spec.md)  
**Supersedes / extends:** [RFC-0008](0008-p2-sim-apply-oracle.md) §P2.1 (sim)  
**Related:** [RFC-0009](0009-rocksdb-class-engine.md) P2.4 (sim flush/compact faults)  
**External pattern:** Railway depot-store `Media` / `FailingMedia` (RBS DST)  
**Hunt notebook:** `determinismo/pedradb-dst/`  

---

## Background

PedraDB P0–P2 (RFC-0003…0008) already had:

- Crash classes via `pedradb-sim::FaultEnv` (process-style `mem::forget`, surgical WAL truncate).  
- Unit tests with truncated tails and CRC checks.  
- Oracle vs in-memory model.

What it **did not** have was a seam between the engine and `std::fs`. WAL (`Wal`), SST (`write_sst*` / `SstTable::open`), and `Db` (flush, compact rename, dir sync, orphan `.sst.tmp` cleanup) all called the filesystem directly. That meant:

- Fault injection was only *outside* the write path (truncate after close), or via OS tools (LD_PRELOAD / LazyFS).  
- There was no `fail_after(N)` sweep over real put/flush/compact ops — the technique that found silent recovery bugs in depot-store (`FailingMedia` + Nth-op).  
- RFC-0009 P2.4 (“sim flush/compact faults”) was blocked on plumbing.

**Industry pattern (facts):** FoundationDB/TigerBeetle-style DST and depot-store both **swap the I/O interface**, not the engine. depot-store documents this as: *shaped by what the code does today*; production `CompioMedia` is a zero-cost passthrough; tests use `FailingMedia` with shared trip state.

PedraDB is **sync** `std::fs` (no io_uring). The same idea applies with a thinner, synchronous trait.

## Problems this solves

- **Problem:** Cannot inject EIO / ENOSPC / fsync failure *during* an acknowledged write path without rewriting callers or relying on host-level interposition.  
- **Problem:** Crash-only sim (truncate WAL after the fact) does not exercise “open fails mid-recover”, “flush SST then fail dir sync”, or “compact rename fails”.  
- **Problem:** DST harnesses (`pedradb-dst`) cannot arm faults at a seed-derived op index against the real `Db` API.  
- **Problem:** Every new disk op (MANIFEST, lock file) risks bypassing sim unless I/O is funneled through one trait.

## Proposed solution

### 1. `Env` / `EnvFile` in `pedradb-core` (production seam)

Small traits covering **exactly** what the engine does today:

| Trait | Surface |
|-------|---------|
| `EnvFile` | `Read + Write + Seek` + `sync_data` / `sync_all` / `set_len` / `len` |
| `Env` | `create_dir_all`, `create`, `open_append`, `open_read`, `sync_dir`, `read_dir_names`, `remove_file`, `rename`, `exists`, `metadata_len` |

- Production: `StdEnv` → `std::fs::File` (zero extra allocation, monomorphized).  
- `Db` is generic: `Db<E: Env = StdEnv>`. Call sites keep `Db::open` / `open_with`.  
- Injected path: `Db::open_with_env(path, opts, env)`.  
- WAL / SST grow `*_on(env, …)` helpers; convenience methods default to `StdEnv`.

**Design rules (deliberate):**

1. **Interface-swap, not crate-swap** — same lesson as TiKV `test_raftstore` / RBS `Media`.  
2. **Shaped by today’s ops** — not a future block-device Substrate.  
3. `Env: Clone` so open handles and the DB can share fault state (`Rc` in test impls).  
4. `exists` on `FailingEnv` does **not** count as a fallible op (metadata; avoids flaky open bookkeeping).  
5. Seek does not decrement the fail budget (not a durability barrier).

### 2. `FailingEnv` in `pedradb-sim` (test media)

Analog of depot `FailingMedia`:

| API | Meaning |
|-----|---------|
| `fail_after(n)` | N fallible ops succeed; then permanent dead disk |
| `fail_after_kind(n, kind)` | Same with explicit `FaultKind` |
| `passing()` | Healthy until armed |
| `arm(after, transient)` / `arm_one_failure()` | Runtime arm (one-shot or permanent) |
| `arm_with_kind(...)` | Arm + kind (incl. sync-only) |
| `disarm()` / `tripped()` | Heal / observe |

`FaultKind`:

| Kind | `io::ErrorKind` | Notes |
|------|-----------------|-------|
| `IoError` | Other | Default dead disk |
| `StorageFull` | StorageFull | ENOSPC path (engines often special-case) |
| `PermissionDenied` | PermissionDenied | |
| `Interrupted` | Interrupted | Retry stress |
| `SyncFail` | Other | Gates **only** `sync_*` / `sync_dir` (write may already have landed) |

Shared trip state: `Rc<FailState>` (Cell counters), clone-shared like RBS.

### 3. What stays outside this RFC

- Path-level crash classes (`FaultEnv::truncate_wal_to`, `mem::forget`) remain — complementary.  
- Bit-rot / byte-flip of file contents (CRC/torn) — corruption hunt, not Env gate.  
- Lying fsync that returns `Ok` but discards bytes — P1 (needs recording media).  
- LD_PRELOAD `det_io` — still useful for full-process validation; Env is the in-process primary.

## Delivery slices

### P0 — seam + fail-after that works (shipped)

- [x] **P0.1** Define `Env` / `EnvFile` / `StdEnv` in `pedradb-core` — status: `done`  
- [x] **P0.2** Route WAL create/append/recover, SST open/write, Db open/flush/compact/tmp-cleanup through `Env` — status: `done`  
- [x] **P0.3** `Db::open_with_env` + `Db<E = StdEnv>` without breaking existing `Db::open` API — status: `done`  
- [x] **P0.4** `FailingEnv` + `FaultKind` in `pedradb-sim` (`fail_after`, arm/disarm, StorageFull, SyncFail) — status: `done`  
- [x] **P0.5** Tests: inject on open; arm mid-put then reopen recovers acked prefix; StorageFull kind — status: `done`  

### P1 — use the seam for real hunts

- [x] **P1.1** Nth-op sweep harness (`fail_after(n)` for n in 0..K over put/flush/compact; reopen healed; assert acked prefix) — status: `done`  
- [x] **P1.2** Seedable schedule arms `FailingEnv` — status: `done`  
  — `FailingEnv::from_seed` / `seed_to_fail_after` in `pedradb-sim` (DST harness can call the same).
- [x] **P1.3** Explicit flush/compact fault windows (fail after SST fsync, before WAL recreate; fail on rename) — status: `done` (F1: flush now tmp+rename; compact already had)  
- [x] **P1.4** Classify outcomes: fail-stop vs silent loss vs contract-ok; ledger findings — status: `done` (F1 REAL+FIXED; SyncFail partial apply = CONTRACT-OK in oracle)  

### P2 — richer media models

- [x] **P2.1** `RecordingEnv` / crash-image: buffer writes until sync; drop unsynced on “crash” — status: `done`  
- [x] **P2.2** Lying sync (`SyncPolicy::Lying`) — status: `done`  
- [x] **P2.3** Short-write / partial `write` then error — status: `done` (`arm_short_write`)  
- [x] **P2.4** Optional `Send + Sync` Env for multi-thread stress — status: `done` (`FailingEnvArc`)  

## Status (living)

| ID | Band | Title | Status | Delivery | Updated |
|----|------|-------|--------|----------|---------|
| P0.1 | p0 | Env traits + StdEnv | done | `crates/pedradb-core/src/env.rs` | 2026-08-11 |
| P0.2 | p0 | Wire WAL/SST/Db | done | `wal/mod.rs`, `sst/table.rs`, `db.rs` | 2026-08-11 |
| P0.3 | p0 | `open_with_env` API | done | `Db::open_with_env` | 2026-08-11 |
| P0.4 | p0 | FailingEnv + FaultKind | done | `crates/pedradb-sim/src/failing.rs` | 2026-08-11 |
| P0.5 | p0 | Unit tests sim | done | failing-env + crash scenarios | 2026-08-11 |
| P1.1 | p1 | Nth-op sweep harness | done | sim nth-op sweep tests | 2026-08-11 |
| P1.2 | p1 | Seed arms FailingEnv | done | `FailingEnv::from_seed` | 2026-08-11 |
| P1.3 | p1 | Flush/compact windows | done | F1 fix: flush via `.sst.tmp`+rename | 2026-08-11 |
| P1.4 | p1 | Finding taxonomy | done | F1 REAL+FIXED; LEDGER §A | 2026-08-11 |
| P2.1 | p2 | RecordingEnv | done | `pedradb-sim/recording.rs` | 2026-08-11 |
| P2.2 | p2 | Lying sync | done | `SyncPolicy::Lying` | 2026-08-11 |
| P2.3 | p2 | Short write | done | `arm_short_write` | 2026-08-11 |
| P2.4 | p2 | Send Env | done | `FailingEnvArc` | 2026-08-11 |

## Acceptance criteria

### Tests

- `cargo test --workspace` green with Env wiring.  
- `FailingEnv::fail_after(0)` → `Db::open_with_env` returns `CoreError::Io`.  
- `fail_after_kind(0, StorageFull)` → `io::ErrorKind::StorageFull`.  
- Put under `passing()` env succeeds; `arm_one_failure()` on next put errors; after `disarm()` + reopen, previously acked keys present.  
- (P1+) Exhaustive or large N sweep leaves no silent data loss for acked `sync=true` writes.

### Telemetry / analytics

- None for P0. Optional later: counter of injected faults in sim (debug only).

### Documentation

- This RFC.  
- Module docs on `env` and `pedradb-sim` point at the seam.  
- Cross-link from RFC-0009 P2.4 and `docs/open-items.md`.  
- Hunt notes: `determinismo/pedradb-dst/{KNOWLEDGE,NEXT,LEDGER}.md`.

### Screenshots

- backend-only.

## API sketch (normative for callers)

```rust
// Production (unchanged ergonomics)
let db = Db::open("/data/pedra")?;

// Fault injection
use pedradb_sim::{FailingEnv, FaultKind};
use pedradb_core::{Db, OpenOptions};

let env = FailingEnv::fail_after(20);
let mut db = Db::open_with_env(
    "/tmp/pedra-fault",
    OpenOptions {
        sync: true,
        auto_flush_bytes: None,
        auto_compact_sst_count: None,
    },
    env.clone(),
)?;
db.put(b"k", b"v")?;
env.arm_with_kind(0, true, FaultKind::SyncFail);
let _ = db.put(b"k2", b"v2"); // may fail on fsync
```

## Out of scope

- Changing the on-disk WAL/SST format.  
- Making production builds pay for fault hooks (no `cfg` required; monomorphized `StdEnv`).  
- Multi-node / network faults.  
- Replacing `FaultEnv` truncate scenarios (they stay).  
- Full Antithesis/FDB-scale clock simulation.

## Risks / non-goals of P0

- **Fail budget counts every Env op**, not every logical put — open/create/sync inflate N. Sweeps must document what “op” means (Env call, not user put).  
- **Drop-on-crash without fsync** is not fully modeled by permanent `FailingEnv` (use truncate + forget, or P2 RecordingEnv).  
- **Generic `Db<E>`** can complicate type inference in exotic wrappers; default `StdEnv` keeps the common path simple.
