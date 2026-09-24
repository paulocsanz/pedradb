# RFC: 0127 — Reopen `participating` must follow disk membership

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0126](0126-crash-reopen-high-water.md), [0105](0105-pending-joint-members-only.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. `is_participating` returns the node's `participating` flag when the node is still in `nodes` — **not** `ids`. After leave, apply sets `participating = ids.contains`. `crash_reopen_engine_on` / `reopen_engine_on` restore `ids` from disk then insert the node with the **captured** pre-reopen flag. A stale `true` makes a removed node count again. AS-IS `participating_if_member` is always true. This slice: reopen sets `participating` from `ids`. 0126 high-water is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- `is_participating` prefers the flag over `ids`.
- Crash-reopen captures `participating` before reloading voters.

## Problems This Solves

- **Problem:** removed node can count after crash-reopen.
- **Problem:** 0126 restored high-water, not the flag.
- **Problem:** AS-IS keeps captured `true`.

## Proposed Solution

- Pure `participating_if_member(in_ids)` = `in_ids`. AS-IS true. Both reopen paths use it after disk `ids` restore. Clone tokens raft=store. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (flag follows ids)
- [x] **P0.1** Kernel + reopen insert — status: `done`
- [x] **P0.2** Regression — status: `done` (`crash_reopen_participating_follows_membership`)

### P1 — next wave
- [x] **P1.1** `reopen_engine_on` same gate — status: `done`
- [x] **P1.2** Verus twin of `participating_if_member` — status: `done` (RFC-0128)

### P2 — later
- [x] **P2.1** Campaign is not ∀ traces — status: `done` (`reopen_participating_campaign_is_not_forall_traces`)
- [x] **P2.2** R-verus still never — status: `done` (`reopen_participating_verus_still_never`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | participating follows ids | done | membership_kernel + crash_reopen | 2026-08-28 |
| P0.2 | p0 | stale true cleared | done | crash_reopen_participating_follows_membership | 2026-08-28 |
| P1.1 | p1 | reopen_engine_on | done | reopen_engine_on | 2026-08-28 |
| P1.2 | p1 | Verus twin | done | RFC-0128 catalog participating_member | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | done | reopen_participating_campaign_is_not_forall_traces | 2026-08-28 |
| P2.2 | p2 | R-verus never | done | reopen_participating_verus_still_never | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `participating_if_member(false)` false; AS-IS true.
  - `crash_reopen_participating_follows_membership`: Queued 4→3 leave; force node 4 `participating=true`; persist; `crash_reopen_engine_on(4)`; `!is_participating(4)` and `!is_member(4)`. 0126 high-water is **not** this tooth. Runs on Darwin. Does not submit io_uring SQEs.
  - P2.1 `reopen_participating_campaign_is_not_forall_traces`: `R-joint` stays continuous; catalog pair `participating_member`; campaign not a theorem.
  - P2.2 `reopen_participating_verus_still_never`: `R-verus` stays in `never_floor`; catalog/twin freeze of `participating_if_member` is not a verified verifier. Does not run `verus`.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0126 P1.2 stays TCP 3-process; `residuals.json` `R-joint` owner 0127.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
