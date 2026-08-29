# RFC: 0108 — Verus twin of `pending_joint_node_counts`

**Status:** in-progress
**Updated:** 2026-08-28
**Parents:** [0107](0107-election-ignores-removed-joint.md), [0105](0105-pending-joint-members-only.md), [0095](0095-verus-joint-still-active.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig**. RFC-0105 production `pending_joint_node_counts(is_member)` skips removed nodes; 0107 election uses that. The Verus twin `membership_joint.rs` still lacked the fn — freeze only required `joint_still_active` / `joint_election_ok`. AS-IS always true. This slice puts the skip in the twin and catalog so dropping it is a freeze fail. A Verus twin is not a verified verifier (R-verus). 0105/0107 live store tests are **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- RFC-0105/0107 P1.1 named this leftover.
- Catalog `joint_leave` entry is `joint_still_active`. Clone already lists `pending_joint_node_counts`.

## Problems This Solves

- **Problem:** members-only pending joint existed in production, not in the Verus twin.
- **Problem:** freeze would still pass if `pending_joint_node_counts` vanished from the twin.
- **Problem:** AS-IS count-removed had no twin lemma.

## Proposed Solution

- Twin exec `pending_joint_node_counts` / `_as_is` (`is_member` / `true`). Catalog pair `pending_joint_node`. Named rust glue tooth. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (twin + freeze pair)
- [x] **P0.1** Twin + catalog pair `pending_joint_node` — status: `done`
- [x] **P0.2** Regression: AS-IS counts removed — status: `done` (`pending_joint_skips_non_member`)

### P1 — next wave
- [x] **P1.1** Verus twin of `compact_through_unleft` — status: `done` (RFC-0109); `joint_leave_ok` stays todo
- [ ] **P1.2** L28 REAL leave on `cluster_real` (0066 P2.2) — status: `todo`

### P2 — later
- [ ] **P2.1** Campaign is not ∀ traces — status: `todo`
- [ ] **P2.2** R-verus still never — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | twin + catalog pending_joint_node | done | membership_joint.rs + catalog.json | 2026-08-28 |
| P0.2 | p0 | AS-IS counts removed | done | pending_joint_skips_non_member | 2026-08-28 |
| P1.1 | p1 | compact_through_unleft twin | done | RFC-0109 | 2026-08-28 |
| P1.2 | p1 | cluster_real leave | todo | — | 2026-08-28 |
| P2.1 | p2 | not ∀ traces | todo | — | 2026-08-28 |
| P2.2 | p2 | R-verus never | todo | — | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `pending_joint_skips_non_member`: `pending_joint_node_counts(false)` false; AS-IS true. Same tokens raft=store.
  - Catalog pair `pending_joint_node` `entry: pending_joint_node_counts`; freeze twins fail if the exec fn is dropped from the twin.
  - `./scripts/verus_membership_joint.sh` when Verus is installed.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0107 P1.1 done as this RFC; `residuals.json` `R-joint` owner 0108.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- 3-process `cluster_real` leave.
- Rocks coluna A/B. crates.io.
