# RFC-0053 Y3 report

**Date:** 2026-08-23  
**Status:** Y3 slices Y3.1–Y3.4 landed. Reopen outcomes machine-checked + bounded liveness under an explicit axiom. π/VerusSync **não disparado** (recorded). **Not** “fsync provado.” **Not** “não há bugs.”

## LOC (this tree, Y3 delta)

| Object | Lines |
|--------|------:|
| New kernel `wal/reopen_kernel.rs` (reopen outcome decision) | 177 |
| New twin `verus/reopen_outcome.rs` (5 named lemmas + link) | 180 |
| `db.rs` reopen arms rewired to the kernel (4 arms) | ~30 changed |
| Liveness witness BFS in `compose_model.rs` | ~45 |

Proof:kernel ≈ 180/177 ≈ **1.0 : 1**. Cumulative program (Y1+Y2+Y3 kernels: vote+AE+commit+apply+recover+reopen ≈ 2 069; twins ≈ 1 741 + Lean 214) stays ≈ **0.9–1.1 : 1** — vs IronRSL 3.6 : 1, because the unit is the kernel the binary already calls, not a rewrite.

## Reopen outcomes (crash dictionary, closed)

The crash spec now runs end-to-end at the decision level:
`RecoverKind` (recover kernel, Y1 twin) → **damage arm** → `reopen_outcome` (Y3 kernel) → {`RefuseOpen`, `ServePrefixReport`, `ServeAll`}.

Named lemmas (all in `verus/reopen_outcome.rs`, `6 verified, 0 errors`, no `sorry`):

- `lemma_recover_failstop_routes_to_damage` — fail-stop recover kinds route into damage arms (the link).
- `lemma_damaged_reopen_never_silent` — damaged reopen ⇒ refuse ∨ report, never `ServeAll` (G8).
- `lemma_fail_closed_refuses_damage` — FailClosed ⇒ `RefuseOpen` on every damage.
- `lemma_clean_reopen_serves_all` — no damage ⇒ `ServeAll` (no false refusal).
- `lemma_mutant_swallows_damage` — AS-IS swallow serves a damaged WAL silently (teeth).

DST teeth unchanged and green: `crash_after_sync_recovers_committed`, `truncate_wal_drops_tail_keeps_prefix`, `explode_choose_crc_fail_stops_reopen`, `rfc20_silent_wrong_gate_matrix`, `crash_after_sync_put_reopen_recovers`.

## Bounded liveness (axiom, never theorem)

**AXIOM quórum-vivo:** the environment always grants majority and persist-Ok — no partitions, no persist failures, during the witness window. Stated in `compose_model.rs` (`bounded_liveness_under_quorum_alive_axiom`) and in the RFC TCB. Under the axiom: grant + commit + apply progress occurs **within ≤ 4 steps** (explicit witness BFS over the production kernels; bound asserted in-test, non-vacuity asserted at init). The axiom is an assumption, **never a theorem** — unconditional liveness would need fairness semantics plus a real network model.

## TCB (Y3)

**In (added):** `reopen_outcome` + twin; the reopen handler `open_with_env` as a fail-closed catalog entry.

**Axioms forever:** `persist` Ok\|Err; `fsync`/OS durability (if the OS lies → RFC-0052 det_io/TCG); CRC32C not forgeable; **quorum-alive for liveness**.

**Out (by design, unchanged):** `ConcurrentDb` (group-commit, OS threads) — π/VerusSync gated on RFC-0051 landing a PCT tooth in-tree; RFC-0051 is still `draft`, so the item is recorded as **not triggered**, which is a recorded state, not a failure. `montanha-tcp` loop; io_uring; libc.

## What DST still covers

Everything the axioms hide: real torn bytes, real CRC flips, escalation counters across processes, silent_wrong seed matrix, World cluster schedules, π on `ConcurrentDb`. A SilentWrong seed after these proofs is an axiom lie, a caller that does not refine, or twin drift.

## Telemetry (2026-08-23 run)

Verus `6 verified, 0 errors` (reopen twin ×2). DST teeth 5/5 green (sim ×3, dst gate, core reopen). Liveness witness BFS bound 4, test 0.00 s. `pedra_formal.py --ci` 169 ok / 0 fail in ~34 s; `cargo test --workspace` 0 failures in ~144 s. No p99 — proof is not a bench.

## Claims

| Allowed | Forbidden |
|---------|-----------|
| “Damaged reopen is never silent: refuse ∨ reported prefix” (Verus, named lemmas) | “dicionário ∀” / “`Db::open` proved” |
| “Under the quorum-alive axiom, election/commit/apply progress ≤ 4 steps” (witness BFS) | “liveness provada” (sem o axioma não é claim) |
| Frase da tese do RFC nos caminhos cobertos pelos kernels | “não há bugs”; “fsync provado”; “Pedra verificado” |
