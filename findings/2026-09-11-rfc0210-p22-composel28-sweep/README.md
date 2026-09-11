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
- `l28_real_tcp_high_water_after_remove` — ver captura abaixo.

## Sweep final (worktree destacado DENTRO de software/)

Capturas adicionadas após o commit da composição (ver histórico
deste diretório): gates 3× GREEN no HEAD, extracts ok, sorry 0.
