# RFC-0215 — Coroa de produto no degrau átomo: spec ×4 + modelo ×4 + fate ×2 + espinha→coroa composta

**Status:** draft
**Data:** 2026-09-12
**Autoria:** agente grind (round 8→9), sucessora direta do RFC-0214
(espinha de durabilidade fechada, `**Status:** done` no HEAD `85855e56`)

## Contexto

O RFC-0214 fechou a espinha de durabilidade (env ×6 + wal ×6 + cqe ×5 +
write_ack ×3 = 20 átomos) e a espinha composta
(`ComposeDurabilitySpine.lean`, coroa `spine_d1_every_reach`). A escada
medida ao vivo no HEAD `85855e56`: extract=136, close=6, atom=142,
count=7, `data_fate 0<=0` GREEN.

A medição ao vivo (`candidates.py` no HEAD) zerou TODOS os boards de
máquina: `unpaid_script 0/17`, `unpaid_compose 0/17`,
`unpaid_concurrency 0/5`, `unpaid_scale 0/3`, `unpaid_product 0`,
buracos A/B/D/C vazios, `data_fate=0`, `atom_to_close none`. O cartoon
restante (`cartoon_twin=4`) vive todo em
`crates/montanha-fdb-recipes/src/children_kernel.rs` — **skip** até o
usuário levantar (regra 13 do caminho). O que resta pagável de máquina
é a ESCADA: 136 pares cujo melhor artefato Lean ainda é teorema de
extrato (valor concreto, sem ∀).

Dentre eles, o bloco de maior valor rumo ao seL4 é a **coroa de
produto** — as quatro garantias que definem o produto (D1 durabilidade
do prefixo acked, R1 sem ressurreição, T1 atomicidade, C1 quorum) ainda
no degrau extrato, em três camadas:

- **spec ×4** — `crates/pedradb-spec/src/properties_kernel.rs`
  (wrapper `Properties.lean`, hoje só placeholder
  `d1_holds_loop_body_is_def : True`):
  `d1_durability` (`d1_holds`), `r1_no_resurrection` (`r1_answer_ok`),
  `t1_atomicity` (`t1_holds`), `c1_quorum` (`c1_holds`) — os
  preditores puros de spec que o TSV `product_guarantees.tsv` nomeia;
- **modelo ×4** — as máquinas que decidem cada garantia:
  `d1_modelo` (core, `D1Modelo.lean`), `r1_modelo` (core,
  `lsm_r1_kernel.rs`, `LsmR1.lean`), `t1_modelo` (store,
  `T1Modelo.lean`), `c1_modelo` (raft, `C1Modelo.lean`);
- **fate ×2** — `d1_put_ok` (core, o desfecho do put no modelo D1,
  `D1Modelo.lean`) e `c1_advance_commit` (raft, `C1Modelo.lean`).

O salto seL4 deste RFC não é só mais 10 átomos: com `d1_modelo` em
átomo, a coroa composta do RFC-0214 passa a citar a iff (o corpo
extraído não é mais aberto) e o circuito fecha em UM teorema composto:
**espinha → modelo → spec** — a garantia de produto D1 como teorema
composto sobre todo caminho de crash, com todas as pernas em degrau
átomo. É o padrão seL4: a garantia de topo apoiada em camadas cada uma
mais baixa que a anterior, sem buraco de extrato no meio.

16 átomos no total (10 coroa + 6 http, a fileira coesa mais barata do
catálogo, wrappers `Auth.lean`/`Form.lean` já inscritos no LIBS).
Zero buracos de inscrição nesta fatia.

## Meta mensurável

Escada final (se os 16 pousarem): `floor_atom 142→158`,
`floor_extract 136→120`, close=6 e `cap_data_fate 0<=0` imutáveis.
Contagem autoritativa: o gate `check_depth_floor.py` no HEAD de cada
promoção. Cadência: 1 promoção = 1 commit (teorema iff no wrapper,
planta DST verde ANTES do commit, gate GREEN no commit).

## Fatias

1. **P0.1** spec ×4 (`Properties.lean`): `d1_holds`, `r1_answer_ok`,
   `t1_holds`, `c1_holds` — iff-∀ sobre o corpo extraído de cada
   preditor de spec (o placeholder `: True` do wrapper é substituído
   pelo teorema real) — floor_atom 142→146, floor_extract 136→132 —
   status: `done` (2026-09-12; floor_atom 146, floor_extract 132)

2. **P0.2** modelo ×4: `d1_modelo` (`D1Modelo.lean`), `r1_modelo`
   (`LsmR1.lean`), `t1_modelo` (`T1Modelo.lean`), `c1_modelo`
   (`C1Modelo.lean`) — o desfecho de cada máquina de garantia é
   EXATAMENTE a conjunção/decisão que o spec nomeia — floor_atom
   146→150, floor_extract 132→128 — status: `todo` (1/4: `d1_modelo`
   feito 2026-09-12)

3. **P1.1** fate ×2: `d1_put_ok` (`D1Modelo.lean` — o put confirma
   exatamente quando o ledger cruza a barreira), `c1_advance_commit`
   (`C1Modelo.lean` — o commit avança exatamente na maioria) —
   floor_atom 150→152, floor_extract 128→126 — status: `todo`

4. **P1.2** coroa↔espinha composta: nova compose lib
   `ComposeProductCrown.lean` (11ª do array COMPOSE) — para todo
   `spine_reach` do RFC-0214, o ledger recuperado passa no preditor de
   spec `d1_holds` via a iff de `d1_modelo` (átomo; corpo NÃO
   reaberto) e a perna spec (`d1_holds`, átomo P0.1); twin kernel
   `product_crown_kernel.rs` + planta DST on live; inscrição
   COMPOSE/lakefile; **SEM TSV** — razão datada em findings (a composição
   atravessa múltiplos átomos: espinha + d1_modelo + d1_holds, não um
   par único do catálogo) — status: `todo`

5. **P2.1** http ×6 primeira fileira (wrappers `Auth.lean`/`Form.lean`
   já inscritos): `is_bearer_scheme`, `is_non_bearer_auth_scheme`,
   `authorization_matches`, `normalize_http_method`, `ascii_lower`,
   `ascii_upper` — floor_atom 152→158, floor_extract 126→120 —
   status: `todo`

6. **P2.2** sweep final em worktree DENTRO de `software/` (gates 3×
   GREEN + campaign ok + extracts ok + sorry 0, capturas
   `{SCRATCH}/r0215_sweep_*`) + nota datada `EXTRACT.md` + flip
   `**Status:** done` — status: `todo`

## Vereditos / riscos

- Se algum corpo extraído não comportar a iff (recusa MEDIDA e
  nomeada), veredito datado em findings + `EXTRACT.md` — sem gate
  inventado; o alvo da meta ajusta-se ao caminho medido, não a uma
  exceção (mesma regra do RFC-0214 P1.2, que fechou "nenhum recusado").
- `promote_atom.py` (round 8, `{SCRATCH}`) segue válido: TSV com tabs
  reais, `floor_atom +1 / floor_extract −1`, `atom_reason` datado,
  `residuals.json` — nunca `json.dump` no catálogo vivo.
- P1.2 abre a espinha do RFC-0214 pela BORDA (import `spine_reach`),
  não reabre corpos extraídos internos.

## Não-metas

- NÃO despejar `db.rs` / `concurrent.rs` (trampolim esvazia-se, não
  vaza).
- NÃO tocar `crates/montanha-fdb-recipes/**` (cartoon em skip até o
  usuário levantar).
- NÃO flipar `forall_schedules` / `media_durable` /
  `lock_interleavings` admitted; campanha ≠ ∀π segue TCB.
- Disk/`fdatasync` Ok não é mídia (RFC-0078); `never_floor` imutável
  (R-cpu R-rustc R-verus R-crc R-deps).

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | spec ×4 átomo (`Properties.lean`): d1_holds, r1_answer_ok, t1_holds, c1_holds | done | `c1_holds_fate_iff`/`d1_holds_fate_iff`/`t1_holds_fate_iff`/`r1_answer_ok_fate_iff` | 2026-09-12 |
| P0.2 | p0 | modelo ×4 átomo: d1_modelo, r1_modelo, t1_modelo, c1_modelo | todo | — | 2026-09-12 |
| P1.1 | p1 | fate ×2 átomo: d1_put_ok, c1_advance_commit | todo | — | 2026-09-12 |
| P1.2 | p1 | coroa↔espinha composta (`ComposeProductCrown.lean` + twin + DST, sem TSV) | todo | — | 2026-09-12 |
| P2.1 | p2 | http ×6 átomo: bearer/auth/method/ascii | todo | — | 2026-09-12 |
| P2.2 | p2 | Sweep final + nota EXTRACT.md + flip done | todo | — | 2026-09-12 |

## Critérios de aceite

- **P0.1–P2.1 (cada átomo)**: teorema iff-∀ no wrapper com `lake
  build` verde; `promote_atom.py` (TSV + floors + `atom_reason` +
  residuals); `python3 scripts/check_depth_floor.py` GREEN; planta DST
  isolada verde com exit checado; **1 teorema/commit** (`git show HEAD
  -- <lean> | grep -c "^+theorem"` == 1); linha da RFC flipada no
  mesmo commit; findings README da fatia.
- **P1.2**: `lake build` verde da compose lib; planta DST on live
  verde; `scripts/lean_extracts.sh --required` exit 0; COMPOSE/lakefile
  inscritos; razão datada sem TSV em findings README.
- **P2.2**: worktree destacado DENTRO de `software/`: depth-floor
  GREEN (atom=158/extract=120 se os 16 pousarem), inventory terminal,
  twin-contracts bound, `test_proof_vs_campaign.py` ok, extracts ok,
  sorry 0 nos wrappers da rodada; capturas `{SCRATCH}/r0215_sweep_*`;
  nota datada em `EXTRACT.md`; `**Status:** done` no mesmo commit.
