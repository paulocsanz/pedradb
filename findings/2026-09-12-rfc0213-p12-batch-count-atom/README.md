# RFC-0213 P1.2 4/6 — atom `catalog:write_record_count` (`write_record_count_ok_fate_iff`, Batch.lean)

Data: 2026-09-12. Par `write_record_count` promovido de `data_fate`
para `atom`, teorema `write_record_count_ok_fate_iff` em
`formal/aeneas/lean/Batch.lean` sobre o extrato Aeneas de
`batch.rs` (`write_record_count_ok`, entry do catálogo).

## O que o teorema diz (fate forall sobre o corpo extraído)

`write_record_count_ok count decoded_len = ok v` ↔ exatamente:

- `lift (UScalar.cast u32→usize count) = ok i` (o cast honesto —
  sem truncamento silencioso) e
- `v = decide (decoded_len = i)` — a comparação decide.

Corpo inteiro coberto: um bind + um decide; nada fica ao sabor do
par.

## Por que data_fate não é mais necessário

O corpo é mínimo e totalmente coberto pelo iff; a única folha é o
`lift` do cast (fronteira Aeneas). O as-is
(`write_record_count_ok_as_is` = `true` sempre) admite batch torcido
(comprimento ≠ count do prefixo) e é refutado ao vivo:
`batch::tests::write_record_count_ok_on_live_torn_batch_is_not_ok`
(1 passed, `cargo test --lib -p pedradb-core`).

## Ratchet

- `close_proofs.tsv`: +1 `atom catalog:write_record_count`
  (`write_record_count_ok_fate_iff`, entry `write_record_count_ok`).
- `proof_depth.tsv`: floor_atom 118→119, floor_extract 160→159,
  cap_data_fate 5→4.
- `catalog.json`: `data_fate` removido, `atom_reason` datado.
- `residuals.json`: atom+1, extract−1, glue.data_fate−1.
- Gate `check_depth_floor.py`: GREEN
  (extract=159, close=6, atom=119, count=7, data_fate=4).

## Grafia

Corpo Bool de igualdade inteira no extrato carrega `decide` —
statement `v = decide (decoded_len = i)`, nunca `v = (… = …)`
(elabora como Prop-eq e mistura os mundos). Wrapper sem `open
Aeneas`: tipos levam `U32`/`Usize` sem o prefixo `Std.`.
