# RFC: 0067 — DST Queued RPC pin fail-closed (Direct cannot skip Net after World)

**Status:** done
**Updated:** 2026-08-27
**Parents:** [0063](0063-fdb-reliability-close-the-system-gap.md), [0050](0050-world-in-tree-fdb-determinism.md), [0051](0051-beyond-fdb-sim-holes.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-direct-rpc` (not `never_floor`). Axis vs FDB Sim2: **G4 network delivery** — Sim2 always goes through `Sim2Conn` (`deque` + `delay()`); Pedra `RpcMode::Direct` is a same-`PeerMsg` sync pump that **skips** `InProcessNet` drop/reorder/delay. `World::run` already calls `set_rpc_mode(Queued)`, but `set_rpc_mode(Direct)` can switch back with no pin. This slice closes that hole: once DST pins Queued, Direct is refused.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. Direct remains a lab leftover for unit tests that never pin. The World fingerprint is Queued; the pin is one named fail-closed, not a theorem that every RPC in the product goes through TCP.

## Background

- RFC-0050 P1.1: Direct and Queued share the `PeerMsg` codec (not a second protocol). Direct is still a second *delivery* the seed does not see.
- RFC-0063 P0.1 named residual `R-direct-rpc` and left it `continuous`. `World::run` forces Queued; `StoreCluster::set_rpc_mode` assigned any mode, including Direct after that force.
- FDB G4: real TCP/TLS is outside Sim2; their sim still *always* queues. Pedra’s Direct path is the extra bypass Sim2 does not have.

## Problems This Solves

- **Problem:** DST / World can be switched back to Direct after `set_rpc_mode(Queued)` — drop/reorder/partition become no-ops.
- **Problem:** `R-direct-rpc` close-text said “World fingerprint is Queued” while the setter was unpinned.
- **Problem:** no kernel decision — the pin lived only as a comment on `World::run`.

## Proposed Solution

- Pure `allow_direct_rpc(dst_pin, want_direct)`: Direct is admitted iff the caller wants Direct **and** DST is not pinned. AS-IS always admits Direct (the hole).
- Production: `StoreCluster::pin_dst_queued()` sets the pin and forces Queued. `set_rpc_mode(Direct)` is a no-op while pinned.
- `World::run` calls `pin_dst_queued()` (not a bare `set_rpc_mode(Queued)`).

## Delivery slices (mandatory)

### P0 — must ship first (pin on the live store path)
- [x] **P0.1** `rpc_mode_kernel::allow_direct_rpc` + AS-IS + catalog pair — status: `done`
- [x] **P0.2** `StoreCluster::pin_dst_queued`; `set_rpc_mode(Direct)` refused while pinned; `World::run` pins — status: `done`
- [x] **P0.3** Regression: after pin, `set_rpc_mode(Direct)` stays Queued; AS-IS would switch — status: `done` (`pin_dst_queued_refuses_direct_switch`)

### P1 — next wave
- [x] **P1.1** World `Action` that attempts Direct after pin (dente: AS-IS fingerprint skips Net) — status: `done` (`world_attempt_direct_after_pin_stays_queued`)
- [x] **P1.2** `cluster_real` / `montanha-tcp` cannot drop to Direct mid-run — status: `done` (`open_single_node` pins; `open_single_node_refuses_direct_switch`; `montanha-tcp` pins)

### P2 — later
- [x] **P2.1** Verus twin of `allow_direct_rpc` (TCB freeze requires the pair) — status: `done`
- [x] **P2.2** Remove Direct from default `open` (Queued-only production ctor; Direct opt-in for unpinned unit tests) — status: `done` (`default_open_starts_queued`; `open_lab_direct` / `enable_lab_direct_rpc`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | allow_direct_rpc kernel + AS-IS + catalog | done | rpc_mode_kernel.rs | 2026-08-27 |
| P0.2 | p0 | pin_dst_queued; World::run pins | done | set_rpc_mode + World::run | 2026-08-27 |
| P0.3 | p0 | pinned Direct tooth | done | pin_dst_queued_refuses_direct_switch | 2026-08-27 |
| P1.1 | p1 | World Action Direct-after-pin | done | world_attempt_direct_after_pin_stays_queued | 2026-08-27 |
| P1.2 | p1 | TCP/real cannot drop to Direct | done | open_single_node_refuses_direct_switch | 2026-08-27 |
| P2.1 | p2 | Verus twin | done | verus/rpc_mode.rs + verus_rpc_mode.sh | 2026-08-27 |
| P2.2 | p2 | Queued-only production open | done | default_open_starts_queued + open_lab_direct | 2026-08-27 |

## Acceptance Criteria

- **Tests**
  - `allow_direct_rpc(true, true)` is false; `allow_direct_rpc_as_is(true, true)` is true.
  - `pin_dst_queued_refuses_direct_switch`: `open_with_rng` starts Queued; lab opt-in Direct then `pin_dst_queued` forces Queued; subsequent `set_rpc_mode(Direct)` leaves Queued. Existing `direct_pump_and_queued_share_peer_msg_semantics` stays green (unpinned lab path).
  - P2.2 `default_open_starts_queued`: production `open`/`open_with_rng` is Queued; `open_lab_direct` opts into Direct; after pin Direct is refused.
  - P1.1 `world_attempt_direct_after_pin_stays_queued`: `Action::AttemptDirectRpc` after pin is `direct_rpc_refused`; `silent_wrong=0`; AS-IS kernel would admit Direct.
  - P1.2 `open_single_node_refuses_direct_switch`: TCP ctor is pinned; `set_rpc_mode(Direct)` stays Queued. `montanha-tcp` calls `pin_dst_queued` after open.
- **Telemetry / Analytics:** none — safety invariant.
- **Documentation:** this RFC; `residuals.json` `R-direct-rpc` close-text + owner 0067; 0063 pointer.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Proving Linux/`fsync`/CPU/`rustc`. Extracting `db.rs` (L46). Flow (L29).
- Closing `never_floor` (`R-cpu` … `R-extract`).
- Making Direct a second codec (already refused, RFC-0050 P1.1).
- RFC-0065 P2 raftdb/cache. RFC-0066 P1 Stateright leave. Rocks coluna A/B. crates.io.
- Claiming every production RPC is TCP (G4 residual stays continuous).
