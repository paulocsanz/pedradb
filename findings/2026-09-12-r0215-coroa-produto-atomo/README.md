# RFC-0215 — coroa de produto no degrau átomo (rodada 9)

Promoções átomo da rodada (régua: iff-∀ sobre o corpo extraído no
wrapper inscrito, `floor_atom +1 / floor_extract −1` no mesmo commit,
`check_depth_floor.py` GREEN no HEAD de cada promoção, planta DST
verde antes do commit, exatamente 1 teorema público por commit).

## P0.1 spec ×4 (`Properties.lean`)

| # | par | teorema | entry | commit | floor atom/extract | planta DST | quando |
|---|-----|---------|-------|--------|--------------------|------------|--------|
| 1/4 | `catalog:c1_quorum` | `c1_holds_fate_iff` | `c1_holds` | `1ae394dc` | 143/135 | `c1_as_is_does_not_imply_c1` ok | 2026-09-12 |
| 2/4 | `catalog:d1_durability` | `d1_holds_fate_iff` | `d1_holds` | `491ff82e` | 144/134 | `d1_as_is_does_not_imply_d1` ok | 2026-09-12 |
| 3/4 | `catalog:t1_atomicity` | `t1_holds_fate_iff` | `t1_holds` | `96377d9c` | 145/133 | `t1_as_is_does_not_imply_t1` ok | 2026-09-12 |
| 4/4 | `catalog:r1_no_resurrection` | `r1_answer_ok_fate_iff` | `r1_answer_ok` | (este commit) | 146/132 | `r1_as_is_does_not_imply_r1` ok | 2026-09-12 |

- **c1_holds (1/4)**: valor servido passa C1 exatamente quando a
  maioria de TODA config ativa replica — joint exige antiga E nova
  (`c1_pass`/`c1_fail` como forma-ramo citando `majority` só onde o
  corpo chama). Mutante AS-IS aceita a maioria antiga sozinha (buraco
  joint-election, RFC-0064); planta `c1_as_is_does_not_imply_c1`
  recusa (exit 0, 1 passed). Fate forall sobre o corpo extraído, sem
  loop; `r1_answer_ok`-style axiomas não usados — só os 3 axiomas
  padrão do Lean (`propext`, `Classical.choice`, `Quot.sound`).

- **d1_holds (2/4)**: D1 aceita `(acked, survives)` exatamente quando
  todo índice ackado fica dentro do prefixo sobrevivente (`d1_ok` como
  ∀-semântica first-order sobre `Slice.val`). Loop real provado por
  `loop.spec_decr_nat` (medida `len − j`, invariante prefixo limpo);
  mutante AS-IS (barreira só para synced) recusado por
  `d1_as_is_does_not_imply_d1` (exit 0, 1 passed). Axiomas: os 3
  padrão do Lean.

- **t1_holds (3/4)**: T1 aceita `(committed, aborted, staged_n,
  visible)` exatamente quando a tx é all-or-nothing — nunca ambos os
  flags, índices visíveis nomeiam writes staged, committed ⇒ todos
  visíveis, senão nenhum (`t1_ok` 4-conjuntiva). Dois loops reais
  (loop0 dos visíveis, loop1 do is_empty) via `spec_decr_nat`; mutante
  AS-IS (só integridade de bytes; tx abortada com efeito parcial
  passa) recusado por `t1_as_is_does_not_imply_t1` (exit 0, 1
  passed). Axiomas: os 3 padrão do Lean.

- **r1_answer_ok (4/4, FECHAMENTO P0.1)**: a resposta de leitura
  passa R1 exatamente quando bate com o primeiro hit na ordem de
  probe a partir do índice 0 (fonte mais nova). Perna semântica
  `r1_first_hit_fate` (private) caracteriza `r1_first_hit` como
  `IsFirstHit` via `IsFirstHitFrom` (∃-testemunha com prefixo `none`;
  unicidade por `isFirstHitFrom_unique`); a perna de resposta usa a
  igualdade de `Option` do extrato (axioma citado, não reaberto).
  Mutante AS-IS (aceita qualquer hit cobridor — a ressurreição do
  delete de
  `findings/2026-09-04-reopen-delete-resurrected`) recusado por
  `r1_as_is_does_not_imply_r1` (exit 0, 1 passed). Fechamento:
  floor_atom 142→146, floor_extract 136→132, gate GREEN no HEAD de
  cada uma das 4 promoções, 1 teorema público por commit.

## P0.2 — modelo ×4 átomo (1/4)

- **d1_modelo (1/4)**: o desfecho da máquina D1 é exatamente a decisão
  que o spec nomeia — `ok false` somente na vereda ackado + crash
  legal + corte antes do fim do registro; `ok true` pelas demais
  veredas ok. Forma de ramos construtiva (`d1m_ok_true`/`d1m_loses`):
  cada disjunto que alcança código interno carrega a igualdade que o
  habilita (`b = true`, `rec_end ≤ s.acked`, `b1 = true`), cita
  `inv_wal`/`CrashModel.of`/`crash_legal` sem reabrir corpos; o corpo
  `ok (cut >= rec_end)` é `ok (decide …)` e as pontes usam
  `of_decide_eq_true/false` + `UScalar.le_equiv/lt_equiv` + omega.
  Planta DST `d1_modelo_on_live_recording_is_not_ok` (pedradb-sim,
  exit 0, 1 passed). Axiomas: [propext, Quot.sound]. Gate GREEN:
  floor_atom 146→147, floor_extract 132→131.

- **Bug do motor achado pela planta**: o compact pode emitir um SST
  v5 vazio (0 entradas, 0 blocos) — no reopen ele caía no ramo
  fail-closed de `materialize_entries` ("no entries cache and no
  index") e o point-seek panicava `fail_stop_corrupt_block` com
  corrupção inventada. Fix 30e572db: tabela vazia com índice vazio
  devolve vazio (v1 corrompido continua erro). As 4 falhas pré-
  existentes do suite `sst` (prefix_era_mixed_sst_opens,
  last_under_user_prefix_mem_hit…, evicted_payload_without_kit…,
  write_sst_bloom_is_sized…) falham igual no HEAD anterior — não são
  regressão deste fix.

## P0.2 — modelo ×4 átomo (2/4)

- **r1_modelo (2/4)**: o desfecho da máquina R1 é exatamente a
  decisão que o spec nomeia — `ok true` quando sonda e mais-novo
  concordam na mesma entrada (ou o inventário nem passa) e `ok false`
  quando discordam. Ramos construtivos (`r1m_ok_true`/`r1m_mismatch`)
  carregam `b = true` no disjunto que alcança as chamadas e citam
  `inv_lsm`/`lsm_probe`/`r1_newest` sem reabrir corpos; a igualdade
  de `Option` do extrato é axioma citado. Planta DST
  `r1_modelo_on_live_delete_shape_is_not_ok` (pedradb-sim, exit 0,
  1 passed — a perna live dependia do fix 30e572db do SST vazio).
  Axiomas: os 3 padrão + os 2 axiomas de extrato documentados
  (`inv_lsm`, eq de Option). Gate GREEN: floor_atom 147→148,
  floor_extract 131→130.

## P0.2 — modelo ×4 átomo (3/4)

- **t1_modelo (3/4)**: o desfecho da máquina T1 é exatamente a
  decisão que o spec nomeia — `ok true` quando a tx recuperada passa
  o preditor `t1_holds_of`, `ok false` quando a quebra. Ramos
  construtivos (`t1m_holds`/`t1m_violates`) citam
  `tx_recover`/`t1_holds_of` sem reabrir corpos. Planta DST
  `t1_modelo_on_live_abort_reopen_is_not_ok` (pedradb-store, exit 0,
  1 passed). Axiomas: os 3 padrão do Lean. Gate GREEN: floor_atom
  148→149, floor_extract 130→129.

## P0.2 — modelo ×4 átomo (4/4, FECHAMENTO P0.2)

- **c1_modelo (4/4, FECHAMENTO P0.2)**: o desfecho da máquina C1 é
  exatamente a decisão que o spec nomeia — `ok false` somente quando
  o modelo serve um ack que o commit não sustenta (o buraco do
  mutante AS-IS, que aceita qualquer ack); `ok true` pelas demais
  veredas ok. Ramos construtivos (`c1m_ok_true`/`c1m_acks_uncommitted`)
  carregam `t.served = true` no disjunto que chama `propose_ack_ok` e
  citam `c1_advance_commit`/`propose_ack_ok` sem reabrir corpos.
  Planta DST `c1_modelo_on_live_queued_joint_is_not_ok` (pedradb-store,
  exit 0, 1 passed). Axiomas: os 3 padrão do Lean. Fechamento:
  floor_atom 146→150, floor_extract 132→128, gate GREEN no HEAD de
  cada uma das 4 promoções, 1 teorema público por commit.

## P1.1 — fate ×2 átomo (1/2)

- **d1_put_ok (1/2)**: o put confirma exatamente quando o ledger
  cruza a barreira — `put_ok s0 rec_len = ok s'` é exatamente a
  cadeia honesta ∃ `wal_append s0 rec_len = ok s1` → `wal_sync s1
  Honest = ok s2` → `s2.synced − s2.acked = ok i` → `wal_ack s2 i =
  ok s'` (corpos citados, não reabertos; quatro `bind_ok_inv` na
  ida, cadeia de `rw`+`bind_tc_ok` na volta). Planta DST
  `d1_modelo_on_live_recording_is_not_ok` (pedradb-sim, exit 0,
  1 passed). Axiomas: os 3 padrão do Lean. Gate GREEN: floor_atom
  150→151, floor_extract 128→127.
