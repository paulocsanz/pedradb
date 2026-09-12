# RFC-0213 P1.2 3/6 — atom `catalog:visible_at` fortalecido (`visible_at_fate_iff`, Merge.lean) — cap-only

Data: 2026-09-12. O par `visible_at` entra no RFC-0213 com
`data_fate: true`, mas o gate `check_depth_floor.py` expôs a
duplicidade: o par JÁ tinha registro atom de 2026-09-10 (RFC-0188
P1.6, `visible_at_deletion_never_live`) — a flag `data_fate` era
resquício não drenado. A cirurgia honesta deste slice é CAP-ONLY.

## O que o teorema diz (fate forall sobre o corpo extraído)

`merge.visible_at kind range_hidden = ok v` ↔ exatamente:

- `kind = Value` ⇒ `v = !range_hidden` (live se e só se nenhum range
  cobre a chave);
- `kind = Deletion ∨ kind = RangeDeletion` ⇒ `v = false` (nunca
  live, qualquer cobertura).

Mais forte que o átomo de 2026-09-10 (que cobria só o braço Deletion
sobre range_hidden arbitrário): agora o destino do valor É o corpo
inteiro, nos três kinds.

## Cirurgia (cap-only — o degrau atom já estava pago)

- `close_proofs.tsv`: NENHUMA linha nova (o registro atom de
  2026-09-10 continua; linha duplicada de `visible_at_fate_iff`
  seria duplo-crédito e o gate recusou — "one credit per pair per
  kind").
- `catalog.json`: `data_fate` removido; `atom_reason` atualizado
  para o fate-iff (o porquê corrente do degrau).
- `proof_depth.tsv`: SÓ cap_data_fate 6→5 (floor_atom 118 e
  floor_extract 160 inalterados — o par já contava nos dois).
- `residuals.json`: SÓ glue.data_fate 6→5.
- Gate `check_depth_floor.py`: GREEN (extract=160, close=6,
  atom=118, count=7, data_fate=5).

## Efeito nas metas da fatia P1.2

O RFC-0213 assumia 6 degraus novos; `visible_at` paga só o cap.
Metas corrigidas na data: floor_atom 116→**121** (não 122),
floor_extract 162→**157** (não 156); cap 8→2 inalterado.

## Refutação ao vivo

O as-is (`visible_at_as_is` = `true` para toda versão — Deletion
scanning live) é refutado por
`merge::tests::visible_at_on_live_range_del_is_not_ok`
(1 passed, `cargo test --lib -p pedradb-core`).
