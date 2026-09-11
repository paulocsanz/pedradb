# RFC-0208 P1.2 (2/4) — atom `disk_membership`: a identidade de disco vence a CLI no reopen

Data: 2026-09-11. Par `disk_membership` (`catalog:disk_membership`,
kernel `crates/pedradb-raft/src/membership_kernel.rs`, entry
`disk_membership_overrides_cli`, chamado ao vivo por
`bind_cluster_identity` no pedradb-store). Escada: cap_data_fate
89→88, floor_atom 42→43, floor_extract 236→235, residuals
atom 42→43 / extract 236→235 / data_fate 89→88 — tudo no MESMO
commit, com a linha do registro
(`atom  catalog:disk_membership  disk_membership_overrides_cli_fate_iff
formal/aeneas/lean/Membership.lean  disk_membership_overrides_cli`).

## O que foi pago

O destino da identidade do cluster no reopen sobre TODOS os inputs
(Membership.lean, `disk_membership_overrides_cli_fate_iff`) — corpo
pure-lift `ok has_disk`:

```lean
theorem disk_membership_overrides_cli_fate_iff :
    ∀ (has_disk : Bool) (v : Bool),
      (disk_membership_overrides_cli has_disk = ok v) ↔
        ((v = true ∧ has_disk = true)
          ∨ (v = false ∧ has_disk = false))
```

A membership persistida em disco vence EXATAMENTE quando ela
existe: a flag de CLI nunca sobrescreve a identidade persistida.
O mutante AS-IS `ok false` sempre deixa a CLI vencer — split-brain
no reopen de um nó que reentra com flag errada.

## Verificação

- `lake build Membership` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=43 (floor 43), extract=235 (floor 235),
  data_fate=88≤88, ledger 299/266/33.
- Planta DST `disk_membership_overrides_cli_on_live_queued_is_not_ok`
  (pedradb-store/three_teeth_queued) — 1 passed.
