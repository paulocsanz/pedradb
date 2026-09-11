# RFC-0210 P1.1 (7/8) — atom `l28_tcp_rdr`: leitor LocalApplied é local, não ids.first() remoto

Data: 2026-09-11. Par `l28_tcp_rdr` (`catalog:l28_tcp_rdr`,
kernel `crates/pedradb-store/src/l28.rs`, entry `l28_tcp_rdr_ok`).
Escada: cap_data_fate 70→69, floor_atom 61→62,
floor_extract 217→216 — tudo no MESMO commit, com a linha do
registro (`atom  catalog:l28_tcp_rdr  l28_tcp_rdr_ok_fate_iff
formal/aeneas/lean/L28.lean  l28_tcp_rdr_ok`).

## O que foi pago

O destino do gate de leitor local sobre TODOS os inputs (L28.lean,
`l28_tcp_rdr_ok_fate_iff`) — corpo pure-lift `ok b`: o ctor TCP não
escolhe o `ids.first()` REMOTO como leitor LocalApplied (`empty`,
não `bad node`) exatamente quando o gate reader-local segurou
(planta TCP REAL + morte de processo). O mutante AS-IS `ok true`
pula o gate (leftover 0141: ids.first sempre) — a mentira que a
planta TCP REAL `l28_real_tcp_removed_rdr` refuta.

## Verificação

- `lake build L28` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=62 (floor 62), extract=216 (floor 216),
  data_fate=69≤69, ledger 299/266/33.
- Planta TCP REAL `l28_real_tcp_removed_rdr`
  (pedradb-store/tests/l28_real_tcp.rs) — 1 passed (375.51s).
