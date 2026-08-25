# RFC-0056: Entregar o 100% (relativo ao TCB)

**Status:** done (P0–P2; P2.1 choice recorded as Verus group-commit kernel, RFC-0057 — not π-reduction of all thread interleavings, not VerusSync)  
**Updated:** 2026-08-24  
**Programa-mãe:** [RFC-0053](0053-ironfleet-years.md) (done Y1–Y3) · checklist: [`../formal/one-hundred-percent.md`](../formal/one-hundred-percent.md)  
**“TUDO” significa:** itens **1–11** do checklist verdes; item **12** continua para sempre (por definição); TCB permanente nomeado e **congelado** (CI recusa se crescer em silêncio).  
**Frase final permitida (a canónica):** kernel K ⊨ spec S; Verus/Lean aceitaram; relativo a axiomas A; mutar K parte S e a máquina recusa; o dicionário, em crash, não reverte para além do último sync **nos caminhos cobertos pelos kernels**.  
**Frase que nunca se diz:** “não há bugs no Pedra” / “Pedra verificado” / “fsync provado”.

---

## Background

O RFC-0053 fechou o horizonte Y1–Y3: kernels Raft (voto/AE/commit/**apply**) com prova ∀ + 2ª máquina nos extracts (voto/AE/commit), callers refinados, reopen do `Db` como kernel com lemmas, liveness bounded sob axioma quórum-vivo, lint fail-closed com handlers. O placar do checklist de 100% hoje: 3 ✅ (itens 4, 11 + spec de reopen em 5), 6 🟡, 2 ❌ (7, 10).

O que sobra cai em três famílias:

1. **Glue do engine ainda é decisão inline** (itens 1/10 — o maior buraco honesto): flush, MANIFEST apply/recovery, compact, vlog GC, 2PC glue, group-commit. ~34k LOC quentes vs ~3,5k de twins.
2. **Composição não chega ao dicionário ∀** (itens 3/5/6): os kernels provados ainda não estão **ligados** por um teorema `put` acked → reopen visível; o loop TCP não é máquina de estados; WAL/apply/reopen não têm 2ª máquina.
3. **Gates externos** (itens 7/8/9): `ConcurrentDb` espera o dente PCT do RFC-0051; liveness completa espera sincronia eventual; `StdEnv` escondidos em layers esperam o mesmo `Env` em todo o path.

O método já está provado (RFC-0053 Y1–Y3): **sanduíche** — kernel puro que produção chama → twin Verus com teorema nomeado → 2ª máquina no extract (Aeneas→Lean) → composição Stateright → mutante AS-IS que dói → catalog fail-closed → relatório. Este RFC é o mesmo sanduíche repetido até o checklist fechar. Nenhuma tecnologia nova, nenhum prover novo.

**Pain / why now:** sem este mapa, cada sessão tira mais um `if` e chama-lhe progresso; o glue (34k) é onde os SilentWrong ainda moram; e o RFC-0053 acabou — a fila precisa de dono.

## Problems This Solves

- **Problem:** itens 1, 3, 5, 6, 7, 8, 9, 10 do checklist de 100% estão abertos sem fatias nomeadas.
- **Problem:** a spec de crash prova **escolhas** (recover, reopen) mas nenhum teorema liga `put` acked → `get` após reopen.
- **Problem:** o loop TCP (`pedra-raft-node` / `montanha-tcp`) não refina nada — é host loop puro fora do TCB.
- **Problem:** TCB cresce em silêncio (nenhum CI recusa entrada nova sem prova).

## Proposed Solution

- **Wave P0 (engine LSM):** extrair as decisões de MANIFEST/flush/compact como kernels — começa pelo MANIFEST recovery porque é o análogo exato do reopen já provado (dano → desfecho), segue flush e compact. Cada uma: kernel + twin + mutante + catalog + caller.
- **Wave P1 (composição ∀):** teorema-link `put→WAL→recover→reopen→get` sobre os kernels existentes; 2ª máquina nos que faltam (WAL, apply, reopen); loop TCP como máquina de estados que refina os kernels Raft; vlog GC + 2PC glue como kernels.
- **Wave P2 (gates + congelamento):** `ConcurrentDb` via π-redução (gate: RFC-0051 PCT aterrar), liveness sob sincronia eventual, `StdEnv`→`DetEnv` nas layers, relatório “glue → zero” com meta, TCB congelado no CI.

Regra inegociável herdada do RFC-0053: se uma sub-prova não descarrega na barra (teorema nomeado, sem `sorry`, mutante dói), vira **linha aberta no Status** — nunca `sorry`, nunca claim enfraquecida.

## Delivery slices (mandatory)

### P0 — Engine LSM como kernels (fecha itens 1/10, primeira leva)

- [x] **P0.1** MANIFEST recovery decision kernel (`manifest_kernel.rs`: dano/ausência/versão → outcome reopen; `open_with_env` chama) + twin Verus com lemmas nomeados + mutante AS-IS (swallow) + catalog fail-closed (`handlers: [recover_ssts]`) — status: `done` (twin `9 verified, 0 errors`; lint 51 ok / `--ci` 173 ok, 0 fail; teste negativo do handler: renomear ⇒ `1 fail`; crash-dictionary ganhou a secção MANIFEST recovery)
- [x] **P0.2** Flush decision kernel (tail spill / rotate / trigger SST; `flush` chama) + twin + mutante (flusha nada / perde tail) + catalog — status: `done` (`flush_kernel.rs`: `flush_plan` + `wal_rotate_decision` chamados por `flush`/`try_rotate_wal`/`ensure_wal_rotated_for_gc`; twin `10 verified, 0 errors`; mutantes lose-tail e ignore-pin com teoremas de domínio finito; catalog `flush_decision` com handlers `flush`/`try_rotate_wal`, lint negativo testado)
- [x] **P0.3** Compact decision kernel (trigger L0, escolha de ficheiros, drop-tombstone guard; `compact` chama) + twin + mutante (compacta por cima de snapshot pinado) + catalog — status: `done` (`compact_kernel.rs`: `compact_pick` chamado por `compact_with_ssts_only`, `point_version_fate`/`lone_tombstone_fate` chamados por `gc_snapshot_safe`; twin `10 verified, 0 errors`; mutantes drop-under-snapshot e ignore-bottommost (F177); catalog `compact_decision` + `compact_retention`, lint negativo testado)
- [x] **P0.4** Relatório wave (`docs/formal/p0-engine-report.md` no formato y1: LOC, rácio, TCB delta) + TCB atualizado no RFC-0053 — status: `done` (relatório: kernels 839 LOC / twins 703 LOC ≈ 0.84:1, 29 verified 0 errors; TCB v2 do RFC-0053 ganhou o delta engine-kernel)

### P1 — Composição dicionário ∀ + loop TCP + 2ª máquina (fecha itens 3/5/6)

- [x] **P1.1** Teorema-link do dicionário: `put` acked (sync) → bytes no WAL (axioma persist) → `recover_collect_act` prefixo → `reopen_outcome` ServeAll/Report → `get` visível — como lemmas Verus encadeados sobre os kernels existentes + teste DST que o exerce ponta-a-ponta — status: `done` (twin `verus/dictionary_link.rs`: axioma persist como hipótese nomeada + tail-no-shadow (A4), teorema topo `lemma_put_acked_survives_crash`, `8 verified, 0 errors`, zero sorry; mutantes torn-tail-silent e reopen-swallows; teste DST `crash_after_flush_and_tail_put_recovers_both_paths` em db.rs exerce put→flush→put→crash→reopen→get; catalog `dictionary_link` com data_fate, lint negativo testado)
- [x] **P1.2** 2ª máquina nos restantes: extracts Aeneas→Lean de `wal/recover_kernel`, `apply_kernel`, `wal/reopen_kernel` (drift-stamps + `lake build` verde, sem `sorry`) — status: `done` (`SOURCE.{wal_recover,reopen,apply}` drift-stamped; `Reopen.lean` 4 theorems, `Apply.lean` 5, `WalRecover.lean` 8 — todos sem sorry; `lean_wal_apply_reopen.sh` verde; `check_extract` do pedra_formal.py agora verifica artefactos + stamps + teoremas por nome e roda o script; `--ci` 198 ok / 0 fail; teste negativo triplas mutações ⇒ 3 FAILs distintos; regressão do P40 auto-repatchada no aeneas_vote.sh)
- [x] **P1.3** Loop TCP como máquina de estados: `pedra-raft-node` step = “read frame → dispatch para o kernel do RPC → persist → send”; modelo Stateright do node inteiro refinando vote∧AE∧commit∧apply; invariantes: sem grant duplo, sem ack sujo, sem apply fora do prefixo — status: `done` (`crates/pedradb-raft/tests/tcp_node_model.rs`: um action = um frame; kernels de produção chamados dentro do passo; invariantes vote-once/F16/F11(send-time witness)/send-after-persist/apply-contiguous/applied-le-commit; 3 mutantes de loop com contraexemplo afirmado; 4/4 testes verdes; registrado no catalog `models` → roda no `--ci`)
- [x] **P1.4** vlog GC + 2PC glue como kernels (decisão de promote/rewrite; decisão de commit/abort 2PC) + twins + catalog — status: `done` (`vlog_gc_kernel.rs`: `vlog_recover_action` (swing do MANIFEST, F51) chamado por `open_with_env` + `blob_gc_action` chamado por `compact_blob_auto`; twin `14 verified`; `tx_glue_kernel.rs`: `tx_range_action` (F47/F34) chamado por `tx_finish`; twin `7 verified`; mutantes com teoremas de domínio finito; catalog `vlog_recover`+`blob_gc_pick`+`tx_glue`, teste negativo tripla ⇒ 5 FAILs)
- [x] **P1.5** Relatório wave (`docs/formal/p1-composition-report.md`) — status: `done` (LOC 339 kernels / 688 twins / 376 modelo; 29 verified novos + 17 theorems Lean; TCB delta: swing vlog e cleanup 2PC saem do glue inline; axioma persist virou hipótese nomeada; regressão P40 auto-repatchada no aeneas_vote.sh)

### P2 — Gates externos + congelamento do TCB (fecha itens 7/8/9/10-como-meta)

- [x] **P2.1** `ConcurrentDb`: **gate** = RFC-0051 P0 (PCT in-tree) aterrar. Depois: π-redução do group-commit (interleaving de threads ⇒ passo atómico no modelo) ou VerusSync — escolha registada quando o gate abrir — status: `done` (gate aberto: RFC-0051 P0–P2. Escolha registada: **nem** π-redução de todos os interleavings **nem** VerusSync — kernel Verus sequencial da decisão de grupo, RFC-0057 P2.1 / RFC-0058 P2.1. `group_commit_kernel.rs` é o passo atómico: `occ_conflict` / `group_validate` / `fence_publish_seq`; twin `verus/group_commit.rs` 14/0; extract Lean `GroupCommit.lean` sem sorry; callers `validate_occ_batch` / `lone_commit` / `GroupInFlight::max_appended_seq`. Residual honesto: glue de lock/WAL I/O/scheduler do OS continua TCB — `∀ interleavings` do `ConcurrentDb` ficou fora de escopo no contrato)
- [x] **P2.2** Liveness IronFleet-class: eleição eventual sob **sincronia eventual** (axioma explícito: crash-restart finito, sem partição infinita) — upgrade do witness BFS para propriedade de eventualidade no modelo do node inteiro — status: `done` (`tcp_node_model.rs`: `Property::eventually` `Evt-apply-quiescence` + `Evt-election` como bounded liveness — comportamentos limitados a `MAX_STEPS = ES_BOUND+2`, precondition do check de eventualidade do BFS (path-acyclic, avaliado nos estados terminais); axiomas **ES-1/ES-2/ES-3 nomeados** no modelo e no TCB (`one-hundred-percent.md` §1); 8/8 testes: verde sob os axiomas, **refutado** sem axiomas (ambos), **refutado** sem ES-3 (só eleição — cada axioma é load-bearing), mutante de loop `broken_drain` apanhado mesmo com todos os axiomas)
- [x] **P2.3** `StdEnv` escondidos → mesmo `Env` injectável em todas as layers (`Db<StdEnv>` hardcodes, persist path, `pedradb-store`) — alvo: zero `StdEnv` fora de `main`/bins; `FailingEnv` consegue exercer o path inteiro — status: `done` (ilhas `pedradb-journal` `catch_up/peek/append/changes_after` e `pedradb-index` `put_row_with_indexes/row_fully_indexed/row_half_indexed` generalizadas para `E: Env`; teste `FailingEnv` end-to-end em ambas as crates prova a injectabilidade; sweep final: zero `Db<StdEnv>` hardcode em API de biblioteca — hits restantes são defaults de type-param (`E: Env = StdEnv`), construtores de conveniência com variante injectável (`_on`/`_with_env`), módulos `#[cfg(test)]`, wrappers Env by-design (io-uring POSIX fallback) e leaf consumers a passar o default à API injectável (rocksdb-compat); 4 imports `StdEnv` mortos removidos)
- [x] **P2.4** Glue → zero (meta mensurável): relatório final com tabela LOC glue vs kernel por crate e trajecto; meta: todo caminho de destino de dados é “abre fd, chama kernel, persiste” — status: `done` (`one-hundred-percent-report.md` §3: kernel 7.723 LOC / twin 5.022 LOC (0.65:1) / handler-files 39.793 LOC por crate; trajecto P0 0.84:1 → P1 +339/+688 → 0.65:1; o shape abre-fd→kernel→persiste é lint-garantido nos 18 pares `data_fate`)
- [x] **P2.5** TCB congelado: `pedra_formal.py` ganha check que recusa **nova** entrada de TCB sem par kernel+twin no catalog (fail-closed contra crescimento silencioso) + relatório final `docs/formal/one-hundred-percent-report.md` declarando o estado dos itens 1–12 — status: `done` (`check_tcb_freeze` no `--lint`/`--ci`: todo `*_kernel.rs` tem de ser par, clone ou allowlist explícita (só cqe_kernel; stale allowlist também falha); data_fate ⇒ twin+script; negativos: kernel sintético ⇒ 1 FAIL, par sem script ⇒ 1 FAIL, restaurado 68 ok/0 fail; relatório §1: 1–6/8/9/11 verdes, 7 gated, 10 à-vista, 12 contínuo)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | MANIFEST recovery kernel + twin + mutant | done | `manifest_kernel.rs` + twin `9 verified` | 2026-08-23 |
| P0.2 | p0 | Flush decision kernel + twin + mutant | done | `flush_kernel.rs` + twin `10 verified` | 2026-08-23 |
| P0.3 | p0 | Compact decision kernel + twin + mutant | done | `compact_kernel.rs` + twin `10 verified` | 2026-08-23 |
| P0.4 | p0 | Relatório wave engine + TCB delta | done | docs/formal/p0-engine-report.md + TCB v2 delta | 2026-08-23 |
| P1.1 | p1 | Teorema-link put→…→get (dicionário) | done | verus/dictionary_link.rs `8 verified`; DST e2e green | 2026-08-23 |
| P1.2 | p1 | 2ª máquina: WAL/apply/reopen extracts | done | SOURCE stamps + Reopen/Apply/WalRecover.lean sem sorry; `--ci` 198 ok 0 fail | 2026-08-23 |
| P1.3 | p1 | Loop TCP como máquina de estados | done | tests/tcp_node_model.rs 4/4; 3 mutantes de loop com contraexemplo | 2026-08-23 |
| P1.4 | p1 | vlog GC + 2PC glue kernels | done | vlog twin `14 verified`; tx_glue twin `7 verified`; catalog ×3 negativo-testado | 2026-08-23 |
| P1.5 | p1 | Relatório wave composição | done | docs/formal/p1-composition-report.md | 2026-08-23 |
| P2.1 | p2 | ConcurrentDb via kernel Verus (não π/VerusSync) | done | RFC-0057 P2.1 `group_commit_kernel` + twin 14/0 + Lean | 2026-08-24 |
| P2.2 | p2 | Liveness sob sincronia eventual | done | tcp_node_model.rs 8/8; axiomas ES-1/2/3 no TCB; 3 refutações (axiomas necessários) | 2026-08-23 |
| P2.3 | p2 | StdEnv → Env injectável em todo o path | done | journal+index generalizadas; testes FailingEnv e2e; sweep sem hardcodes | 2026-08-23 |
| P2.4 | p2 | Glue → zero (relatório + trajecto) | done | one-hundred-percent-report.md §3 (7.723/5.022/39.793 LOC) | 2026-08-23 |
| P2.5 | p2 | TCB congelado no CI + relatório final 1–12 | done | check_tcb_freeze + 2 negativos; itens 1–6/8/9/11 verdes, 7 gated | 2026-08-23 |

## Acceptance Criteria

### Tests

- Cada kernel P0/P1: twin `N verified, 0 errors` (2 runs), zero `sorry`; mutante AS-IS com contraexemplo afirmado em teste; catalog `data_fate` + `handlers` fail-closed (teste negativo: renomear handler ⇒ lint vermelho).
- P1.1: teste DST ponta-a-ponta que exerce o link (put acked → crash → reopen → get) **e** os lemmas Verus do encadeamento verdes.
- P1.3: modelo do node inteiro com invariantes (vote-once, F16, F11, apply-contiguous) e mutantes que dóem no nível do node.
- P2.2: propriedade de eventualidade verde sob o axioma de sincronia eventual **declarado como axioma** no modelo e no TCB.
- P2.3: `grep -rn "StdEnv" crates/` sem hits fora de bins/tests (ou lista explícita das ilhas restantes no relatório).
- P2.5: `pedra_formal.py --ci` vermelho quando uma entrada de TCB nova aparece sem par kernel+twin.

### Telemetry / Analytics

- Contagens `verified` por script; tempo do `--ci`; LOC glue vs kernel por onda (relatórios). Nenhum p99 — prova não é bench (RFC-0041 intacto).

### Documentation

- Relatórios por wave (`docs/formal/p{0,1,2}-*.md`) no formato y1-report; RFC-0053 TCB atualizado a cada kernel que entra; `one-hundred-percent.md` checklist atualizado a cada item que vira ✅.

### Screenshots

- backend-only (`N verified, 0 errors`).

### Claims permitidos no fim

| Depois de | Permitido | Proibido |
|-----------|-----------|----------|
| P0 | “Decisões de MANIFEST/flush/compact são kernels com prova e dentes” | “engine LSM verificado” |
| P1 | “put acked ⇒ visível após reopen, nos caminhos cobertos (teorema-link)” | “dicionário ∀” (fora dos caminhos cobertos) |
| P2 (TUDO) | Frase canónica (§ acima) + “itens 1–9, 11 verdes; 7 = kernel de grupo (não ∀ interleavings); 10 à vista; 12 contínuo” | “não há bugs”; “fsync provado”; “Pedra verificado”; “ConcurrentDb ∀ interleavings” |

## Out of scope

- Provar o Linux/fsync/CPU (RFC-0052); reescrever em Dafny/Coq; geo replication; mais predicados HTTP como prioridade.
- Fechar o item 12 — axiomas são atacados para sempre, por definição.
- Performance/benchmarks: piso 2× Rocks default (RFC-0041) intocado; VeriBetrKV-style 8× não entra.

## Relation

| Doc | Papel |
|-----|-------|
| RFC-0053 | método + Y1–Y3 shipped; este RFC é a continuação até o checklist fechar |
| one-hundred-percent.md | o contrato (itens 1–12); este RFC é o plano de entrega dele |
| RFC-0050/51/52 | gates: World in-tree (P2.3 facilita), PCT (P2.1 gate), Miri/TCG (ataque aos axiomas) |
| IronFleet / VeriBetrKV | referência de rácio e spec de crash; língua e performance não |
