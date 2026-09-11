# RFC-0210 P2.1 — cadência l28 final 6/6 (FECHAMENTO): atom `catalog:l28_tcp_pj`

Data: 2026-09-11
Par: `l28_tcp_pj` (kernel `crates/pedradb-store/src/l28.rs`, RFC-0068 P2.2)
Teorema: `l28_tcp_pj_ok_fate_iff` em `formal/aeneas/lean/L28.lean`
(cadência l28 6/6; extração pure-lift do corpo `ok b`).

## Semântica

Após morte de processo TCP REAL, o ctor TCP de um eleitor de 3 nós
com um C-old,new comprometido PLANTADO (sem leave) recusa uma
eleição por maioria C-old exatamente quando a recusa aconteceu. O
mutante AS-IS `ok true` pula o joint plantado (leftover 0148:
eleger no C-old).

## Escada (fechamento do bloco l28)

- `cap_data_fate` 56→55, `floor_atom` 68→69, `floor_extract` 210→209.
- Números finais do 0210 (22 promoções + aposentadoria −7 do P1.2):
  cap 84→55, floor_atom 47→69, floor_extract 231→209.
- Medido ao vivo no HEAD (pós-cirurgia): 26 pares l28 no catálogo,
  ZERO com `data_fate` pendente.

## O que foi pago

- Teorema ∀ fate sobre o extract do corpo de produção.
- Planta TCP REAL verde ANTES do commit:
  `cargo test --test l28_real_tcp -- l28_real_tcp_plant_joint` —
  1 passed, 1.80s (ctor sobre estado plantado; captura no scratch
  do round 5).
- Gates 3× GREEN após a cirurgia (depth 209/69/6, data_fate
  55<=55, ledger total=292 proof=266 campaign=26).
