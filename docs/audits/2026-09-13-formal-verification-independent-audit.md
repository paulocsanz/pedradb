# Auditoria independente da verificação formal do PedraDB

**Data:** 2026-09-13
**Auditor:** externo ao projeto (nenhuma linha desta verificação foi escrita pelo auditor)
**Método:** toda contagem deste relatório foi re-derivada do repo com `find`/`grep`/`wc`/`python3` nesta árvore; nenhum número foi copiado dos docs de resumo do projeto sem re-conferência. Comandos executados e saídas capturadas estão referenciados como `{EVIDÊNCIA: <arquivo>}` (dir privado do auditor).
**Pergunta do cliente:** qual o impacto real, a %, a confiança, o custo de reprodução (rodar 1 script vs 100), e se dá para vender como par do seL4 em garantias — ou chegar perto, com passos.

---

## 0. Veredito em uma frase

**Não é par do seL4, nem "bem próximo", no eixo que define o seL4 (correção funcional completa da implementação embarcada refinando uma spec abstrata).** É, isso sim, o que o próprio repo diz em `docs/rfc/0061-residuals-sel4-ironfleet.md`: **a mesma classe de claim** (prova relativa a um TCB escrito e publicado; residual honesto; nunca "sem bugs") com **uma classe de garantia inferior** — e a auditoria confirma isso com números re-derivados. A distância para o par é mensurável e o §7 dá o roteiro; vender como "par do seL4" hoje seria exatamente a frase que o RFC-0061 do projeto proíbe.

Achado crítico de estado: **no dia da auditoria, a árvore está vermelha nos próprios gates do projeto** (CI do GitHub 7/7 runs `proof-check` falhos; `pedra_formal.py --ci` exit 1 com 140 FAIL; gates ledger e de barreiras vermelhos; 4 dos 18 scripts Verus falham). Os mecanismos são fail-closed de verdade — o drift é visível, não silencioso — mas **nenhum corpus de prova verde em HEAD é hoje evidenciável externamente**.

---

## 1. Inventário re-derivado (o que existe de fato na árvore)

### 1.1 Contagens centrais — meus números vs números declarados

| Objeto | Eu contei (2026-09-13) | Declarado em | Δ |
|---|---|---|---|
| Pares no catálogo | **312** (`len(pairs)` em `scripts/formal/catalog.json`) | marker do ledger L12: `total=292`; recount do próprio ledger L133: `298` | **marker defasado em 14–20 pares**; o gate do ledger está VERMELHO por isso |
| proof / campaign | **286 / 26** (regra default: `l28_*` = campaign) | marker: `266/26` | proof defasado em 20 |
| absent | **0** | marker: `0` | ok |
| single_artifact | **305** | marker: `285` | defasado em 20 |
| `aeneas_scripts` | **245** pares com rota Aeneas (73 scripts `scripts/aeneas_*.sh` no disco; 1 script cobre vários pares, ex. `aeneas_ae.sh` → `ae_entry`+`ae_ack`) | marker: `224` | defasado; e o nome "scripts" conta pares, não scripts |
| pares com rota Verus | **68**, via **18 scripts** distintos (`scripts/verus_*.sh`); 1 par com ambas as rotas | ledger L20 "Verus twin / Aeneas Lean" | consistente em estrutura |
| twins separados `crates/*/verus/` | **3 arquivos, 285 LOC** (só `montanha-fdb-recipes/verus/`) — legado | plano e docs antigos sugeriam gêmeos por crate | a arquitetura migrou para **twin in-file** (ver §1.3) |
| kernels `*_kernel.rs` | **68 arquivos, 27.912 LOC** sob `crates/*/src` | `docs/formal/coverage-map.md` L19: "36 arquivos, 6.285 LOC" (2026-08-24) | **mapa defasado ~3 semanas; superfície de kernel cresceu 4,4×** |
| src das crates com kernel no catálogo | **164.420 LOC** (16 crates) | coverage-map L22: "75.662 LOC (11 crates)" | idem |
| src total do workspace | **182.725 LOC** | — | — |
| Lean próprio | **245 arquivos, 85.633 LOC** (excl. `.lake`/mathlib vendada: 9.584 arquivos, 2,48M LOC) | — | — |
| `sorry` no corpus Lean construído (`formal/aeneas/lean/**`, não-`.lake`) | **0** | ledger/claims: "zero sorry" | **confirmei por grep e por build** |
| `sorry` em outputs brutos (fora da build) | **9**: `formal/aeneas/out/lean/LockfreeKernel.lean` (5), `formal/out/lean/LsmR1Kernel.lean` (1), `formal/aeneas/repro/dyn-iterator/` (3) | não declarado em lugar nenhum | achado menor: dirs de regeneração/experimento carregam sorries que o corpus construído não usa |
| `sorry` na stdlib Aeneas vendada | **4** (`Aeneas/Std/Slice.lean`:363,586; `StringIter.lean`:12,15 — aparecem como warnings na build) | não declarado como TCB | **achado: a biblioteca de suporte da extração tem lemas admitidos; é parte do TCB de prova e não está nomeada na tabela TCB do ledger** |
| ratchets TSV | `proof_depth` (floors: extract 12 / close 6 / atom 286 / count 7, `cap_data_fate 0`, `handler_loc 111.927`), `close_proofs` (315 linhas: **286 atom + 6 close + 7 count**), `product_guarantees` (4 linhas, D1=close, R1=atom, T1=atom, C1=close), `barrier_sites` (64), `coverage_floor` (29), `twin_contracts` (18), `host_anchors` (24) | ledger seções respectivas | consistente com o gate depth-floor GREEN que executei |
| scripts totais | **169** em `scripts/` (73 `aeneas_*.sh`, 18 `verus_*.sh`, 8 `check_*.py` + `parity_gate_closed.py`, `sel4_coverage.py`, `tv_write_admission_ir.py`…) | — | — |
| model tests | **34** `*model*.rs` em `crates/*/tests/` (73 test files totais); 34 `models` no catálogo | catalog `models: 34` | ok |
| clones (tokens) | **7** | ledger `clones=7` | ok |

### 1.2 A que cada prova está amarrada (twin vs extract vs produção)

A pergunta central: **quando uma prova passa, o que exatamente foi provado?**

1. **Verus in-file (a amarração mais forte, 68 pares → 18 kernels).** O script roda o Verus **sobre o próprio arquivo de produção** — ex. `scripts/verus_group_commit_kernel.sh` executa `verus crates/pedradb-core/src/group_commit_kernel.rs` (bloco `verus!` inline nas linhas 566–1052; código-ghost isolado por `#[cfg(not(verus_keep_ghost))]`). Reproduzi aqui: **39 verified, 0 errors, 0,6s** (`{EVIDÊNCIA: repro_verus_groupcommit.txt}`). É literalmente o RFC-0171 "o artefato que corre é o termo da prova".
2. **Aeneas/Charon→Lean (245 pares → 73 scripts).** O script extrai o kernel de produção via Charon (LLBC) e traduz para Lean. O corpus curado `formal/aeneas/lean/*Kernel.lean` tem **68 symlinks** para o output regenerado `formal/aeneas/out/lean/` (ex. `lean/VoteKernel.lean -> ../out/lean/VoteKernel.lean`) — o Lean construído **é** o extract, não uma cópia à mão; e o `pedra_formal.py --ci` carimba **sha256 do fonte Rust** ("aeneas SOURCE.<nome> sha256 matches …", 101 referências no script). Reproduzi o ciclo completo do voto: **verde em 3,3s** (`{EVIDÊNCIA: repro_aeneas_vote.txt}`). Exceções nomeadas: **4 cópias reais (não-symlink) no corpus** — `ApplyKernel.lean`, `ReopenKernel.lean`, `StoreTxnKernel.lean`, `WalRecoverKernel.lean` — cuja amarração depende de re-copiar manualmente; e o `EXTRACT.md` L328 admite um caso de cópia stale (GroupCommit, hoje já symlink e em sync).
3. **Exclusões de extração (o que a 2ª máquina não vê).** `scripts/aeneas_locktab.sh` exclui `crate::LockTable{,::lock,::unlock_all,::new,::alloc_id}` ("lock is nested-borrows sorry") e usa `--start-from` em outros (ex. `aeneas_posix.sh` parte de `rc_ok`). Cada exclusão é um pedaço de kernel Rust **sem prova Lean** — o par `wait_for_deadlock` prova o que sobra.
4. **Model twins (34).** Mesmo nome, domínio stand-in (ex. `u64` por `&[u8]`) — o catálogo rotula `twin_kind: model`; é o degrau mais fraco da escada `extract → close → atom` (RFC-0188).
5. **Campaign (26 pares `l28_*`).** Plantas de bug/seed replay — provam que o dente morde, não ∀; o catálogo e o ledger dizem isso corretamente.

### 1.3 Discrepâncias entre docs do projeto e árvore (todas em direção de docs defasados)

- `docs/verification-ledger.md` marker (L12) **vermelho** vs catálogo vivo — e o próprio gate do projeto acusa: `GATE ledger: RED` (`{EVIDÊNCIA: gate_ledger_consistency.txt}`). O selftest do gate se recusa a rodar com o gate vivo vermelho (fail-closed correto).
- `docs/formal/coverage-map.md` (2026-08-24): 36 kernels/6.285 LOC/46 pares/75.662 src — **hoje 68 kernels/27.912 LOC/312 pares/164.420 src**. O mapa tem regra própria de re-medição ("quando um par entrar/sair do catálogo") que não foi cumprida.
- `docs/rfc/0061` (2026-08-25): rácio prova:impl "twins ~0,65:1" — hoje o corpus Lean próprio (85.633 LOC) sobre os kernels (27.912 LOC) dá **~3,1:1** (denominador e numerador diferentes do claim antigo; ambos os números convivem com denominador declarado).

---

## 2. A "%" — com denominador nomeado (a mesma realidade dá 3 números diferentes)

| Pergunta | Resposta | Denominador (nomeado) |
|---|---|---|
| % do código com prova ∀ em kernel | **17,0%** (27.912 / 164.420) | src das 16 crates que têm kernel no catálogo (inclui módulos `#[cfg(test)]` inline) |
| idem, workspace inteiro | **15,3%** (27.912 / 182.725) | todo `crates/*/src` (20 crates, inclui harness world/sim) |
| % dos pares do catálogo com teorema ∀ verde em HEAD | **88,5%** (276/312) ou **96,5%** (276/286) | todos os pares / só os proof-tier |
| % no degrau `atom`+ (escada 0188) | **93,6%** (292/312: 286 atom + 6 close) | pares no catálogo; 12 ainda no degrau extract; 26 campaign fora |
| garantias de produto (RFC-0191) | **4/4** no piso (D1 close, R1 atom, T1 atom, C1 close) | as 4 frases D1/R1/T1/C1 |
| TCB declarado de glue | **~136,5 kLOC** (164.420 − 27.912) | src das crates formalizadas fora de kernel |

Os 10 pares fora do "verde em HEAD": roteados para 4 scripts Verus que **falham na árvore atual** (`verus_vote_decision.sh`, `verus_dictionary_link.sh`: alvo sem `vstd` importado — podridão de rota pós-RFC-0171; `verus_changelog_rebuild.sh`, `verus_lookup.sh`: *"cannot call function with mode exec"* — drift real de prova) — todos **sem** rota Aeneas de backup no catálogo (`{EVIDÊNCIA: verus_all_scripts.txt, verus_failures_detail.txt, pairs_on_failing_scripts.txt}`).

Comparação de escala com os pares da literatura (figuras do seL4/IronRSL: citação in-repo `docs/rfc/0061` L15 e `docs/formal/one-hundred-percent.md` L11 — web fora do ar no dia, erro 402; ver §8): seL4 ~10 kLOC C provados ~200 kLOC Isabelle (**~20:1**, **100% do kernel**); IronRSL 5.114 impl + 39.253 prova (3,6:1, loop principal no TCB); Pedra 27,9 kLOC de kernel com 85,6 kLOC Lean (**3,1:1**, **17% do src das crates formalizadas**).

---

## 3. O que NÃO está provado (lista que o repo nomeia e eu confirmei)

1. **Refinamento top-level.** Não existe UM teorema "a implementação refina a spec do dicionário com crash". Existem 286 atoms per-função + libs de composição (`Compose*`, 24) — composição existe como bibliotecas separadas, não como o teorema único que define o seL4.
2. **Glue (~136,5 kLOC).** Declarado TCB à vista (RFC-0056 P2.5), exercitado por DST/World/oráculos, não provado. E **no dia da auditoria o caller-lint acusa 12 quebras reais** ("db.rs/concurrent.rs não chama mais `probe_order_covering()`, `group_validate()`, `occ_member_fate()`…") — a amarração caller→kernel está rompida nesses pontos (`{EVIDÊNCIA: repro_pedra_formal_ci.txt}`).
3. **∀ interleavings / concorrência.** PCT d=2–4 é campanha estatística; o kernel de group-commit tem teorema de atomicidade de grupo; o `ConcurrentDb` inteiro não tem redução provada (item 7 do `one-hundred-percent.md` — "verde relativo ao kernel de grupo").
4. **Persistência física.** "fdatasync persistiu" é axioma de SO (TCB); TCG power-cut e F_FULLFSYNC são experimento nightly.
5. **rustc/LLVM/Z3/Verus/Charon/Aeneas/Lean + deps (`lz4_flex`, `parking_lot`, `bytes`)** — `never` congelado em `scripts/formal/residuals.json` (R-cpu, R-rustc, R-verus, R-crc, R-deps). A stdlib Aeneas vendada com 4 sorries (§1.1) pertence a esta lista e **não está nomeada**.
6. **Liveness** — axiomas ES-1/2/3 declarados (`one-hundred-percent.md` §1), refutáveis, não teorema.
7. **Exclusões de extração** (§1.3 ponto 3) e as 4 cópias não-symlink do corpus.
8. **Exaustivo/crash-injection com fronteira nomeada**: N≤3 (66/66), grid crash T≤12/S≤4 (33/33) — ∀ dentro da fronteira declarada, não além.

---

## 4. Confiança — o que EU executei vs o que confio por desenho

Executado nesta máquina (macOS/arm64, toolchains do host: verus pinado 0.2026.08.09.92f466f em `~/.local`, charon 0.1.232, aeneas daa85d7, lean 4.31.0 via elan — os mesmos pins de `formal/aeneas/PINS.md`):

| Artefato | Resultado | Custo |
|---|---|---|
| `scripts/lean_extracts.sh` (corpus Lean inteiro) | **verde** — "Build completed successfully (1941 jobs)", 64 libs + 24 compose, 0 sorry próprios | **12,5s** (morno; `.lake`/mathlib **não** está no git — frio exige elan+rede) `{EVIDÊNCIA: skip_behavior_lean.txt}` |
| `scripts/aeneas_vote.sh` (ciclo Charon→Aeneas completo) | **verde** (extract regenera VoteKernel.lean idêntico ao symlink) | **3,3s** `{EVIDÊNCIA: repro_aeneas_vote.txt}` |
| 18 × `scripts/verus_*.sh` | **14 verde / 4 vermelho** (§2) | **5,7s** no total `{EVIDÊNCIA: verus_all_scripts.txt}` |
| `gate_exhaustive` (cargo, exaustivo N≤3) | **verde** — 66/66 schedules, buggy 60/60, clean 0/0 | **40,6s** `{EVIDÊNCIA: repro_gate_exhaustive.txt}` |
| 8 gates python (`check_*.py`) | **5 verde / 3 vermelho** (ledger RED; barrier RED 3 sítios; no_prod_time_spawn 98 violações todas em `three_teeth_queued.rs` de teste) | <1s cada `{EVIDÊNCIA: gates_cheap_all.txt}` |
| `python3 scripts/formal/pedra_formal.py --ci` (gate central) | **vermelho** — 2.527 ok / 0 gap / **140 FAIL** (107 enrollment de kernel novo, 12 caller-lint reais, 8 tcb-freeze, 8 three-teeth, 1 Lean `Properties.lean`, 4 metadata) | 3min54s `{EVIDÊNCIA: repro_pedra_formal_ci.txt}` |
| CI GitHub (`gh run list` em `paulocsanz/pedradb-internal`) | **7/7 runs `proof-check` = failure; `verification-gates` idem** (últimos pushes 2026-09-11; HEAD está 335 commits à frente) | — `{EVIDÊNCIA: ci_runs_status.txt, ci_push_state.txt}` |

Confiado por desenho (não re-executado): Kani harnesses (não instalado aqui), os demais 72 scripts Aeneas (o padrão é o mesmo do voto que executei), TCG nightly, TSan. Fronteira honesta: eu **não** re-verifiquei os 1941 jobs Lean do zero (build morna com cache do host) nem todos os extracts.

**Julgamento de confiança:** os mecanismos são real e incomodamente fail-closed — todo gate que executei tem selftest de sabotagem e recusa estado inconsistente em vez de esconder. O corpus Lean é genuíno (0 sorry no que é construído; teoremas ∀ com `native_decide` pontual para exemplos concretos, 91 usos). Mas **a afirmação "o corpus está verde" não é portátil para terceiros hoje**: (a) CI nunca foi verde no GitHub; (b) scripts Aeneas/Lean saem com `skip … exit 0` quando a toolchain falta (`{EVIDÊNCIA: skip_behavior_fakehome.txt}`: sem lake, `lean_extracts.sh` imprime "skip" e sai 0; só `--required` falha) — e **nenhum workflow de CI roda Lean/Aeneas/Charon**, então ninguém fora do host do autor é forçado a pagar isso; (c) 335 commits inéditos não passaram por CI nenhum.

---

## 5. Matriz de reprodutibilidade (quanto custa provar a um cético)

| Classe | Entrada | Toolchain | Roda aqui? | Custo medido | Observação |
|---|---|---|---|---|---|
| Gates python (ledger/depth/product/barrier/twin/inventory/seam) | `python3 scripts/check_*.py` | python3 | sim | <1s cada | 5/8 verdes; os 3 vermelhos são Estado da árvore, não falha de gate |
| Verus in-file | `bash scripts/verus_<kernel>.sh` | verus pinado (sha no `proof-check.yml`) | sim (local) | 0,3–0,6s/script; 5,7s todos os 18 | 4 falham em HEAD |
| Extract Aeneas | `bash scripts/aeneas_<nome>.sh` | rustc nightly pinado + charon + aeneas | sim | 3,3s/par medido; 73 scripts ≈ **~4 min** estimados | skip→exit 0 sem tools |
| Corpus Lean | `scripts/lean_extracts.sh [--required]` | elan/lake + lean 4.31.0 + mathlib (rede; `.lake` fora do git) | sim | **12,5s morno**; frio = install elan + fetch mathlib | **não está em nenhum CI** |
| Exaustivo/crash/seed-ratchet/coverage | `cargo run -p pedradb-world --features pct --bin gate_*` | cargo | sim | 40,6s (exaustivo, já compilado) | estão no `verification-gates.yml` (CI vermelho) |
| Catálogo/freeze central | `scripts/pedra_formal.sh --ci` | python3 + cargo (crates de extração) | sim | 3min54s | **exit 1 em HEAD (140 FAIL)** |
| Kani | `proof-check.yml` job kani | kani 0.67.0 | não (sem kani aqui) | — | pinado por sha no CI |

**Conjunto mínimo de um cético** (não os 169 scripts): ~10 comandos — os 8 gates python, `lean_extracts.sh --required`, os 18 `verus_*.sh`, os 73 `aeneas_*.sh` (ou uma amostra + `pedra_formal --ci` que carimba os sha), e os 4 bins `gate_*` de cargo. Total medido no meu hardware: **~10 minutos, tudo incluso**, após ~30–60 min de setup de toolchain (pins documentados em `formal/aeneas/PINS.md`). Isso é excepcionalmente barato para o tamanho do corpus — o gargalo não é custo, é o estado vermelho e a ausência de Lean no CI.

---

## 6. Veredito seL4 — eixo a eixo

Proveniência das figuras seL4: tentativa de web no dia da auditoria falhou (erro 402 do balance). Figuras citadas do próprio repo (`docs/rfc/0061` L15, `docs/formal/one-hundred-percent.md` L11 — ambas consistentes com a memória do auditor: ~10 kLOC C, ~200 kLOC Isabelle, TCB = hardware+boot+spec+gcc/Isabelle; depois validation de tradução ARM 2015; provas de integridade/autoridade e inicializador separadas; ~20+ pessoas-ano). Rotulo tudo abaixo como **[in-repo]** ou **[memória]**.

| Eixo (o que define o seL4) | seL4 | Pedra hoje | Par? |
|---|---|---|---|
| Correção funcional completa: **a implementação inteira** refina spec abstrata, um refinamento só | sim [in-repo/memória] | **não**: 286 ∀ per-função sobre kernels (17% do src das crates formalizadas); sem teorema top-level; composição em libs separadas | **não** |
| Superfície provada | 100% do kernel (10k C) | 27,9k kernels / 164,4k src formalizado; glue 136,5k = TCB | **não** |
| Integrity/authority confinement (2º teorema) | sim [memória] | sem counterparte de prova (fail-closed/CRC são atoms+testes, não fluxo de informação) | **não** |
| Inicializador provado (capDL) | sim [memória] | kernels de recovery (`wal_recover`, `manifest_recover`, `reopen_outcome`, `vlog_recover`) com close/atom — counterparte **parcial**, per-função | parcial |
| Rácio prova:impl | ~20:1 [in-repo] | ~3,1:1 (85,6k Lean : 27,9k kernel) | não comparável, mas ordem diferente |
| Concorrência/interrupção modelada na prova | irq model explícito [memória] | `∀ interleavings` explicitamente fora; PCT campanha + teorema do kernel de grupo | **não** |
| Binário | translation validation ARM (2015) [memória] | rustc/LLVM = TCB `never` (R-rustc); RFC-0171 P2 planeja | **não** |
| Classe de claim (TCB escrito, residual publicado, frase "sem bugs" proibida) | sim | **sim** — ledger de 3 camadas, residuals.json congelado, never-list; o repo proíbe a si mesmo "tão robusto quanto seL4" | **sim** |
| Reprodutibilidade externa | builds públicos há anos | CI 7/7 falhos; Lean/Aeneas fora do CI; skip→0 silencioso | **não** |
| Ferramentas de prova | Isabelle | Verus+Z3, Charon, Aeneas, Lean+mathlib (com 4 sorries na stdlib Aeneas) — mais componentes confiados | classe igual, cadeia maior |

**Frase de venda honesta disponível hoje:** *"programa de verificação na classe de claim seL4/IronFleet: teoremas ∀ machine-checked sobre kernels de produção em Rust (o arquivo que o rustc liga é o termo), TCB escrito com ~136 kLOC de glue declarado, residuais publicados e atacados por DST — reprodutível em ~10 minutos com pins de toolchain"* — desde que precedida do conserto do §7-P0, senão nem isso.

**Distância até o par:** os itens 1–3 do roadmap abaixo são engenharia (semanas–meses); itens 4–5 são pesquisa (o seL4 pagou ~20 pessoas-ano; IronFleet pagou o loop no TCB e ainda assim 3,6:1).

---

## 7. Roadmap priorizado até (ou perto de) o par — cada passo nomeia o mecanismo do repo que estende

**P0 — reaver o verde que o próprio sistema define (dias; sem isso nada é vendível):**
1. Repinar o marker do ledger (`docs/verification-ledger.md` L12 → total 312 / proof 286 / single_artifact 305 / aeneas 245) — o gate já diz os números.
2. Enrolar os 107+8 kernels novos no `glue.kernel_paths`/catálogo (ou allowlist com dono) e consertar as 12 quebras de caller-lint (`db.rs`/`concurrent.rs` voltarem a chamar `probe_order_covering`, `group_validate`, `occ_member_fate`…) — são exatamente a classe "produção desamarrou do kernel verificado".
3. Consertar os 4 scripts Verus: `vote`/`dictionary_link` (rota podre pós-RFC-0171: ou restauram import vstd ou o par migra para rota Aeneas no catálogo) e `changelog_rebuild`/`lookup` (drift exec-mode: provar de novo a assinatura atual).
4. Atualizar `scripts/ratchet/barrier_sites.tsv` com os 3 sítios novos (2× `sync_dir` db.rs, 1× `sync_data` wal/mod.rs, 2× `sync_all` fullfsync_anchor) **com o teste de injeção no mesmo commit**, como a regra manda; dar destino às 98 violações de clock do `three_teeth_queued.rs` (exempt de harness ou conserto).
5. Re-medir o `coverage-map.md` (a própria regra dele) e nomear os 4 sorries da stdlib Aeneas na tabela TCB.

**P1 — CI ou não aconteceu (semanas):**
6. Estender o `proof-check.yml` (que já pinna verus por sha256) com elan+lean+charon+aeneas pinados e rodar `lean_extracts.sh --required` + os 73 `aeneas_*.sh --required` (~4 min) — mata o padrão skip→exit 0 e dá ao corpus Lean a mesma evidência externa que o Verus tem no desenho. Verde sustado no push (o branch está 335 commits à frente da origem — nenhum deles viu CI).

**P2 — o eixo que falta (meses, engenharia pesada):**
7. **Teorema top-level de refinamento**: subir as 24 libs `Compose*` a UM teorema "implementação ⊨ spec dicionário+crash (prefixo ackado sobrevive)" — é a definição do seL4 que falta; cada atom existente vira lema dele.
8. Continuar o dreno de glue do RFC-0219 (trampolim 1-if-por-kernel; `concurrent.rs` já foi 15→6 no commit f0671326) até o glue ser só `Env`/trampoline — o análogo do assembly que o seL4 deixou de fora; a % de kernel provado sobe de 17% na direção de 100% do que resta.

**P3 — pesquisa (o preço real do par):**
9. Redução de concorrência do `ConcurrentDb` via o kernel de group-commit (ou lógica separacional/VerusSync) — o análogo do modelo de irq do seL4.
10. Translation validation de um rustc/LLVM pinado num alvo (RFC-0171 P2) — o movimento ARM-2015 deles.

---

## 8. Limitações desta auditoria

- Web indisponível (402) no dia: figuras seL4 são in-repo + memória rotulada, não re-conferidas na fonte.
- **Árvore em movimento durante a auditoria:** uma sessão paralela de desenvolvimento editou a árvore enquanto eu media (kernel LOC foi de 27.912 às 16:28 para 27.960 às 16:45; `docs/rfc/0217*` e `findings/2026-09-13-rfc0217*` mudaram no intervalo). Todas as contagens deste relatório se referem à janela 16:20–16:45 de 2026-09-13; contagens derivadas de `catalog.json` são instantâneas de um catálogo que cresce ~20 pares/mês.
- Não re-executei: Kani, TSan, TCG nightly, os 72 scripts Aeneas restantes, build Lean a frio, campanhas longas. Amostrei cada classe com o exemplar canônico e executei todos os gates de ratchet.
- LOC inclui código de teste inline (`#[cfg(test)]`) nos denominadores de src; o repo nunca publicou regra diferente para src inteiro.
- O estado vermelho pode ser um artefato do ritmo de trabalho (commits "snapshot WIP" com CI tolerado a vermelho, herdados e nomeados no próprio RFC-0191 P1.6) — mas para um auditor externo, vermelho é vermelho até o dia em que fica verde no CI.
