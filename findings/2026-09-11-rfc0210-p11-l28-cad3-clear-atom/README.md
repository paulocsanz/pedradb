# RFC-0210 P1.1 (3/8) — atom `l28_tcp_clear`: clear force-local derruba intents presos iff derrubou

Data: 2026-09-11. Par `l28_tcp_clear` (`catalog:l28_tcp_clear`,
kernel `crates/pedradb-store/src/l28.rs`, entry `l28_tcp_clear_ok`).
Escada: cap_data_fate 74→73, floor_atom 57→58,
floor_extract 221→220 — tudo no MESMO commit, com a linha do
registro (`atom  catalog:l28_tcp_clear  l28_tcp_clear_ok_fate_iff
formal/aeneas/lean/L28.lean  l28_tcp_clear_ok`).

## O que foi pago

O destino do clear force-local sobre TODOS os inputs (L28.lean,
`l28_tcp_clear_ok_fate_iff`) — corpo pure-lift `ok b`: o clear TX
force-local derruba os intents presos numa réplica tirada de `ids`
(planta TCP REAL + morte de processo) EXATAMENTE quando os
derrubou. O mutante AS-IS `ok true` pula o clear (leftover 0137:
só ids) — a mentira que a planta TCP REAL
`l28_real_tcp_removed_clear` refuta.

## Verificação

- `lake build L28` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=58 (floor 58), extract=220 (floor 220),
  data_fate=73≤73, ledger 299/266/33.
- Planta TCP REAL `l28_real_tcp_removed_clear`
  (pedradb-store/tests/l28_real_tcp.rs) — verde.
