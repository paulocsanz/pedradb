# RFC-0210 P1.1 (6/8) — atom `l28_tcp_lid`: first-key de HashMap não é identidade

Data: 2026-09-11. Par `l28_tcp_lid` (`catalog:l28_tcp_lid`,
kernel `crates/pedradb-store/src/l28.rs`, entry `l28_tcp_lid_ok`).
Escada: cap_data_fate 71→70, floor_atom 60→61,
floor_extract 218→217 — tudo no MESMO commit, com a linha do
registro (`atom  catalog:l28_tcp_lid  l28_tcp_lid_ok_fate_iff
formal/aeneas/lean/L28.lean  l28_tcp_lid_ok`).

## O que foi pago

O destino do gate de identidade local sobre TODOS os inputs
(L28.lean, `l28_tcp_lid_ok_fate_iff`) — corpo pure-lift `ok b`: o
ctor TCP de uma réplica tirada de `ids` NÃO trata o first-key do
HashMap como identidade exatamente quando o gate local-id segurou
(planta TCP REAL + morte de processo). O mutante AS-IS `ok true`
pula o gate (leftover 0140: first-key sempre) — a mentira que a
planta TCP REAL `l28_real_tcp_removed_lid` refuta.

## Verificação

- `lake build L28` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=61 (floor 61), extract=217 (floor 217),
  data_fate=70≤70, ledger 299/266/33.
- Planta TCP REAL `l28_real_tcp_removed_lid`
  (pedradb-store/tests/l28_real_tcp.rs) — 1 passed (348.30s).
