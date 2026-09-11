# RFC-0210 P1.1 (2/8) — atom `l28_tcp_fence`: abort fence persistido iff o persist aconteceu

Data: 2026-09-11. Par `l28_tcp_fence` (`catalog:l28_tcp_fence`,
kernel `crates/pedradb-store/src/l28.rs`, entry `l28_tcp_fence_ok`).
Escada: cap_data_fate 75→74, floor_atom 56→57,
floor_extract 222→221 — tudo no MESMO commit, com a linha do
registro (`atom  catalog:l28_tcp_fence  l28_tcp_fence_ok_fate_iff
formal/aeneas/lean/L28.lean  l28_tcp_fence_ok`).

## O que foi pago

O destino do persist do abort fence sobre TODOS os inputs (L28.lean,
`l28_tcp_fence_ok_fate_iff`) — corpo pure-lift `ok b`: o abort
fence é persistido numa réplica tirada de `ids` (planta TCP REAL +
morte de processo) EXATAMENTE quando o persist aconteceu. O
mutante AS-IS `ok true` pula o persist (leftover 0136: só ids) — a
mentira que a planta TCP REAL `l28_real_tcp_removed_fence` refuta.

## Verificação

- `lake build L28` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=57 (floor 57), extract=221 (floor 221),
  data_fate=74≤74, ledger 299/266/33.
- Planta TCP REAL `l28_real_tcp_removed_fence`
  (pedradb-store/tests/l28_real_tcp.rs) — verde.
