# RFC-0051: Além do Sim2 — π, layers e contrato OS

**Status:** done (P0+P1+P2 entregues; residual honesto: trial io_uring só roda em Linux, logs de skip fora dele)  
**Updated:** 2026-08-23  
**Depends on:** [RFC-0050](0050-world-in-tree-fdb-determinism.md) (World in-tree; P0.1–P0.2 desta página podem começar em paralelo no extract)  
**Method parent:** [RFC-0018](0018-fdb-method-parity-and-fault-coverage.md)  
**DCT (sibling repo):** `../determinismo/rfcs/0003-deterministic-concurrency-testing.md` (PCT Fig. 7; Pedra World ready-queue **não** conduz `World::run`)  
**Catálogo de buracos FDB:** `../determinismo/fdb-dst/KNOWLEDGE.md` G1–G8; paper R043 §4  
**Não reabrir:** L29 / L30; “somos mais confiáveis que CloudKit”  
**Caixas (Miri/ASan/TCG):** [RFC-0052](0052-dst-inside-boxes.md)

---

## Background

### O que o FDB declara não fechar

Paper SIGMOD’21 §4 (R043, p. 8–9), citação:

> Simulation is not able to reliably detect performance issues […] It is also unable to test third-party libraries or dependencies, or even first-party code not implemented in Flow. […] bugs in critical dependent systems, such as a filesystem or the operating system, or misunderstandings of their contract, can lead to bugs in FDB.

Apple `client-testing.html` 7.3.79 (fonte persistida `fdb-dst/fontes/03-apple-client-testing.md`):

> simulation tests rely on the actor model to execute the tests deterministically in single-threaded mode, they are **not suitable for testing various multi-threaded aspects of the FDB client**.

O `fdb_c_api_tester` existe *porque* o Sim2 não fecha MVC / Thread-Safe Client. Workloads com OS threads no sim: “test failures will be non-deterministic”.

Catálogo G1–G8 (`fdb-dst/KNOWLEDGE.md`, 2026-08-13):

| ID | Buraco Sim2 | Evidência |
|----|-------------|-----------|
| **G1** | Cliente / threads OS | Apple, citação acima |
| **G2** | Retry de layer em `commit_unknown_result` (1021) | Zemb 2025; índices duplicados |
| **G3** | Disco/FS/kernel reais | `AsyncFileNonDurable` é modelo, não ext4/io_uring |
| **G4** | TCP/TLS reais | `Sim2Conn` = `deque` + `delay()` |
| **G5** | Preempção / bg threads (RocksDB, gRPC) | Wiki 8.0: sim determinístico de Rocks/gRPC = trabalho a fazer |
| **G6** | S3 / backup client | bugs chegaram a 7.3 |
| **G7** | Operator / fdbcli / multi-binário | fora de `fdbserver -r simulation` |
| **G8** | Relógio de parede / NTP | sim = tempo lógico |

RFC-0050 copia o **eixo em que o FDB é ouro** (mundo discreto, um seed, buggify). Sem este RFC, Pedra fica um Sim2 *mais pobre* (menos CPU-hours) no mesmo eixo — e herda os mesmos buracos.

### O que Pedra já tem que o Sim2 estruturalmente não tem

| Peça | Estado | Buraco FDB que toca |
|------|--------|---------------------|
| `ConcurrentDb` (RwLock + group commit + dual-mem + fsync *off* lock) | produção, threads OS | G1 / G5 |
| DCT crate: PCT d=2 acha plantado de atomicidade **66/256**; sequencial+grosso **0/256** | `../determinismo/dct/` | G1 |
| `FailingEnv` / `RecordingEnv` no *mesmo* trait que POSIX/io_uring | in-tree | G3 |
| Index/journal canaries | `StdEnv` only até RFC-0050 P1.2 | G2 se o retry entrar no World |
| Kernels Verus/Lean (voto, AE, lease, …) | in-tree | *outro tipo* de garantia (local, não DST) |
| TCP lab `PeerMsg` | `peer_msg_tcp_lab` out-of-tree | G4 parcial |

Testemunha já medida noutro alvo (RBS F2, RFC-0003): round-robin *scriptado* 200/200 CLEAN; `spawn` real entre volumes → reopen permanente. O World que só troca *quem é o peer* (Sim2-class) **mascara** a mesma classe.

### Pain / why now

1. Flow foi a escolha do FDB para *obter* determinismo: o servidor **não tem** preempção OS no caminho simulado. Pedra **já tem** paralelismo de produção (`ConcurrentDb`). Copiar Flow agora seria recuar o produto para caber no sim.  
2. O tester que o FDB pôs em cima de G1 **não é determinístico**. Nós podemos ser: PCT + replay, o que o Apple não oferece no cliente.  
3. G2 (maybe-committed na *layer*) é a classe que o sim do servidor não executa. Montanha já tem `NotCommitted` / 2PC; o retry da canary index ainda não é um `Action` do World.

### O que “mais garantia que o FDB” **não** significa

| Inflado | Honesto |
|---------|---------|
| Mais confiável que 0.5 M disk-years CloudKit | **Não** — field e CPU-hours continuam deles |
| DST acha bugs de load-balance / p99 | Paper: o sim **não** detecta performance; TCG também não (finding TCG-vs-PMU) |
| Prova de ausência de races | PCT encontra; não prova ∀π |
| Controlar o scheduler do Linux | Antithesis / Determinator / RFC-0005 TCG; relógio guest ainda não bit-exact |

Eixo que **podemos** ganhar: classes de bug que o Sim2 *publicou* como fora, com **repro por seed** — o FDB cobre-as com tester não-determinístico ou não cobre.

---

## Problems This Solves

- **Problem:** World estilo Sim2 (um actor/peer de cada vez) não vê races de `ConcurrentDb` / OCC / group-commit.  
- **Problem:** O FDB admite G1 e responde com um tester *não* replayável; Pedra ainda não tem o dente determinístico no próprio `ConcurrentDb`.  
- **Problem:** Retry após commit incerto (G2) vive na layer; o World de 0050 não o agenda.  
- **Problem:** io_uring e POSIX são dois caminhos (G3/G5); compact em background sem π reabre o buraco RocksDB do FDB 8.0.

---

## Proposed Solution

Não aprofundar o Sim2. **Alargar o modelo** com o 5º seam (π) *sobre o código que já é paralelo*, e meter G2/G3 no mesmo seed.

```text
seed → (disk ∧ net ∧ time ∧ rng ∧ membership ∧ π)
         π = PCT(d=2) sobre tasks enabled (ConcurrentDb writers,
             OCC, layer retry, harvest io_uring)
         oráculo = silent_wrong / half-index / double-hold
         replay = mesmo trace_hash

FDB Sim2  = profundidade no universo Flow
Pedra 0051 = largura no universo que o Flow recusou
```

Três dentes obrigatórios em cada experiência (RFC-0003): **AS-IS acha**, **grosso perde**, **replay bit-estável**. CLEAN sem “grosso perde” não conta.

---

## Delivery slices (mandatory)

### P0 — ConcurrentDb sob PCT (shippable sozinho)

Um extract in-tree prova a classe que o Sim2 recusa: duas writers, check-then-act / group-commit, sequencial CLEAN, PCT vermelho, mesmo seed 8× igual.

- [x] **P0.1** Runner PCT in-tree (`pedradb-dst` ou `pedradb-world`) sobre `ConcurrentDb`: 2–4 tasks, yield nos pontos de lock/group; **sem** `dst_core` obrigatório (PCT mínimo copiado, ~Fig. 7) — status: `done` (`crates/pedradb-core/src/pct_hooks.rs` turnstile feature `pct` + hooks `op_entry`/`lead_write_lock`/`follower_wait` no commit path real; `crates/pedradb-world/src/pct_concurrent.rs` `run_pcts` 3 tasks; teste `pct_runner_drives_real_concurrentdb`: mesmo seed 2× ⇒ mesmo `schedule_hash`, 36/36 puts visíveis, durable no reopen)  
- [x] **P0.2** Plantado de profundidade 2: sequencial + round-robin grosso = CLEAN; PCT d=2 viola em ≥1 seed `0..255`; o seed que viola, 8×, mesmo `trace_hash` e a mesma violação — status: `done` (`planted_depth2_three_teeth`: sequential 0/256 — run-to-completion stickiness; RR grosso 0/256; PCT d=2 **3/256**, seed 43 `double_spend: balance=-100`, replay 8× mesma violação + mesmo `schedule_hash`; barrier `wait_all_entered` ⇒ schedule função pura de (seed, policy, n))  
- [x] **P0.3** CI: job curto no `synthetic-field`; verde = plantado ainda é encontrado (regressão do *dente*, não do engine) — status: `done` (job `world-determinism`, step "PCT over real ConcurrentDb (RFC-0051 P0; planted d=2 stays found)": `cargo test -p pedradb-world --features pct --lib pct_concurrent -- --test-threads=1`; o assert ≥1/256 derruba CI se o dente parar de ser achado)

### P1 — π × disco × layer (G1 ∩ G2 ∩ F2)

- [x] **P1.1** `FailingEnv` armado *durante* a secção crítica do group-commit (crash/EIO no fsync off-lock); sequencial pode passar; PCT×disk reproduz silent_wrong ou fence — status: `done` (hook `submit_decision` entre `active.fetch_add` e a decisão lone-vs-grupo + `blocking_section("follower_reply")` no turnstile (token sai durante esperas reais); `FailingEnvArc` com `FaultKind::SyncFail` armado pós-open; EIO no fsync off-lock cerca o grupo inteiro (`group wal write/sync failed` → `DurabilityFenced` recusa o resto). Dente `pct_disk_fence_three_teeth`: seq **0/256** (miss), PCT d=2 **9/256** (seed 44: 2 membros cercados pelo EIO single), replay 8× bit-estável; oráculo reopen: todo `Ok` sobrevive)  
- [x] **P1.2** World `Action` `CommitUnknown` / `NotCommitted-after-majority`: canary index **não** duplica secundário no retry (G2); oráculo = `row_half_indexed == false` — status: `done` (`Action::CommitUnknown` + `World::run_with_schedule`; batch Raft atômico multi-key particiona o range leader mid-flight → `NotCommitted` → heal → retry do mesmo batch; oráculo por nó: participantes com 0 ou 1 conjunto completo, `row_half_indexed == 0` e `silent_wrong == 0` em `commit_unknown_retry_never_half_indexed`, trace_hash estável no replay)  
- [x] **P1.3** OCC `OccTransaction` sob o mesmo runner (não só o wrap `ConcurrentDb`); plantado ou REAL com “grosso perde” — status: `done` (`occ_three_teeth`: sequencial **0/256**; plantado (leitor lê cru sem declarar `occ/x` no read set) **38/256** cross-group, seed 8 `snap=2 w=5 seq=9`; correto **0/256** com conflitos em 39/256 seeds — validação real do engine converte cada janela cross-group em `TransactionConflict`. Achado de semântica: membros do **mesmo grupo atômico** commitam no mesmo instante (um lock hold, um fsync) e seq per-member ali **não** é ordem de serialização — oráculo group-aware via `pct_hooks::GROUP_RANGES`; 25 janelas mesmo-grupo contadas como simultâneas, doc de semântica do `occ.rs` atualizada)

### P2 — G3/G5 sem criar um RocksDB-bg no escuro

- [x] **P2.1** Um trial Linux: `FailingEnv<IoUringEnv>` no mesmo schedule que P0 (não path canónico Darwin) — status: `done` (`FailingEnvArc` generalizado sobre o env interno (`with_inner_passing`); teste `pct_linux_iouring_env_trial` cfg(target_os="linux"): runner sobre `ConcurrentDb<FailingEnvArc<IoUringEnv>>`, **mesmo** schedule-hash do run StdEnv na mesma seed (schedule é função pura de seed/policy/n), writes visíveis e duráveis no reopen; não-Linux imprime skip explícito com o residual documentado)  
- [x] **P2.2** Compact / flush background, se existirem threads: entram no π **ou** ficam single-thread no World. Proibido: bg thread de compact sem yield no runner — status: `done` (guard mecânico `scripts/check_no_prod_time_spawn.py`: `thread::spawn`/`thread::Builder` proibidos em produção do `pedradb-core` (antes do marcador de testes do arquivo); todos os spawns atuais do core estão em `#[cfg(test)]`; matriz negativa pega violação plantada; passo CI "Spawn / wall-clock guards")  
- [x] **P2.3** G8: qualquer TTL/lease no path de store usa `Clock` lógico (DCS crate RAM-only continua fail-safe F7; não misturar `SystemTime` no oráculo) — status: `done` (mesmo guard: `SystemTime`/`UNIX_EPOCH` proibidos na produção de lease/dcs/store/world (bins de bench que carimbam relatório ficam fora do escopo do guard); produção já estava limpa — todos os usos eram helpers de teste; `temp_parent` do world deixou de usar wall-clock (pid + contador))

---

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Runner PCT in-tree sobre ConcurrentDb | done | pct_hooks.rs + pct_concurrent.rs; `pct_runner_drives_real_concurrentdb` | 2026-08-23 |
| P0.2 | p0 | Plantado d=2: grosso CLEAN, PCT acha, replay 8× | done | `planted_depth2_three_teeth` 3/256, seed 43, replay 8× | 2026-08-23 |
| P0.3 | p0 | CI regressa o dente PCT | done | `world-determinism` step "PCT over real ConcurrentDb" | 2026-08-23 |
| P1.1 | p1 | π × FailingEnv no fsync off-lock | done | `pct_disk_fence_three_teeth`: seq 0/256, PCT 9/256 (seed 44), replay 8× | 2026-08-23 |
| P1.2 | p1 | Maybe-committed / NotCommitted na canary index | done | `commit_unknown_retry_never_half_indexed` (row_half_indexed == 0, replay estável) | 2026-08-23 |
| P1.3 | p1 | OCC no mesmo runner | done | `occ_three_teeth`: plantado 38/256 cross-group, correto 0/256 (conflitos 39/256) | 2026-08-23 |
| P2.1 | p2 | FailingEnv⟨IoUringEnv⟩ num trial Linux | done | `pct_linux_iouring_env_trial` (skip explícito fora Linux) | 2026-08-23 |
| P2.2 | p2 | Bg compact só com π ou single-thread no World | done | guard `check_no_prod_time_spawn.py` + passo CI | 2026-08-23 |
| P2.3 | p2 | TTL/lease de store só via Clock lógico | done | mesmo guard (SystemTime/UNIX_EPOCH fora de produção) | 2026-08-23 |

---

## Acceptance Criteria

### Tests

**P0**

- Sequencial e round-robin de *ops atómicas* (π grosso): 0 violações no plantado em `0..255`.  
- PCT d=2: ≥1 violação em `0..255` (o dct já mediu 66/256 noutro plantado; este bar é “≥1”, não copiar a taxa).  
- Seed que viola: 8 replays, mesmo `trace_hash`, mesma task/passo da violação.  
- CI vermelho se o plantado deixar de ser encontrado (alguém “consertou” o teste).

**P1**

- P1.1: pelo menos um seed em que π×disk viola e o mesmo workload sem preempção no fsync **não** viola (dente F2).  
- P1.2: após `NotCommitted` / unknown, retry da canary → 0 ou 1 conjunto index completo, nunca `row_half_indexed`.  
- P1.3: OCC com o mesmo ritual AS-IS / grosso / replay.

**P2**

- P2.1: job Ubuntu (não Darwin) documentado; skip explícito se não-Linux.  
- P2.2: grep/CI recusa `thread::spawn` em compact/flush do core **ou** o spawn está registado no runner PCT.  
- P2.3: teste de lease/TTL com `ManualClock` — sem `SystemTime::now` no oráculo.

### Telemetry / Analytics

- `trace_hash`, seed, taxa de hit PCT (`hits/256`).  
- Nenhum produto UX.  
- **Não** reportar “mais garantia que FDB” como métrica.

### Documentation

- Esta tabela Status na mesma mudança que o código.  
- Claims: ver tabela. Linkar G1–G8 se alguém escrever “ultrapassámos o Simulation”.  
- RFC-0050 continua o eixo Sim2; este RFC é o complemento.

### Screenshots

- backend-only (tabela hits/256 + `trace_hash`).

### Claims permitidos

| Depois de | Permitido | Proibido |
|-----------|-----------|----------|
| P0 done | “ConcurrentDb tem DST de concorrência replayável; o Sim2 do FDB não testa esta classe no cliente/servidor Flow” | “Mais robustos que o FDB” |
| P1 done | “π × disk × retry de layer no mesmo seed (G1∩G2)” | “API Tester do FDB é obsoleto / Apple não testa threads” |
| P2 done | “io_uring e compact bg não são o buraco RocksDB 8.0” | “Contrato POSIX/ext4 provado”; “performance DST” |

---

## Out of scope

- RFC-0050 P0–P2 (World copy, Queued canónico, swarm L28) — irmão, não este.  
- Empilhar DST×Miri×ASan×TCG (matriz AND/NEST/NO) — [RFC-0052](0052-dst-inside-boxes.md).  
- Controlar APIC / 1-vCPU Antithesis / caixa TCG bit-exact (RFC-0005 determinismo; relógio guest ainda jitter).  
- Achar bugs de load-balance ou p99 via World.  
- Provar ∀ interleavings (Loom não escala a 5+ threads — já medido no lease RBS).  
- Clonar `fdb_c_api_tester` TOML.  
- Trazer tesoura para o core.  
- Field / CPU-years.  
- Reabrir L29 (unbundled TS) ou L30 (janela 5 s).

---

## Relação com o mapa G1–G8

| Gap | Este RFC | Residual honesto |
|-----|----------|------------------|
| G1 | P0 + P1.3 | threads de `montanha-tcp` accept/health: I/O só; cluster no worker (já) |
| G2 | P1.2 | SDKs externos / SQL completo |
| G3 | P1.1 + P2.1 | ext4 journal / page cache — det_io sibling (0050 P2.4) |
| G4 | — | TCP real = 0050 P1.1 + lab existente; não duplicar |
| G5 | P2.2 | não adicionar Rocks-style bg compact sem π |
| G6 | — | Pedra não tem S3 client no kernel (history remote é outro RFC) |
| G7 | — | `pedra` CLI / ops: canaries 0020, não Sim2 |
| G8 | P2.3 | NTP de produção nunca no World |

Kernels Verus/Lean **não** são fatia deste RFC: são garantia local já shipped. Não misturar “teorema do voto” com “PCT achou o race”.

---

## Como actualizar este doc

1. Ship slice → checkbox + Status **na mesma mudança**.  
2. REAL de concorrência → LEDGER in-tree (0050 P1.4) + seed PCT + “grosso perde”.  
3. Taxa de hit é evidência do *dente*, não do produto.  
4. Nunca promover P0 done a “ultrapassámos o FDB” — só a G1 no `ConcurrentDb`.
