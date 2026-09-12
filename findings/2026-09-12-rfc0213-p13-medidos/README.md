# RFC-0213 P1.3 — veredito datado dos medidos ausentes

Data: 2026-09-12, HEAD `44bd1b67` (P1.2 fechado).

## Veredito

**ZERO pares medidos ausentes** nas 24 promoções drenadas pelo
RFC-0213 até aqui (P0.1 wa×9, P0.2 lookup×4, P1.1 flush×3+cf×2,
P1.2 leveling×2+singletons×4): 24 fns `entry` presentes nos kernels,
24 as_is presentes, 24 plantas DST verdes no HEAD (uma por par —
recontadas agora, não só no commit de cada promoção), defs Lean
presentes nos wrappers (todos os `lake build` verdes por promoção).
Nenhuma reescrita para fn viva, nenhuma aposentadoria, nenhuma
recusa.

Os 2 pares restantes (`wal_recover`, `dictionary_link`) são a fatia
P2.1, ainda `todo` — não são "medidos ausentes", são o trabalho
restante do RFC.

## Evidência mecânica (capturas em {SCRATCH}/r0213_p13_sweep.txt)

- `cargo test` por planta: 24/24 `1 passed` no HEAD —
  pedradb-core ×19, pedradb-sim ×4 (cf×2, visible_at,
  write_record_count), rocksdb-compat ×1 (iter_window).
  (Uma primeira passada mostrou 0 num filtro meu digitado errado
  — `write_admission_pit_resync_…` — o nome real
  `pit_resync_needs_rewrite_on_live_resync_is_not_ok` passa 1.)
- `python3` sobre `catalog.json`: para cada um dos 24 pares, `fn
  <entry>(` e `fn <as_is>(` presentes no arquivo de kernel do par —
  `missing: NONE`.
- Âncoras GREEN antes e depois: worktree no início da rodada
  (`9df25b47`, autoria do RFC) e HEAD (`44bd1b67`):
  `check_inventory_terminal.py` GREEN (7/7 terminal, 0 deferido,
  0 todo) e `check_twin_contracts.py` GREEN (7/7 count rows bound,
  0 derived) nos dois extremos.

## Lição

A drenagem storage encontrou catálogo limpo: nenhum par com fn
morta ou planta fantasma (ao contrário de rounds anteriores com
aposentadorias). O cap segue 2 — sobra exatamente a fatia P2.1.
