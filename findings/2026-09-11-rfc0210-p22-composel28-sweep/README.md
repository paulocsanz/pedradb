# RFC-0210 P2.2 — composição ∀ do protocolo de remoção TCP + sweep final

Data: 2026-09-11

## Composição (ComposeL28.lean — 20ª compose lib)

- `l28_removal_protocol_fate_composed`: remove → left ∧ high-water
  preservado, cada conjunção é o atom registrado especializado
  (`l28_tcp_left_ok_fate_iff` do `catalog:l28_tcp_left` ×
  `l28_tcp_hw_ok_fate_iff` do `catalog:l28_tcp_hw`).
- `l28_removal_protocol_fused`: as duas checagens fundidas em um
  passe de ctor (bind do left alimenta o hw) — o veredito ok
  exatamente quando AMBOS os fates valem.
- **Sem registro no TSV** — razão: uma linha exige par/entry único
  do catálogo; a composição atravessa dois kernels/atoms (mesma
  regra das demais compose libs, ex. ComposeStoreRaft.lean).
- Sem buracos: `grep -c sorry ComposeL28.lean` = 0; lake build
  `ComposeL28` verde ("Build completed successfully (1700 jobs)");
  `bash scripts/lean_extracts.sh --required` verde — "ok lean
  extracts (61 libs + 20 compose)". Wiring: `[[lean_lib]]
  ComposeL28` no lakefile.toml + array COMPOSE no
  `scripts/lean_extracts.sh`.

## Plantas DST dos atoms compostos (TCP reais, verdes)

- `l28_real_tcp_remove_member_left_on_disk` — 1 passed, 250.48s.
- `l28_real_tcp_high_water_after_remove` — 1 passed, 233.43s.

## Sweep final (worktree destacado DENTRO de software/)

- `git worktree add --detach /Users/paulo/software/pedradb-wt-r0210
  80f2344f` (HEAD do commit da composição):
  - depth-floor: GREEN — extract=209 (floor 209), ladder close=6
    (floor 6) / atom=69 (floor 69), residuals close=7/atom=69 ==
    live, count=7, data_fate=55<=55.
  - product-floor: GREEN — D1=close R1=atom T1=atom C1=close,
    promoted=4>=floor 4.
  - ledger: GREEN — 19 pointers resolvem, total=292 proof=266
    campaign=26.
  - `test_proof_vs_campaign` — ok.
- Worktree removido após a captura (`git worktree remove --force`).
- Árvore principal: `bash scripts/lean_extracts.sh --required` —
  ok (61 libs + 20 compose); `rg -c sorry` nos wrappers tocados
  (L28.lean, ComposeL28.lean) = 0.
