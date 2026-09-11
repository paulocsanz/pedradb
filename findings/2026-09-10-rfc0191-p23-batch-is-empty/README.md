# RFC-0191 P2.3 passo 29 — `batch_is_empty` a atom (deficit-2, primeiro fora da fila cf)

Fire 787. A fila cf esgotou no passo 28 com `cap_data_fate` 102 e alvo ≤100
(déficit 2). Auditoria do catálogo achou 67 pares sem campo `aeneas` cujos
kernels JÁ têm extrato Aeneas em `formal/aeneas/out/lean/` — a promoção é um
fire normal, sem trabalho novo de pipeline. Este é o primeiro: a fila
`write_admission_kernel.rs`.

## Par

- `batch_is_empty` (`crates/pedradb-core/src/write_admission_kernel.rs` L240),
  extrato `WriteAdmissionKernel.lean` L287, pago já antes pelo script verus
  (`scripts/verus_write_admission.sh`).
- Corpo extraído: `ok (n = 0#u64)` — um lift puro, zero passos monádicos.

## Teorema (WriteAdmission.lean, 0 sorry)

`batch_is_empty_ok_iff_zero : ∀ (n : U64) (v : Bool),
  (batch_is_empty n = ok v) ↔ ((n = 0#u64 : Bool) = v)`

A iff é a regra de computação inteira do do-block: sem `let ←`, nenhum passo
monádico existe atrás do qual uma disposição possa se esconder. A anotação
`: Bool` no RHS elabora pela MESMA coerção (decide) que o corpo do extrato —
os dois lados da iff são o mesmo termo Lean.

Prova: `unfold` + `constructor`; forward `injection` (auto-substitui v);
backward `rw [h]`. Nota de máquina: `injection h with h` seguido de `exact h`
falha com "No goals" — a injection já fecha o objetivo quando v é local livre.

## Números

| métrica | antes | depois |
|---|---|---|
| floor_atom | 29 | 30 |
| floor_extract | 250 | 249 |
| cap_data_fate | 102 | 101 |
| df live | 102 | 101 |

Gates GREEN (depth-floor extract=249/atom=30/df 101≤101; product-floor).
Plant `batch_is_empty_on_live_zero_is_not_ok` 1 passed. Lint:
`ok batch_is_empty: proof_depth=atom (2026-09-10)`, counts
`extract=249 close=2 atom=30 model=17`; 153 FAILs todos herdados da sessão
paralela (drift handler_loc/kernel_loc + surface do `ratio_curve_kernel.rs`).

## Déficit

Falta 1 descida (101→≤100). Próximo candidato da mesma fila:
`dir_sync_required (sync : Bool)` — corpo `ok sync`, identidade pura.
