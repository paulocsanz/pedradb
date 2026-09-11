# RFC-0210 P1.1 (1/8) — atom `l28_tcp_hist`: SI hist persistido iff o persist aconteceu

Data: 2026-09-11. Par `l28_tcp_hist` (`catalog:l28_tcp_hist`,
kernel `crates/pedradb-store/src/l28.rs`, entry `l28_tcp_hist_ok`).
Escada: cap_data_fate 76→75, floor_atom 55→56,
floor_extract 223→222 — tudo no MESMO commit, com a linha do
registro (`atom  catalog:l28_tcp_hist  l28_tcp_hist_ok_fate_iff
formal/aeneas/lean/L28.lean  l28_tcp_hist_ok`).

## O que foi pago

Primeira promoção da cadência 3/4 — o destino do persist de SI
hist sobre TODOS os inputs (L28.lean, `l28_tcp_hist_ok_fate_iff`)
— corpo pure-lift `ok b`: o SI hist é persistido numa réplica
tirada de `ids` (planta TCP REAL + morte de processo) EXATAMENTE
quando o persist aconteceu. O mutante AS-IS `ok true` pula o
persist (leftover 0135: só ids) — a mentira que a planta TCP REAL
`l28_real_tcp_removed_hist` refuta.

## Verificação

- `lake build L28` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=56 (floor 56), extract=222 (floor 222),
  data_fate=75≤75, ledger 299/266/33.
- Planta TCP REAL `l28_real_tcp_removed_hist`
  (pedradb-store/tests/l28_real_tcp.rs) — verde.
