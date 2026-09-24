# RFC: 0129 — Verus twin of `membership_identity_before_applied`

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0128](0128-is-participating-requires-ids.md), [0124](0124-disk-membership-overrides-cli.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **G1 process death + G7 reconfig**. RFC-0124 P1.1 persists C-new identity **before** advancing `applied`. The kernel lives in both membership clones and `apply_range`. The Verus twin has `disk_membership_overrides_cli` / `high_water_at_least` / `participating_if_member` but **not** this gate — freeze would still pass if it vanished. AS-IS `membership_identity_before_applied` is false (applied first). A Verus twin is not a verified verifier (R-verus). 0124 apply order is **not** this tooth.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX.

## Background

- Production: `if membership_identity_before_applied(true) { install_applied_membership; then persist applied }`.
- Twin `membership_joint.rs` lacks the fn.

## Problems This Solves

- **Problem:** identity-before-applied existed in production, not in the twin.
- **Problem:** freeze twins would pass if the gate vanished.
- **Problem:** AS-IS applied-first had no twin lemma.

## Proposed Solution

- Twin exec `membership_identity_before_applied` / `_as_is` (`identity_first` / `false`). Catalog pair `identity_before_applied`. Named glue already in kernel tests. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (twin + freeze pair)
- [x] **P0.1** Twin + catalog pair — status: `done`
- [x] **P0.2** Regression — status: `done` (`disk_membership_overrides_cli_when_present` asserts identity-first)

### P1 — next wave
- [x] **P1.1** RFC-0128 campaign is not ∀ traces — status: `done` (`is_participating_campaign_is_not_forall_traces`)
- [x] **P1.2** TCP 3-process participating (0128 P1.2) — status: `done` (`l28_real_tcp_participating_after_remove`)

### P2 — later
- [x] **P2.1** R-verus still never — status: `done` (`identity_before_applied_verus_still_never`)
- [x] **P2.2** none — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | twin + catalog | done | membership_joint.rs + identity_before_applied | 2026-08-28 |
| P0.2 | p0 | AS-IS applied first | done | disk_membership_overrides_cli_when_present | 2026-08-28 |
| P1.1 | p1 | 0128 not ∀ traces | done | is_participating_campaign_is_not_forall_traces | 2026-08-28 |
| P1.2 | p1 | TCP 3-process | done | RFC-0128 P1.2 l28_real_tcp_participating_after_remove | 2026-08-28 |
| P2.1 | p2 | R-verus never | done | identity_before_applied_verus_still_never | 2026-08-28 |
| P2.2 | p2 | none | done | — | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `membership_identity_before_applied(true)` true; AS-IS false. Same tokens raft=store.
  - Catalog pair `identity_before_applied` `entry: membership_identity_before_applied`; freeze twins fail if the exec fn is dropped.
  - `is_participating_campaign_is_not_forall_traces`: `R-joint` close names campaign not a theorem.
  - `identity_before_applied_verus_still_never`: `never_floor` still lists `R-verus`.
  - `./scripts/verus_membership_joint.sh` when Verus is installed.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0128 P2.1 done; `residuals.json` `R-joint` owner 0129.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”. Closing R-verus.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Spawning 3-process L28 as this P0.
- Rocks coluna A/B. crates.io.
