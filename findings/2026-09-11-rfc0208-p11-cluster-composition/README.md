# RFC-0208 P1.1 — composição do cluster: a corrente da eleição e o triplo de recovery sobre os atoms registrados

Data: 2026-09-11. Módulo novo
`formal/aeneas/lean/ComposeStoreRaft.lean` (19º lib de composição;
`lean_extracts.sh` conta 61 libs + 19 compose). Zero `sorry`.

## O que foi pago

Duas composições ∀ sobre ATOMS REGISTRADOS (padrão ponte do 0205:
componhe com o registrado, nunca duplica):

1. **A corrente da eleição** (`election_grant_chain_fate`) — compõe
   `catalog:vote` (`vote_decision_fate_iff`, pago 0205 P0.2) com
   `catalog:grant_persist` (`grant_after_persist_fate_iff`, pago
   0208 P0.1) na mônada Result (bind): o grant que sai do handler é
   `true` EXATAMENTE quando (mesmo termo ∧ pode votar ∧ log
   atualizado ∧ persistência durável Ok) — a análise do
   `bind` usa `bind_ok_inv`/`bind_intro` privados; o lado Deny e o
   lado persistência falhada fecham por `vote_decision_total`
   (totalidade extraída do spec, tornado público em Vote.lean).

2. **O triplo de recovery** (`recovery_fate_composed`) — compõe
   `catalog:recover_apply` (`recover_must_apply_fate_iff`) com
   `catalog:recover_drop_orphan` (`recover_drop_orphan_seg_fate_iff`,
   ambos do 0205 P1.2) e o corpo extraído ainda não registrado
   `recover_apply_node_counts`: re-aplica exatamente quando
   commit > applied, dropa o segmento órfão exatamente quando
   seg > new_hi, e o gate de contagem de nodos passa exatamente no
   nodo local — o seam que uma passagem de recovery percorre.

## Por que SEM registro no close_proofs.tsv

A regra do registro exige um par/entrada ÚNICO do catálogo; a
corrente da eleição atravessa vote_kernel × vote_kernel e o triplo
atravessa membership_kernel × membership_kernel × corpo não
registrado — mesmos moldes dos outros 18 libs de composição
(ComposeConcurrent, ComposeC1Membership, …), nenhum deles carrega
linha no TSV.

## Verificação

- `lake build ComposeStoreRaft` verde (lakefile.toml com o novo
  `[[lean_lib]]`); zero `sorry` no módulo.
- `bash scripts/lean_extracts.sh --required` ok (61 + 19).
- Twins DST do cluster 7/7 verdes (pedradb-store,
  three_teeth_queued): vote_decision, grant_after_persist,
  election_grant_from_counts, propose_ack_ok, recover_must_apply,
  recover_drop_orphan_seg, recover_apply_node_counts — todos
  `_on_live_queued_is_not_ok`.
- Gates 3× GREEN neste HEAD (sem promoção: floors não se movem —
  composição não é par do catálogo).
