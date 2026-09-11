# RFC-0205 P1.2 (2/2) — atom `recover_drop_orphan_seg`: o destino do drop de órfãos na recovery

Data: 2026-09-11. Par `recover_drop_orphan` (`catalog:recover_drop_
orphan`, kernel `crates/pedradb-raft/src/membership_kernel.rs`, entry
`recover_drop_orphan_seg`, handler `persist_log_db`). Escada:
cap_data_fate 93→92, floor_atom 38→39, floor_extract 240→239,
residuals atom 38→39 / extract 240→239 / data_fate 93→92 — tudo no
MESMO commit, com a linha do registro (`atom  catalog:recover_drop_
orphan  recover_drop_orphan_seg_fate_iff  formal/aeneas/lean/
Membership.lean  recover_drop_orphan_seg`).

## O que foi pago

O destino do drop de segmento órfão sobre TODOS os inputs
(Membership.lean, `recover_drop_orphan_seg_fate_iff`) — mesmo molde
pure-lift da primeira promoção (corpo `ok (seg_index > new_hi)`,
`decide` de comparação U64 Prop-valued):

```lean
theorem recover_drop_orphan_seg_fate_iff :
    ∀ (seg_index : U64) (new_hi : U64) (v : Bool),
      (recover_drop_orphan_seg seg_index new_hi = ok v) ↔
        ((v = true ∧ seg_index > new_hi)
          ∨ (v = false ∧ ¬(seg_index > new_hi)))
```

Segmento FORA do inventário commitado (índice além do high-water) é
dropado EXATAMENTE aí; segmento dentro NUNCA é dropado. O mutante
AS-IS `ok false` preserva todo órfão — inventário commitado nunca é
esvaziado, mas lixo de crash fica eternamente (a mentira que a planta
crava antes do trampolim).

## Verificação

- `lake build Membership` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=39 (floor 39), extract=239 (floor 239),
  data_fate=92≤92, ledger 299/266/33.
- `bash scripts/lean_extracts.sh --required` ok (61 libs + 17 compose).
- Planta DST `recover_drop_orphan_seg_on_live_queued_is_not_ok`
  (pedradb-store/three_teeth_queued) 1 passed; 0 failed.

## Cadência P1.2 fechada

2/2 promoções do pool, uma por commit (1/2 `e356a071`
recover_apply; 2/2 este recover_drop_orphan), cap 94→92 como
planejado no RFC. Sem quedas medidas — os dois candidatos nomeados
eram pure-lifts tratáveis.
