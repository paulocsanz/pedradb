# RFC: 0069 — Eventual-election claim fail-closed without ES-1/ES-2/ES-3

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0056](0056-one-hundred-percent-delivery.md), [0061](0061-residuals-sel4-ironfleet.md), [0053](0053-ironfleet-years.md)

**Residual:** `R-es` (not `never_floor`). Axis vs FDB Sim2: **liveness**. IronFleet names eventual-synchrony axioms; FDB Sim2 does not distinguish a bounded elect from a liveness theorem. Pedra’s tcp_node_model already *refutes* eventuality without ES-1/ES-2/ES-3, but the live store could still treat `elect_all` Ok as “election is live.” This slice closes that rounding: an eventual-election **claim** is admitted only when all three axioms hold.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Bounded `elect_all` still works without axioms. Unconditional liveness stays a non-theorem.

## Background

- RFC-0056 P2.2: `tcp_node_model` `Property::eventually` holds under ES-1 (finite adversary), ES-2 (internal drain), ES-3 (live retrying candidate); both eventualities are refuted without the axioms.
- `StoreCluster::elect_all` is a bounded tick loop. Success is not liveness. Nothing on the store API named that.
- `R-es` close-text said “never a theorem; model refutes eventuality without the axioms” while production had no kernel for the claim.

## Problems This Solves

- **Problem:** a green `elect_all` can be rounded to “election is live.”
- **Problem:** the ES axioms lived only in a Stateright test file, not a production kernel the store calls.
- **Problem:** AS-IS admits a liveness claim with any/none of the axioms.

## Proposed Solution

- Pure `liveness_admitted(es1, es2, es3)` = all three true. AS-IS always true (the hole).
- Clone in raft+store `membership_kernel` (existing catalog pair, no new TCB file).
- Production `StoreCluster::claim_eventual_election` calls the kernel. Bounded `elect_all` is unchanged.

## Delivery slices (mandatory)

### P0 — must ship first (claim on the live store path)
- [x] **P0.1** `membership_kernel::liveness_admitted` + AS-IS + raft/store clone — status: `done`
- [x] **P0.2** `StoreCluster::claim_eventual_election` on the live cluster — status: `done`
- [x] **P0.3** Regression: after real elect, claim without axioms is false; AS-IS would admit — status: `done` (`claim_eventual_election_refused_without_es_axioms`)

### P1 — next wave
- [x] **P1.1** `tcp_node_model` liveness tests call `liveness_admitted` (kernel on the model path) — status: `done` (`tcp_node_model_liveness_claim_needs_es_axioms`; existing BFS tests call `claim_eventual_election`)
- [x] **P1.2** World refuses a liveness flag unless axioms are set — status: `done` (`world_run_refuses_eventual_election_without_es_axioms`)

### P2 — later
- [x] **P2.1** Verus twin of `liveness_admitted` — status: `done` (payment now the Aeneas extract of the raft kernel `membership_kernel.rs` — `scripts/aeneas_membership.sh`; the `verus/membership_joint.rs` mirror twin was deleted, catalog `liveness_claim`)
- [x] **P2.2** montanha-tcp / cluster_real cannot print “live” without naming ES — status: `done` (`elect_claim_banner`; `cluster_real` + `montanha-tcp` elect-wait)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | liveness_admitted kernel + AS-IS + clone | done | membership_kernel.rs (raft+store) | 2026-08-27 |
| P0.2 | p0 | claim_eventual_election on StoreCluster | done | StoreCluster::claim_eventual_election | 2026-08-27 |
| P0.3 | p0 | claim without ES tooth | done | claim_eventual_election_refused_without_es_axioms | 2026-08-27 |
| P1.1 | p1 | tcp_node_model calls kernel | done | tcp_node_model_liveness_claim_needs_es_axioms | 2026-08-28 |
| P1.2 | p1 | World liveness flag | done | world_run_refuses_eventual_election_without_es_axioms | 2026-08-28 |
| P2.1 | p2 | Verus twin | done | membership_joint.rs liveness_admitted + catalog liveness_claim | 2026-08-28 |
| P2.2 | p2 | TCP/real cannot say live | done | elect_claim_banner + cluster_real + montanha-tcp elect-wait | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `liveness_admitted(true,true,true)` is true; any false axiom → false; AS-IS is always true.
  - `claim_eventual_election_refused_without_es_axioms`: `open` + `elect_all` succeeds (bounded); `claim_eventual_election(false,true,true)` is false; all-true is true.
  - P1.1 `tcp_node_model_liveness_claim_needs_es_axioms`: `LivenessModel::fixed()` admits; unrestricted / no ES-3 / broken drain refuse; AS-IS would admit. Existing BFS tests (`liveness_holds_under_eventual_synchrony_axioms`, `liveness_is_refuted_without_axioms`, …) call the same kernel on the model path.
  - P1.2 `world_run_refuses_eventual_election_without_es_axioms`: live `World::run` default axioms off → `claim_eventual_election` false; AS-IS would admit; naming ES-1∧ES-2∧ES-3 admits.
  - P2.1 catalog pair `liveness_claim` entry `liveness_admitted` paid by the Aeneas extract of `crates/pedradb-raft/src/membership_kernel.rs` (`scripts/aeneas_membership.sh`, single artifact; the `verus/membership_joint.rs` twin was deleted — twin bodies can drift).
  - P2.2 `elect_claim_banner(false,false,false)` is `bounded-elect not-eventual` (no `live`); AS-IS is `live`. `claim_eventual_election_refused_without_es_axioms` after live `elect_all`. `cluster_real` / `montanha-tcp` elect-wait print the banner via `liveness_admitted`.
- **Telemetry / Analytics:** none — honesty invariant.
- **Documentation:** this RFC; `residuals.json` `R-es` close-text + owner 0069.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Proving Linux/`fsync`/CPU/`rustc`. Extracting `db.rs`. Flow.
- Closing `never_floor`. Making `elect_all` require ES axioms (it is bounded, not a liveness theorem).
- RFC-0066 Stateright leave. RFC-0067 Direct. Rocks coluna A/B. crates.io.
