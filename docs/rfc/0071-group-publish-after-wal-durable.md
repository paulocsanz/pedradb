# RFC: 0071 — Group publish only after WAL I/O (glue fail-closed)

**Status:** done
**Updated:** 2026-08-27
**Parents:** [0057](0057-maximum-intensity-parallel-dst-boxes-formal.md), [0045](0045-multi-writer-async-5x.md), [0070](0070-pct-depth-not-forall-schedules.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-group-glue` (not `never_floor`). Axis vs FDB Sim2: **G1 / G5** — OS threads, lock, WAL I/O around group commit. FDB Sim2 has no `ConcurrentDb`. Pedra’s group-commit *kernel* already decides OCC/fence; the glue still chose “publish vs fence” with an inline `if io_err`. This slice names that decision: visibility publish is admitted only when off-lock (or lone) WAL I/O succeeded. AS-IS would publish after a failed fsync.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Lock/scheduler interleavings stay TCB. `∀π` of ConcurrentDb stays R-group-glue / R-pct.

## Background

- RFC-0045 P2.1: drop the write lock for `fdatasync`, reacquire to apply+publish. G1: Ok waits for fd.
- RFC-0057 P2.1: `occ_conflict` / `group_validate` / `fence_publish_seq` are kernels. The publish-after-I/O gate was still glue in `finish_group_off_lock` and `lone_sync_commit`.
- `R-group-glue` close-text: “kernel proves group decision; interleavings stay TCB.” The remaining named hole this slice can close without ∀π is the durability-to-visibility gate.

## Problems This Solves

- **Problem:** a failed WAL `fdatasync` could be rounded to “still publish” if the `if io_err` glue drifts.
- **Problem:** lone G1 and multi-writer off-lock paths duplicated the gate without a kernel.
- **Problem:** AS-IS publishes even when WAL I/O failed (Ok with a lie).

## Proposed Solution

- Pure `may_publish_group(wal_io_ok)` = `wal_io_ok`. AS-IS always true.
- Existing `group_commit_kernel` (no new TCB file). Production `finish_group_off_lock` and `lone_sync_commit` call it before apply/publish.

## Delivery slices (mandatory)

### P0 — must ship first (gate on the live ConcurrentDb path)
- [x] **P0.1** `may_publish_group` + AS-IS in `group_commit_kernel` — status: `done`
- [x] **P0.2** `finish_group_off_lock` and `lone_sync_commit` call the kernel — status: `done`
- [x] **P0.3** Regression: injected WAL sync fail → put Err, key not published; AS-IS would publish — status: `done` (`failed_wal_sync_does_not_publish_group`)

### P1 — next wave
- [x] **P1.1** Multi-writer group (N threads) hits the same kernel on off-lock fd — status: `done` (`multi_writer_failed_sync_does_not_publish_group`)
- [x] **P1.2** PCT plant: yield after failed fd, assert no publish — status: `done` (`after_wal_sync` yield; `pct_after_failed_fd_does_not_publish`)

### P2 — later
- [x] **P2.1** Verus twin of `may_publish_group` — status: `done` (`verus/group_commit.rs` + catalog `group_publish`)
- [x] **P2.2** ∀ interleavings of the lock around this gate stay TCB — status: `done` (`lock_interleavings_admitted`; `claim_lock_interleavings_refused_after_put`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | may_publish_group kernel + AS-IS | done | group_commit_kernel.rs | 2026-08-27 |
| P0.2 | p0 | production publish gated | done | finish_group_off_lock + lone_sync_commit | 2026-08-27 |
| P0.3 | p0 | failed sync does not publish | done | failed_wal_sync_does_not_publish_group | 2026-08-27 |
| P1.1 | p1 | multi-writer off-lock path | done | multi_writer_failed_sync_does_not_publish_group | 2026-08-27 |
| P1.2 | p1 | PCT plant after failed fd | done | pct_after_failed_fd_does_not_publish | 2026-08-27 |
| P2.1 | p2 | Verus twin | done | group_commit.rs may_publish_group + catalog group_publish | 2026-08-27 |
| P2.2 | p2 | lock interleavings TCB | done | claim_lock_interleavings_refused_after_put | 2026-08-27 |

## Acceptance Criteria

- **Tests**
  - `may_publish_group(true)` true; `may_publish_group(false)` false; AS-IS true on false.
  - `failed_wal_sync_does_not_publish_group`: `ConcurrentDb::open_with_env` + real put, inject WAL sync fail, next put Err, `get` of that key is None (not published); AS-IS kernel would have published.
  - P1.1 `multi_writer_failed_sync_does_not_publish_group`: N threads, `finish_group_off_lock`, failed fd, no member published.
  - P1.2 `pct_after_failed_fd_does_not_publish`: sequential misses `after_wal_sync`; PCT d=2 hits it; unpublished; replay 8×.
  - P2.2 `claim_lock_interleavings_refused_after_put`: live ConcurrentDb open/put; `claim_lock_interleavings_proven` is false; AS-IS `lock_interleavings_admitted_as_is` would admit.
  - P2.1 catalog pair `group_publish` entry `may_publish_group` with Verus twin in `verus/group_commit.rs`.
- **Telemetry / Analytics:** none — durability invariant.
- **Documentation:** this RFC; `residuals.json` `R-group-glue` close-text + owner 0071.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- ∀ OS interleavings of ConcurrentDb (still TCB). PCT depth (RFC-0070).
- Extracting `db.rs`. Flow. `never_floor`. Rocks coluna A/B. crates.io.
