# RFC-0191 P2.3 passo 22 — `cf_encode_effective` graduado a atom

Data: 2026-09-10 · Fire 780

## O que subiu

`cf_encode_effective` (cf_kernel.rs, par `cf_encode_effective`,
RFC-0150 P0) saiu de `close` para `atom` com um ∀ iff registrado em
`formal/aeneas/lean/Cf.lean`:

```lean
theorem cf_encode_effective_empty_iff_default_raw_else_identity :
    ∀ (cf : Str) (default_raw : Bool),
      (cf_encode_effective cf default_raw = ok (toStr ""))
        ↔ ((Str.Insts.CoreCmpPartialEqStr.eq cf (toStr "default") = ok true
            ∧ default_raw = true)
          ∨ (((Str.Insts.CoreCmpPartialEqStr.eq cf (toStr "default") = ok true
              ∧ default_raw = false)
              ∨ Str.Insts.CoreCmpPartialEqStr.eq cf (toStr "default") = ok false)
            ∧ cf = toStr ""))
```

0 sorry, `lake build Cf` verde. Planta (dente) já verde:
`cf_encode_effective_on_live_default_raw_is_not_ok`.

## Por que essa forma (soundness do disjuncto cf = "")

O branch else devolve `ok cf` intacto — então "encoding vazio" também
acontece com cf já vazio (b=true, default_raw=false incluso). Pior: a
igualdade de `Str` é opaca e seu resultado é `Result` — se o `eq`
puder fail/div, um simples `∨ cf = ""` tornaria o iff FALSO (LHS
fail/div, RHS verdadeiro). A segunda disjunção exige explicitamente
`eq = ok true ∧ default_raw = false` OU `eq = ok false`, então
fail/div da comparação nunca finge um encoding vazio.

## Prova

`cases he : …eq cf "default"` substitui o argumento do bind NO GOAL
(cases `h : e` generaliza e; rw só seria preciso em hipóteses):
branch `ok b` fecha com `cases b <;> cases default_raw <;> simp`
(bind_ok + literais + injEq de `ok`); fail/div: forward `simp at h`
(bind_fail/bind_div), backward `rintro (⟨h1, _⟩ | ⟨(⟨h1, _⟩ | h1), _⟩)
<;> exact absurd h1 (by simp)`.

## Lore nova

`rintro` com or aninhado `(A | ((B | C) ∧ D))` produz **3** goals,
não 2 — bullets contados errado deixam `inr.inr` em aberto (erro
"unsolved goals" com `h1 : fail e = ok false` no contexto). Usar
`<;>` no lugar de bullets manuais.

## Contas (mesma commit)

- `cap_data_fate` 109 → 108; `floor_atom` 22 → 23; `floor_extract` 257 → 256
- `close_proofs.tsv`: linha `atom catalog:cf_encode_effective …`
- gates GREEN (`data_fate=108<=108`)

## Estado da fila (após este passo)

Faltam 8 descidas para cap ≤ 100; fila viva = 6 (pin_gc, leveling,
compact_rewrites_sst_cf, decode_cf_key, encode_cf_key, infer_sst_cf)
→ melhor caso cap 102 (déficit 2; plano no diário Fire 775). Os 4 cf_*
restantes compõem Str/Vec opacos (encode_cf_key/decode_cf_key/
infer_sst_cf/compact_rewrites_sst_cf) — mais pesados que este.
