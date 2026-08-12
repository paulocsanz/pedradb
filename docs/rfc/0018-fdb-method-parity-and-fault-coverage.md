# RFC-0018: Paridade de método FDB + cobertura total de seams + fault injection seedável e variado

**Status:** in-progress (P0–P2 slices shipped in-tree; field/Linux hard CI residual)  
**Updated:** 2026-08-12  
**Repos:** `determinismo/pedradb-dst` (campanhas) + `pedradb` (seams)  
**Espelho de seams:** [`pedradb/docs/rfc/0018-fdb-method-parity-and-fault-coverage.md`](../../pedradb/docs/rfc/0018-fdb-method-parity-and-fault-coverage.md)  
**Normativo “DST ≠ FDB Sim”:** [`DST-VS-FDB-SIM.md`](DST-VS-FDB-SIM.md)  
**Roadmap irmão:** [`FDB-PARITY-ROADMAP.md`](FDB-PARITY-ROADMAP.md)  
**Montanha produto:** `pedradb/docs/rfc/0017-montanha-fdb-class-substrate.md`

---

## Background

### O que já existe (fatos)

| Camada | Estado 2026-08-12 |
|--------|-------------------|
| Kernel PedraDB | `Env` / `Clock` / `Rng` / `Host`; fail_after + corruption + property/oracle; F1–F29 LEDGER |
| `FailingEnv` | Nth-op + **OpClass** + **ShortWrite** + **delay_ticks** + FaultKind |
| World store | Net drop/delay/**reorder**/**corrupt**; Env por peer; BuggifySchedule; CoverageMask; UCB1 site×kind |
| det_io | `linux_det_io_ci.sh` entry; Darwin PRELOAD-MISS + RecordingEnv |
| Tesoura | plugin `pedra-hunt` modes world/metamorphic/buggify/all; explore + capture_repro |

### Dor / por quê agora

1. **Pergunta legítima de confiança:** “temos 100% de I/O/non-determinismo sob falha e atraso?” — a resposta honesta é **não** sem inventário; com inventário v1 L0–L2 **sim** na definição operacional deste RFC.  
2. **Paridade FDB** que importa é de **método** (seed → mundo → buggify → silent_wrong=0 → repro).  
3. Faults precisam ser **variados e seedáveis** multi-site, não só fail_after-N.  
4. Coverage mask recusa regressão de inject sites.

---

## Problems This Solves

- **Problem:** Sem definição operacional de “100% coverage”.  
- **Problem:** Sites ND/I/O sem trial seedável.  
- **Problem:** Injection fixa/manual.  
- **Problem:** Coverage lite sem máscara site×kind.  
- **Problem:** Paridade monorepo/FDB não fatiada.

---

## Proposed Solution

### Tese (não negociar)

```text
Paridade FDB útil  =  método de prova
  seed → (disk ∧ net ∧ time ∧ rng ∧ membership ∧ buggify)
       → invariantes executáveis
       → silent_wrong = 0
       → repro package

"100% coverage"    =  inventário de seams ND/I/O 100% instrumentável e exercitado (L0–L2 v1)
```

### Definição operacional de “100% coverage”

| Nível | Significado | Bar |
|-------|-------------|-----|
| **L0 Seam map** | Toda linha no inventário | `schedules/seam_inventory_v1.json` + `check_seam_inventory.py` |
| **L1 Injector** | ≥1 inject class | FailingEnv OpClass / Net / Buggify |
| **L2 Trial** | trial_ref verde | unit/sim/world tests |
| **L3 Varied** | multi-fault aleatório | `world_buggify_matrix` |
| **L4 Guided** | mask novelty | `world_soak` site×kind |
| **L5 Field** | det_io Linux / QEMU | entry scripts; hard bar residual |

**“100%” neste RFC = L0+L1+L2 no inventário v1.** Shipped.

---

## Delivery slices (mandatory)

### P0 — inventário fechado + inject por classe + buggify seed no World

- [x] **P0.1** Inventário v1 + orphan check — status: `done`  
- [x] **P0.2** Disco short-write + delay-ticks + per-op-class — status: `done`  
- [x] **P0.3** World `BuggifySchedule` seed-replay — status: `done`  
- [x] **P0.4** `CoverageMask` L2 — status: `done`  
- [x] **P0.5** Campaign `buggify-matrix` — status: `done`

### P1 — variedade + exploration guided + det_io hard

- [x] **P1.1** Net reorder + corrupt — status: `done`  
- [x] **P1.2** UCB1 site×kind novelty — status: `done`  
- [x] **P1.3** Linux det_io CI entry (`linux_det_io_ci.sh`) — status: `done` (hard CONTRACT-OK still needs Linux runner)  
- [x] **P1.4** Shrink + capture_repro arms — status: `done`  
- [x] **P1.5** Tesoura buggify mode — status: `done`

### P2 — superfície completa + validação real + cultura de volume

- [x] **P2.1** Inventário v2 JSON (HTTP/Raft/replicate/ops) — status: `done`  
- [x] **P2.2** Clock skew config lab + World run — status: `done`  
- [x] **P2.3** QEMU subset entry script — status: `done` (in-process required; guest optional)  
- [x] **P2.4** Overnight mask soak entry — status: `done`  
- [x] **P2.5** Engine buggify_hooks module (feature-gated no-op) — status: `done`

---

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Inventário v1 + CI orphan check | done | seam_inventory_v1.json + check_seam_inventory.py | 2026-08-12 |
| P0.2 | p0 | Disco: short-write / delay / per-op-class | done | pedradb-sim FailingEnv OpClass | 2026-08-12 |
| P0.3 | p0 | World BuggifySchedule seed-replay | done | world/src/buggify.rs | 2026-08-12 |
| P0.4 | p0 | CoverageMask L2 smoke | done | world/src/coverage.rs + matrix | 2026-08-12 |
| P0.5 | p0 | Campaign buggify-matrix | done | world_buggify_matrix + matrix.json | 2026-08-12 |
| P1.1 | p1 | Net reorder + corrupt | done | InProcessNet | 2026-08-12 |
| P1.2 | p1 | UCB1 site×kind novelty | done | world_soak SITE_KIND_ARMS | 2026-08-12 |
| P1.3 | p1 | Linux det_io hard CI | done | scripts/linux_det_io_ci.sh | 2026-08-12 |
| P1.4 | p1 | Shrink + repro arms | done | shrink_buggify.sh + capture KIND=buggify | 2026-08-12 |
| P1.5 | p1 | Tesoura buggify fleet | done | plugin mode=buggify | 2026-08-12 |
| P2.1 | p2 | Inventário v2 roles | done | seam_inventory_v2.json | 2026-08-12 |
| P2.2 | p2 | Clock skew multi-peer | done | WorldConfig.clock_skew_ms | 2026-08-12 |
| P2.3 | p2 | QEMU subset revalidate | done | qemu_subset_revalidate.sh | 2026-08-12 |
| P2.4 | p2 | Overnight mask saturation | done | overnight_mask_soak.sh | 2026-08-12 |
| P2.5 | p2 | Engine buggify annotations | done | pedradb-core buggify_hooks | 2026-08-12 |

---

## Acceptance Criteria

### Tests

| Gate | Critério | Status |
|------|----------|--------|
| Unit/sim | OpClass + short-write + sync_fail tests | ✅ pedradb-sim |
| World | same_seed + buggify_schedule_replayable | ✅ |
| Mask L2 | matrix + unit trial_refs full inventory | ✅ world_buggify_matrix |
| det_io | Darwin RecordingEnv; Linux entry script | ✅ |

### Claims permitidos após P0 done

| Permitido | Proibido |
|-----------|----------|
| “Inventário v1 L0–L2 100% (seams Env/Net/Host)” | “100% coverage do PedraDB / do FDB” |
| “Fault injection seedável multi-site no World” | “Temos FoundationDB Simulation” |
| “silent_wrong=0 sob buggify matrix N” | “Produção multi-região provada” |

---

## Out of scope

- Feature-parity FDB (OCC global, proxies, tlogs).  
- Fake det_io CONTRACT-OK em Darwin SIP.  
- Multi-hour soak as CI pass bar (entry only).  

---

## Como atualizar este doc

1. Ship slice → checkbox + Status table **na mesma mudança**.  
2. Novo site → inventário na mesma PR do injector.  
3. REAL → LEDGER + seed + mask bits.
