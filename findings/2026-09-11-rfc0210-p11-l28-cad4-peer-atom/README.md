# RFC-0210 P1.1 (5/8) — atom `l28_tcp_peer`: timeout de eleição segue o C-new do disco

Data: 2026-09-11. Par `l28_tcp_peer` (`catalog:l28_tcp_peer`,
kernel `crates/pedradb-store/src/l28.rs`, entry `l28_tcp_peer_ok`).
Escada: cap_data_fate 72→71, floor_atom 59→60,
floor_extract 219→218 — tudo no MESMO commit, com a linha do
registro (`atom  catalog:l28_tcp_peer  l28_tcp_peer_ok_fate_iff
formal/aeneas/lean/L28.lean  l28_tcp_peer_ok`).

## O que foi pago

O destino do timeout de eleição sobre TODOS os inputs (L28.lean,
`l28_tcp_peer_ok_fate_iff`) — corpo pure-lift `ok b`: o timeout de
eleição do ctor TCP segue o C-new do DISCO exatamente quando leu a
membership de disco — nunca a CLI stale (planta TCP REAL + morte
de processo). O mutante AS-IS `ok true` pula o gate (leftover
0139: n_nodes da CLI) — a mentira que a planta TCP REAL
`l28_real_tcp_removed_peer` refuta.

## Verificação

- `lake build L28` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=60 (floor 60), extract=218 (floor 218),
  data_fate=71≤71, ledger 299/266/33.
- Planta TCP REAL `l28_real_tcp_removed_peer`
  (pedradb-store/tests/l28_real_tcp.rs) — 1 passed (382.15s).
