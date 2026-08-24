# RFC-0053 Y2 report

**Date:** 2026-08-23  
**Status:** Y2 slices Y2.1–Y2.3 landed. Caller refinement AE + store apply as kernels. **Not** a ∀ of `handle_append_entries` byte-for-byte. **Not** “Pedra verificado.”

## LOC (this tree, Y2 delta)

| Object | Lines |
|--------|------:|
| New kernel `apply_kernel.rs` (apply-loop decision) | 139 |
| New twin `verus/apply_advance.rs` | 102 |
| AE caller twin `verus/ae_ack_success.rs` (lemma added) | 49 |
| `compose_model.rs` (vote ∧ AE ∧ commit ∧ apply ∧ reopen) | 465 |
| Production callers wired (`lib.rs` `apply_committed` → `apply_advance`) | ~15 changed |

Proof:kernel on the Y2 pair ≈ 151/139 ≈ **1.1 : 1** (same shape as Y1's 886/784 ≈ 1.1 : 1). IronRSL was 3.6 : 1 on a Dafny rewrite — the rácio stays ~1 : 1 because the sandwich adds a twin per kernel, not a rewrite.

## The Y2 reduction (TCB v2)

Um passo de host = “ler inputs → kernel → persist → outputs”:

| Host step | Kernel(s) | Persist | Output gate | Twin lemma |
|---|---|---|---|---|
| `handle_request_vote_with_persist` | `vote_decision` | hard state | `grant_after_persist` | `g ⇒ persist Ok` (Y1) |
| `handle_append_entries` | `ae_prev_log_ok` + `ae_entry_action` | `persist_log` | `ae_ack_success` | `lemma_success_reply_only_after_persist` (Y2.1, `3 verified`) |
| `apply_committed` | `apply_advance` | (applied state) | `last_applied += 1` | `lemma_apply_only_contiguous_committed_prefix` (Y2.2, `3 verified`) |

Interleaving de hosts = Stateright (`tests/compose_model.rs`, 6 testes: FIXED + 4 mutantes AS-IS + liveness). Interleaving de threads dentro do host = RFC-0051 (fora).

## TCB (Y2)

**In (added):** `apply_advance` + its twin; the AE caller refinement lemma; AE handlers (`handle_append_entries`, `rpc_append_entries`) and the apply handler (`apply_committed`) as fail-closed catalog entries (`data_fate` + `handlers` — o lint fica vermelho se o handler largar o kernel; teste negativo feito).

**Axioms forever:** `persist` Ok\|Err atómico; store write durability; CRC; clock.

**Out (unchanged):** `ConcurrentDb`; `montanha-tcp` loop; io_uring; libc.

## What DST still covers

Crash/torn/CRC teeth (unchanged, `docs/formal/crash-dictionary.md`); World cluster schedules (RFC-0050, sibling); π on `ConcurrentDb` (RFC-0051 — no in-tree tooth yet); det_io/TCG (RFC-0052). The apply path's store-write durability is still DST/axiom — the twin proves the *loop decision*, not the LSM write.

## Telemetry (2026-08-23 run)

Verus `3 verified, 0 errors` per twin run (AE caller ×2, apply ×2). `compose_model` 6/6 in 0.07 s. `pedra_formal.py --ci` 169 ok / 0 fail in ~34 s. No p99 — proof is not a bench (RFC-0041 floor untouched).

## Claims

| Allowed | Forbidden |
|---------|-----------|
| “AE ack success ⇒ (clean log ∨ persist Ok)” (Verus, named lemma) | “AppendEntries verificado ∀” |
| “Apply loop only advances on contiguous committed entries” (Verus + Stateright) | “store apply provado” (durability é axioma) |
| “4 mutantes AS-IS dóem no modelo composto” | “Pedra verificado” |
