# RFC-0200 P2.2 — sweep final: todos os gates GREEN no HEAD, zero sorry

Data: 2026-09-11. HEAD varrido: `6cc06468` (worktree destacado
`git worktree add --detach`, sem tocar o worktree vivo).

## Gates no HEAD destacado

```
GATE depth-floor: GREEN — extract=245 (floor 245), registered ladder close=4 (floor 4) / atom=33 (floor 33), residuals close=5/atom=33 == live 5/33, count=6 (floor 6, residual 6 == live 6), data_fate=98<=98, handler_loc=112092 (series; TSV 111927)
GATE product-floor: GREEN — D1=close R1=atom T1=atom C1=close, promoted=4>=floor 4
GATE ledger: GREEN — 18 catalog pointers resolve, counts match (total=299 proof=266 campaign=33)
```

## sorry nos wrappers tocados pelo RFC-0200

```
WalState: 0
Merge: 0
WriteAdmission: 0
Flush: 0
```

## Escada no fechamento do 0200

- extract 246→245, atom 32→33 (P2.1 `wal_sync_required`), close 4
  registrados (+1 residual live = 5), count 6 (0199), cap_data_fate
  99→98, floor_close 4 (P0.2 `wal_rotate_decision`).
- Lean: `lake build WriteAdmission` verde (1699 jobs); builds dos outros
  módulos tocados verdes nos commits das suas fatias; `lean_extracts
  --required` ok (61 libs + 11 compose).
- Planta DST do átomo novo: `wal_sync_required_on_live_client_true_is_not_ok`
  1 passed / 0 failed no kernel de produção.

## Admissões que seguem recusadas (herdadas, intocadas)

`media_durable_admitted`, `forall_schedules_admitted`,
`lock_interleavings_admitted` — todas "always false" em
`scripts/formal/residuals.json` no HEAD (R-group-glue / R-pct /
R-fsync-lie). Nenhum flip; nenhuma claim de equivalência seL4.

## Fechamento

RFC-0200 fecha com P0.1, P0.2, P1.1, P1.2, P2.1, P2.2 todos `done`
(frase seL4 completa do WAL em qualquer ordem de passos, base+ponte do
merge, quarto close, primeiro atom data-fate do ciclo).
