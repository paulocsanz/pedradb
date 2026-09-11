# RFC-0210 P1.1 (4/8) — atom `l28_tcp_pre`: preimages de TX caem iff a queda aconteceu

Data: 2026-09-11. Par `l28_tcp_pre` (`catalog:l28_tcp_pre`,
kernel `crates/pedradb-store/src/l28.rs`, entry `l28_tcp_pre_ok`).
Escada: cap_data_fate 73→72, floor_atom 58→59,
floor_extract 220→219 — tudo no MESMO commit, com a linha do
registro (`atom  catalog:l28_tcp_pre  l28_tcp_pre_ok_fate_iff
formal/aeneas/lean/L28.lean  l28_tcp_pre_ok`).

## O que foi pago

O destino da queda de preimages sobre TODOS os inputs (L28.lean,
`l28_tcp_pre_ok_fate_iff`) — corpo pure-lift `ok b`: as preimages
de TX caem numa réplica tirada de `ids` (planta TCP REAL + morte
de processo) EXATAMENTE quando a queda aconteceu. O mutante AS-IS
`ok true` pula a queda (leftover 0138: só ids) — a mentira que a
planta TCP REAL `l28_real_tcp_removed_pre` refuta.

## Verificação

- `lake build L28` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=59 (floor 59), extract=219 (floor 219),
  data_fate=72≤72, ledger 299/266/33.
- Planta TCP REAL `l28_real_tcp_removed_pre`
  (pedradb-store/tests/l28_real_tcp.rs) — verde.
