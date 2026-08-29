# RFC: 0079 — Native World is not TCG guest coverage (fail-closed)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0052](0052-dst-inside-boxes.md), [0061](0061-residuals-sel4-ironfleet.md), [0050](0050-world-in-tree-fdb-determinism.md)

**Residual:** `R-tcg-guest` (not `never_floor`). Axis vs FDB Sim2: **G3 disk / TCG box** — FDB Sim2 is not a QEMU TCG guest either. Pedra’s native `World::run` is the in-tree DST fingerprint (seed → `trace_hash`). `scripts/tcg_guest_status.sh` already prints `C2.2=residual_no_guest` when `PEDRA_QEMU_SSH` is unset. A green World smoke can still be rounded to “TCG covered.” This slice names the gate on the **live World path**: native `World::run` does not invent a guest and `claim_tcg_guest` is refused.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. This does **not** invent a QEMU guest. CI without `PEDRA_QEMU_SSH` stays residual. `TCG_REQUIRED=1` remains the script fail-closed. Native=guest `trace_hash` is still RFC-0052 P2.1 when a guest exists.

## Background

- RFC-0052 P2.2: `tcg_guest_status.sh` — no SSH ⇒ `C2.2=residual_no_guest` (exit 0, not a fake TCG_PASS).
- `World::run` is native (Darwin/Linux host). It never SSHs. A CLEAN World campaign is not C2.1.
- FDB Sim2 also does not run under TCG. Pedra must not round the native fingerprint to a guest.

## Problems This Solves

- **Problem:** TCG honesty lived only in the shell script; `Trace` had no named decision.
- **Problem:** AS-IS treats a native World run as guest coverage.
- **Problem:** inventing a guest in-process would be a fake-green.

## Proposed Solution

- Pure `tcg_guest_admitted(guest_reachable)` = `guest_reachable`. AS-IS always true.
- Production `World::run` records `tcg_guest_admitted(false)` on `Trace` (native World does not SSH). `Trace::claim_tcg_guest` returns that bit. No new `*_kernel.rs`. Do not invent a guest.

## Delivery slices (mandatory)

### P0 — must ship first (gate on live World::run)
- [x] **P0.1** `tcg_guest_admitted` + AS-IS — status: `done`
- [x] **P0.2** `World::run` records the gate on `Trace`; `claim_tcg_guest` — status: `done`
- [x] **P0.3** Regression: live `World::run`, claim is false; AS-IS would admit — status: `done` (`claim_tcg_guest_refused_on_native_world`)

### P1 — next wave
- [x] **P1.1** `tcg_guest_status.sh` prints the same kernel name (`tcg_guest_admitted`) — status: `done` (`tcg_guest_status_script_names_kernel`)
- [x] **P1.2** `world_smoke` bin refuses a `--claim-tcg` flag unless admitted — status: `done` (`allow_claim_tcg_flag`; `world_smoke_refuses_claim_tcg_on_native`)

### P2 — later
- [x] **P2.1** Catalog / Verus token — status: `done` (`verus/tcg_guest.rs` + catalog `tcg_guest`)
- [x] **P2.2** Guest SSH probe stays the script; World still does not SSH — status: `done` (`world_runs_guest_ssh`; `world_still_does_not_ssh`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | tcg_guest_admitted + AS-IS | done | world/lib.rs | 2026-08-27 |
| P0.2 | p0 | World::run records gate | done | Trace.tcg_guest | 2026-08-27 |
| P0.3 | p0 | native World claim tooth | done | claim_tcg_guest_refused_on_native_world | 2026-08-27 |
| P1.1 | p1 | script names kernel | done | tcg_guest_status_script_names_kernel | 2026-08-28 |
| P1.2 | p1 | world_smoke --claim-tcg | done | world_smoke_refuses_claim_tcg_on_native | 2026-08-28 |
| P2.1 | p2 | catalog + Verus | done | tcg.rs tcg_guest_admitted + catalog tcg_guest | 2026-08-28 |
| P2.2 | p2 | World still does not SSH | done | world_runs_guest_ssh + world_still_does_not_ssh | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `tcg_guest_admitted(false)` is false; AS-IS true.
  - `claim_tcg_guest_refused_on_native_world`: `World::run` (live `StoreCluster` + schedule), `Trace::claim_tcg_guest` is false. Runs on Darwin. Does not set `PEDRA_QEMU_SSH`. Does not invent a guest.
  - P1.1 `tcg_guest_status_script_names_kernel`: script prints `kernel=tcg_guest_admitted` and `tcg_guest_admitted=0` on the residual path (`C2.2=residual_no_guest`).
  - P1.2 `world_smoke_refuses_claim_tcg_on_native`: live `world_smoke --claim-tcg` exits 2; `allow_claim_tcg_flag(true, false)` is false; AS-IS would admit. No guest invented.
  - P2.1 catalog pair `tcg_guest` entry `tcg_guest_admitted` with Verus twin in `verus/tcg_guest.rs` (freeze of twin files; `verus` not on PATH). No new `*_kernel.rs`.
  - P2.2 `world_still_does_not_ssh`: `world_runs_guest_ssh()` false; AS-IS would admit; live `World::run` `claim_tcg_guest` false; SSH probe stays `tcg_guest_status.sh`.
- **Telemetry / Analytics:** none — honesty invariant.
- **Documentation:** this RFC; `residuals.json` `R-tcg-guest` close-text + owner 0079 (keep `script` + `C2.2=residual_no_guest`).
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Inventing a QEMU guest. SSHing from `World::run`. Closing CI-without-env.
- Restoring production WAL onto io_uring. Extracting `db.rs`. `never_floor`.
- Rocks coluna A/B. crates.io.
