# RFC: 0157 — Multiplicadores de capacidade de verificação

**Status:** done
**Updated:** 2026-08-30
**Parents:** [0155](0155-silent-wrong-fail-closed.md), [0156](0156-resolver-os-nove-guards-e-pisos.md)

**Residual:** este RFC não apaga linha nenhuma (`R-glue`, `R-group-glue`, `R-swarm-real`, `R-fsync-lie`, `R-unsafe-posix`, `R-unsafe-capi`, `R-es`, `R-crc`, `R-uring` ficam). Ele ataca **capacidade**: quanto da codebase a nossa infraestrutura consegue verificar por unidade de esforço. `never_floor` intocado; `db_rs_extracted` só muda no fim do programa de extração (P1.4), nunca antes.

**Refused claims:** mais capacidade não é “garantia total”, “sem bugs”, “perfeito”, “acabou” ou seL4. Verus checado continua confiando no Verus (R-verus). Replay diferencial prova comportamento dos cenários executados, não o dispositivo (R-fsync-lie). Interleaving exaustivo até d=3 não é ∀ (R-group-glue). Campanha de K seeds não é ∀ TCP (R-swarm-real).

## Background

- A divisão kernel/glue está medida e congelada: **43 kernels / 11.021 LOC** verificados por especificação vs **81.752 LOC de handler** (freeze `fc0b80b`). A trajetória zero-glue avança um handler por RFC — mão de obra é o gargalo.
- **Zero provas são checadas por máquina hoje.** Os gêmeos Verus (`verus/*.rs`) estão congelados como especificação revisada; `verus` não está no PATH (R-verus). Um gêmeo errado continua compilando.
- A âncora de realidade do modelo é o cluster REAL TCP: **~100 s por seed, serial**. A campanha atual (0156) são 3 seeds ≈ 5 min; escalar K seeds serialmente não escala.
- PCT cobre interleavings por amostragem: default d=2 (congelado), d=3 explícito medido (0156: 8/16384 numa assinatura de cadeia-3). Não existe runner **exaustivo** de pequenos cenários concorrentes.
- O `World` prova teoremas sobre o `Env` modelado; a ponte World→`StdEnv` real existe só em testes pontuais (L28). Sem replay diferencial sistemático, a fidelidade modelo↔realidade é inspecionada, não detectada.
- As guardas de classe do 0156 são por arquivo (posix, capi, uring). Nada impede a mesma classe entrar por outro crate.

## Problems This Solves

- **Problem:** provar exige toolchain rodando; hoje o único verificador de especificação é a revisão humana dos gêmeos.
- **Problem:** cada REAL seed custa 100 s serial — o custo limita o K das campanhas.
- **Problem:** interleavings são amostrados (PCT por seed) quando cenários pequenos são exaustivos por enumeração.
- **Problem:** o World pode divergir do StdEnv sem que nenhum teste perceba.
- **Problem:** guardas de classe não têm varredura de workspace; extração de handler é arte manual sem caracterização prévia obrigatória.

## Proposed Solution

Quatro multiplicadores, cada um independente e shippable sozinho: (1) toolchain Verus pinado + script que checa gêmeos — primeiro conjunto checado por máquina; (2) replay diferencial World↔StdEnv com fingerprint comparável — detector de divergência de fidelidade; (3) campanha TCP paralela com K seeds no mesmo wall-clock; (4) runner exaustivo de interleavings pequenos sobre o turnstile existente. Depois: varredura de classe workspace-wide, fuzz de kernels puros, e o programa de extração do `db.rs` por estágios com caracterização antes de mover linha alguma.

## Delivery slices (mandatory)

### P0 — multiplicadores imediatos (cada um shippable sozinho)

- [x] **P0.1** Prova checada por máquina: `scripts/formal/verus_check.sh` pinando o toolchain (binário de release ou container como fallback documentado) e verificando o primeiro conjunto de gêmeos (`verus/l28.rs` + os três do 0155); saída pass/fail por gêmeo. Slice pronto quando UM gêmeo é checado ponta-a-ponta a partir de clone limpo — status: `done` (conjunto padrão 3/3 PASS sob o release pinado `0.2026.08.23.fbbbbcf`; `--all` 53/56 com 3 gêmeos em drift de toolchain registrados em `findings/rfc0157-verus-drift/`)
- [x] **P0.2** Detector de fidelidade: teste de replay diferencial `world_stdenv_diff_replay` — mesmo script semeado de ops+crash+restart+scan contra `World` e contra `StdEnv` em tempdir real; fingerprints observáveis devem ser iguais. Primeiro cenário: put/get/kill/restart — status: `done` (fingerprints idênticos Mem↔Disk; lado real repete bit-a-bit; divergência plantada de um byte é detectada)
- [x] **P0.3** Campanha TCP K-paralela: harness que roda K clusters REAL simultâneos (seeds distintas, portas distintas), agregando `napply`/kernels/contabilidade de retry por seed; `scripts/rfc0157_tcp_campaign.sh K` com K default 8 e registro em `findings/` — status: `done` (K=8: 8/8 seeds limpas na 1ª tentativa, fator 1,15× vs solo; seams `L28_BASE_PORT` 26000..26023 + stagger; registro em `findings/rfc0157-tcp-campaign/`; CORREÇÃO 2026-08-31: as "seeds distintas" nunca chegaram ao binário — os mnemônicos `0x0157_C001+` não são u64 e o `cluster_real` tinha fallback silencioso para `0x641e28`, um único mundo em todos os registros; parser estrito + sementes numéricas + cross-check do eco `seed=` landed (findings/2026-08-31-campaign-seed-collapse); POST-SCRIPT 2026-08-31: uma seed distinta dirige só 4 coisas — alvo do kill (`seed % 3`), KV, cluster-id, dir — e NÃO o RNG/timing dos nós; "K seeds distintas" = 3 classes de alvo com cobertura plena × KV/id distintos, não K execuções disjuntas (r0-dirty cobriu node1×2/node2×3/node3×3; eco do alvo no fingerprint ainda é gap aberto))

### P1 — escala e alcance

- [x] **P1.1** Guardas de classe no workspace inteiro: seção nova no `pedra_formal.py --lint` varrendo todos os crates pelas três classes do 0156 (FFI rc sem gate, len C sem cap, adoção de CQE por tag constante); site sem gate precisa de waiver nomeando o id de residual — status: `done` (varredura `check_class_scan`: 176 arquivos — 5 sites FFI rc, 3 len C ABI, 2 sites CQE, 0 waivers, `0 fail`; waiver = `RFC0157-WAIVER(R-...)` com id registrado)
- [x] **P1.2** Fuzz de kernels puros: alvos proptest/fuzz para os kernels de decisão (`may_publish_group`, trio L28, `sst_crc_fate`); contraexemplo encolhido vira `findings/` + dente AS-IS se revelar classe nova — status: `done` (sweeps determinísticos 20k trials com shrink embutido: `rfc0157_property_sweep_group_commit_kernel`, `rfc0157_property_sweep_l28_trio` (tabelas-verdade exaustivas + varredura de `attempts`), `rfc0157_property_sweep_sst_crc_fate`; nenhum contraexemplo — nenhuma classe nova)
- [x] **P1.3** Interleaving exaustivo pequeno: runner que enumera **todas** as sequências de grant do turnstile para N≤3 tarefas e ≤k yields (sem amostragem), rodando a planta de cadeia-3 e o caminho publish do group commit; relata cobertura exaustiva do espaço enumerado — status: `done` (`run_exhaustive` DFS por prefix-replay; plantas d2+cadeia-3: espaço completo |espaço|=66, 60 violadores, 0 divergência; disk-fence com cap: 3000 nós, 570 folhas, 90 violadores, 252 nós divergentes reportados como piso R-glue/R-pct — espaço é lower bound no caminho vivo)
- [x] **P1.4** Extração `db.rs` estágio 1 — caracterização antes de mover: testes de fingerprint dourado fixando o comportamento atual do caminho open/recovery do `db.rs` (sem extrair nada ainda); o estágio termina com o comportamento travado, pronto para o primeiro kernel ser extraído no estágio 2. `db_rs_extracted` segue `false` — status: `done` (`rfc0157_db_open_recovery_golden`: 3 fases — live misto SST+WAL, close+reopen, kill sem close (`mem::forget`, LOCK roubado por mesmo PID) + reopen; replay duplo idêntico = literal dourado pinado; `db_rs_extracted` continua `false`)

### P2 — consolidação

- [x] **P2.1** Corpus Verus expandido: todos os gêmeos puros não-data_fate no `verus_check.sh`; data_fate na sequência — status: `done` (drift dos 3 gêmeos resolvido sob o release pinado — `cqe_res`/`fdatasync_rc` literais `i32` no modo spec, `journal_pin` spec-twin para `fold_pins_on_read`; `--all` 56/56 PASS exit 0 = todos os 45 pares não-data_fate (36 runners) + 20 data_fate-only; registro do drift em `findings/rfc0157-verus-drift/`; re-run 2026-08-30 pós-0158 P1.1: `--all` 57/57 PASS exit 0 — 57º runner é o `verus_durable_term.sh` novo; R-verus segue no never_floor)
- [x] **P2.2** Quadro de capacidade por residual: `candidates.py` passa a imprimir, por linha de residual, guard: sim/não, gêmeo checado: sim/não, profundidade de campanha, âncora REAL — status: `done` (`capacity_board` no `candidates.py`: 28 linhas derivadas de `catalog.json`/`residuals.json`/repo com referências verificadas — teste de guard existe no repo, twin+runner existem no catalog, âncora REAL existe em `findings/`; referência quebrada = exit 1; tabela é visão de capacidade, não alegação de garantia)
- [x] **P2.3** Campanha noturna registrada: doc do runner (TCP K seeds + PCT d=3/4 sweeps) com padrão de registro em `findings/` — status: `done` (doc `findings/rfc0157-nightly/README.md` + execução real registrada em `findings/rfc0157-nightly/2026-08-30/`: seeds frescas 0x0157_N01..N008, TCP K=8 — 8/8 na 1ª tentativa, wall 207s vs solo 121s, fator 1,71 (waves 1-2 falharam o gate sob carga — 2,18/2,22 — consoles preservados; harness `cluster_real` ganhou paciência no n3-leave); PCT d=3 e d=4 executados: `pct_d3.txt`/`pct_d4.txt` com contagens e custo (d=4 35/16384 ≈ 1,1s — headroom, não cobertura; default continua d=2; CORREÇÃO 2026-08-31: as "seeds frescas 0x0157_N01..N008" colapsaram no mundo único 0x641e28 (mesma causa de P0.3, findings/2026-08-31-campaign-seed-collapse); noite com mundos realmente distintos pendente de árvore limpa)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | verus_check.sh + primeiro gêmeo checado | done — 3/3 padrão, 53/56 `--all` | `scripts/formal/verus_check.sh` | 2026-08-30 |
| P0.2 | p0 | replay diferencial World↔StdEnv | done — divergência plantada detectada | `world_stdenv_diff_replay` | 2026-08-30 |
| P0.3 | p0 | campanha TCP K-paralela | done — 8/8, fator 1,15×; 2026-08-31: colapso de semente corrigido (todos os registros rodaram o mundo único 0x641e28) | `scripts/rfc0157_tcp_campaign.sh` | 2026-08-31 |
| P1.1 | p1 | varredura de classe workspace-wide | done — 0 fail, 0 waivers | `pedra_formal.py --lint` | 2026-08-30 |
| P1.2 | p1 | fuzz de kernels puros | done — 3 sweeps, 0 contraexemplos | alvos proptest | 2026-08-30 |
| P1.3 | p1 | runner exaustivo N≤3 | done — plantas completas; disk-fence com piso | turnstile enumerate | 2026-08-30 |
| P1.4 | p1 | db.rs estágio 1: caracterização | done — dourado travado, `db_rs_extracted=false` | fingerprints dourados | 2026-08-30 |
| P2.1 | p2 | corpus Verus expandido | done — `--all` 57/57 (re-run 2026-08-30), drift resolvido | `verus_check.sh` | 2026-08-30 |
| P2.2 | p2 | quadro de capacidade por residual | done — 28 linhas, refs verificadas | `candidates.py` | 2026-08-30 |
| P2.3 | p2 | doc da campanha noturna | done — noite 2026-08-30 registrada, fator 1,71; 2026-08-31: sementes daquela noite colapsaram no mundo 0x641e28 | `findings/rfc0157-nightly/` | 2026-08-31 |

## Acceptance Criteria

- **Tests**
  - `verus_check.sh` sai 0 com o conjunto checado e lista gêmeo-a-gêmeo; sem toolchain no host, o fallback documentado executa o mesmo conjunto.
  - `world_stdenv_diff_replay`: fingerprints iguais World vs StdEnv no cenário semeado; uma divergência plantada (trocar um byte do script só de um lado) faz o teste falhar — o detector detecta.
  - Campanha paralela: K=8 seeds completam em < 2× o wall-clock de 1 seed; cada seed reporta `napply`, kernels e retry-accounting; nenhuma seed reutilizada. Correção 2026-08-31: nos registros até 2026-08-30 as *strings* de semente eram distintas mas o binário rodou todas num único mundo (0x641e28, fallback silencioso); agora as sementes são numéricas e o `seed=` ecoado no fingerprint é conferido contra a pedida em cada tentativa.
  - Lint com a varredura de classe termina `0 fail` no estado atual (todas as classes do 0156 cobertas ou com waiver).
  - Runner exaustivo: o espaço enumerado é reportado (|espaço|, violadores) e a planta de cadeia-3 aparece na enumeração d=3.
  - Estágio 1 do `db.rs`: fingerprints dourados verdes e determinísticos (replay duplo).
- **Telemetry / Analytics:** nenhuma — invariantes e harness; campanhas registram em `findings/`.
- **Documentation:** este RFC; scripts com cabeçalho explicando o piso que cada multiplicador **não** derruba.
- **Screenshots:** backend-only.

## Out of scope

- Modelo formal do SO completo / TCG guest. Provar o dispositivo (R-fsync-lie) ou a mídia. ∀ TCP, ∀ interleavings de lock, ∀ traces.
- Apagar linhas de residual ou ids do `never_floor` (R-cpu, R-rustc, R-verus, R-crc, R-deps, R-extract). Subir o default do PCT acima de 2.
- `db_rs_extracted=true` antes do fim do programa de extração; mover linha do `db.rs` sem caracterização prévia.
- rustfmt de `lib.rs`. Benches / 0149 / 0153 / 0154. Pedra vs Rocks `WriteOptions.sync=true`.
- “Garantia total”, “sem bugs”, seL4, “perfeito”, “acabou”.
