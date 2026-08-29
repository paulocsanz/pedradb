# RFC: 0116 — Verus twin of `election_grant_from_counts`

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0115](0115-joining-grant-during-joint-add.md), [0114](0114-election-grant-from-member.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig**. RFC-0114/0115 production `election_grant_from_counts(in_ids, in_pending_old_or_new)` = or. The Verus twin lacked the fn — freeze would still pass if it vanished. AS-IS always true. This slice puts the grant gate in the twin and catalog. A Verus twin is not a verified verifier (R-verus). 0114/0115 live reply tests are **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- RFC-0114 P1.1 / RFC-0115 P1.1 named this leftover.
- Clone `membership_raft_store` already lists `election_grant_from_counts`.

## Problems This Solves

- **Problem:** grant-from existed in production, not in the Verus twin.
- **Problem:** freeze would still pass if the fn vanished from the twin.
- **Problem:** AS-IS record-any had no twin lemma.

## Proposed Solution

- Twin exec `election_grant_from_counts` / `_as_is`. Catalog pair `election_grant_from`. Named rust glue `election_grant_from_requires_ids_or_pending`. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (twin + freeze pair)
- [x] **P0.1** Twin + catalog pair `election_grant_from` — status: `done`
- [x] **P0.2** Regression: AS-IS records any grant — status: `done` (`election_grant_from_requires_ids_or_pending`)

### P1 — next wave
- [ ] **P1.1** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`
- TCP LeaveJoint wire is RFC-0117 (not this P1 live run)
- [ ] **P1.2** Campaign is not ∀ traces — status: `todo`

### P2 — later
- [ ] **P2.1** R-verus still never — status: `todo`
- [ ] **P2.2** none yet

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | twin + catalog election_grant_from | done | membership_joint.rs + catalog.json | 2026-08-28 |
| P0.2 | p0 | AS-IS records any grant | done | election_grant_from_requires_ids_or_pending | 2026-08-28 |
| P1.1 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P1.2 | p1 | not ∀ traces | todo | — | 2026-08-28 |
| P2.1 | p2 | R-verus never | todo | — | 2026-08-28 |
| P2.2 | p2 | none yet | todo | — | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `election_grant_from_requires_ids_or_pending`: `(false,false)` false; AS-IS true; `(false,true)` and `(true,false)` true. Same tokens raft=store.
  - Catalog pair `election_grant_from` `entry: election_grant_from_counts`; freeze twins fail if the exec fn is dropped from the twin.
  - `./scripts/verus_membership_joint.sh` when Verus is installed.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0115 P1.1 done as this RFC; `residuals.json` `R-joint` owner 0116.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
