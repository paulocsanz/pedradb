# RFC: 0059 — Escala massiva paralela de DST e invariantes de cluster

**Status:** draft (P0 done; P1 done — P2 abertos)
**Updated:** 2026-08-24

## Background

- O World (`crates/pedradb-world`) simulava uma seed por vez, com nós em
  disco real: cada run era dominado por `F_FULLFSYNC` (~1.4 seeds/s medidos
  nesta máquina) — paralelismo de threads não escalava (1.3× em 8 workers)
  porque a barreira forte serializa no volume e >85% da CPU ficava idle em
  syscalls.
- A escala de cluster coberta era 3–5 nós e os oráculos existentes
  (silent_wrong, row_half_indexed, fold_mismatch) são por-run; não
  existia um checker de invariantes cross-node no estado convergido
  (a classe de invariant checker que o simulation do FDB roda).
- O paralelismo só é alavanca de escala se for **sound**: mesma seed ⇒
  mesmo trace independente de preempção/ordem de threads (gate
  serial-vs-paralelo por `trace_hash`).

## Problems This Solves

- **Problem:** explorar o espaço de schedules em 1–2 seeds/s não compra
  cobertura; anos de soak se compensam com núcleos, não com paciência —
  mas só se o executor paralelo for determinístico e o gargalo físico
  (barrier de disco) sair do caminho dos runs de simulação.
- **Problem:** sem invariantes no estado convergido, divergências
  cross-node (valor fantasma, split brain, ressurreição de delete) só
  aparecem se um oráculo por-run passar exatamente na janela errada.
- **Problem:** a escala de cluster travada em 3–5 nós não exercita
  quorums maiores, mais destinos por exchange e braços de partição em
  mais nós.

## Proposed Solution

- Executor `world_swarm`: `run_swarm` com work-stealing por `AtomicU64`,
  workers = núcleos, um parent temporário por worker, relatório
  `SwarmReport` com `seeds_per_s` (telemetria honesta; sem claim de
  CPU-hours vs FDB).
- Backend de storage dos nós alternável (`WorldEnv`: disco real vs
  `RecordingEnv` in-memory atrás da mesma `FailingEnv`) — mesmos seams de
  falha (`OpClass`, arm/trip/short-write), sem I/O do host.
- Classe de barrier escolhível no harness (`StoreOpenOptions::
  pedra_wal_full_fsync`); produto mantém a classe mais forte por padrão.
- Invariant checker no fim do run (cura a rede → pump até quiescência →
  checa autenticidade / split-brain / ressurreição sobre o changelog
  comprometido), escopado ao keyspace visível ao usuário.
- Gate de determinismo serial-vs-paralelo + testes de escala 7/9 nós.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)
- [x] **P0.1** Swarm paralelo + backend in-memory (`WorldEnv`, `mem_storage`)
  + classe de barrier fraca no harness; gate `world_swarm_parallel_matches_serial`
  (mesmo `trace_hash` por seed em 4 workers vs serial) e
  `world_swarm_throughput_scales` — status: `done`
- [x] **P0.2** Invariant checker de consistência cross-node no estado
  convergido (autenticidade, split-brain, ressurreição; escopo = keyspace
  de usuário; DCS rows isentas de ressurreição porque deletes de lease são
  locais por design) — status: `done`
- [x] **P0.3** Escala 7 e 9 nós (multi-range, buggify, invariantes on):
  `world_scale_7_9_nodes_consistency` — status: `done`
- [x] **P0.4** Bugs achados pela campanha 4096-seed, corrigidos com
  regressão pinada: CRC-32C no frame `PeerMsg` (fail-stop em decode; a
  doc prometia "CRC fail-stop at PeerMsg" e o codec não tinha), guarda
  fail-closed nos handlers de snapshot para range/nó desconhecido,
  escape-proof discard (`sent_through`), verificação de payload na
  resolução CommitUnknown (`proposed_entries`) — status: `done`
- [x] **P0.4b** Bugs achados pelas campanhas de confiança (16384@3n,
  4096@7n, 4096@9n-4ranges): InstallSnapshot stale-wipe (seed 104853 —
  snapshot com `last_included_index` < commit do follower apagava estado
  aplicado mais novo que o prefixo retido nunca re-aplica; follower agora
  rejeita snapshot estritamente mais velho que seu commit e responde
  success no próprio commit) + 2 correções do checker/oráculo (ground
  truth = união dos changelogs dos participantes, não um "best reader"
  que pode estar atrás; probe dual-claim lê chave do próprio range)
  — status: `done`
- [x] **P0.5** Bin de campanha `world_swarm` (JSONL + summary, exit 1 em
  falha) documentado no README — status: `done` (args + env knobs +
  modo diagnóstico `PEDRA_SWARM_DUMP`/`PEDRA_SWARM_TRACE`)

### P1 — next wave (depends on P0 or clearly deferrable)
- [x] **P1.1** CI job `world-parallel` (RFC-0057 P0.4): swarm + gate de
  determinismo no synthetic-field; nota documentada sobre preempção real
  do OS — status: `done` (job `world-parallel`: swarm tests, campanhas
  3n/7n/9n-4ranges, determinismo por `cmp` do JSONL das duas corridas)
- [x] **P1.2** Caixas no CI (RFC-0052 P0.2/P1.1/P1.2, RFC-0057 P1):
  `scripts/miri_dst_smoke.sh` (2 testes `pedradb-sim`, MIRIFLAGS,
  `MIRI_REQUIRED=1` no job) + jobs irmãos TSan (`PEDRA_RUN_TSAN=1`) e
  ASan (`scripts/capi-asan.sh`) — status: `done` (supply-chain
  `miri-dst-smoke`; synthetic-field `tsan-box` com `TSAN_REQUIRED=1`;
  `capi-asan-harness` pré-existente)
- [x] **P1.3** Campanhas noturnas escalonadas (16k+ seeds, 7/9 nós) como
  artifact de CI — status: `done` (workflow `world-nightly`: cron diário +
  `workflow_dispatch`, escala de referência 16384@3n / 4096@7n /
  4096@9n-4ranges com base de seed rotativa por dia (YYYYMMDD) — seeds
  frescas toda noite em vez de replay; gate = exit 1 do bin em qualquer
  falha de oráculo, nunca wall-clock; artifact `world-nightly-<base>`)

### P2 — later / polish
- [ ] **P2.1** Schedules de upgrade/rollback de membership (o "lado
  upgrade" que ainda não modelamos — nó entra/sai durante o run,
  re-configuração de quorum) — status: `todo`
- [ ] **P2.2** Checker de invariantes sobre a trajetória (não só estado
  final): monotonicidade de termo/index por nó entre exchanges — status: `todo`
- [ ] **P2.3** Referência TCG (RFC-0052 P2 — não antecipar aqui) — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Swarm paralelo + mem backend + gate determinismo | done | this change | 2026-08-24 |
| P0.2 | p0 | Invariant checker cross-node (convergido) | done | this change | 2026-08-24 |
| P0.3 | p0 | Escala 7/9 nós com invariantes | done | this change | 2026-08-24 |
| P0.4 | p0 | 3 F-found corrigidos + regressões pinadas (49/865/1093) | done | this change | 2026-08-24 |
| P0.4b | p0 | Stale-snapshot wipe (104853) + checker união/probe por range | done | this change | 2026-08-24 |
| P0.5 | p0 | Bin de campanha documentado | done | this change | 2026-08-24 |
| P1.1 | p1 | CI world-parallel | done | this change | 2026-08-24 |
| P1.2 | p1 | Miri/TSan/ASan no CI | done | this change | 2026-08-24 |
| P1.3 | p1 | Campanhas noturnas como artifact | done | workflow `world-nightly` (cron + dispatch, seed base rotativa) | 2026-08-24 |
| P2.1 | p2 | Schedules upgrade/rollback membership | todo | — | 2026-08-24 |
| P2.2 | p2 | Invariantes de trajetória | todo | — | 2026-08-24 |
| P2.3 | p2 | Referência TCG (herda 0052 P2) | todo | — | 2026-08-24 |

## Acceptance Criteria

- **Tests:** `world_swarm_parallel_matches_serial`,
  `world_swarm_throughput_scales`, `world_scale_7_9_nodes_consistency`,
  `world_regression_seed49_commit_unknown_index_reuse`,
  `world_regression_seed865_index_reuse_phantom`,
  `world_regression_seed1093_dcs_local_delete_scope`,
  `world_regression_seed104853_stale_snapshot_wipe`, suíte
  `pedradb-store` inteira (213) verde — o CRC, os handlers de snapshot e
  o guard de snapshot stale são caminhos de produto.
- **Telemetry / Analytics:** `SwarmReport.seeds_per_s` + JSONL por seed
  (`PEDRA_SWARM_LOG`); campanha de referência registrada em
  `findings/2026-08-24-world-swarm/`. Sem claim de CPU-hours vs FDB;
  machine-years = núcleos × wall-clock × orçamento, declarado como
  capacidade operacional, não como mérito comparativo.
- **Documentation:** este RFC, README (linha 0059), `docs/open-items.md`,
  `research/LEDGER.md`, flips P0.3/P0.4 no RFC-0057.
- **Screenshots:** backend-only — não se aplica.

## Bugs achados pela escala (F-found)

A campanha de 4096 seeds apanhou três classes que os oráculos por-run e
a suíte existente não pegavam (todos reproduzem igual em disco e
in-memory; regressões pinadas por seed):

1. **Frame sem CRC** (`PeerMsg`): bit-flip da rede decodificava
   `range_id` lixo e um nó participante entrava em panic no handler de
   snapshot. A doc do `InProcessNet` prometia "CRC fail-stop at PeerMsg";
   o codec não tinha. Correção: CRC-32C tail no frame (mesma família de
   todo registro durável) + guards fail-closed nos handlers de snapshot.
2. **Descarte de índice escapado** (seed 865): entrada abortada ainda em
   voo na rede era descartada em todas as réplicas e o índice reusado no
   mesmo termo — dois payloads diferentes num mesmo (index, term)
   aplicando valores divergentes (phantom no checker). Correção:
   `sent_through` por follower (máximo já posto no wire, incl. in-flight)
   e discard respeitando o escape.
3. **CommitUnknown resolvido por watermark** (seed 49): abort não-escapado
   liberava o índice; a entrada seguinte commitava nele; o `finish` do
   put original via `commit >= index` e reportava Ok com o valor apagado
   em todos os nós (false majority). Correção: `proposed_entries`
   lembra a entrada exata e o finish compara contra o log vivo.

As campanhas de confiança (16384@3n, 4096@7n, 4096@9n-4ranges; novas
faixas de seed 100000/200000/300000) apanharam mais um de produto e duas
imprecisões do próprio checker:

4. **InstallSnapshot stale-wipe** (seed 104853, reproduz em 3n): o leader
   re-eleito (após rm/add de membership + partição) enviou um
   InstallSnapshot com `last_included_index` **menor** que o commit do
   follower (export vivo vazio porque o novo leader ainda não tinha
   aplicado); o install limpou as user keys mais novas do follower e o
   prefixo de log retido nunca re-aplica (`applied` inalterado) — dado
   comprometido some com bookkeeping raft convergido (checker acusa
   ressurreição: um nó prova o delete, maioria serve o valor). Correção:
   follower rejeita snapshot estritamente mais velho que seu commit e
   responde success no próprio commit (AE continua dali); snapshot
   at-commit continua instalando (path idempotente + rollback de
   persist-fail testados). Regressão:
   `world_regression_seed104853_stale_snapshot_wipe`.
5. **Checker: ground truth de um leitor só** (seeds 103906 e outros 4):
   `changelog_after` lê o changelog do "best reader" local; se esse nó
   ficou atrás (crash/partição), valores comprometidos apareciam como
   phantoms. Correção: ground truth = união dos changelogs de todos os
   participantes; delete-último julgado por nó (seq de WAL é local, não
   comparável entre nós).
6. **Oráculo: probe dual-claim cross-range** (seeds 304064/304074,
   9n/4ranges): o probe de dual-claim lia a chave `k\x01` (do range 0)
   enquanto iterava claims de todos os ranges — Strong Ok num range são
   com claims≠1 noutro range era falsamente fail-open. Correção: probe
   por range com chave que o range realmente possui (`Strong` já falha
   fechado no range da chave).

## Out of scope

- Kernel formal Verus do group-commit (RFC-0057 P2) e o fallback
  derivado do kernel (RFC-0058 P2) — tracks próprias.
- Matriz AND/NEST/NO das caixas (RFC-0052 P1.2 normativa) — só os jobs
  irmãos no CI aqui.
- Claims comparativos contra o soak de 15 anos do FDB: o que este RFC
  declara é capacidade de exploração paralela (seeds/s, núcleos) e
  invariantes cross-node; não declara superioridade de confiabilidade.
