# RFC-0198 — sucessor do 0191 (composição registrada + invariantes indutivos)

Escrito no fechamento completo do RFC-0191 (P0/P1/P2 done). Id 0198
verificado livre (0192–0197 tomados: write-cycle, pwrite-ticket,
leftover-cache, scan-readahead, meter-unification, ratio-curve).

## Por que esses slices (grounding do board ao escrever)

- `unpaid_compose=0`, `unpaid_concurrency=0`, `unpaid_scale=0`,
  `unpaid_product=0`: o crédito compose/concorrência existe NO LEAN mas
  NÃO na escada (`close_proofs.tsv` tem 1 close; `Compose*.lean` tem 16
  teoremas sem degrau). P0 = registrar os dois primeiros closes de glue
  com iff ∀ (∃-mold dos dois corpos), floor_close no mesmo commit.
- Invariantes do 0191 são de UM PASSO (`wal_append_preserves_inv_wal`,
  `inv_lsm_newest_first_never_non_live`). P1 = base inicial +
  corolário de alcançabilidade (forma indutiva seL4) + o passo
  sync/fence que falta.
- `unpaid_script=2/17`: `finish_group_off_lock`/`group_finish` sem
  `write_pending_frame`; ambos em `concurrent.rs` NÃO-commitado da
  sessão paralela — fatia de coordenação nomeada, não silenciada.
- Tier model 17 pares (`scale_*`, `leveling_*`, …): P2 gradua o
  primeiro em N concreto.
- Herdados 0187 seguem terminais (0191 P2.4); Montanha cartoons
  user-gated; admitted flags seguem false.

## Regras carregadas do 0191

Um commit por promoção; close/atom exige ∀ sem sorry; gates
depth/product/ledger verdes no mesmo commit; registro e floor no mesmo
commit; nunca flipar `media_durable_admitted` /
`forall_schedules_admitted` / `lock_interleavings_admitted`.
