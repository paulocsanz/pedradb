# RFC-0212 P2.2 — composição ∀ do fim-de-fila queued + sweep final

Data: 2026-09-11

## Composição (ComposeStoreFinish.lean — 21ª compose lib)

- `queued_finish_chain_fate`: o encadeamento discard-leader local →
  discard conta → cerca persiste → hist persiste (cada bind alimenta
  o atom seguinte com sua saída) landa `ok v` com `v` EXATAMENTE a
  localidade do nó, para TODO `in_ids` — as pernas cerca/hist
  disparam na réplica local já removida de `ids` (a semântica 0136
  sobrevive à composição). Cada conjunção é o atom registrado
  (`discard_leader_local_fate_iff` × `discard_node_counts_fate_iff` ×
  `persist_fence_node_counts_fate_iff` ×
  `persist_hist_node_counts_fate_iff`) — os corpos não são abertos.
- **Sem registro no TSV** — razão: uma linha exige par/entry único
  do catálogo; a composição atravessa quatro kernels/atoms (mesma
  regra das demais compose libs, ex. ComposeStoreRaft.lean,
  ComposeL28.lean).
- Sem buracos: `grep -c sorry ComposeStoreFinish.lean` = 0; lake
  build `ComposeStoreFinish` verde ("Build completed successfully
  (1700 jobs)"); wiring: `[[lean_lib]] ComposeStoreFinish` no
  lakefile.toml + array COMPOSE no `scripts/lean_extracts.sh`.

## Twin kernel DST (verde ANTES do commit)

- `membership_kernel::queued_finish_from_counts(is_local, in_ids)`:
  a conjunção das quatro pernas sobre os kernels reais; o as-is
  `queued_finish_from_counts_as_is` porta a cerca/hist em
  `is_local && in_ids` (a sobra 0136).
- `queued_finish_from_counts_on_live_queued_is_not_ok` — 1 passed
  (dentro da regressão do módulo `three_teeth_queued::` +
  `membership_kernel::tests`: 150 passed, 0 failed). Vivacidade:
  put não cometido na réplica 4 (com o prev DE LA, técnica da
  planta discard_uncommitted) → joint leave remove a 4 de `ids` →
  `fence_txn_aborted` persiste "abort" no db da 4 (pernas
  cerca/hist; o as-is composto pularia) → `discard_uncommitted_from`
  (com F-found `sent_through` neutralizado) derruba o sufixo não
  cometido da réplica removida (perna discard visita nó local fora
  de `ids`).

## Sweep final (worktree destacado DENTRO de software/)

- `git worktree add --detach /Users/paulo/software/pedradb-wt-r0212
  9f94b167` (HEAD do commit 1/2 da composição):
  - depth-floor: GREEN — extract=180 (floor 180), ladder close=6
    (floor 6) / atom=98 (floor 98), residuals close=7/atom=98 ==
    live 7/98, count=7, data_fate=26<=26.
  - product-floor: GREEN — D1=close R1=atom T1=atom C1=close,
    promoted=4>=floor 4.
  - ledger: GREEN — 19 pointers resolvem, total=292 proof=266
    campaign=26.
  - `test_proof_vs_campaign` — ok.
- Worktree removido após a captura (`git worktree remove --force`).
- Árvore principal: `bash scripts/lean_extracts.sh --required` —
  "ok lean extracts (62 libs + 21 compose)" (StoreTxn incluído no
  LIBS neste RFC; ComposeStoreFinish incluída no COMPOSE);
  `grep -c sorry` nos wrappers tocados (Membership.lean,
  StoreTxn.lean, StoreCompact.lean, ComposeStoreFinish.lean) = 0.
