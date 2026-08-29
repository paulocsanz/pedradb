# RFC: 0094 — Stateright leave-joint: C-old majority is not an election after commit without leave

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0066](0066-joint-leave-fail-closed.md), [0064](0064-joint-election-fail-closed.md), [0068](0068-world-plant-committed-joint-fail-closed.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-joint` (not `never_floor`). Axis vs FDB Sim2: **membership / reconfig** — Sim2’s reconfig is out-of-band. Pedra’s `joint_still_active` keeps C-old∧C-new until leave (`old == new`). RFC-0064’s Stateright model only covers **uncommitted** joint election (`joint_election_ok`). RFC-0066 P1.1 named the missing model: after a joint **commits** and leave has not, AS-IS (`joint_still_active_as_is` always false) treats the config as single C-old and elects on old majority. This slice is that model, calling the **same** kernels production uses.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Campaign, not ∀ traces. Production G1 stays POSIX (do not restore WAL onto io_uring).

## Background

- Store `pending_joint_on` + `election_has_joint_quorum` call `joint_still_active` then `joint_election_ok`.
- `tests/joint_model.rs` is RFC-0064 (in-flight joint), not leave.
- World plant is RFC-0068. Stateright leave was still `todo`.

## Problems This Solves

- **Problem:** leave-joint had a kernel + store tooth, no Stateright discovery of the AS-IS hole.
- **Problem:** 0064’s model cannot see `joint_still_active_as_is`.
- **Problem:** `R-joint` close-text still pointed at 0066 P1.1 as unpaid.

## Proposed Solution

- Stateright model: joint committed (C-old=3, C-new=4), optional leave. TryElect calls `joint_still_active` then `joint_election_ok` (same order as store). AS-IS drops the joint and elects on 2/3 C-old. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (Stateright leave-joint)
- [x] **P0.1** Model drives `joint_still_active` + `joint_election_ok` — status: `done`
- [x] **P0.2** Fixed holds; AS-IS discovers old-only election — status: `done` (`fixed_leave_joint_holds`, `as_is_elects_without_leave`)

### P1 — next wave
- [x] **P1.1** Verus twin of `joint_still_active` (0066 P2.1) — status: `done` (RFC-0095)
- [x] **P1.2** Store clone tokens stay identical — status: `done` (`membership_raft_store_clone_tokens_stay_identical`)

### P2 — later
- [x] **P2.1** L28 REAL leave-joint on `cluster_real` (0066 P2.2) — status: `done` (RFC-0121 P1.2)
- [x] **P2.2** Campaign is not ∀ traces — status: `done` (`leave_joint_campaign_is_not_forall_traces`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Stateright drives leave kernels | done | tests/joint_leave_model.rs | 2026-08-28 |
| P0.2 | p0 | AS-IS elects without leave | done | as_is_elects_without_leave | 2026-08-28 |
| P1.1 | p1 | Verus twin | done | RFC-0095 | 2026-08-28 |
| P1.2 | p1 | store clone tokens | done | membership_raft_store_clone_tokens_stay_identical | 2026-08-28 |
| P2.1 | p2 | L28 REAL leave | done | RFC-0121 P1.2 l28_real_tcp_remove_member_left_on_disk | 2026-08-28 |
| P2.2 | p2 | not ∀ traces | done | leave_joint_campaign_is_not_forall_traces | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `joint_still_active([1,2,3],[1,2,3,4])` true; AS-IS false.
  - `fixed_leave_joint_holds`: BFS, Inv-leave (never `elected_old_only`).
  - `as_is_elects_without_leave`: AS-IS discovers Inv-leave (2/3 C-old after committed joint, no leave).
  - Existing `fixed_joint_election_holds` / `as_is_elects_on_old_only` (0064) stay green.
  - P1.2 `membership_raft_store_clone_tokens_stay_identical`: raft and store `joint_still_active` / `joint_leave_ok` bodies collapse to the same tokens; catalog clone `membership_raft_store` lists both. Freeze `--clones` is the mechanical trap.
  - P2.2 `leave_joint_campaign_is_not_forall_traces`: `R-joint` close-text names campaign not a theorem; catalog still lists `joint_leave_model`. Not ∀ Raft traces. L28 REAL leave stays 0066 P2.2.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; RFC-0066 P1.1 done; `residuals.json` `R-joint` owner 0094.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`.
- Changing `joint_still_active` (already shipped 0066 P0).
- Rocks coluna A/B. crates.io.
