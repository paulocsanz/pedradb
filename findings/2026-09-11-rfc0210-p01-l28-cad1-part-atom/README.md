# RFC-0210 P0.1 (2/4) — atom `l28_tcp_part`: votante removido participa iff o scan diz

Data: 2026-09-11. Par `l28_tcp_part` (`catalog:l28_tcp_part`,
kernel `crates/pedradb-store/src/l28.rs`, entry `l28_tcp_part_ok`).
Escada: cap_data_fate 83→82, floor_atom 48→49,
floor_extract 230→229, residuals atom 48→49 / extract 230→229 /
data_fate 83→82 — tudo no MESMO commit, com a linha do registro
(`atom  catalog:l28_tcp_part  l28_tcp_part_ok_fate_iff
formal/aeneas/lean/L28.lean  l28_tcp_part_ok`).

## O que foi pago

Segunda promoção da cadência 1/4 — o destino do reporte de
participação sobre TODOS os inputs (L28.lean,
`l28_tcp_part_ok_fate_iff`) — corpo pure-lift `ok b`:

```lean
theorem l28_tcp_part_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_part_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false))
```

Depois de uma remoção, um votante removido é reportado
participante EXATAMENTE quando o scan de participação diz — um mapa
CLI/`nodes` stale não pode contá-lo. O mutante AS-IS `ok true` pula
o scan (o leftover 0127: só a flag de reopen) — a mentira que a
planta TCP REAL (`l28_real_tcp_participating_after_remove`)
refuta.

## Verificação

- `lake build L28` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=49 (floor 49), extract=229 (floor 229),
  data_fate=82≤82, ledger 299/266/33.
- Planta TCP REAL `l28_real_tcp_participating_after_remove`
  (pedradb-store/tests/l28_real_tcp.rs) — 1 passed (381.29s).
