# RFC-0198 — sweep final (2026-09-11)

Sweep do goal "implementar todos os P0/P1/P2 do RFC-0198 + RFC
sucessor". Estado: 9/9 fatias done — P2.1 fechado 2026-09-11 pelo
commit 750de460 da sessão paralela (abaixo). Capturas completas no
scratch do goal.

## Fatias landadas (um commit por promoção)

| Fatia | Commit | Conteúdo |
|---|---|---|
| P0.1 | 874b8240 | close registrado `wal_commit_plan∘fence_on_sync_fail` (floor_close 1→2) |
| P1.1 | 969640f0 | Inv-WAL base + alcançabilidade (append-chains) |
| P1.2 | 4de0fa61 | Inv-WAL passo sync/fence + ack + corolário da classe `wal_write_step_preserves_inv_wal` |
| P1.3 | f232b71c | Inv-LSM cadeia k-merges `merge_chain_preserves_inv_lsm` |
| P2.3 | 91789eee | herdados 0187 terminais citados (docs-only) |
| P1.4 | b7a9c64b | tokens write_pending_frame pagos (board 0/17, sem editar o voo da paralela) |
| P0.2 | aa5059bd | close registrado `occ_batch_plan∘occ_conflict` (floor_close 2→3; snippet preservado re-inserido verbatim) |
| P2.2 | ade73704 | atom `occ_snap_uses_published` (cap_data_fate 100→99, floor_extract 247→246, floor_atom 31→32) |
| P2.1 | 750de460 (paralela — rfc0199 P0.3) | graduação model→count: `point_get_probes_le_levels_l0_max` (ProbeLadderCount.lean, 0 sorry) + linhas count `scale_predict`/`probe_order_covering` + floor_count 1→3 no MESMO commit |

## Aceitação do goal — dimensão por dimensão

- **Closes registrados = 3 (floor 3):** ✓ `check_depth_floor.py`
  GREEN com `registered ladder close=3 (floor 3), residuals close=4 ==
  live 4` (aa5059bd).
- **Invariantes indutivos CITANDO os lemas um-passo registrados:**
  ✓ `wal_write_step_preserves_inv_wal` cita
  `wal_append_preserves_inv_wal` (construtor append);
  `merge_chain_preserves_inv_lsm` cita
  `inv_lsm_newest_first_never_non_live` (caso do passo). Zero sorry nos
  cinco wrappers tocados.
- **P1.4 board `unpaid_script=0/17`:** ✓ (b7a9c64b).
- **P2:** ≥1 atom df com cap −1 no mesmo commit ✓ (ade73704); herdados
  terminais ✓ (91789eee); **P2.1 FECHADO 2026-09-11** — a graduação
  model→count era o P0.3 do RFC-0199 da sessão paralela ("absorve a
  P2.1 do 0198") e landou no commit 750de460 dela:
  `point_get_probes_le_levels_l0_max` sobre o extract ScaleKernel
  (ProbeLadderCount.lean, 0 sorry) com as linhas count +
  floor_count 1→3 no MESMO commit; o checkbox do RFC-0198 flipou no
  próprio commit dela — este commit fecha a row da tabela, este
  findings e o status.md (monitor do bloqueio morto pelo usuário; o
  assentamento foi visto por checagem manual no fire seguinte).
- **Todos os flips no mesmo commit do slice:** ✓ (cada commit acima
  carrega checkbox+row do RFC-0198).

## Gates no HEAD do sweep

- depth-floor GREEN: extract=246 (floor 246), close=3 (floor 3),
  atom=32 (floor 32), residuals 4/32 == live, count=1 (floor 1),
  data_fate=99<=99.
- product-floor GREEN (D1=close R1=atom T1=atom C1=close, promoted
  4≥4); ledger GREEN (298/265/33).
- `lean_extracts --required`: GREEN no commit ade73704 (61 libs + 6
  compose, capturado naquele fire); no instante do sweep final está RED
  APENAS pelo WIP não-commitado da paralela (ProbeLadderCount.lean
  mid-proof + edit dela no lean_extracts.sh) — não é arquivo deste
  goal. Os cinco módulos tocados pelo goal buildam verde
  individualmente (1713 jobs, sorry 0).

## RFC sucessor

`docs/rfc/0200-alcancabilidade-completa-write-path-merge.md` — id 0200
verificado livre na escrita (máximo era 0199). P0 fecha a frase seL4
completa do WAL (`wal_write_step_reach`: qualquer sequência de
append/sync/ack a partir do inicial preserva Inv-WAL) + quarto close de
glue; P1 base de saída do merge + ponte sift↔newest-first; P2 cadência.
Linha nova em `docs/status.md` apontando para ele (linha do 0198
refreshada com o estado 8/9 no sweep; agora 9/9 — flip pós-750de460).
Pós-sweep: P0.1 (7fe497f5) e P1.1 (a2e6f712) do 0200 já landados.
