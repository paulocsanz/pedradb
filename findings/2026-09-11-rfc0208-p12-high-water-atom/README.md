# RFC-0208 P1.2 (3/4) — atom `high_water`: o high-water de inventário é max(disk, ram)

Data: 2026-09-11. Par `high_water` (`catalog:high_water`, kernel
`crates/pedradb-raft/src/membership_kernel.rs`, entry
`high_water_at_least`, chamado ao vivo por
`open_single_node_with_rng_opts` no pedradb-store). Escada:
cap_data_fate 88→87, floor_atom 43→44, floor_extract 235→234,
residuals atom 43→44 / extract 235→234 / data_fate 88→87 — tudo no
MESMO commit, com a linha do registro
(`atom  catalog:high_water  high_water_at_least_fate_iff
formal/aeneas/lean/Membership.lean  high_water_at_least`).

## O que foi pago

O destino do high-water de inventário no reopen sobre TODOS os
inputs (Membership.lean, `high_water_at_least_fate_iff`) — corpo é
o trait-default `Ord::max` sobre a ordem U64
(`max_body lt x y = do if ← lt x y then ok y else ok x`):

```lean
theorem high_water_at_least_fate_iff :
    ∀ (disk_hw ram_hw : U64) (v : U64),
      (high_water_at_least disk_hw ram_hw = ok v) ↔
        ((v = ram_hw ∧ disk_hw < ram_hw)
          ∨ (v = disk_hw ∧ ¬ (disk_hw < ram_hw)))
```

O high-water resultante é EXATAMENTE max(disk, ram): se o disco
está abaixo da ram, a ram vence; se não, o disco vence. O mutante
AS-IS `ok ram_hw` fica com a ram e pode PERDER inventário committado
(high-water durável acima da ram no reopen). Prova: a semântica do
`lt` escalar é `ok (decide (x < y))` por `rfl` (liftFun2), e o
do-block do `max_body` reduz por `simp` com o `decide` caso a caso.

## Verificação

- `lake build Membership` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=44 (floor 44), extract=234 (floor 234),
  data_fate=87≤87, ledger 299/266/33.
- Planta DST `high_water_at_least_on_live_queued_is_not_ok`
  (pedradb-store/three_teeth_queued) — 1 passed.
