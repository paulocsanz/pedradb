# RFC-0210 P1.1 (8/8) — atom `l28_tcp_dsc`: descarte vivo derruba o sufixo não-commitado iff derrubou

Data: 2026-09-11. Par `l28_tcp_dsc` (`catalog:l28_tcp_dsc`,
kernel `crates/pedradb-store/src/l28.rs`, entry `l28_tcp_dsc_ok`).
Escada: cap_data_fate 69→68, floor_atom 62→63,
floor_extract 216→215 — tudo no MESMO commit, com a linha do
registro (`atom  catalog:l28_tcp_dsc  l28_tcp_dsc_ok_fate_iff
formal/aeneas/lean/L28.lean  l28_tcp_dsc_ok`).

## O que foi pago

O destino do descarte vivo sobre TODOS os inputs (L28.lean,
`l28_tcp_dsc_ok_fate_iff`) — corpo pure-lift `ok b`: o descarte
vivo derruba o sufixo não-commitado numa réplica tirada de `ids`
(planta TCP REAL + morte de processo) EXATAMENTE quando o
derrubou. O mutante AS-IS `ok true` pula o descarte (leftover
0142: só ids) — a mentira que a planta TCP REAL
`l28_real_tcp_removed_dsc` refuta.

## Verificação

- `lake build L28` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=63 (floor 63), extract=215 (floor 215),
  data_fate=68≤68, ledger 299/266/33.
- Planta TCP REAL `l28_real_tcp_removed_dsc`
  (pedradb-store/tests/l28_real_tcp.rs) — 1 passed (369.92s).

## P1.1 fechado nos números exatos do RFC

cap_data_fate 76→68, floor_atom 55→63, floor_extract 223→215 —
8 atoms, 8 commits (f6526f0d, e04ab047, bfba89b5, 71ef2f8f,
191a4a1a, bc9ad570, cd508216, este), plantas TCP REAIS 8/8 verdes
rodadas em paralelo (348–382s cada).
