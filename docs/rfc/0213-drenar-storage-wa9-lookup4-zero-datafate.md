# RFC-0213 — drenar o storage: write_admission ×9 + lookup ×4 + flush ×3 + cf ×2 + leveling ×2 + singletons ×6 — ZERO `data_fate`

**Status:** draft

## Tese

O bloco cluster está DRENADO (0212: 29 atoms — membership 22 +
txn 6 + compact 1 — ZERO `data_fate` medido ao vivo; composição
∀ do fim-de-fila queued em `ComposeStoreFinish.lean`). TODO par
com `data_fate` pendente agora mora em kernels de STORAGE: os 26
nomeados no EXTRACT.md datado de 2026-09-11 —
`write_admission_kernel.rs` ×9 (`write_admission`, `write_admit`,
`seq_exhausted`, `fence_on_sync_fail`, `wal_commit_plan`,
`torn_head_empty_log`, `torn_tail_needs_cut`, `seq_after_feed`,
`pit_resync_rewrite`), `lookup_kernel.rs` ×4 (`snap_empty`,
`snap_below_watermark`, `mem_point_decides`, `prefer_newer_seq`),
`flush_kernel.rs` ×3 (`flush_decision`, `flush_publish`,
`auto_flush_due`), `cf_kernel.rs` ×2 (`cf_family`,
`cf_family_of`), `leveling.rs` ×2 (`leveling_pick`,
`leveling_pushdown`) e 6 singletons (`visible_at` em `merge.rs`,
`write_record_count` em `batch.rs`, `iter_window` em
`rocksdb-compat/iter_kernel.rs`, `occ_batch_plan` em
`group_commit_kernel.rs`, `wal_recover` em `wal/recover_kernel.rs`,
`dictionary_link` em `wal/reopen_kernel.rs`). Drenar os 26 leva o
CATÁLOGO INTEIRO a ZERO `data_fate` pendente — fim da campanha de
drenagem: nenhum par do catálogo fica sem veredito medido.

Números vivos no HEAD do 0212 (`c892b57d`): 292 pares (266 proof /
26 campaign), `cap_data_fate` 26, `floor_atom` 98,
`floor_extract` 180, close 6, count 7. Se os 26 virarem atoms:
**cap 26→0, floor_atom 98→124, floor_extract 180→154**. Wrappers
JÁ existem no gate para 24 dos 26 (`WriteAdmission`, `Lookup`,
`Flush`, `Cf`, `Leveling`, `Merge`, `Batch`, `Iter`, `GroupCommit`
— todos no LIBS do `lean_extracts.sh`); `WalRecover` e `Reopen`
existem mas estão FORA do LIBS (mesmo buraco pré-existente do
`StoreTxn` no 0212) — a inscrição é parte da fatia que os toca
(P2.1), com o extrato re-carimbado. Corpos REAIS: cada promoção é
um iff sobre o extract Aeneas do corpo de produção — o molde é a
cadência do 0212 (`*_fate_iff` em cada wrapper), com planta DST
verde ANTES do commit e veredito datado para qualquer par que
meça ausente (fn/planta inexistente ⇒ aposentadoria com recusa —
nunca gate inventado; o pool honesto manda sobre a meta numérica).
ATENÇÃO de fronteira: `pedradb-core` é terreno com sessão paralela
ativa — nunca tocar arquivos não commitados dela; promoções
tocam o kernel + wrapper + ratchets, nada de edits fora do meu.

## Fatias

### P0 — core

1. **P0.1:** cadência write_admission ×9 (wrapper
   `WriteAdmission.lean`; entry `wal_commit_plan` até
   `pit_resync_rewrite`): ×9, cap 26→17, floor_atom 98→107,
   floor_extract 180→171 — status: `doing`

   — 1/9 `done`: `write_admission_idle_fate_iff`
   (WriteAdmission.lean; o gate idle é EXATAMENTE "nenhum knob de
   stall armado" — mem, pressure-l0 e stall-l0 todos off; o as-is
   respondia idle com knobs armados, RFC-0170 P2.4), cap 26→25,
   floor_atom 98→99, floor_extract 180→179; planta DST verde
   (`write_admission_idle_on_live_stall_is_not_ok`, no lote
   paralelo das 9: 9 passed)
2. **P0.2:** cadência lookup ×4 (wrapper `Lookup.lean`):
   `snap_empty`, `snap_below_watermark`, `mem_point_decides`,
   `prefer_newer_seq` — cap 17→13, floor_atom 107→111,
   floor_extract 171→167 — status: `todo`

### P1 — core

3. **P1.1:** cadência flush ×3 (wrapper `Flush.lean`) + cf ×2
   (wrapper `Cf.lean`): ×5, cap 13→8, floor_atom 111→116,
   floor_extract 167→162 — status: `todo`
4. **P1.2:** cadência leveling ×2 (`Leveling.lean`) + os 4
   singletons com wrapper no LIBS: `visible_at` (`Merge.lean`),
   `write_record_count` (`Batch.lean`), `iter_window`
   (`Iter.lean`), `occ_batch_plan` (`GroupCommit.lean`): ×6, cap
   8→2, floor_atom 116→122, floor_extract 162→156 — status:
   `todo`
5. **P1.3:** veredito datado dos medidos ausentes — SE alguma
   promoção acima medir fn/planta/handler inexistente, veredito
   por par em findings (reescrever para fn viva SE o caminho
   existir em produção; senão aposentadoria com recusa datada),
   espelhos/âncoras verdes antes/depois — status: `todo`

### P2 — core

6. **P2.1:** os 2 wal finais — `wal_recover` (wrapper
   `WalRecover.lean`) + `dictionary_link` (wrapper `Reopen.lean`),
   AMBOS inscritos no LIBS do `lean_extracts.sh` nesta fatia
   (buraco pré-existente; extratos re-carimbados): cap 2→0,
   floor_atom 122→124, floor_extract 156→154 — CATÁLOGO ZERO
   `data_fate` medido ao vivo — status: `todo`
7. **P2.2:** composição ∀ do caminho de storage (write-admission:
   admission → wal plan → torn-tail cut sobre atoms registrados)
   em nova compose lib (zero buracos; twins DST verdes; razão de
   SEM registro em findings — não é par único) + sweep final
   (worktree destacado DENTRO de `software/`, gates 3× GREEN,
   extracts ok, sorry 0, capturas em findings, nota datada em
   EXTRACT.md: catálogo inteiro drenado — ZERO `data_fate`) +
   flip `**Status:** done` — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Cadência write_admission ×9 | doing | 1/9: este commit | 2026-09-12 |
| P0.2 | p0 | Cadência lookup ×4 | todo | — | 2026-09-11 |
| P1.1 | p1 | Cadência flush ×3 + cf ×2 | todo | — | 2026-09-11 |
| P1.2 | p1 | Cadência leveling ×2 + singletons ×4 | todo | — | 2026-09-11 |
| P1.3 | p1 | Veredito datado dos medidos ausentes | todo | — | 2026-09-11 |
| P2.1 | p2 | Wal finais ×2 — CATÁLOGO ZERO data_fate | todo | — | 2026-09-11 |
| P2.2 | p2 | Composição ∀ storage + sweep final + flip done | todo | — | 2026-09-11 |

## Critérios de aceite

- Cada promoção: teorema iff no wrapper (build `lake build <Lib>`
  verde ANTES do commit), planta DST verde ANTES do commit,
  cirurgia `promote_atom.py` (floor_atom +1, floor_extract −1,
  cap_data_fate −1, linha `atom` no `close_proofs.tsv`,
  `atom_reason` datado no catálogo, residuals),
  `check_depth_floor.py` GREEN no commit — 1 promoção = 1 commit,
  `git show <c> -- <lean> | grep -c "^+theorem"` = 1.
- Nenhuma promoção pode: quebrar âncoras 0203/0204, registrar
  gate inventado, tocar arquivos não commitados da sessão
  paralela em `pedradb-core`/`wal`.
- P2.1 exige `data_fate=0<=0` GREEN no gate e os wrappers
  `WalRecover` + `Reopen` no LIBS (libs contando o que eram 62 +
  os 2 novos).
- P2.2 exige: gates 3× GREEN + `test_proof_vs_campaign` ok no
  worktree destacado dentro de `software/`, extracts
  `ok` com a contagem nova, sorry 0, capturas em findings, nota
  datada em EXTRACT.md, `**Status:** done`.
