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
   floor_extract 180→171 — status: `done`

   — 1/9 `done`: `write_admission_idle_fate_iff`
   (WriteAdmission.lean; o gate idle é EXATAMENTE "nenhum knob de
   stall armado" — mem, pressure-l0 e stall-l0 todos off; o as-is
   respondia idle com knobs armados, RFC-0170 P2.4), cap 26→25,
   floor_atom 98→99, floor_extract 180→179; planta DST verde
   (`write_admission_idle_on_live_stall_is_not_ok`, no lote
   paralelo das 9: 9 passed)

   — 2/9 `done`: `write_admit_fate_iff`
   (WriteAdmission.lean; o veredito hard-admit é StallMem
   exatamente quando o eixo mem armado estoura o limite, StallL0
   exatamente quando o mem passou e o eixo L0 armado estoura, Ok
   exatamente quando nenhum eixo armado estoura; o as-is admitia
   sempre, RFC-0170 P2.4), cap 25→24, floor_atom 99→100,
   floor_extract 179→178; planta DST verde
   (`write_admit_on_live_mem_over_is_not_ok`, 1 passed)

   — 3/9 `done`: `seq_exhausted_fate_iff`
   (WriteAdmission.lean; o contador de sequência está esgotado
   EXATAMENTE quando queimou além do teto — `seq > max` decidido; o
   as-is nunca reporta esgotamento (wrap / burn past the ceiling),
   RFC-0170 P2.4), cap 24→23, floor_atom 100→101,
   floor_extract 178→177; planta DST verde
   (`seq_exhausted_on_live_ceiling_is_not_ok`, 1 passed)

   — 4/9 `done`: `fence_on_sync_fail_fate_iff`
   (WriteAdmission.lean; a cerca (fence) dispara EXATAMENTE quando
   um sync era exigido e esse sync falhou — `sync_required &&
   sync_failed`; o as-is nunca cerca, RFC-0170 P2.4), cap 23→22,
   floor_atom 101→102, floor_extract 177→176; planta DST verde
   (`fence_on_sync_fail_on_live_required_fail_is_not_ok`, 1 passed)

   — 5/9 `done`: `wal_commit_plan_fate_iff`
   (WriteAdmission.lean; o plano de append do WAL é
   AppendSyncFence EXATAMENTE em sync exigido+falhado,
   AppendSyncApplyOk EXATAMENTE em sync exigido+bem-sucedido,
   AppendApplyOk EXATAMENTE sem sync exigido; o as-is devolve o
   plano errado, RFC-0170 P2.4), cap 22→21, floor_atom 102→103,
   floor_extract 176→175; planta DST verde
   (`wal_commit_plan_on_live_sync_fail_is_not_ok`, 1 passed). O par
   já tinha linha `close` registrada: subiu o degrau
   close→atom (linha close mantida, residual close 7→6 no mesmo
   commit — a escada conta o par no degrau mais fundo)

   — 6/9 `done`: `torn_head_empty_log_fate_iff`
   (WriteAdmission.lean; uma cabeça tornada conta como log vazio
   EXATAMENTE quando o comprimento está abaixo do limite tiny —
   `len < tiny_max` decidido; o as-is chama toda cabeça de vazia,
   RFC-0170 P2.4), cap 21→20, floor_atom 103→104,
   floor_extract 175→174; planta DST verde
   (`torn_head_is_empty_log_on_live_large_wal_is_not_ok`, 1 passed)

   — 7/9 `done`: `torn_tail_needs_cut_fate_iff`
   (WriteAdmission.lean; uma cauda tornada precisa do corte
   EXATAMENTE quando o comprimento passa do último offset bom —
   `len > last_good` decidido; o as-is nunca corta, RFC-0170 P2.4),
   cap 20→19, floor_atom 104→105, floor_extract 174→173; planta
   DST verde (`torn_tail_needs_cut_on_live_overhang_is_not_ok`,
   1 passed)

   — 8/9 `done`: `seq_after_feed_fate_iff`
   (WriteAdmission.lean; uma sequência está depois do feed
   EXATAMENTE quando passa do teto do feed — `seq > feed_max`
   decidido; o as-is nunca vê além do feed, RFC-0170 P2.4),
   cap 19→18, floor_atom 105→106, floor_extract 173→172; planta
   DST verde (`seq_after_feed_on_live_newer_is_not_ok`, 1 passed)

   — 9/9 `done`: `pit_resync_rewrite_fate_iff`
   (WriteAdmission.lean; um resync point-in-time precisa do rewrite
   EXATAMENTE quando o registro é um resync — identidade decidida;
   o as-is pula o rewrite, RFC-0170 P2.4), cap 18→17,
   floor_atom 106→107, floor_extract 172→171; planta DST verde
   (`pit_resync_needs_rewrite_on_live_resync_is_not_ok`, 1 passed)

   — (FECHAMENTO) P0.1 completa: 9/9 átomos, cap_data_fate 26→17,
   floor_atom 98→107, floor_extract 180→171; residual close 7→6
   (wal_commit_plan subiu o degrau close→atom no mesmo commit);
   wrapper WriteAdmission.lean com 9 teoremas `_fate_iff` novos,
   1 por commit; plantas DST do write_admission_kernel todas verdes
2. **P0.2:** cadência lookup ×4 (wrapper `Lookup.lean`):
   `snap_empty`, `snap_below_watermark`, `mem_point_decides`,
   `prefer_newer_seq` — cap 17→13, floor_atom 107→111,
   floor_extract 171→167 — status: `done`

   — 1/4 `done`: `snap_empty_fate_iff`
   (Lookup.lean; um snapshot está vazio EXATAMENTE quando sua
   sequência é zero — `seq = 0` decidido; o as-is nunca vê snapshot
   vazio, RFC-0170 P2.4), cap 17→16, floor_atom 107→108,
   floor_extract 171→170; planta DST verde
   (`snap_is_empty_on_live_zero_is_not_ok`, 1 passed)

   — 2/4 `done`: `snap_below_watermark_fate_iff`
   (Lookup.lean; um snapshot está abaixo da marca d'água EXATAMENTE
   quando sua sequência é mais velha que a mínima visível —
   `seq < earliest` decidido; o as-is nunca cai abaixo,
   RFC-0170 P2.4), cap 16→15, floor_atom 108→109,
   floor_extract 170→169; planta DST verde
   (`snap_below_watermark_on_live_below_is_not_ok`, 1 passed)

   — 3/4 `done`: `mem_point_decides_fate_iff`
   (Lookup.lean; o veredito de ponto na memtable É a própria flag de
   hit — identidade; o as-is sempre reporta miss, RFC-0170 P2.4),
   cap 15→14, floor_atom 109→110, floor_extract 169→168; planta
   DST verde (`mem_point_decides_on_live_hit_is_not_ok`, 1 passed)

   — 4/4 `done`: `prefer_newer_seq_fate_iff`
   (Lookup.lean; um candidato vence EXATAMENTE quando não há
   incumbente, ou há incumbente e a sequência do candidato é mais
   nova — `new_seq > best_seq` decidido no ramo com best; o as-is
   prefere tudo, velho incluso, RFC-0170 P2.4), cap 14→13,
   floor_atom 110→111, floor_extract 168→167; planta DST verde
   (`prefer_newer_seq_on_live_older_first_is_not_ok`, 1 passed)

   — (FECHAMENTO) P0.2 completa: 4/4 átomos, cap_data_fate 17→13,
   floor_atom 107→111, floor_extract 171→167; wrapper Lookup.lean
   com 4 teoremas `_fate_iff` novos, 1 por commit; plantas DST do
   lookup_kernel todas verdes

### P1 — core

3. **P1.1:** cadência flush ×3 (wrapper `Flush.lean`) + cf ×2
   (wrapper `Cf.lean`): ×5, cap 13→8, floor_atom 111→116,
   floor_extract 167→162 — status: `doing`

   — 1/5 `done`: `flush_publish_fate_iff`
   (Flush.lean; o manifesto pode publicar EXATAMENTE quando o SST
   está durável — identidade sobre `sst_durable`; o as-is publica
   SST sem sync, RFC-0170 P2.4), cap 13→12, floor_atom 111→112,
   floor_extract 167→166; planta DST verde
   (`may_publish_manifest_on_live_unsynced_sst_is_not_ok`, no
   pedradb-sim, 1 passed)

   — 2/5 `done`: `auto_flush_due_fate_iff`
   (Flush.lean; o auto-flush vence EXATAMENTE quando o eixo está
   armado e os bytes chegaram ao limite — `mem_bytes >= limit`
   decidido no ramo armado; o as-is nunca auto-flusha,
   RFC-0170 P2.4), cap 12→11, floor_atom 112→113,
   floor_extract 166→165; planta DST verde
   (`auto_flush_due_on_live_over_limit_is_not_ok`, 1 passed)

   — 3/5 `done`: `flush_decision_fate_iff`
   (Flush.lean; o WAL rota EXATAMENTE com o pipeline totalmente
   quiescente — memtable vazia e sem imm, sem pin vivo, nada
   estacionado sem flush, nenhum commit em voo; qualquer retenção
   mantém o WAL; o as-is ignora o pin vivo, RFC-0170 P2.4),
   cap 11→10, floor_atom 113→114, floor_extract 165→164; planta
   DST verde (`wal_rotate_decision_on_live_pin_is_not_ok`, 1 passed)

   — 4/5 `done`: `cf_family_fate_iff`
   (Cf.lean; a familia é decidida EXATAMENTE pela rota extraída
   — default: sem separador ou com o prefixo byte-a-byte
   "default", com `position` decidindo o primeiro 0; nomeada:
   overhang estrito dos bytes da familia + `starts_with` + 0
   logo após; o as-is responde in-family para toda chave,
   RFC-0170 P2.4), cap 10→9, floor_atom 114→115,
   floor_extract 164→163; planta DST verde
   (`key_in_cf_family_on_live_scan_is_not_ok`, 1 passed)

   — 5/5 `done`: `cf_family_of_fate_iff`
   (Cf.lean; a familia de uma chave é decidida EXATAMENTE pela
   rota extraída — sem NUL ou NUL líder ⇒ string "default";
   senão decode lossy UTF-8 dos bytes antes do primeiro NUL,
   owned pelo Cow; o as-is responde "default" para toda chave,
   RFC-0170 P2.4), cap 9→8, floor_atom 115→116,
   floor_extract 163→162; planta DST verde
   (`cf_family_of_on_live_sst_bounds_is_not_ok`, 1 passed).
   FECHAMENTO P1.1: cap 13→8, floor_atom 111→116,
   floor_extract 167→162
4. **P1.2:** cadência leveling ×2 (`Leveling.lean`) + os 4
   singletons com wrapper no LIBS: `visible_at` (`Merge.lean`),
   `write_record_count` (`Batch.lean`), `iter_window`
   (`Iter.lean`), `occ_batch_plan` (`GroupCommit.lean`): ×6, cap
   8→2, floor_atom 116→122, floor_extract 162→156 — status:
   `doing`

   — 1/6 `done`: `pick_l0_to_l1_fate_iff`
   (Leveling.lean; o job L0→L1 é decidido EXATAMENTE pela rota
   extraída — L0 vazio ou cap 0 ⇒ sem job; senão o cap
   `min(len l0, max_l0)` limita a caminhada de seleção, o
   primeiro arquivo semeia o hull e os loops de sel/slice (átomos
   de loop) decidem o job com cada passo monádico ok; o as-is
   reabsorve o L1 inteiro, RFC-0170 P2.4), cap 8→7,
   floor_atom 116→117, floor_extract 162→161; planta DST verde
   (`pick_l0_to_l1_on_live_slice_is_not_ok`, 1 passed)
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
| P0.1 | p0 | Cadência write_admission ×9 | done | 9/9: este commit (FECHAMENTO) | 2026-09-12 |
| P0.2 | p0 | Cadência lookup ×4 | done | 4/4: este commit (FECHAMENTO) | 2026-09-12 |
| P1.1 | p1 | Cadência flush ×3 + cf ×2 | done | 5/5: este commit (FECHAMENTO) | 2026-09-12 |
| P1.2 | p1 | Cadência leveling ×2 + singletons ×4 | doing | 1/6: este commit | 2026-09-12 |
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
