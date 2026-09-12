# RFC-0214 — Drenar a espinha de durabilidade: env ×6 + wal_state ×6 + cqe ×5 + write_ack ×3 no degrau átomo

**Status:** draft
**Data:** 2026-09-12
**Autoria:** agente grind (round 8), sucessora direta do RFC-0213
(catálogo ZERO `data_fate`)

## Contexto

O RFC-0213 fechou o catálogo: 292 pares, ZERO `data_fate` pendente —
os três blocos (membership 0211, cluster 0212, storage 0213) drenados
ao degrau átomo. A escada viva medida ao vivo no HEAD `7f1c7cf4`:
extract=156, close=6, atom=122, count=7, `cap_data_fate 0<=0` GREEN.

O próximo avanço significativo rumo ao seL4 não é um cap novo de
`data_fate` (zerado e congelado) — é SUBIR A ESCADA dos que ficaram:
156 pares cujo melhor artefato Lean ainda é um teorema de extrato
(valor concreto, sem ∀). Dentre eles, o bloco de maior valor de
produto é a **espinha de durabilidade** — o caminho que decide o que
sobrevive a um crash e quando o Ok é honesto:

- `wal_state_kernel.rs` ×6 — `wal_state`, `wal_append`, `wal_sync`,
  `wal_ack`, `wal_rotate`, `wal_acked_survives` (a máquina de estados
  do WAL: o que o ack garante);
- `env_crash_kernel.rs` ×6 — `env_crash`, `env_append`, `env_sync`,
  `env_barrier_floor`, `env_no_invented`, `env_honest_sync` (a costura
  Env: crash model e sync honesto);
- `cqe_kernel.rs` ×5 — `cqe_res`, `cqe_tags`, `cqe_leftover`,
  `cqe_submit`, `cqe_ring_refusal` (o anel de completions io_uring:
  a honestidade do fsync barrier);
- `write_ack_kernel.rs` ×3 — `write_ack_append`, `write_ack_barrier`,
  `write_ack_ack` (o ledger write→ack da coluna G1).

20 pares, todos com wrapper Lean já inscrito no LIBS
(`WalState`, `EnvCrash`, `Cqe`, `WriteAck`) e extrato carimbado —
zero buracos de inscrição nesta fatia. Wrappers existem e buildam;
o que falta é o degrau: cada `entry` com teorema iff-∀ sobre o corpo
extraído (o padrão `fate_iff` dos RFCs 0205–0213), promovido por
`promote_atom.py` com `floor_atom +1 / floor_extract −1` no mesmo
commit da planta DST.

## Meta mensurável

Escada final (se os 20 pousarem): `floor_atom 122→142`,
`floor_extract 156→136`, close=6 e `cap_data_fate 0<=0` imutáveis.
Vereditos datados (P1.2) ajustam o alvo SEM inventar gate — o caminho
previsto, não uma exceção. Contagem autoritativa: o gate
`check_depth_floor.py` no HEAD de cada promoção.

## Fatias (cadência: 1 promoção = 1 commit, teorema iff no wrapper,
planta DST verde ANTES do commit, gate GREEN no commit)

1. **P0.1** wal_state ×6: `wal_state`, `wal_append`, `wal_sync`,
   `wal_ack`, `wal_rotate`, `wal_acked_survives` (wrapper
   `WalState.lean`) — floor_atom 122→128, floor_extract 156→150 —
   status: `done`

   — 1/6 `done`: `inv_wal_fate_iff`
   (WalState.lean; o desfecho de `inv_wal` é EXATAMENTE a conjunção
   Booleana `acked ⊆ synced ∧ synced ⊆ written`; perna citada
   `wal_inv_closed`, RFC-0191 P2.1; o as-is esquece o braço
   acked⊆synced — dente `inv_wal_as_is_dente`), floor_atom 122→123,
   floor_extract 156→155; planta DST verde
   (`wal_inv_on_live_recording_is_not_ok`, 1 passed)

   — 2/6 `done`: `wal_append_fate_iff`
   (WalState.lean; o append é ok EXATAMENTE quando a soma `written+n`
   não estoura — único futuro `{s with written := w}`, barreira e
   acked não se movem; perna citada `wal_append_closed`,
   RFC-0191 P2.1; o as-is acka os bytes junto com o write, antes da
   barreira), floor_atom 123→124, floor_extract 155→154; planta DST
   verde (`wal_inv_on_live_recording_is_not_ok`, 1 passed)

   — 3/6 `done`: `wal_sync_fate_iff`
   (WalState.lean; o sync tem EXATAMENTE dois futuros ok, um por
   honestidade do Env — Honest promove a barreira a `written`; Lying
   devolve as watermarks recortadas pelo min de `CrashModel.of`;
   pernas citadas `wal_sync_honest_closed`/`wal_sync_lying_closed`,
   RFC-0198 P1.2; o as-is promove sempre, mesmo sync mentiroso),
   floor_atom 124→125, floor_extract 154→153; planta DST verde
   (`wal_inv_on_live_recording_is_not_ok`, 1 passed)

   — 4/6 `done`: `wal_ack_fate_iff`
   (WalState.lean; o ack tem EXATAMENTE dois futuros ok — dentro da
   barreira (valor saturado contido em `synced`): `{s with acked :=
   a}`; fora dela: recusado fail-closed, o estado volta inteiro; o
   as-is acka incondicionalmente — `acked` passa da barreira),
   floor_atom 125→126, floor_extract 153→152; planta DST verde
   (`wal_inv_on_live_recording_is_not_ok`, 1 passed)

   — 5/6 `done`: `wal_rotate_fate_iff`
   (WalState.lean; o rotate zera o log EXATAMENTE quando tudo é
   durável e acked — `acked = synced = written`; qualquer cauda
   não-durável é recusada e o estado volta inteiro; o as-is derruba
   o log sempre — bytes acked somem), floor_atom 126→127,
   floor_extract 152→151; planta DST verde
   (`wal_inv_on_live_recording_is_not_ok`, 1 passed)

   — 6/6 `done` (FECHAMENTO): `acked_survives_fate_iff`
   (WalState.lean; a corolária de sobrevivência tem EXATAMENTE dois
   futuros ok, decididos pela costura Env — corte legal
   (`crash_legal cm cut = ok true`): `v = cut ≥ acked`; corte
   ilegal: `v = true`; a legalidade é a do Env, não re-provada —
   P0.2 pinará o `crash_legal`; o as-is chama sobrevivável um corte
   abaixo do piso da barreira), floor_atom 127→128,
   floor_extract 151→150; planta DST verde
   (`wal_inv_on_live_recording_is_not_ok`, 1 passed). FECHAMENTO
   P0.1: 6/6 átomos, floor_atom 122→128, floor_extract 156→150,
   gate GREEN
2. **P0.2** env_crash ×6: `env_crash`, `env_append`, `env_sync`,
   `env_barrier_floor`, `env_no_invented`, `env_honest_sync` (wrapper
   `EnvCrash.lean`) — floor_atom 128→134, floor_extract 150→144 —
   status: `done`

   — 1/6 `done`: `crash_legal_fate_iff`
   (EnvCrash.lean; um corte é legal EXATAMENTE quando sobrevive
   entre o piso da barreira e o teto escrito — `synced ⊆ cut ⊆
   written`; o as-is ignora o piso — corte abaixo de `synced` é
   chamado de legal e come bytes prometidos), floor_atom 128→129,
   floor_extract 150→149; planta DST verde
   (`env_crash_on_live_recording_is_not_ok`, 1 passed)

   — 2/6 `done`: `append_fate_iff`
   (EnvCrash.lean; o append do Env é ok EXATAMENTE quando a soma
   `written+n` não estoura — único futuro `{m with written := w}`,
   a barreira não se move), floor_atom 129→130,
   floor_extract 149→148; planta DST verde
   (`env_crash_on_live_recording_is_not_ok`, 1 passed)

   — 3/6 `done`: `sync_fate_iff`
   (EnvCrash.lean; o sync do Env tem EXATAMENTE dois futuros ok, um
   por honestidade — Honest promove a barreira ao comprimento todo;
   Lying devolve Ok e o modelo volta inteiro (RFC-0078); o as-is
   promove sempre — sync mentiroso tratado como barreira feita),
   floor_atom 130→131, floor_extract 148→147; planta DST verde
   (`env_crash_on_live_recording_is_not_ok`, 1 passed)

   — 4/6 `done`: `barrier_floor_fate_iff`
   (EnvCrash.lean; a corolária do piso vale SEMPRE — `ok v` com
   `v = true` exato: corte ilegal não perde nada; corte legal
   mantém `cut ≥ synced` — perna citada `crash_legal_fate_iff`,
   átomo 1/6 da fatia; o as-is é o `crash_legal` sem piso),
   floor_atom 131→132, floor_extract 147→146; planta DST verde
   (`env_crash_on_live_recording_is_not_ok`, 1 passed)

   — 5/6 `done`: `no_invented_bytes_fate_iff`
   (EnvCrash.lean; a corolária do teto vale SEMPRE — `ok v` com
   `v = true` exato: corte ilegal ou `cut ≤ written` (teto da
   janela do `crash_legal_fate_iff`); a recuperação nunca inventa
   byte; o as-is é o `crash_legal` sem piso), floor_atom 132→133,
   floor_extract 146→145; planta DST verde
   (`env_crash_on_live_recording_is_not_ok`, 1 passed)

   — 6/6 `done` (FECHAMENTO): `honest_sync_fate_iff`
   (EnvCrash.lean; a corolária do sync honesto vale SEMPRE — `ok v`
   com `v = true` exato: após a barreira honesta a janela legal
   colapsa num ponto (`written ≤ cut ≤ written` força
   `cut = written` — pernas `sync_fate_iff` + `crash_legal_fate_iff`,
   átomos 3/6 e 1/6 da fatia); todo crash legal preserva o log
   inteiro; o as-is promove sync mentiroso — a barreira prometida
   não existe), floor_atom 133→134, floor_extract 145→144; planta
   DST verde (`env_crash_on_live_recording_is_not_ok`, 1 passed).
   FECHAMENTO P0.2: 6/6 átomos, floor_atom 128→134,
   floor_extract 150→144, gate GREEN
3. **P1.1** cqe ×5 + write_ack ×3: `cqe_res`, `cqe_tags`,
   `cqe_leftover`, `cqe_submit`, `cqe_ring_refusal` (wrapper
   `Cqe.lean`) + `write_ack_append`, `write_ack_barrier`,
   `write_ack_ack` (wrapper `WriteAck.lean`) — floor_atom 134→142,
   floor_extract 144→136 — status: `doing`
   — 1/8 `done`: `cqe_res_ok_fate_iff` (Cqe.lean; CQE é
   sucesso iff `res >= 0`; res-gate do fsync no ring; as-is
   `cqe_res_ok_as_is` recusado pela planta DST
   `cqe_res_ok_on_live_uring_is_not_ok`, 1 passed), floor_atom
   134→135, floor_extract 144→143; planta DST verde
   (`cqe_res_ok_on_live_uring_is_not_ok`, 1 passed)
   — 2/8 `done`: `next_user_data_fate_iff` (Cqe.lean; tags
   únicas por SQE — c≠0: `(c, c+1)` wrapping; c=0: pula o zero
   reservado e devolve `(1, 2)`; loop real via `loop.spec_decr_nat`;
   as-is `next_user_data_as_is` recusado pela planta DST
   `unique_tags_discard_leftover_same_opcode`, 1 passed), floor_atom
   135→136, floor_extract 143→142
   — 3/8 `done`: `cqe_act_fate_iff` (Cqe.lean; CQE tomado iff
   `user_data = want`; leftover descartado; as-is `cqe_act_as_is`
   recusado pela planta DST `cqe_act_as_is_adopts_leftover`,
   1 passed), floor_atom 136→137, floor_extract 142→141
   — 4/8 `done`: `submit_complete_act_fate_iff` (Cqe.lean;
   CQE colhido é usado (UseHarvested), senão espera (WaitMore);
   submit_err não solta o buffer sob DMA — F208; as-is
   `submit_complete_act_as_is` recusado pela planta DST
   `harvest_on_submit_err_uses_cqe`, 1 passed), floor_atom
   137→138, floor_extract 141→140
   — 5/8 `done`: `cqe_ring_model_admitted_fate_iff` (Cqe.lean;
   modelo de anel Verus não admitido, porta fechada — RFC-0074
   P2.2; as-is `cqe_ring_model_admitted_as_is` recusado pela
   planta DST `cqe_ring_model_is_not_admitted`, 1 passed),
   floor_atom 138→139, floor_extract 140→139
4. **P1.2** veredito datado dos medidos ausentes (SE houver: par
   cujo entry/planta não existe ou cuja classe é campanha/capacidade,
   ex. liar-campaign — recusa/aposentadoria datada SEM quebrar
   âncoras 0203/0204; nunca gate inventado) — status: `todo`
5. **P2.1** composição ∀ da espinha de durabilidade (env → wal →
   ack sobre atoms registrados) em NOVA compose lib (zero buracos;
   twins kernel/planta DST verdes; SEM registro TSV — razão datada em
   findings, não é par único) — status: `todo`
6. **P2.2** sweep final (worktree destacado DENTRO de `software/`,
   gates 3× GREEN — depth-floor, inventory-terminal, twin-contracts —
   + `test_proof_vs_campaign` ok, extracts `ok` com a contagem nova,
   sorry 0, capturas em findings, nota datada em EXTRACT.md:
   espinha de durabilidade no degrau átomo) + flip
   `**Status:** done` — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | wal_state ×6 no degrau átomo | done | 6/6: este commit (FECHAMENTO) | 2026-09-12 |
| P0.2 | p0 | env_crash ×6 no degrau átomo | done | 6/6: este commit (FECHAMENTO) | 2026-09-12 |
| P1.1 | p1 | cqe ×5 + write_ack ×3 no degrau átomo | doing | 5/8: este commit | 2026-09-12 |
| P1.2 | p1 | Veredito datado dos medidos ausentes | todo | — | 2026-09-12 |
| P2.1 | p2 | Composição ∀ env→wal→ack + twins DST | todo | — | 2026-09-12 |
| P2.2 | p2 | Sweep final + nota EXTRACT.md + flip done | todo | — | 2026-09-12 |

## Critérios de aceite

- Cada promoção: teorema iff no wrapper (build `lake build <Lib>`
  verde ANTES do commit), planta DST verde ANTES do commit, cirurgia
  `promote_atom.py` (floor_atom +1, floor_extract −1, linha `atom`
  no `close_proofs.tsv`, `atom_reason` datado no catálogo,
  residuals), `check_depth_floor.py` GREEN no commit — 1 promoção =
  1 commit, `git show <c> -- <lean> | grep -c "^+theorem"` = 1.
- Nenhuma promoção pode: quebrar âncoras 0203/0204, registrar gate
  inventado, tocar arquivos não commitados da sessão paralela em
  `pedradb-core`/`wal` (checar `git status` por arquivo antes de cada
  commit; se um kernel alvo estiver sujo dela, adiar SÓ aquela
  promoção e seguir as demais).
- `cap_data_fate 0<=0` permanece GREEN e imutável — esta RFC não
  reabre cap de `data_fate`; a série que desce é `floor_extract`.
- Pares de classe campanha/capacidade (ex. liar-campaign) NÃO são
  promovidos à força: veredito datado no P1.2 com a recusa medida.
- P2.1 sem registro TSV (razão datada em findings: atravessa três+
  kernels).
- P2.2 exige: gates 3× GREEN + `test_proof_vs_campaign` ok no
  worktree destacado dentro de `software/`, extracts `ok` com a
  contagem nova, sorry 0, capturas em findings, nota datada em
  EXTRACT.md, `**Status:** done`.

## Fora de escopo (nomeado, não esquecido)

- O bloco `group_commit_kernel.rs` ×11 (inclui pares de campanha do
  escalonador — `forall_schedules`, `lock_interleavings` — que
  PERMANECEM `ok false` por decisão TCB): sucessora natural após esta
  RFC, com veredito de classe antes da meta numérica.
- Dump de `db.rs`/`concurrent.rs`; re-pin de Aeneas/charon sem widen
  medido; Montanha (congelada); qualquer cap novo de `data_fate`.
