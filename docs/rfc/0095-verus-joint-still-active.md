# RFC: 0095 — Verus twin of `joint_still_active` (leave-joint is not a cartoon)

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0094](0094-stateright-leave-joint-fail-closed.md), [0066](0066-joint-leave-fail-closed.md), [0064](0064-joint-election-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig**. The production kernel `joint_still_active` (`old != new`) is what store `pending_joint_on` uses after a joint commits. The Verus twin `membership_joint.rs` only proved `joint_election_ok` (RFC-0064). Freeze twins only checked that entry. AS-IS (`joint_still_active_as_is` always false) is the 0066 hole. This slice puts the leave gate in the twin and in the catalog so dropping it is a freeze fail.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. A Verus twin is not a verified verifier (R-verus). Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- RFC-0066 P2.1 / RFC-0094 P1.1 named this leftover.
- `scripts/formal/catalog.json` pair `joint_election` has `entry: joint_election_ok`.
- Clone `membership_raft_store` already lists `joint_still_active` (identical tokens).

## Problems This Solves

- **Problem:** leave-joint existed in production and Stateright, not in the Verus twin.
- **Problem:** freeze would still pass if `joint_still_active` vanished from the twin.
- **Problem:** AS-IS drop-joint had no twin lemma.

## Proposed Solution

- Twin exec `joint_still_active` / `_as_is` with production tokens (`old != new` / `false`). Catalog pair `joint_leave` (`entry: joint_still_active`). Named rust glue tooth. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (twin + freeze pair)
- [x] **P0.1** Twin + catalog pair `joint_leave` — status: `done`
- [x] **P0.2** Regression: AS-IS elects 2/3 C-old — status: `done` (`leave_joint_as_is_elects_old_only`)

### P1 — next wave
- [ ] **P1.1** L28 REAL leave-joint on `cluster_real` (0066 P2.2) — status: `todo`
- [x] **P1.2** `joint_leave_model` in catalog models — status: `done`

### P2 — later
- [ ] **P2.1** Campaign is not ∀ traces — status: `todo`
- [ ] **P2.2** R-verus still never — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | twin + catalog joint_leave | done | membership_joint.rs + catalog.json | 2026-08-28 |
| P0.2 | p0 | AS-IS 2/3 C-old elects | done | leave_joint_as_is_elects_old_only | 2026-08-28 |
| P1.1 | p1 | L28 REAL leave | todo | — | 2026-08-28 |
| P1.2 | p1 | catalog models | done | catalog.json models | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | todo | — | 2026-08-28 |
| P2.2 | p2 | R-verus never | todo | — | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `leave_joint_as_is_elects_old_only`: `joint_still_active` → `joint_election_ok(2,3,Some((2,4)))` is false; AS-IS glue (`joint_still_active_as_is` → `None`) elects. Same fns as store.
  - Freeze `--twins` covers `joint_still_active` tokens (`old != new`).
  - Clone `membership_raft_store` still identical.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0066 P2.1 and RFC-0094 P1.1 done; `residuals.json` `R-joint` owner 0095.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. Closing `never_floor` (R-verus). New `*_kernel.rs`.
- Changing `joint_still_active` production body.
- Rocks coluna A/B. crates.io.
