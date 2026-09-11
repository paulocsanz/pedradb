# RFC-0205 P1.2 (1/2) — atom `recover_must_apply`: o destino do re-apply na recovery

Data: 2026-09-11. Par `recover_apply` (`catalog:recover_apply`,
kernel `crates/pedradb-raft/src/membership_kernel.rs`, entry
`recover_must_apply`, handler `recover_apply_committed`, chamado ao
vivo por pedradb-store/src/lib.rs). Escada: cap_data_fate 94→93,
floor_atom 37→38, floor_extract 241→240, residuals atom 37→38 /
extract 241→240 / data_fate 94→93 — tudo no MESMO commit, com a linha
do registro (`atom  catalog:recover_apply  recover_must_apply_fate_iff
formal/aeneas/lean/Membership.lean  recover_must_apply`).

## O que foi pago

O destino do re-apply sobre TODOS os inputs (Membership.lean,
`recover_must_apply_fate_iff`) — corpo pure-lift
`ok (commit > applied)`, comparação U64 Prop-valued elaborada como
`decide (commit > applied)`:

```lean
theorem recover_must_apply_fate_iff :
    ∀ (applied : U64) (commit : U64) (v : Bool),
      (recover_must_apply applied commit = ok v) ↔
        ((v = true ∧ commit > applied)
          ∨ (v = false ∧ ¬(commit > applied)))
```

Recuperação re-aplica EXATAMENTE quando commit > applied: entrada
commitada não-aplicada nunca é pulada; entrada já aplicada nunca é
repetida. O mutante AS-IS `ok false` pula tudo — committed-but-
unapplied fica perdido (a mentira que a planta DST crava antes do
trampolim).

Prova: unfold + `cases hd : decide (commit > applied)` + 
`of_decide_eq_true/false` + `cases v <;> simp [h]`.

## Verificação

- `lake build Membership` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=38 (floor 38), extract=240 (floor 240),
  data_fate=93≤93, ledger 299/266/33.
- `bash scripts/lean_extracts.sh --required` ok (61 libs + 17 compose).
- Planta DST `recover_must_apply_on_live_queued_is_not_ok`
  (pedradb-store/three_teeth_queued) — resultado capturado no commit.

## Lição Lean (banco)

`>` de U64 no extrato Aeneas é Prop-valued: o corpo é
`ok (decide (commit > applied))`. Enunciar o RHS como Prop pura
(`commit > applied`, `¬(…)`) e casoar sobre o `decide` — NUNCA
escrever `(commit > applied) = true` (parser/elaborador quebra).
