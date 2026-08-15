# RFC-0031: Rocks parity budget — compat within 10× of real RocksDB

**Status:** draft  
**Updated:** 2026-08-15  
**Parents:** [0019](0019-local-primitive-for-platform-and-scylla-need.md) (CHANGELOG é cache), [0025](0025-montanha-perf-parity-vs-peers.md) (perf parity method), [rocksdb-compat](../rocksdb-compat.md) (par + diagnóstico)

## Background

- O par de paridade (`crates/rocksdb-parity-bench`, suítes `ycsb` + `deps`) mede compat (rocksdb-compat sobre pedradb-core) vs **RocksDB real** com agenda de ops idêntica e durabilidade casada (sync-por-write nos dois lados). Números de máquina, não template.
- Lab (2026-08-15): writes **0,002–0,016×** RocksDB; leituras via iterador **0,003–0,005×**; point-get puro 0,10×. Gate `ROCKS_PARITY_RATIO_FLOOR` está report-only porque sabemos que falha.
- Dois mecanismos já diagnosticados no fonte (não é disco — RocksDB prova fsync ~26 µs nesta máquina):
  1. **CHANGELOG no caminho crítico:** todo write sincronizado regrava o CHANGELOG inteiro (`change_feed.rs::store_on`: encode de todas as entradas + tmp + `sync_all` + rename + `sync_dir`) — 3 barreiras de durabilidade por put e custo **quadrático** (runs menores pontuam relativamente melhor; batch amortiza: apply 0,016 vs overwrite 0,006).
  2. **Iteradores eager:** cada `iterator_cf` materializa o snapshot do CF inteiro em `Vec` — `deps_mvcc_latest`/`deps_scan` ficam ~600× abaixo do point-get do próprio compat (156k qps).

## Problems This Solves

- **Problem:** distância de 60–350× sem plano de fechamento; sem orçamento explícito por shape.
- **Problem:** o CHANGELOG em disco é cache reconstruído do WAL (RFC-0019) e **nunca deve gatear commit** — hoje paga 3 fsyncs por write.
- **Problem:** iterador do compat tem semântica correta mas custo O(CF inteiro) por chamada.

## Proposed Solution

1. **CHANGELOG fora do caminho crítico** (inline, sem thread): store debounce a cada N commits duráveis (`PEDRA_CHANGELOG_INTERVAL`, default 64) + no flush e no close; falha de store continua warn-and-continue. Contrato de durabilidade inalterado (WAL fsync antes de Ok permanece).
2. **Iterador com janela bornada:** materializar no máximo K entradas por passo sobre o snapshot (seek + lookahead), em vez do CF inteiro.
3. **Orçamento ≤10× com gate real:** floor 0.1 por shape no script de lab quando P0 pousar; tabela viva no doc do compat.

## Garantias invariáveis (este RFC não relaxa nenhuma)

Paridade se compra com engenharia, nunca com garantia. Cada slice cita quais destes contratos poderia tocar e re-roda os gates correspondentes no mesmo PR:

| # | Garantia | Por que o P0/P1 não toca (verificado no fonte) |
|---|---|---|
| G1 | **WAL fsync antes de Ok** — write ack'ado sobrevive a crash/power | `commit_ops_with` mantém `wal.sync_all()` no caminho antes de aplicar mem e retornar Ok; o debounce move só `change_log.store_on` (cache), nunca o sync do WAL |
| G2 | **CRC fail-closed / never silent-wrong** | Nada muda em recovery/codec; suites de reopen fail-closed re-verdes |
| G3 | **TX all-or-nothing** | Batch continua um único `apply_batch` atômico; só a frequência do store do cache muda |
| G4 | **Accept-set pós-crash** (Ok pin exatamente um resultado; Err uni os possíveis; nunca-escrito admite None) | Suite adversarial FailingEnv do rocksdb-compat re-verde **sem editar uma asserção**; CHANGELOG ausente é caso já aceito (F33/F53: reopen reconstrói de WAL/SST) |
| G5 | **Fencing em sync-fail (RFC-0015 H1)** — append Ok + sync Err cerca o DB | `durability_fenced` permanece ligado ao sync do WAL; store do CHANGELOG falhando segue warn-and-continue (já era) |
| G6 | **Sem thread no core** — operador força via `pedra maintain` | Debounce é inline e determinístico (a cada N commits, flush, close); sem timer, sem worker |
| G7 | **Read-your-writes / feed** — leitura vê o próprio write | Feed vive em memória (`change_log.extend` no commit); o disco é cache — ler nunca depende do store debounce |
| G8 | **Números honestos** — peer real, agenda idêntica, durabilidade rotulada | Re-medida sempre com o par completo (compat + rocksdb sync=1) na mesma máquina/commit |

Regra de PR: se uma fatia precisar editar teste existente para ficar verde, é relaxação — volta para o desenho, não para o diff.

## Orçamento por shape (floor = rocksdb_sync1 / 10)

Baseline lab: ycsb @e5c6b80, deps @863df07, records=1024 ops=200 batch=32.

| shape | rocksdb sync=1 (qps) | floor 10× (qps) | compat hoje | diagnóstico |
|---|---:|---:|---:|---|
| ycsb_a 50/50 | 37.728 | 3.773 | 94 | CHANGELOG/write |
| ycsb_b 95/5 | 85.674 | 8.567 | 913 | CHANGELOG/write |
| ycsb_c 100r | 1.556.420 | 155.642 | 156.103 | **já ≥ floor** |
| ycsb_d 95r/5i | 88.484 | 8.848 | 817 | CHANGELOG/write |
| ycsb_e scan+5i | 147.289 | 14.729 | 1.073 | CHANGELOG + iterador |
| ycsb_f RMW | 47.001 | 4.700 | 110 | CHANGELOG/write |
| deps_apply_batch | 1.720 | 172 | 27 | CHANGELOG (amortiza) |
| deps_mvcc_latest | 84.367 | 8.437 | 260 | iterador eager |
| deps_scan | 104.943 | 10.494 | 508 | iterador eager |
| deps_raftlog | 4.744 | 474 | 32 | CHANGELOG/write |
| deps_cache_overwrite | 8.976 | 898 | 52 | CHANGELOG/write |

Expectativa mecânica: removendo o CHANGELOG do caminho crítico, um put fica ~append WAL + fsync + memtable (≈50–100 µs single-writer) → todos os shapes de escrita acima do floor com folga; resta o iterador nos shapes de leitura reversa/scan.

## Delivery slices (mandatory)

### P0 — must ship first (useful alone)

- [ ] **P0.1** CHANGELOG store debounce inline (a cada N commits duráveis + flush + close; knob `PEDRA_CHANGELOG_INTERVAL`; sem thread) — status: `todo`
- [ ] **P0.2** Re-medir ycsb+deps no par; se todos os shapes de escrita ≥ floor, virar default `ROCKS_PARITY_RATIO_FLOOR=0.1` no script de lab (template mode continua sem gate) — status: `todo`
- [ ] **P0.3** RFC + Status vivo (este doc) — status: `done`

### P1 — next wave

- [ ] **P1.1** Iterador com janela bornada no rocksdb-compat (≤ K entradas por passo; `From+Forward/Reverse` preservados) — status: `todo`
- [ ] **P1.2** Re-medir `deps_mvcc_latest`/`deps_scan`/`ycsb_e` ≥ floor; adversarial de iterator positioning re-verde — status: `todo`

### P2 — later / polish

- [ ] **P2.1** Tabela final @novo commit no `rocksdb-compat.md` + nota de orçamento (min_ratio global ≥ 0.1) — status: `todo`
- [ ] **P2.2** Se algum shape ainda < floor com mecanismo novo identificado: abrir seção de follow-up com número (não engessar) — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | CHANGELOG debounce inline | todo | — | 2026-08-15 |
| P0.2 | p0 | re-medida + floor 0.1 default (lab) | todo | — | 2026-08-15 |
| P0.3 | p0 | RFC + status vivo | done | este doc | 2026-08-15 |
| P1.1 | p1 | iterador janela bornada | todo | — | 2026-08-15 |
| P1.2 | p1 | leituras ≥ floor + adversarial | todo | — | 2026-08-15 |
| P2.1 | p2 | tabela final + nota de orçamento | todo | — | 2026-08-15 |
| P2.2 | p2 | follow-up de shape remanescente | todo | — | 2026-08-15 |

## Acceptance Criteria

- **Tests:** suite adversarial do rocksdb-compat inteira verde **sem relaxação** (G4: nenhuma asserção editada — Accept-set pós-crash inalterado: CHANGELOG ausente nunca falha read — reopen reconstrói do WAL, já coberto por SyncFail/ShortWrite/dead-disk/F33/F53); unit novo do debounce (N-1 commits sem store → reopen equivalente ao com store; N-ésimo storea; flush/close stoream; store falhando não vira erro de commit — G5/G7); iterator positioning ×8 re-verde com janela (G2: sem mudança de semântica de invalidação/reverse).
- **Gate por slice (G1–G8):** cada PR de slice lista quais garantias poderia tocar e o comando exato re-executado (`cargo test -p rocksdb-compat`, suites de reopen fail-closed, codec fuzz) no corpo do commit.
- **Telemetry:** bench JSON ganha campo `changelog_interval` no report do compat; compare passa a exibir `min_ratio` vs floor 0.1 quando peer real.
- **Documentation:** este RFC + tabela atualizada em `rocksdb-compat.md` + linha em `open-items.md`.
- **Screenshots:** backend-only.

## Out of scope

- Concorrência multi-thread no compat (gap #9 da matriz TiKV — `ConcurrentDb` não é wired aqui).
- Per-CF options, ingest, compaction filters e demais gaps L da matriz TiKV.
- Paridade com RocksDB async-WAL (referência report-only; o contrato comparado é sync-por-write).
- Claims distribuídos/campo: o par é single-node, single-client, lab.
- Thread de background no core (regra em pé: debounce é inline e determinístico; forço de store via `pedra maintain`).
