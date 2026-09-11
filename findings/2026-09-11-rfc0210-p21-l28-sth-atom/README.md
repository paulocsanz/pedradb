# RFC-0210 P2.1 — cadência l28 final 5/6: atom `catalog:l28_tcp_sth`

Data: 2026-09-11
Par: `l28_tcp_sth` (kernel `crates/pedradb-store/src/l28.rs`, RFC-0148 P1.2)
Teorema: `l28_tcp_sth_ok_fate_iff` em `formal/aeneas/lean/L28.lean`
(cadência l28 6/6; extração pure-lift do corpo `ok b`).

## Semântica

Após morte de processo TCP REAL, o ctor TCP de um eleitor
remanescente de 3 nós esquece `sent_through` de uma réplica remota
no `remove_member` out-of-band exatamente quando o drop aconteceu.
O dente 0147 `drop_repl_slot` (joint) NÃO é este dente. O mutante
AS-IS `ok true` pula o drop oob (leftover 0147: manter
sent_through).

## Escada

- `cap_data_fate` 57→56, `floor_atom` 67→68, `floor_extract` 211→210.
- `close_proofs.tsv` +1 linha atom; catálogo: `data_fate` removido,
  `atom_reason` datado.

## O que foi pago

- Teorema ∀ fate sobre o extract do corpo de produção.
- Planta TCP REAL verde ANTES do commit:
  `cargo test --test l28_real_tcp -- l28_real_tcp_drop_st` —
  1 passed, 22.19s (ctor sobre estado plantado — não paga a espera
  de kill; captura no scratch do round 5).
- Gates 3× GREEN após a cirurgia (depth 210/68/6, data_fate 56<=56).
