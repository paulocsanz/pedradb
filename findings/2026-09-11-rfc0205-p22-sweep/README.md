# RFC-0205 P2.2 — sweep final em worktree destacado (HEAD `93c9434a`)

Data: 2026-09-11. Worktree destacado `git worktree add --detach`
do HEAD `93c9434a` (fora da árvore viva, que carrega hunks da sessão
paralela). Nota de método: o worktree precisa ficar DENTRO de
`software/` — o `lean_extracts.sh` resolve o backend Aeneas por
caminho RELATIVO (`../../../../aeneas`); de `/var/folders` ele não
resolve (primeira tentativa falhou por isso, não por regressão).

## Captura (verbatim)

```
== HEAD: 93c9434a ==
GATE depth-floor: GREEN — extract=239 (floor 239), registered ladder
  close=6 (floor 6) / atom=39 (floor 39), residuals close=7/atom=39
  == live 7/39, count=7 (floor 7, residual 7 == live 7),
  data_fate=92<=92, handler_loc=112092 (series; TSV 111927)
GATE product-floor: GREEN — D1=close R1=atom T1=atom C1=close,
  promoted=4>=floor 4
GATE ledger: GREEN — 19 catalog pointers resolve, counts match
  (total=299 proof=266 campaign=33)
ok    lean extracts (61 libs + 18 compose)
-- sorry nos wrappers tocados --
GroupCommit.lean sorry=0
Vote.lean sorry=0
ComposeConcurrent.lean sorry=0
Membership.lean sorry=0
-- admissions sempre false (residuals.json) --
lock_interleavings_admitted always false  → 1
forall_schedules_admitted … always false  → 2
media_durable_admitted always false       → 1
```

## Escada terminal do RFC-0205

close 6 (5→6 no P0.1) / atom 39 (36→39 no P0.2+P1.2) / count 7 /
extract 239 (242→239) / cap_data_fate 92 (95→92) / compose 18 libs.
Worktree removido após a captura. RFC-0205 flipado `**Status:** done`.
