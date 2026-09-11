# RFC: 0210 — drenar o L28: o protocolo TCP real inteiro no registro e o fim dos fantasmas

**Status:** draft
**Updated:** 2026-09-11
**Parents:** [0208](0208-seam-store-raft-cadencia-cluster-data-fate.md)
(o seam store/raft: kernels raft fechados, banda l28 ×2 aberta com
plano datado das 29 restantes — fechou 6/6),
[0205](0205-degrau-composto-offlock-forall-data-fate.md) (o padrão de
composição sobre o registrado), [0155](0155-silent-wrong-fail-closed.md)
(as recusas admitidas nunca flipam)

Nota de régua: fatias de prova; nenhum claim de perf. Cartaz continua
sendo Pedra vs RocksDB default `sync=false` (`ROCKS_PARITY_SYNC=0`).
A escada `count`, espelhos e âncoras são do 0203/0204 (sessão
paralela) — este RFC não os toca.

> **Tese:** o 0208 fechou os kernels raft (zero `data_fate` pendente
> em vote/commit/membership) e pagou as duas primeiras promoções da
> banda l28 — deixando o plano datado das 29 restantes em
> `formal/aeneas/EXTRACT.md`: **22 pure-lifts extraíveis** (corpo é a
> IDENTIDADE num Bool — risco de corpo opaco ZERO, molde verificado
> 5×) e **7 fantasmas de catálogo** que nomeiam `fn`/handler/planta
> que NÃO existem no arquivo vivo. O board vivo conta **84** pares
> `data_fate`, dos quais **29 são o L28** — o protocolo TCP REAL
> entre nós (o caminho que a planta de 4 min exerce de ponta a
> ponta). Este RFC drena o bloco inteiro em três movimentos:
> (1) as 22 promoções em cadências de 4 (uma por commit, planta TCP
> real verde dirigindo cada uma) — alvo `cap_data_fate` 84→62,
> `floor_atom` 47→69, `floor_extract` 231→209; (2) o veredito datado
> dos 7 fantasmas — re-escrita para `fn` viva SE o caminho existir,
> senão aposentadoria com recusa datada SEM quebrar espelho/âncora
> nenhum do 0203/0204, nunca gate de identidade inventado para
> agradar o catálogo; (3) a composição ∀ do protocolo de remoção TCP
> sobre os atoms registrados (padrão 0205/0208: compor com o
> registrado, nunca duplicar) + sweep final em worktree destacado. O
> bloco l28 termina com ZERO `data_fate` pendente — sem flipar
> admission nenhuma e sem claim de equivalência seL4.

## Background

- Escada no fechamento do 0208 (`637eda76`): extract 231 / close 6
  registrados (residual 7) / atom 47 / count 7 / cap_data_fate 84 /
  pairs 299 / 19 libs compose. Gates 3× GREEN no worktree destacado
  do HEAD, extracts `--required` ok, sorry 0 nos wrappers tocados.
- Plano datado do bloco l28 (EXTRACT.md, 2026-09-11): 22 extraíveis
  em 6 cadências — cada corpo `pub fn l28_tcp_X_ok(ok: bool) -> bool
  { ok }`, teorema `(l28_tcp_X_ok b = ok v) ↔ ((v=true ∧ b=true) ∨
  (v=false ∧ b=false))`, prova `unfold; cases b <;> cases v <;>
  simp`; meta relativa cap 84→62 / floor_atom 47→69 /
  floor_extract 231→209.
- Cada extraível tem planta TCP REAL nomeada em
  `crates/pedradb-store/tests/l28_real_tcp.rs` (~4 min cada; as duas
  do 0208 rodaram 235s e 232s).
- Os 7 fantasmas (`l28_tcp_add/cnew/svget/newget/jleft/caught/grown`)
  nomeiam `l28_tcp_*_ok` que não existe (kernels TCP terminam em
  `l28_tcp_pj_ok`); os handlers (`tcp_node_disk_added_joint`,
  `tcp_node_disk_caught_up`), a planta
  (`l28_real_tcp_add_member_joint_cnew`) e o `cluster_real
  --add-member` também são ausentes — o caminho add-member joint
  NUNCA foi construído (EXTRACT.md: "Catalog `entry`s with no Lean
  `def`").
- Restante do cluster após o L28 (nomeado no EXTRACT.md): 22 em
  `membership_kernel.rs` (pedradb-raft), 6 em `txn_kernel.rs`, 1
  singleton `compact_unleft` — próximos RFCs.

## Problems This Solves

- **Problem:** 29 pares `data_fate` do L28 são ifs cujo destino só a
  execução decide — a definição operacional de silent-wrong em
  potencial, concentrada no protocolo TCP real que a planta crava.
- **Problem:** 7 pares do catálogo nomeiam funções que não existem —
  o catálogo mente sobre o que está inscrito; dívida precisa de
  veredito datado (conserto ou aposentadoria honesta).
- **Problem:** o protocolo de remoção TCP (remove → left ∧ high-water
  preservado) existe como atoms isolados; a composição ∀ que amarra
  o protocolo inteiro ainda não existe.

## Proposed Solution

- Cadências de 4 promoções pure-lift (uma por commit, planta TCP
  real verde de cada par no MESMO commit de flip), na ordem do plano
  datado.
- Veredito datado por fantasma: medição do caminho add-member em
  produção; se ausente, aposentadoria com recusa datada — gates do
  0203/0204 (espelhos/âncoras) verdes no HEAD antes e depois; nenhum
  gate de identidade inventado.
- Composição ∀ em `ComposeL28.lean` (20º compose lib) sobre os atoms
  registrados do bloco; razão de (não-)registro em findings (padrão
  0205/0208 P1.1).
- Sweep final em worktree destacado DENTRO de `software/` + nota
  datada em EXTRACT.md.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)

1. **P0.1:** cadência l28 1/4 — `l28_tcp_dterm`, `l28_tcp_part`,
   `l28_tcp_apply`, `l28_tcp_napply` (atoms registrados; cap
   84→80, `floor_atom` 47→51, `floor_extract` 231→227); plantas
   `l28_real_tcp_removed_durable_term`,
   `l28_real_tcp_participating_after_remove`,
   `l28_real_tcp_recover_apply`,
   `l28_real_tcp_removed_recover_apply` verdes — status: `done`
   — 1/4 `done`: `l28_tcp_dterm_ok_fate_iff` (L28.lean; RequestVote
   de termo novo com persist de hard state falhando rola o termo de
   volta ⟺ o rollback segurou — memória e disco ficam no termo
   anterior), cap 84→83, floor_atom 47→48, floor_extract 231→230;
   planta TCP REAL verde
   — 2/4 `done`: `l28_tcp_part_ok_fate_iff` (L28.lean; votante
   removido participa ⟺ o scan diz — mapa CLI/nodes stale não
   conta), cap 83→82, floor_atom 48→49, floor_extract 230→229;
   planta TCP REAL verde
   — 3/4 `done`: `l28_tcp_apply_ok_fate_iff` (L28.lean; recover
   apply fecha `commit > applied` ⟺ a recuperação aplicou — ctor
   TCP de produção), cap 82→81, floor_atom 49→50, floor_extract
   229→228; planta TCP REAL verde
   — 4/4 `done`: `l28_tcp_napply_ok_fate_iff` (L28.lean; recover
   apply fecha `commit > applied` numa réplica JÁ TIRADA de `ids`
   ⟺ a recuperação aplicou), cap 81→80, floor_atom 50→51,
   floor_extract 228→227; planta TCP REAL verde — cadência 1/4
   fechada nos números exatos do RFC (cap 84→80, floor_atom
   47→51, floor_extract 231→227)
2. **P0.2:** cadência l28 2/4 — `l28_tcp_trunc`, `l28_tcp_odrop`,
   `l28_tcp_abort`, `l28_tcp_nowms` (cap 80→76, `floor_atom`
   51→55, `floor_extract` 227→223); plantas `removed_*` verdes —
status: `done`
   — 1/4 `done`: `l28_tcp_trunc_ok_fate_iff` (L28.lean; disco sem
   `index > commit` na réplica tirada de `ids` ⟺ o truncate
   persistiu), cap 80→79, floor_atom 51→52, floor_extract 227→226;
   planta TCP REAL verde
   — 2/4 `done`: `l28_tcp_odrop_ok_fate_iff` (L28.lean; linhas
   `log_entry_key` além do novo hi apagadas ⟺ os órfãos caíram),
   cap 79→78, floor_atom 52→53, floor_extract 226→225; planta TCP
   REAL verde
   — 3/4 `done`: `l28_tcp_abort_ok_fate_iff` (L28.lean; intents 2PC
   remanescentes apagados ⟺ o abort apagou), cap 78→77,
   floor_atom 53→54, floor_extract 225→224; planta TCP REAL verde
   — 4/4 `done`: `l28_tcp_nowms_ok_fate_iff` (L28.lean; `now_ms`
   persistido na réplica tirada de `ids` ⟺ o persist aconteceu),
   cap 77→76, floor_atom 54→55, floor_extract 224→223; planta TCP
   REAL verde — cadência 2/4 fechada nos números exatos do RFC (cap
   80→76, floor_atom 51→55, floor_extract 227→223)

### P1 — next wave (depends on P0 or clearly deferrable)

3. **P1.1:** cadências l28 3/4 + 4/4 — `l28_tcp_hist`,
   `l28_tcp_fence`, `l28_tcp_clear`, `l28_tcp_pre`,
   `l28_tcp_peer`, `l28_tcp_lid`, `l28_tcp_rdr`, `l28_tcp_dsc`
   (cap 76→68, `floor_atom` 55→63, `floor_extract` 223→215);
   plantas verdes — status: `done`
   — 1/8 `done`: `l28_tcp_hist_ok_fate_iff` (L28.lean; SI hist
   persistido na réplica tirada de `ids` ⟺ o persist aconteceu),
   cap 76→75, floor_atom 55→56, floor_extract 223→222; planta TCP
   REAL verde
   — 2/8 `done`: `l28_tcp_fence_ok_fate_iff` (L28.lean; abort fence
   persistido ⟺ o persist aconteceu), cap 75→74, floor_atom 56→57,
   floor_extract 222→221; planta TCP REAL verde
   — 3/8 `done`: `l28_tcp_clear_ok_fate_iff` (L28.lean; clear
   force-local derruba intents presos ⟺ derrubou), cap 74→73,
   floor_atom 57→58, floor_extract 221→220; planta TCP REAL verde
   — 4/8 `done`: `l28_tcp_pre_ok_fate_iff` (L28.lean; preimages de
   TX caídas ⟺ a queda aconteceu), cap 73→72, floor_atom 58→59,
   floor_extract 220→219; planta TCP REAL verde
   — 5/8 `done`: `l28_tcp_peer_ok_fate_iff` (L28.lean; timeout de
   eleição do ctor TCP segue o C-new do disco, não a CLI stale),
   cap 72→71, floor_atom 59→60, floor_extract 219→218; planta TCP
   REAL verde
   — 6/8 `done`: `l28_tcp_lid_ok_fate_iff` (L28.lean; ctor TCP não
   trata first-key de HashMap como identidade ⟺ o gate local-id
   segurou), cap 71→70, floor_atom 60→61, floor_extract 218→217;
   planta TCP REAL verde
   — 7/8 `done`: `l28_tcp_rdr_ok_fate_iff` (L28.lean; ctor TCP não
   escolhe `ids.first()` remoto como leitor LocalApplied — `empty`,
   não `bad node`), cap 70→69, floor_atom 61→62, floor_extract
   217→216; planta TCP REAL verde
   — 8/8 `done`: `l28_tcp_dsc_ok_fate_iff` (L28.lean; descarte vivo
   derruba o sufixo não-commitado ⟺ derrubou), cap 69→68,
   floor_atom 62→63, floor_extract 216→215; planta TCP REAL verde
   — P1.1 fechado nos números exatos do RFC (cap 76→68,
   floor_atom 55→63, floor_extract 223→215; 8 atoms, 8 commits,
   plantas 8/8 verdes em paralelo)
4. **P1.2:** veredito datado dos 7 fantasmas — por fantasma:
   re-escrever para `fn` viva SE o caminho add-member existir em
   produção; senão aposentadoria com recusa datada EM FINDINGS,
   espelhos/âncoras do 0203/0204 verdes no HEAD antes/depois (se a
   aposentadoria quebrar conta ancorada, registrar a recusa e deixar
   o fantasma nomeado como dívida — nunca forçar); pool honesto:
   −7 SE aprovada — status: `done`

   — Veredito (2026-09-11, medição a fresco no HEAD): o binário real
   `cluster_real.rs` NÃO despacha add-member (grep `add_member`
   `AddMemberJoint` vazio no arquivo; o helper wire tag 20 de
   `tcp.rs` não é usado lá); `tcp_node_disk_added_joint` /
   `tcp_node_disk_caught_up` não existem em `crates/` (o
   `tcp_node_disk_high_water` existe e já é planta do atom pago
   `l28_tcp_hw`); a planta `l28_real_tcp_add_member_joint_cnew` não
   existe (31 testes em `tests/l28_real_tcp.rs`, nenhum com esse
   nome); zero defs Lean. Por fantasma — os 7 APOSENTADOS com
   recusa datada (`add`, `cnew`, `svget`, `newget`, `jleft`,
   `caught`, `grown`): nenhum caminho vivo para re-escrita (o
   produto decide construir, não o catálogo). Cirurgia: catálogo −7
   (299→292), `glue.data_fate` 68→61, cap 61, `single_artifact` 285
   (corrigida contagem stale 291 vs live 292), marker do ledger
   total=292/campaign=26/sa=285/aeneas=224, nota datada na residual
   R-joint, seção datada no EXTRACT.md. Verdes ANTES e DEPOIS: 3
   gates + ledger + `host_anchor_table` (0203/0204) +
   `test_proof_vs_campaign` + `test_twin_mutation`. Veredito em
   `findings/2026-09-11-rfc0210-p12-fantasmas-l28/`.

### P2 — later / polish

5. **P2.1:** cadência l28 final — `l28_tcp_pld`, `l28_tcp_std`,
   `l28_tcp_hnt`, `l28_tcp_slot`, `l28_tcp_sth`, `l28_tcp_pj`
   (cap 68→62, `floor_atom` 63→69, `floor_extract` 215→209; com
   P1.2 aprovada o pool vai a 55); bloco l28 ZERO `data_fate`
   pendente — status: `doing`

   — 1/6 `done`: `l28_tcp_pld_ok_fate_iff` (L28.lean; no-leader
   abort persist-leader é local — `next_index` repair roda —
   exatamente quando o persist aconteceu), cap 61→60, floor_atom
   63→64, floor_extract 215→214; planta TCP REAL verde
   (`l28_real_tcp_removed_pld`, 231.81s)

   — 2/6 `done`: `l28_tcp_std_ok_fate_iff` (L28.lean; re-instalação
   do C-new derruba Leader plantado em réplica fora de `ids`
   exatamente quando o step-down aconteceu), cap 60→59,
   floor_atom 64→65, floor_extract 214→213; planta TCP REAL verde
   (`l28_real_tcp_removed_std`, 368.96s)

   — 3/6 `done`: `l28_tcp_hnt_ok_fate_iff` (L28.lean; ctor TCP de
   eleitor remanescente não roteia `leader_hint` à réplica
   removida exatamente quando o hint foi filtrado), cap 59→58,
   floor_atom 65→66, floor_extract 213→212; planta TCP REAL verde
   (`l28_real_tcp_hint`, 366.61s)

   — 4/6 `done`: `l28_tcp_slot_ok_fate_iff` (L28.lean; ctor TCP de
   eleitor remanescente esquece next/match/sent_through da réplica
   removida exatamente quando o slot caiu), cap 58→57,
   floor_atom 66→67, floor_extract 212→211; planta TCP REAL verde
   (`l28_real_tcp_drop_repl`, 349.23s)
6. **P2.2:** composição ∀ do protocolo de remoção TCP (remove →
   left ∧ high-water preservado) sobre atoms registrados em
   `ComposeL28.lean` (zero sorry; twins DST verdes; razão de
   registro em findings) + sweep final (worktree destacado DENTRO
   de `software/`, gates 3× GREEN, extracts ok, sorry 0, capturas
   em findings, nota datada em EXTRACT.md: l28 drenado; 29 do
   cluster restantes nomeados) + flip `**Status:** done` —
   status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Cadência l28 1/4 (dterm, part, apply, napply) | done | 9a0869ee + d6ea1e8b + 3d00e259 + este commit (4 atoms, 4 commits) | 2026-09-11 |
| P0.2 | p0 | Cadência l28 2/4 (trunc, odrop, abort, nowms) | done | 4b44881b + df80dd90 + a4ef110f + este commit (4 atoms, 4 commits) | 2026-09-11 |
| P1.1 | p1 | Cadências l28 3/4 + 4/4 (hist…dsc, ×8) | done | f6526f0d + e04ab047 + bfba89b5 + 71ef2f8f + 191a4a1a + bc9ad570 + cd508216 + este (8 atoms, 8 commits) | 2026-09-11 |
| P1.2 | p1 | Veredito datado dos 7 fantasmas de catálogo | done | este commit (catálogo −7, 299→292; recusa datada em findings; gates+âncoras verdes antes/depois) | 2026-09-11 |
| P2.1 | p2 | Cadência final l28 ×6 — bloco ZERO data_fate | todo | — | 2026-09-11 |
| P2.2 | p2 | Composição ∀ do protocolo de remoção + sweep final | todo | — | 2026-09-11 |

## Acceptance Criteria

- **Tests:** cada promoção com planta TCP REAL do par verde
  (`tests/l28_real_tcp.rs`, ~4 min cada) dirigindo a produção;
  twins DST da composição verdes; uma promoção por commit.
- **Telemetry:** nenhuma — fatias de prova (régua do repo).
- **Documentation:** plano datado em `formal/aeneas/EXTRACT.md`
  atualizado a cada cadência; veredito dos fantasmas datado em
  findings; nota do sweep datada; flips de status NO MESMO COMMIT
  de cada promoção.
- **Screenshots:** backend-only — capturas de gates em findings.

## Out of scope

- Qualquer claim de equivalência seL4 (o proof-term cobre kernels
  que o rustc liga; o TCB nomeado no 0205 P2.1 permanece).
- Flipar `media_durable_admitted` / `forall_schedules_admitted` /
  `lock_interleavings_admitted` (recusas plantadas em produção).
- Extrair db.rs inteiro (112.092 LOC seguem TCB); semântica de
  HashMap (fronteira datada 0202 P1.1); ∀π sobre interleavings de
  ConcurrentDb.
- Tocar a escada `count`, espelhos ou âncoras do 0203/0204
  (sessão paralela); construir o caminho add-member joint de
  produção (se o P1.2 medir ausência, o veredito é aposentadoria —
  o produto decide construir, não o catálogo).
- Perf/cartaz (RocksDB parity segue o peer default `sync=false`).
