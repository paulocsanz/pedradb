# RFC-0198 P0.2 — close registrado: occ_batch_plan ∘ occ_conflict

Segundo close de glue registrado na escada (P0.1 foi
`wal_commit_plan ∘ fence_on_sync_fail`, commit 874b8240). Aterrissado
em 2026-09-11 depois que o commit `4df272d5` da sessão paralela
(rfc0199 P0.1+P0.2, escada `count`) assentou os arquivos de registro
compartilhados — o teorema estava provado e preservado desde 2026-09-10
(`theorem.snippet.lean` neste diretório; um `git reset` estranho da
sessão paralela havia limpo o insert não-commitado).

## O que o teorema diz

`occ_batch_plan_member_fate_iff` (GroupCommit.lean, zero sorry): para
TODO membro do lote, o fate que o plano `occ_batch_plan` atribui é
decidido EXATAMENTE pela cadeia do callee — o corpo do loop faz bind
da resposta de `occ_conflict` em `occ_member_fate` (o glue por membro
extraído):

- `TooOld` vence pela flag do membro sozinha (`too_old_i = true`);
- `Conflict` exige flag falsa E callee respondendo `ok true` em chave
  tocada;
- `Ok` exige flag falsa E callee respondendo `ok false`.

Molde ∃: `bind (occ_conflict …) (occ_member_fate …) = ok f ↔ ∃ c,
occ_conflict … = ok c ∧ (disjunção em três vias)` — mesma regra de
computação sobre os dois corpos extraídos do P0.1. O comportamento do
loop inteiro fica pinado pelos teoremas concretos já existentes
(`occ_batch_plan_lagging_conflict`, `occ_batch_plan_n3_one_lagging`,
`occ_batch_plan_too_old_wins`).

## Registro (mesmo commit)

- `close_proofs.tsv`: `close catalog:occ_batch_plan
  occ_batch_plan_member_fate_iff formal/aeneas/lean/GroupCommit.lean
  occ_batch_plan`
- `proof_depth.tsv`: `floor_close 2→3`
- `residuals.json`: `glue.proof_depth.close 3→4` (replace cirúrgico)

## Verificação (mesmo commit)

- `lake build GroupCommit` verde; `grep sorry` = 0;
  `lean_extracts --required` verde (61 libs + 6 compose — o +1 é o
  `LsmCompactCount` da sessão paralela, não meu).
- Gates GREEN: depth `registered ladder close=3 (floor 3), residuals
  close=4 == live 4`; product `D1=close R1=atom T1=atom C1=close`;
  ledger `298/265/33`.

## História da fatia (coordenação)

Prova concluída 2026-09-10; bloqueio nomeado: shared registry com
hunks não-commitados da sessão paralela (floor_count/RFC-0199) + um
`git reset` estranho que limpou o insert. Teorema preservado em
`theorem.snippet.lean`; re-inserido verbatim hoje (build verde de
primeira — a prova foi salva exatamente como provada). Desvio de ordem
(P1.1–P1.3 primeiro) registrado no plan do goal.
