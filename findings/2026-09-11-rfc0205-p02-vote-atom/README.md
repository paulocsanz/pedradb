# RFC-0205 P0.2 — atom `vote_decision`: o primeiro data-fate do cluster store/raft

Data: 2026-09-11. Escada: cap_data_fate 95→94, floor_atom 36→37,
floor_extract 242→241, residuals atom 36→37 / extract 242→241 /
data_fate 95→94. Par `vote` (`crates/pedradb-raft/src/vote_kernel.rs`,
entry `vote_decision`, handlers `handle_request_vote*`/`rpc_request_vote`,
chamado ao vivo por `on_request_vote` em pedra-store).

## O que foi pago

O fate do voto sobre TODOS os inputs (Vote.lean,
`vote_decision_fate_iff`) — os DOIS construtores:

```lean
theorem vote_decision_fate_iff :
    ∀ (i : VoteInputs) (d : VoteDecision),
      (vote_decision i = ok d) ↔
        ((d = .WouldGrant ∧ mesmo-termo ∧ can_vote = ok true ∧
            log_up_to_date = ok true) ∨
         (d = .Deny ∧ ¬(mesmo-termo ∧ can_vote = ok true ∧
            log_up_to_date = ok true)))
```

O P40 (`vote_decision_iff`, RFC-0053) já pinhava o LADO GRANT com
binders diretos — insuficiente para o registro (o gate exige ∀
literal no enunciado) e incompleto (sem o lado Deny). Este atom:
(a) reenuncia em forma ∀; (b) adiciona o lado Deny; (c) deriva a
TOTALIDADE (`vote_decision_total`: ∃ d, vote_decision i = ok d) do
`vote_decision_matches_spec` — se o corpo falhasse/divergisse, o bind
do spec-match não poderia ser `ok true`; é essa totalidade que fecha
Deny por exclusão (ok WouldGrant e ok Deny são os únicos destinos).

Leitura: um voto é concedido exatamente na conjunção mesmo-termo ∧
pode-votar (voted_for livre ou igual ao candidato) ∧ log do candidato
atualizado; na negação, Deny — o mutante as-is (ignore-log-and-vote)
concede onde o kernel real nega.

## Registro (mesmo commit)

- `close_proofs.tsv`:
  `atom catalog:vote vote_decision_fate_iff formal/aeneas/lean/Vote.lean vote_decision`
- `catalog.json`: par `vote` — del `data_fate`, add `atom_reason`
  datado 2026-09-11 (molde rwlock/occ da ronda 2)
- `proof_depth.tsv`: floor_atom 36→37, floor_extract 242→241,
  cap_data_fate 95→94
- `scripts/formal/residuals.json`: glue.proof_depth.atom 37,
  extract 241, glue.data_fate 94

## Verificação

- `lake build Vote` verde (1699 jobs), zero `sorry` em Vote.lean
- gates: depth-floor GREEN (extract=241 floor 241, atom=37 floor 37,
  residuals 7/37 == live, data_fate 94≤94), product GREEN, ledger
  GREEN 299/266/33
- `scripts/lean_extracts.sh --required`: ok
- planta DST `vote_decision_on_live_queued_is_not_ok`
  (pedradb-store, three_teeth_queued.rs): verde

## Lições

- `cases h : e with | ok d =>` reescreve as ocorrências de `e` no
  GOAL (a equação fica na forma original) — no caso ok a testemunha
  do ∃ fecha por `rfl`, não pela equação.
- `simp at h` com h reduzida a False já encerra o goal — a linha
  `exact h.elim` seguinte explode com "No goals to be solved".
