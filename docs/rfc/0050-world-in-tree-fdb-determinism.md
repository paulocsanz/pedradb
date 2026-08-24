# RFC-0050: World in-tree — determinismo FDB-shaped sem Flow

**Status:** done (P0+P1+P2 entregues; L28 segue `MEASURE` até repro de cluster REAL)  
**Updated:** 2026-08-23  
**Parent:** [RFC-0018](0018-fdb-method-parity-and-fault-coverage.md) (método), [RFC-0020](0020-synthetic-field-maturity.md) (volume / canaries)  
**Cluster substrate:** [RFC-0017](0017-montanha-fdb-class-substrate.md)  
**Seams:** [`../dst-seams.md`](../dst-seams.md)  
**Normativo “DST ≠ FDB Sim”:** `../determinismo/pedradb-dst/DST-VS-FDB-SIM.md`  
**Ledger de paper:** L28 `MEASURE` (swarm/buggify); L29/L30 `REFUSE` (não reabrir)  
**Fonte a trazer:** `../determinismo/pedradb-dst/world/` (~3.5k LOC)  
**Complemento (buracos publicados do Sim2):** [RFC-0051](0051-beyond-fdb-sim-holes.md) — π / ConcurrentDb / G2, **não** este RFC

---

## Background

### Facts (2026-08-23)

| Camada | Onde vive hoje | O que prova |
|--------|----------------|-------------|
| Seams `Env` / `Clock` / `Rng` / `Host` | in-tree | disco/tempo/entropia injectáveis |
| `FailingEnv` / `RecordingEnv` | `pedradb-sim` | fail_after, OpClass, short-write, lying fsync |
| Kernel DST | `pedradb-dst` + CI `silent_wrong_gate` | 32 seeds, um `Db`, `silent_wrong=0` |
| Cluster World (Net ∧ Env-por-peer ∧ clock ∧ membership ∧ buggify ∧ `trace_hash`) | **`crates/pedradb-world`** (in-tree desde P0; origem `../determinismo/pedradb-dst/world/`) | seed → mundo de 3 peers |
| Tesoura / det_io / QEMU | `../determinismo` | envelope multi-alvo; Linux hard residual |
| Produção TCP | `montanha-tcp` (`thread::spawn` + wall-clock) | path distinto de `RpcMode::Queued` |
| Layers canary | `pedradb-index` / journal / SQL | **hardcoded `Db<StdEnv>`** |
| Buggify no engine | `buggify_hooks` (8 sites, feature-gated) | `maybe_arm`; não delay/erro/knob |
| PCT (5º seam) | `world/src/scheduler.rs` via `dst_core` | **não altera** `World::run` |

RFC-0018/0020 **já shipparam** paridade de *método* no lab (inventário L0–L2, World, canaries, soak de ops). O World **não corre** no `cargo test` / CI deste repo a menos que o checkout irmão exista — e mesmo aí os scripts in-tree tratam o sibling como best-effort.

`docs/dst-seams.md` ainda lista “Network / RPC … Not a trait yet”. Isso está **obsoleto**: `InProcessNet` + `PeerMsg` + `RpcMode::Queued` existem no World fora da árvore.

### Pain / why now

1. O nível FDB **não** é ter seams. É o **binário de produção** ser um mundo discreto: um seed reproduz disco ∧ rede ∧ tempo ∧ RNG ∧ falhas, sem um segundo protocolo para o lab.  
2. A fronteira RFC-0020 (“campanhas no determinismo”) deixou o **runtime** do cluster fora do produto. `cargo test` prova LSM, não 3 peers + Net.  
3. Dual path (`Direct` vs `Queued`, POSIX vs io_uring, layers em `StdEnv`) é a classe de bug que o FDB matou reescrevendo o que não cabia no sim.  
4. L28 continua `MEASURE`: não há swarm in-tree cujo gate seja “bug de cluster reproduzido por seed”.

### O que este RFC **não** é

| Claim inflado | Escopo honesto |
|---------------|----------------|
| “Temos FoundationDB Simulation” | World in-tree + um path de RPC + π no `World::run` |
| Reescrever em Flow | **Recusado** (L29; ficha R043) |
| Clonar Sequencer/Proxy/Resolver | **L29 `REFUSE`** |
| Janela MVCC 5 s no kernel | **L30 `REFUSE`** |
| Fundir tesoura / det_io C / QEMU no core | **Continua fora** (RFC-0020) |
| CPU-years Apple | volume de *método*; field continua residual |

---

## Problems This Solves

- **Problem:** CI / `cargo test` deste repo não reproduzem um mundo de cluster por seed.  
- **Problem:** O runtime FDB-shaped vive noutro git; um PR no pedradb não vê o `trace_hash` partir.  
- **Problem:** Dois protocolos de RPC (Direct vs Queued) e layers presos a `StdEnv` — o sim não é o produto.  
- **Problem:** Buggify e PCT existem como ganchos mortos relativamente a `World::run`.  
- **Problem:** Doutrina de seams desactualizada (“Net ainda não é trait”).

---

## Proposed Solution

Emendar a fronteira RFC-0020:

```text
In-tree:  seams + World runtime + inventário + CI de seed→trace_hash
Sibling:  tesoura, det_io LD_PRELOAD, QEMU/TCG, hunts multi-alvo
```

Tese (não negociar):

```text
Determinismo FDB-shaped (Pedra) =
    o mesmo PeerMsg / Env / Clock / Rng
  ∧ um seed → (disk ∧ net ∧ time ∧ rng ∧ membership ∧ π)
  ∧ invariantes executáveis (silent_wrong=0, dual_leader_fail_open=0)
  ∧ repro no checkout do pedradb, sem ../determinismo

Não é Flow. Não é o unbundled TS. Não é field Apple.
```

P0 traz o World. P1 fecha o dual path o suficiente para o lab ser o mesmo codec/Env que o produto. P2 liga π e swarm (L28).

---

## Delivery slices (mandatory)

### P0 — World no workspace (shippable sozinho)

O checkout do pedradb, **sem** `../determinismo`, prova seed → `trace_hash` estável e `silent_wrong=0` num cluster de 3 peers.

- [x] **P0.1** Crate `pedradb-world` no workspace, copiado de `determinismo/pedradb-dst/world/` **sem** dep `dst_core` (módulo PCT fica stub / `cfg` off até P2.1) — status: `done` (copiado de `../determinismo/pedradb-dst/world/` @ commit do sibling em **2026-08-23**; `dst_core` substituído por `src/pct.rs` in-tree — PCT mínimo com semântica idêntica à do sibling, permitido pelo escopo; `cargo build/test -p pedradb-world` verde (30/30); `grep dst_core|determinismo|\.\./\.\.` no crate ⇒ vazio)  
- [x] **P0.2** Mesmo seed ⇒ mesmo `trace_hash`; smoke `silent_wrong=0` e `dual_leader_fail_open=0` como testes do crate — status: `done` (`world_seed_replayable` + banda `world_fixed_smoke_seeds_invariants` seeds 0..=7: silent_wrong=0, dual_leader_fail_open=0, false_majority=0, hashes distintos; `world_smoke 0xC0FFEE` ×2 ⇒ hash `462569ab70dc15cc`, seed≠ ⇒ hash≠)
- [x] **P0.3** Job CI in-tree (`synthetic-field` ou equivalente) corre `world_smoke` / testes P0.2; verde com o sibling **ausente** — status: `done` (job `world-determinism` no `synthetic-field.yml`: `cargo test -p pedradb-world --lib -- --test-threads=1` + launch `world_smoke` com asserção hash; prova local com `../determinismo` renomeado ⇒ 31/31 + mesmo hash; log scratch `world_intree.log`)  
- [x] **P0.4** Inventário de seams v1 JSON + check script **neste** repo; CI recusa site órfão — status: `done` (`scripts/seam_inventory_v1.json` portado do sibling @ 2026-08-23, 15 sites, `trial_ref`s re-pointed para tests in-tree (C.tick/H.open/W.crash/D.bitrot eram do harness sibling); `scripts/check_seam_inventory.py`: exit 0, 15/15 L2, ids ≡ `SEAM_IDS`; matriz negativa 4/4 rejeita (órfão, trial_ref insolúvel, id extra, site faltante); passo CI no job `world-determinism`)  
- [x] **P0.5** Doutrina: actualizar [`../dst-seams.md`](../dst-seams.md) (Net existe; World in-tree; tesoura/det_io continuam sibling), README, open-items; RFC-0020 regra 1 emendada — status: `done` (`dst-seams.md` Updated/intro/harness/gaps sem “ainda sibling”; README + `open-items.md` P0 done; RFC-0020 linha de contexto + baseline row: World in-tree `crates/pedradb-world`, campanhas continuam sibling)

### P1 — o lab é o mesmo codec / Env que o produto

- [x] **P1.1** `RpcMode::Queued` é o caminho canónico; `Direct` é pump síncrono do **mesmo** `PeerMsg` (não um segundo protocolo) — status: `done` (teste `direct_pump_and_queued_share_peer_msg_semantics`: mesmo seed ⇒ mesmo líder/topologia e mesmo estado aplicado nos dois modos; bytes drenados no Queued decodificam/re-codificam byte-estáveis como `PeerMsg` — o Direct despacha exactamente essa mensagem; `send_peer_rpc` Direct = `dispatch_peer_msg` do mesmo tipo, sem segundo codec)  
- [x] **P1.2** Canaries `pedradb-index` e `pedradb-journal` genéricos em `E: Env`; um trial World abre-os sob `FailingEnv` — status: `done` (`crates/pedradb-world/tests/layer_canary.rs`: trial World seed 0xCA110002 `silent_wrong=0` + canary journal (pin watermark) e canary index (`row_fully_indexed`, nunca half) ambas abertas com `FailingEnv::set_delay_per_op(1)` — sem ilha `StdEnv`)  
- [x] **P1.3** Feature `buggify`: os 8 sites injectam delay ou `io::Error` seedável (não só `maybe_arm` no-op); matrix in-tree dispara ≥1 site — status: `done` (`Injection::{None,DelayMs,IoErrorKind}` seedável em `buggify_hooks::inject_checked`; 8/8 sites wired (`AFTER_OPEN_LOCK`, `BEFORE_SST_RENAME`, `BEFORE_MANIFEST_RENAME`, `AFTER_MEM_INSERT`, `AFTER_WAL_APPEND`, `BEFORE_COMPACT_WRITE`, `BEFORE_VLOG_APPEND` no core; `BEFORE_RAFT_APPLY` no store apply); `engine_matrix_fires_and_survives_fixed_seed` seed 0xB175F1ED: fires>0, ≥1 injeção fail-stop, reopen limpo; default sem feature continua no-op (4/4))  
- [x] **P1.4** LEDGER de REALs do **engine/cluster** em `crates/pedradb-dst/findings/LEDGER.md` (processo; não dump dos 213 F-ids); World CI que achar silent_wrong escreve aqui na mesma mudança — status: `done` (LEDGER in-tree com processo + F47/F49 existentes + regra same-change para REALs do World CI)  
- [x] **P1.5** CI de cluster: World in-tree é autoridade; sibling determinismo passa a opcional e não mascara falha in-tree — status: `done` (job `world-determinism` renomeado P0+P1 com steps: lib tests, launch hash, seam inventory, PeerMsg/canaries, buggify; **nenhum** passo do workflow referencia o sibling nem usa WARN/`continue-on-error` — falha in-tree falha o CI)

### P2 — π, swarm, um papel extra

- [x] **P2.1** PCT ordena `World::run` (5º seam); mesmo seed ⇒ mesma ordem de ready-queue **e** mesmo `trace_hash` — status: `done` (`WorldConfig::node_step_pct`: cada rodada de exchange processa os inbound agrupados por nó-destino na ordem da ready-queue PCT seedada (`NodeOrder::Pct` sobre `pct_ready_queue`); default `Arrival` mantém hashes byte-idênticos; `trace_hash` ganha sufixo `order=pct|q={pct_ready_queue_hash}` só em modo pct. Teste `pct_orders_world_run`: fila pura em (seed), nenhum nó starved, replay ×2 mesmo hash, ≥1 seed em 0..8 reordenado de fato, contadores de segurança limpos)  
- [x] **P2.2** Swarm L28: `n_nodes ∈ {3,5}` × máscara buggify × grammar de workload; gate = cluster REAL reproduzido por `world_smoke --seed S` — status: `done` (teste `swarm_l28_mask_matrix`: 2×3×4 = 24 runs com máscaras `0xAAAA…`/`0x5555…`/`0x0F0F` (≠ all), `silent_wrong=0`, `dual_leader_fail_open=0`, `false_majority=0`, `row_half_indexed=0` em todas; gate documentado no teste: achado aqui é candidato — REAL só com repro de cluster via `world_smoke --seed S`; L28 segue `MEASURE`)  
- [x] **P2.3** Um papel extra no mesmo seed (HTTP **ou** fold, não ambos no mesmo slice) — status: `done` (fold: `WorldConfig::fold_role` abre um `PedraFold` Storage em `FailingEnv` e consome `cluster.changelog_after(0)`; oráculo = replay independente das mesmas changes (keyset+valores ⇒ `fold_mismatch==0`) e cursor = última seq; teste `fold_role_extra_on_same_seed` com buggify também limpo; `trace_hash` ganha sufixo `fold|cur|mis` só com o papel ligado)  
- [x] **P2.4** Job Ubuntu: det_io hard `CONTRACT-OK` **quando** o sibling está no checkout (não vendorar `.so`); residual explícito se ausente — status: `done` (step "det_io hard CONTRACT-OK when sibling checkout present" chama `scripts/det_io_status.sh`: sibling presente ⇒ roda `linux_det_io_ci.sh` hard (exit 1 propaga); ausente ⇒ imprime `residual_no_determinismo_tree` e passa; `.so` não vendorado)

---

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | `pedradb-world` no workspace sem `dst_core` | done | crates/pedradb-world + src/pct.rs; 30/30 tests; world_smoke seed→hash estável | 2026-08-23 |
| P0.2 | p0 | seed → `trace_hash` + silent_wrong=0 in-tree | done | 31/31 lib tests; seeds 0..=7 invariants=0; smoke hash estável | 2026-08-23 |
| P0.3 | p0 | CI World verde sem sibling | done | job world-determinism; prova com sibling renomeado | 2026-08-23 |
| P0.4 | p0 | Inventário seams + orphan check in-tree | done | scripts/seam_inventory_v1.json + check_seam_inventory.py (15/15 L2, negativa 4/4); CI world-determinism | 2026-08-23 |
| P0.5 | p0 | dst-seams / RFC-0020 / README alinhados | done | dst-seams.md + README + open-items + RFC-0020 (regra 1 vigente: World in-tree) | 2026-08-23 |
| P1.1 | p1 | Queued canónico; Direct = pump do mesmo codec | done | direct_pump_and_queued_share_peer_msg_semantics (mesmo líder/apply, codec estável) | 2026-08-23 |
| P1.2 | p1 | index + journal genéricos em `Env` no World | done | world tests/layer_canary.rs sob FailingEnv | 2026-08-23 |
| P1.3 | p1 | Buggify injecta delay/erro seedável | done | Injection API + 8/8 sites; matrix seed 0xB175F1ED; default no-op | 2026-08-23 |
| P1.4 | p1 | LEDGER in-tree para REALs World/engine | done | crates/pedradb-dst/findings/LEDGER.md (F47/F49 + processo) | 2026-08-23 |
| P1.5 | p1 | World in-tree autoridade de CI cluster | done | job world-determinism P0+P1; workflow sem refs sibling/WARN | 2026-08-23 |
| P2.1 | p2 | PCT conduz `World::run` | done | `node_step_pct` + `pct_orders_world_run` (5º seam) | 2026-08-23 |
| P2.2 | p2 | Swarm L28 (n, máscara, grammar) | done | `swarm_l28_mask_matrix` (24 runs clean; gate = repro cluster) | 2026-08-23 |
| P2.3 | p2 | HTTP ou fold no mesmo seed | done | `fold_role` + `fold_role_extra_on_same_seed` | 2026-08-23 |
| P2.4 | p2 | det_io Ubuntu hard se sibling presente | done | step CI `det_io_status.sh` (residual explícito se ausente) | 2026-08-23 |

---

## Acceptance Criteria

### Tests

**P0**

- `cargo test -p pedradb-world` no checkout **sem** `../determinismo`: mesmo seed, dois runs, `trace_hash` igual.  
- Seeds fixos (ex. `0..=7` ou o smoke actual): `silent_wrong=0`, `dual_leader_fail_open=0`, `false_majority=0`.  
- `scripts/check_seam_inventory.py` (ou equivalente in-tree) exit 0; site novo sem `trial_ref` falha CI.  
- Workflow `synthetic-field` (job World) verde em PR; não depende de path absoluto do sibling.

**P1**

- Store: um teste que envia o **mesmo** `PeerMsg` bytes por Direct-pump e por Queued+Net e compara apply/commit.  
- Index TX crash (W3) e journal pin (W4) passam com `FailingEnv` (não só `StdEnv`).  
- `cargo test -p pedradb-core --features buggify`: ≥1 site dispara sob seed fixo; default (sem feature) continua no-op.  
- Falha de `pedradb-world` no CI **não** é mascarada por `WARN: determinismo gate exited`.

**P2**

- Mesmo seed, PCT on: `trace_hash` estável; seed diferente ⇒ hash diferente (já exigido no P0, agora com π no run).  
- Swarm: pelo menos uma combinação `(n_nodes=5, buggify mask ≠ all)` no CI nightly/entry; L28 fecha só quando um REAL de cluster tiver repro `seed S` in-tree.  
- det_io: Ubuntu + sibling ⇒ `CONTRACT-OK` ou job vermelho; sem sibling ⇒ residual nomeado, exit 0 (não verde falso de fsync).

### Telemetry / Analytics

- Artefactos: `trace_hash`, `silent_wrong`, `dual_leader_fail_open`, CoverageMask (já no World).  
- Nenhum produto UX — infra de correção.  
- Soak CPU-years **não** é métrica deste RFC (RFC-0020 volume de ops permanece).

### Documentation

- Esta tabela de Status actualiza-se **na mesma mudança** que o código.  
- [`../dst-seams.md`](../dst-seams.md) deixa de dizer que Net não existe; aponta World in-tree vs tesoura sibling.  
- RFC-0020 regra 1: World runtime in-tree; campanhas tesoura/det_io/QEMU continuam no determinismo.  
- Claims: ver tabela abaixo. Linkar `DST-VS-FDB-SIM.md` se alguém escrever “simulation” sem nuance.

### Screenshots

- backend-only (`trace_hash` / JSON de smoke).

### Claims permitidos após cada banda

| Depois de | Permitido | Proibido |
|-----------|-----------|----------|
| P0 done | “World cluster seedável corre neste repo” | “Temos Simulation do FDB” |
| P1 done | “Lab e produto partilham PeerMsg + Env nas canaries index/journal” | “montanha-tcp é single-thread Flow” |
| P2 done | “Método FDB com π + swarm L28 in-tree” | “Paridade de CPU-hours / field Apple” |

---

## Out of scope

- Feature-parity FDB (OCC global, proxies, tlogs, janela 5 s).  
- Reescrita em Flow / actor runtime novo.  
- Trazer tesoura, `determinism-hooks` C, QEMU images, `fdb-dst` hunt, adversarial-kb.  
- Vendorar `dst_core` no P0 (P2.1 pode copiar PCT mínimo, não o envelope multi-alvo).  
- Fake `CONTRACT-OK` det_io em Darwin SIP.  
- `IoUringEnv` como path canónico do World (POSIX `FailingEnv` fica; io_uring continua ilha Linux).  
- Fechar C2.4 field deploy.  
- Reabrir L29 / L30.  
- Ultrapassar os buracos G1–G8 do Sim2 (threads, maybe-committed, io_uring bg) — [RFC-0051](0051-beyond-fdb-sim-holes.md).  
- DST interpretado (Miri / ASan / TCG) — [RFC-0052](0052-dst-inside-boxes.md).

---

## Relação com RFCs / ledgers vivos

| Doc | Papel depois deste RFC |
|-----|------------------------|
| RFC-0018 | Método (inventário, buggify, mask) **permanece**; runtime World deixa de ser só sibling |
| RFC-0020 | Volume + canaries **permanece**; regra 1 emendada (World in-tree) |
| RFC-0017 | Substrate de cluster; este RFC é o sim desse substrate, não o TCP de produção |
| L28 | Continua `MEASURE` até P2.2; este RFC é o programa |
| L29 / L30 | `REFUSE` intacto |
| RFC-0051 | Complemento: classes que o Sim2 **declara** fora (π, layer 1021, compact bg) |
| `DST-VS-FDB-SIM.md` | Continua normativo para claims; P0 não autoriza “FDB Simulation” |

---

## Como actualizar este doc

1. Ship slice → checkbox + linha da tabela Status **na mesma mudança**.  
2. Trazer ficheiros do sibling → P0.1 lista o path de origem e a data do copy.  
3. REAL encontrado pelo World in-tree → P1.4 LEDGER + seed de regressão na mesma janela.  
4. Não marcar “paridade FDB” sem a tabela de claims desta página.
