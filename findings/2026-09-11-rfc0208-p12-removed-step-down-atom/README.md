# RFC-0208 P1.2 (1/4) — atom `removed_step_down`: o step-down do nó removido

Data: 2026-09-11. Par `removed_step_down` (`catalog:removed_step_down`,
kernel `crates/pedradb-raft/src/membership_kernel.rs`, entry
`removed_steps_down`, chamado ao vivo por `install_applied_membership`
no pedradb-store). Escada: cap_data_fate 90→89, floor_atom 41→42,
floor_extract 237→236, residuals atom 41→42 / extract 237→236 /
data_fate 90→89 — tudo no MESMO commit, com a linha do registro
(`atom  catalog:removed_step_down  removed_steps_down_fate_iff
formal/aeneas/lean/Membership.lean  removed_steps_down`).

## O que foi pago

O destino do step-down sobre TODOS os inputs (Membership.lean,
`removed_steps_down_fate_iff`) — corpo pure-lift `ok (¬ in_ids)`:

```lean
theorem removed_steps_down_fate_iff :
    ∀ (in_ids : Bool) (v : Bool),
      (removed_steps_down in_ids = ok v) ↔
        ((v = true ∧ ¬ (in_ids = true))
          ∨ (v = false ∧ in_ids = true))
```

O nó sai do cargo EXATAMENTE quando seu id saiu do conjunto
committado: quem ficou nunca sai, quem saiu sempre sai. O mutante
AS-IS `ok false` nunca sai do cargo — um líder removido seguiria
liderando.

## Verificação

- `lake build Membership` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=42 (floor 42), extract=236 (floor 236),
  data_fate=89≤89, ledger 299/266/33.
- Planta DST `removed_steps_down_on_live_queued_is_not_ok`
  (pedradb-store/three_teeth_queued) — 1 passed.
