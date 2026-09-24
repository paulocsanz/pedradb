# RFC-0058: Modo verificado — fallback seguro derivado dos kernels

**Status:** P0+P1+P2 done  
**Updated:** 2026-08-24

## Em uma frase

Um perfil de produto **verificado**: desliga o que não tem kernel provado
(io_uring ring — gate documentado, P2.2), liga o que tem (WAL sync, recovery
provado, decisões provadas — e, desde o P2.1, o group-commit pelo kernel
provado do 0057 P2.1), e o caminho resultante é a
**composição declarada dos kernels** — com teste de derivação (mesmo oráculo
DST, mesma semântica) e residual publicado. O fallback deixa de ser um
acidente de plataforma (`PosixFallback` quando o kernel rejeita o ring) e
vira o **default declarado e seguro** — e uma linha de configuração
(`PEDRA_VERIFIED=1`, P2.3).

## A tese e os seus limites (honestidade primeiro)

- **"100% safe gerado de formalização"** aqui significa: 100% das seções
  críticas do modo verificado são kernels provados (Verus + Aeneas→Lean) e a
  cola é verificada por DST com oráculo independente. **Não** significa
  extração de código do teorema para o engine inteiro — isso foi medido pela
  literatura (VeriBetrKV: Dafny→C++, 8× mais lento que Rocks) e **recusado**
  neste repo (LEDGER L46). O que a extração total ganha em garantia, o método
  kernel+DST ganha de volta em escala: provamos as decisões e as seções
  críticas, executamos a composição sob falha.
- **Residual que permanece**: o OS mentir no fsync (det_io/TCG — 0052),
  hardware, campo. `DST-VS-FDB-SIM.md` continua normativo; nada aqui autoriza
  "mais confiável que FDB" ou "sem bugs".
- **O que a era das IAs muda de verdade**: o custo marginal do **próximo
  kernel** caiu de meses para dias (pipeline Aeneas→Lean + caller refinements
  do 00553). O gargalo que não caiu: revisão de prova e disciplina de claims.
  Este RFC é barato *porque* 0053/0056/0057 já pagaram o pipeline.

## Fatos que sustentam o design (verificados 2026-08-23)

1. O bench oficial de paridade usa `ConcurrentDb<StdEnv>` — o piso **2×**
   (Agents.md) é medido no path POSIX. Modo verificado sobre `StdEnv` não
   sacrifica o piso por construção.
2. `IoUringEnv` já tem fallback transparente (`PosixFallback`,
   `backend()` reporta); mas é **default de tipo** em `pedradb-fold`,
   `pedradb-lease`, `pedradb-dcs` — em Linux o ring liga sozinho. O ring é a
   única exceção da allowlist do freeze (`cqe_kernel.rs`; twin bloqueado num
   modelo de ring).
3. Kernels existentes cobrem: wal recover, reopen, compact/flush/manifest
   decision, vlog GC, vote/AE/apply, commit/Ae. Falta: group-commit (0057
   P2.1) e a **composição** como perfil.

## Problems This Solves

- **Problem:** robustez de produção hoje depende de features sem teorema
  (ring io_uring, fusão de grupo) ligadas por default de tipo, não por
  decisão.
- **Problem:** "fallback seguro" existe mas é implícito e não-verificado
  como *modo* — ninguém roda a suíte de oráculos nele de propósito.
- **Problem:** não há uma resposta de produto para "e se o caminho exótico
  falhar?" que seja uma linha de configuração com contrato.

## Proposed Solution

1. **`VerifiedProfile`** (core): builder que fixa a composição — `StdEnv`,
   `sync=true`, `wal_full_fsync=true`, commit **lone** (single-writer
   critical section) até o kernel do group-commit existir; reporta por
   componente o que está ON/OFF e o kernel responsável.
2. **Construtores `open_verified`** em fold/lease/dcs (pinam `StdEnv`, sem
   ring), para o produto inteiro rodar no modo.
3. **Teste de derivação**: mesmo seed/schedule, modo verificado e modo
   completo ⇒ oráculos de segurança idênticos (`silent_wrong=0`,
   `row_half_indexed=0`, reopen exato). A derivação é da *semântica*, não do
   código.
4. **Piso re-medido no modo verificado** (P1.2): a expectativa é ≥2× por
   construção (bench já é StdEnv+sync); se uma shape oficial falhar, o número
   é publicado, não escondido.
5. **Group-commit volta ao modo verificado** quando 0057 P2.1 provar a
   atomicidade de grupo + first-committer-wins (P2.1 daqui).

## Delivery slices (mandatory)

### P0 — o perfil existe e a suíte roda nele

- [x] **P0.1** `pedradb_core::VerifiedProfile` + `OpenOptions::verified()`:
   fixa `sync`, `wal_full_fsync`, lone-commit-only (`write_group` desativado
   ou `catchup_window=0` com força-lone), env `StdEnv`; `profile_report()`
   lista componente⇒estado⇒kernel — status: `done`
   (`crates/pedradb-core/src/verified_kernel.rs`: `PROFILE_VERSION = "verified-v1"`,
   report com os 44 kernels ON + 4 OFF + 2 contratos; `OpenOptions::verified()`
   fixa `sync=true`, `wal_full_fsync=true`, `WalRecovery::FailClosed`;
   lone-only é um pin de runtime **de-uma-via-só** no `WriteGroup`
   (`lone_only`, precedente dos knobs `catchup_window`/`PEDRA_*`), aplicado
   junto pelas construtoras `ConcurrentDb::open_verified` /
   `VerifiedProfile::open{,_with_env}` — decisão registrada em Deviations;
   `verified_report_matches_catalog` prova report⇔catalog idênticos (44/44)
   e `verified_profile_forces_safe_composition` prova `queued == 0`,
   `batches == submits` e reopen 100/100 sob 4 threads × barrier)  
- [x] **P0.2** Suíte DST no perfil: bateria `FailingEnv` (crash/reopen/EIO)
   e World em modo verificado, oráculos idênticos (`silent_wrong=0`,
   reopen exato) — status: `done` (sim: `verified_crash_after_sync_survives`,
   `verified_truncated_tail_loses_unsynced_suffix`,
   `verified_nth_put_eio_reopen_keeps_prefix`,
   `verified_sync_fail_fences_fail_closed`,
   `verified_multi_key_tx_crash_no_half` — 5/5; world:
   `verified_world_run_oracles` (seeds `0x0058_0001/2` sob buggify:
   `silent_wrong=0`, `fold_mismatch=0`, `row_half_indexed=0`) e
   `pct_verified_lone_never_merges` (64 seeds × {Sequential, PCT d=2} com
   os **três shapes de escrita** — sync put, OCC tx, async `no_sync`:
   `queued==0` sempre, fence EIO **≤ 1 escritor** — no modo completo o mesmo
   dente cerca 2+ membros; a forma está estruturalmente ausente aqui — todo
   Ok sync/OCC sobrevive ao reopen e sobrevivente async nunca tem valor
   errado; async Ok não promete durabilidade antes de barreira, oráculo
   honesto de crash); **metade async do pin**:
   `verified_async_close_and_barrier_reopen` (close drena o tail; barreira
   `sync()` explícita + kill de processo mantém o que passou pela barreira)
   e `verified_async_concurrent_never_merges` (4 threads × mistura
   sync/no_sync: `queued==0`, reopen 100/100); nós do World via
   `StoreOpenOptions::pedra_verified` + `WorldConfig.verified`)  
- [x] **P0.3** Docs ritual: claims table do modo (pode: "seções críticas do
   modo verificado são kernels provados + DST"; não pode: "imune a bugs",
   "mais seguro que FDB"), README, open-items, LEDGER — status: `done`
   (claims table abaixo; README/open-items/LEDGER L46 na mesma mudança)

### P1 — o produto inteiro no modo

- [x] **P1.1** `open_verified` em `pedradb-fold` / `pedradb-lease` /
   `pedradb-dcs` (StdEnv pinado por **tipo** — `PedraFold<StdEnv>`,
   `LeaseStore<StdEnv>`, `Dcs<C, StdEnv>`; não existe `backend()` no
   `StdEnv`, a asserção de plano-B é o próprio tipo) + `DB::open_verified`
   no `rocksdb-compat` (força `OpenOptions::verified()` + pin; knobs de
   performance do caller intactos) — status: `done`
   (`open_verified_pins_std_env` ×3 + `open_verified_pins_lone_profile`:
   CAS/apply/put sob o pin com `queued==0`, `batches==submits`)  
- [x] **P1.2** Piso no modo verificado: rodar `rocks-parity-compare` nas
  shapes oficiais com o perfil; expectativa `compat_qps/rocks_default_qps
  ≥ 2.0` (regra Agents.md); resultado publicado em tabela no RFC — status:
  `done (medido; a expectativa 2× **não** se sustenta no perfil — ver
  tabela e Deviation)`  
  Run `findings/2026-08-24-verified-parity/` (macOS, load 11–35, protocolo
  do audit oficial: records=1024 ops=30000 payload=1000 zipfian clients=4,
  peer `ROCKS_PARITY_SYNC=0`; compatv 85 min fsync-bound vs rocks 39 min):

  | shape | compatv/rocks_default | | shape | ratio |
  |---|---|---|---|---|
  | ycsb_c (100% read) | **5,24×** | | deps_apply_batch | 0,009× |
  | deps_mvcc_latest | **5,17×** | | deps_raftlog | 0,003× |
  | deps_scan | **9,09×** | | deps_lock_prewrite | 0,013× |
  | ycsb_e (95% scan) | 0,019× | | ycsb_a/f (writes) | ~0,001× |
  | ycsb_b/d (5% writes) | 0,001× | | mc4 shapes | 0,001–0,011× |

  Leitura honesta: **reads ≥ 5×; writes 3 ordens de magnitude abaixo** —
  cada commit no perfil é uma seção crítica single-writer com barreira
  forte própria (`F_FULLFSYNC` no Darwin), sem merge: throughput ≈
  clientes/latência_fsync (~100–660 qps). O piso 2× do Agents.md é a regra
  do **produto oficial (modo completo/compat)** e continua valendo lá; o
  perfil verificado não reivindica paridade de throughput — reativá-lo é
  exatamente o P2.1 (group-commit por teorema). Gate `floor=2.0` sai
  `exit 2` como esperado (min_ratio 0,000; relatório completo no
  `compare_report.json`).
- [x] **P1.3** Teste de derivação de semântica: mesmo seed/schedule, modo
   verificado vs modo completo ⇒ oráculos de segurança idênticos; diferenças
   permitidas apenas em métricas de performance — status: `done`
   (`verified_vs_full_same_oracles`: 64 seeds × {Sequential, PCT d=2} ×
   {full, verified}, três shapes de escrita + EIO one-shot; oráculos
   **compartilhados**: `silent_wrong==0`, sobrevivente async com valor
   certo, scan do reopen exato e sem fantasma em ambos os modos; diferença
   só no agendamento — verificado `queued==0`/`batches==submits`/fence
   ≤ 1, full permite 2+ no mesmo fence e a comparação exige merge real
   (`queued_total > 0` sob PCT — não-vacuidade))
- [x] **P1.4** CI: job `verified-mode` (ubuntu) rodando P0.2+P1.3 em toda
   mudança; freeze (`pedra_formal --ci`) já cobre os kernels — status:
   `done` (job `verified-mode` no `synthetic-field.yml`: invariant do
   catálogo, suíte FailingEnv do perfil, World oracles, os dois testes PCT
   (lone-only + derivação) e smoke do engine de bench `compatv` com asserts
   `engine/sync/durability/status`)

### P2 — reativação por teorema

- [x] **P2.1** Group-commit no modo verificado quando 0057 P2.1 (lema de
   atomicidade + first-committer-wins) estiver `done`; `profile_report()`
   passa a listar o kernel do grupo — status: `done` (kernel provado em
   `group_commit_kernel.rs`/`verus/group_commit.rs`; `pin_verified()` = merge
   decidido pelo kernel provado + catch-up window 0 + bypass async; report:
   `write_group_merge` ON (kernel `group_commit`) + `group_fence` ON;
   `PROFILE_VERSION = "verified-v2"`; bateria re-escrita para exigir merge
   ativo: `verified_profile_forces_safe_composition` (`queued > 0`,
   `batches < submits`), `pct_verified_merges_under_preemption`,
   `verified_vs_full_same_oracles` com não-vacuidade `queued_total > 0` nos
   dois modos; bypass async continua provado all-async
   (`verified_async_concurrent_never_merges`))  
- [x] **P2.2** Ring io_uring: permanece **fora** do modo verificado até
   existir modelo de ring provável (ou para sempre, documentado); o modo
   completo continua a usá-lo em Linux com o `PosixFallback` de hoje —
   status: `done` (gate documentado em três lugares amarrados: row `off!`
   `io_uring_ring` do `profile_report()` ("no proven ring model — cqe_kernel
   twin blocked; verified constructors pin StdEnv; full mode keeps
   PosixFallback"), doc do `OpenOptions::verified` (instrui abrir com
   `StdEnv`, como `PEDRA_VERIFIED=1` faz), e este slice; sem promessa de
   prova do ring — non-goal vivo)
- [x] **P2.3** Binário/flag de produto: `PEDRA_VERIFIED=1` no CLI/store open
   como uma linha de configuração com contrato publicado — status: `done`
   (`pedradb-cli`: `verified_requested()` + `LiveDb` (Full/Verified); todo
   comando live-open vira `Db<StdEnv>` + `OpenOptions::verified()` com uma
   env var — demo/backup/ship-wal/stats/compact/reclaim/maintain/compact-vlog/
   compact-blob/blob-gc; banner no stderr com `PROFILE_VERSION`; contrato no
   usage (`pedra` sem args) e no doc do `OpenOptions::verified`; testes
   `verified_flag_pins_the_profile_and_survives_reopen` (banner + TX
   all-or-nothing sob o perfil + reopen) e `default_mode_has_no_verified_banner`)

---

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | `VerifiedProfile` + `OpenOptions::verified()` | done | `verified.rs` + `verified_report_matches_catalog` (44/44) + `verified_profile_forces_safe_composition` | 2026-08-23 |
| P0.2 | p0 | Suíte DST no perfil | done | sim 5/5 + `verified_world_run_oracles` + `pct_verified_lone_never_merges` | 2026-08-23 |
| P0.3 | p0 | Docs ritual + claims | done | esta mudança (claims table, README, open-items, LEDGER L46) | 2026-08-23 |
| P1.1 | p1 | `open_verified` em fold/lease/dcs (+compat) | done | `open_verified_pins_std_env` ×3 + `open_verified_pins_lone_profile` | 2026-08-24 |
| P1.2 | p1 | Piso 2× medido no modo verificado | done (medido; 2× refutado no perfil — reads ≥5×, writes ~0,001×; tabela acima) | `findings/2026-08-24-verified-parity/` | 2026-08-24 |
| P1.3 | p1 | Derivação de semântica (oráculos idênticos) | done | `verified_vs_full_same_oracles` (64 seeds × 2 políticas × 2 modos; merge full não-vacuo) | 2026-08-24 |
| P1.4 | p1 | CI `verified-mode` | done | job `verified-mode` (synthetic-field.yml) + assert CI do flip 0054 corrigido | 2026-08-24 |
| P2.1 | p2 | Group-commit reativado por teorema | done | `group_commit_kernel.rs` (Verus 14/0 + Lean no-sorry) + `verified-v2` + bateria com merge ativo | 2026-08-24 |
| P2.2 | p2 | Ring fora do modo (gate documentado) | done | `profile_report` row off + doc `OpenOptions::verified` + este slice | 2026-08-24 |
| P2.3 | p2 | `PEDRA_VERIFIED=1` como contrato de produto | done | `pedradb-cli` `LiveDb` + banner + `verified_flag_*` (2/2) | 2026-08-24 |

---

## Acceptance Criteria

- **Tests** (unit + e2e scenarios named)
  - P0: `verified_profile_forces_safe_composition` (opções fixas, report
    fecha com os kernels existentes); suíte `FailingEnv` crash/reopen/EIO e
    World rodando no perfil, `silent_wrong=0`; metade async:
    `verified_async_close_and_barrier_reopen`,
    `verified_async_concurrent_never_merges` e o shape async em
    `pct_verified_lone_never_merges`.
  - P1: `open_verified_pins_std_env` em fold/lease/dcs; derivação
    `verified_vs_full_same_oracles` (seed × schedule, oráculos idênticos);
    bench do piso em tabela no RFC.
- **Telemetry / Analytics**
  - `profile_report()` (componente⇒estado⇒kernel) logável no open; nada de
    métrica nova de produto.
- **Documentation**
  - Status table na mesma mudança que o código; LEDGER L46 ganha o programa;
    README/open-items atualizados; cross-ref 0057 P2.1 no P2.1 daqui.
- **Screenshots**
  - backend-only — não se aplica.

## Claims (o que se pode / não se pode dizer)

| Quando | Pode dizer | **Não** pode dizer |
|--------|-----------|-------------------|
| P0 done | "existe um perfil de produto cujas seções críticas são kernels provados, coberto pela suíte DST" | "o modo verificado não tem bugs" |
| P1 done | "o produto inteiro roda no modo verificado; oráculos idênticos ao modo completo; throughput medido no modo (reads ≥5× vs Rocks default; writes serializados por commit até o P2.1 — sem claim de paridade no perfil)" | "mais rápido E mais seguro que Rocks/FDB em tudo"; "piso 2× vale também no perfil verificado" |
| P2 done | "group-commit reativado por teorema; ring explicitamente fora, com contrato" | "io_uring é seguro porque ninguém reclamou" |
| Sempre | "fallback seguro é default declarado e derivado dos kernels" | "extraído 100% de um teorema"; "100% de certeza" |

## Out of scope (non-goals)

- Extração de código do engine a partir dos teoremas (VeriBetrKV 8× —
  recusado, L46).
- Provar o ring io_uring (fica no modo completo; gate P2.2 é documentação,
  não promessa).
- Mudar o piso de paridade ou o peer oficial (Agents.md intacto:
  `sync=false` do Rocks default, piso 2×).
- TSan/Miri/ASan como condição do modo (são jobs irmãos do 0052/0057, não
  propriedades do perfil).
- Renomear produto/marca; "verified mode" não é produto vendido — é perfil
  de engenharia com contrato.

## Deviations

- **Lone-commit-only é um pin de runtime, não campo do `OpenOptions`**
  (P0.1). A política de grupo sempre viveu no `WriteGroup` como knob
  (`PEDRA_CATCHUP_US`, `set_write_group_catchup_window`,
  `PEDRA_ASYNC_GROUP`) — o `OpenOptions` governa o nível de arquivo. Um
  campo novo no `OpenOptions` quebraria ~150 literais completos em 30+
  arquivos sem ganho de segurança: a superfície de produto
  (`ConcurrentDb::open_verified`, `VerifiedProfile::open{,_with_env}`,
  `StoreOpenOptions::pedra_verified`) aplica **as duas metades juntas**, e
  o pin é de-uma-via-só (não existe un-pin — a composição é declarada,
  não alternada). `is_verified()` + `write_group_stats().queued == 0`
  tornam o estado observável em teste.
- **`b` visível após reopen no teste de fence verificado** (P0.2): o
  contrato é o mesmo do twin não-verificado — o append aterrissou antes do
  sync falhar, então o recovery **pode** mostrar a escrita nunca-ackada; o
  que o oráculo exige (e testa) é: acked `a` presente, recusado `c`
  ausente, fence fail-closed. A asserção inicial `b == None` era mais
  forte que o contrato e foi corrigida para espelhar o twin.
- **P1.2 — a expectativa de 2× no perfil foi refutada pela medição**:
  o item foi escrito com "expectativa ≥ 2.0" herdada da regra do produto.
  Medido no protocolo oficial, o perfil verificado sustenta ≥ 5× só nas
  shapes read-only (`ycsb_c` 5,2×; `mvcc_latest` 5,2×; `scan` 9,1×) e cai
  para 0,001–0,013× nas shapes de escrita — cada commit é lone
  (write lock + `F_FULLFSYNC` próprio, sem amortização de grupo). Isso é o
  custo estrutural do perfil até o P2.1, não um bug da medição; o item foi
  fechado como `done (medido)` com a refutação publicada, e o claim de
  paridade **não** foi movido para o perfil — o piso 2× do Agents.md
  permanece propriedade do modo completo (produto). Nenhuma coluna
  sync-peer foi usada (peer `sync=false` default; compare gate
  `ROCKS_PARITY_RATIO_FLOOR=2.0` saiu `exit 2` de propósito).
- **P1.1 pin por tipo, não `backend()`**: o StdEnv não expõe `backend()`
  (esse acessor existe no `IoUringEnv`); os construtores verificados fixam
  `StdEnv` **no tipo** (`PedraFold<StdEnv>`, `LeaseStore<StdEnv>`,
  `Dcs<C, StdEnv>`, `CompatEngine<StdEnv>` com label `compatv`), então a
  ausência do ring é garantida pelo sistema de tipos, não por checagem em
  runtime.
- **P1.4 — assert CI obsoleto corrigido junto**: o job
  `montanha-scale-and-compare` ainda afirmava `r['sync'] is True` para o
  engine `compat`, morto desde o flip do drop-in para async (RFC-0054,
  commit `1bcf7d6`) — o passo estava vermelho por razão errada. Corrigido
  para `is False` na mesma mudança que adiciona o job `verified-mode`.
  Nota de estado: o workflow `synthetic-field` de `origin/main` já vinha
  vermelho em **vários** jobs antes desta mudança (5+ runs `failure` —
  logs expirados, causas não inspecionadas aqui); isso é dívida pré-existente,
  não regredida por este RFC.

## Relação com o mapa existente

| Doc | Relação |
|-----|---------|
| RFC-0053 | Pipeline de kernels que torna o perfil barato (35 kernels + refinements) |
| RFC-0056 | "100% relativo ao TCB" — o modo verificado é o TCB **virando produto** |
| RFC-0057 | P2.1 daqui é gated pelo P2.1 de lá (kernel group-commit) |
| RFC-0052 | Caixas (Miri/TSan/ASan) continuam jobs irmãos, não parte do perfil |
| L46 (LEDGER) | Sanduíche IronFleet — este RFC é a face "produto" do sanduíche; extração total segue REFUSE |
| Agents.md | Piso 2× vs Rocks default intocado; P1.2 mede o piso **no** modo |
