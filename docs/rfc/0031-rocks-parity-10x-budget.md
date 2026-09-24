# RFC-0031: Rocks parity budget — compat within 2× of real RocksDB (same durability class)

**Status:** in-progress  
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
3. **Orçamento ≤2× na mesma classe de durabilidade:** floor 0.5 por shape contra o peer `ROCKS_PARITY_FULL_SYNC=1` (ambos pagam `F_FULLFSYNC` no WAL). A coluna RocksDB-fdatasync fica rotulada e **não é o gate** — comparar `F_FULLFSYNC` (~4.8 ms) com `fdatasync` (~50 µs) é misturar contratos, não medir o motor.

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
| G8 | **Números honestos** — peer real, agenda idêntica, **mesma classe de sync** | Re-medida com o par completo. Gate 2× só contra `ROCKS_PARITY_FULL_SYNC=1`. Pedra não desce de `File::sync_all`. |

Regra de PR: se uma fatia precisar editar teste existente para ficar verde, é relaxação — volta para o desenho, não para o diff.

## Orçamento por shape (floor = rocksdb_same_class / 2)

Teorema medido neste Mac (80 samples, arquivo crescente 200 B):

| syscall | p50 |
|---|---:|
| `os.fsync` / `fdatasync` | **49–56 µs** |
| `F_FULLFSYNC` (`File::sync_all`) | **4.8 ms** (~100×) |

`librocksdb-sys` 0.16 **não** define `HAVE_FULLFSYNC` no `build.rs` — `WriteOptions.sync=true` é `fdatasync`. Pedra `File::sync_all` é `F_FULLFSYNC`. **2× contra fdatasync, mantendo G1, é fisicamente impossível** num writer sequencial (3.8 ms vs 50 µs). Pau a pau = mesma classe.

Peer same-class: `ROCKS_PARITY_FULL_SYNC=1` faz `File::sync_all` em cada `*.log` do Rocks depois do write (rotulado no JSON).

Lab 2026-08-15, records=1024 ops=200 batch=32, Pedra interval=64:

| shape | compat | rocks fdatasync | ratio (unfair) | rocks F_FULLFSYNC | ratio (2× gate) |
|---|---:|---:|---:|---:|---|
| ycsb_a 50/50 | 424 | 13.519 | 0.031 | 399 | **1.06 ✓** |
| ycsb_b 95/5 | 4.467 | 231.147 | 0.019 | 3.640 | **1.23 ✓** |
| ycsb_c 100r | 83.507 | 758.052 | 0.110 | 683.275 | 0.12 — point-get |
| ycsb_d 95r/5i | 2.822 | 84.319 | 0.033 | 3.198 | **0.88 ✓** |
| ycsb_e scan+5i | 3.043 | 78.353 | 0.039 | 5.356 | **0.57 ✓** |
| ycsb_f RMW | 490 | 10.542 | 0.047 | 470 | **1.04 ✓** |
| deps_apply_batch | 95 | 1.814 | 0.052 | 97 | **0.98 ✓** |
| deps_mvcc_latest | 291 | 144.036 | 0.002 | 122.374 | 0.002 — iterador |
| deps_scan | 458 | 120.694 | 0.004 | 118.346 | 0.004 — iterador |
| deps_raftlog | 157 | 10.413 | 0.015 | 203 | **0.77 ✓** |
| deps_cache_overwrite | 173 | 219 | 0.79 | 191 | **0.90 ✓** |

Seed 1024 puts: Pedra 6.2 s · Rocks F_FULLFSYNC 5.4 s · Rocks fdatasync 0.1 s.

**Escritas já estão dentro de 2× (várias >1×).** Falham o floor 2×: `ycsb_c` (point-get ~8×) e `deps_mvcc_latest`/`deps_scan` (iterador eager). Gate all-shapes só depois do P1; até lá o script pode gatear só os shapes de escrita (`ycsb_a/b/d/f`, `deps_apply_batch/raftlog/cache_overwrite`).

## Orçamento legado (floor 10× vs fdatasync — superado, não é mais o alvo)

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

Re-medida P0.1 (2026-08-15, worktree @origin/main + debounce, records=1024 ops=200 batch=32, `PEDRA_CHANGELOG_INTERVAL=64`):

| shape | compat antes | compat P0.1 | rocksdb sync=1 (esta run) | ratio | ≥ floor RFC? |
|---|---:|---:|---:|---:|---|
| ycsb_a | 94 | **390** | 75.148 | 0.005 | não (floor 3.773) |
| ycsb_b | 913 | **4.245** | 573.477 | 0.007 | não |
| ycsb_c | 156.103 | 206.629 | 1.798.432 | 0.115 | **sim** |
| ycsb_f | 110 | **352** | 81.454 | 0.004 | não |
| deps_apply_batch | 27 | **78** | 7.219 | 0.011 | não |
| deps_mvcc_latest | 260 | 813 | 378.549 | 0.002 | não (iterador) |
| deps_cache_overwrite | 52 | **215** | 36.680 | 0.006 | não |

Seed 1024 puts: 39.7 s → **6.5 s**. `interval=0` ycsb_a = 379 qps (p50 3.8 ms) ≈ interval 64 — CHANGELOG saiu do caminho crítico.

Expectativa mecânica (rev. P0.1): o debounce remove as 2–3 barreiras extras do CHANGELOG. Residual medido com `PEDRA_CHANGELOG_INTERVAL=0` (zero stores no commit path) é **quase idêntico** ao default 64 — o que resta é **um** `File::sync_all` do WAL por write (~3.8 ms p50 em ycsb_a neste Mac; `std::fs::File::sync_all` = `F_FULLFSYNC`). RocksDB `WriteOptions.sync=true` usa `fsync`, não `F_FULLFSYNC` — classes de durabilidade diferentes. **Não** trocamos `sync_all` por `sync_data` para ganhar o bench (G1). Floor 0.1 não vira default até o residual de escrita cruzar (group commit já existe em `ConcurrentDb`; single-writer compat ainda é 1 fsync/put).

## Delivery slices (mandatory)

### P0 — must ship first (useful alone)

- [x] **P0.1** CHANGELOG store debounce inline (a cada N commits duráveis + flush + close + WAL rotate + checkpoint; knob `PEDRA_CHANGELOG_INTERVAL`; sem thread) — status: `done`
- [x] **P0.2** Re-medir; floor 2× same-class nas **escritas** já passa — gate all-shapes fica para P1 (iterador). Script aceita `ROCKS_PARITY_FULL_SYNC=1` + `ROCKS_PARITY_RATIO_FLOOR=0.5` em modo write-shapes — status: `done`
- [x] **P0.3** RFC + Status vivo (este doc) — status: `done`
- [x] **P0.4** Peer same-class: `ROCKS_PARITY_FULL_SYNC=1` (`File::sync_all` nos `*.log` do Rocks) + labels de durabilidade — status: `done`

### P1 — next wave

- [x] **P1.1** Iterador com janela bornada no rocksdb-compat (≤ K entradas por passo; `From+Forward/Reverse` preservados) — status: `done` (`StreamingVisibleIter` / `ITER_WINDOW=64`; RFC-0050 P1.1)
- [ ] **P1.2** Re-medir `deps_mvcc_latest`/`deps_scan`/`ycsb_c` ≥ 0.5 vs same-class; adversarial de iterator positioning re-verde — status: `todo` (parked: quiet-box remesure; dirty sandbox is not the official floor)

### P2 — later / polish

- [ ] **P2.1** Tabela final no `rocksdb-compat.md` + nota de orçamento (min_ratio global ≥ 0.5 same-class) — status: `todo` (parked: quiet-box remesure)
- [x] **P2.2** Se algum shape ainda < floor com mecanismo novo identificado: abrir seção de follow-up com número (não engessar) — status: `done` (residual = 1× WAL `sync_all`/`F_FULLFSYNC` ≈ 3.8 ms/put; interval=0 ≈ interval=64)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | CHANGELOG debounce inline | done | e28d5bd | 2026-08-15 |
| P0.2 | p0 | re-medida + floor 2× same-class (writes) | done | este commit | 2026-08-15 |
| P0.3 | p0 | RFC + status vivo | done | este doc | 2026-08-15 |
| P0.4 | p0 | peer F_FULLFSYNC (`ROCKS_PARITY_FULL_SYNC`) | done | este commit | 2026-08-15 |
| P1.1 | p1 | iterador janela bornada | done | `ITER_WINDOW=64` / RFC-0050 P1.1 | 2026-08-24 |
| P1.2 | p1 | ycsb_c + mvcc/scan ≥ 0.5 + adversarial | todo | parked: quiet remesure | 2026-08-24 |
| P2.1 | p2 | tabela final + min_ratio ≥ 0.5 | todo | parked: quiet remesure | 2026-08-24 |
| P2.2 | p2 | follow-up de shape remanescente | done | classe de sync medida; 2× writes already | 2026-08-15 |

## Acceptance Criteria

- **Tests:** suite adversarial do rocksdb-compat inteira verde **sem relaxação** (G4: nenhuma asserção editada — Accept-set pós-crash inalterado: CHANGELOG ausente nunca falha read — reopen reconstrói do WAL, já coberto por SyncFail/ShortWrite/dead-disk/F33/F53); unit novo do debounce (N-1 commits sem store → reopen equivalente ao com store; N-ésimo storea; flush/close stoream; store falhando não vira erro de commit — G5/G7); iterator positioning ×8 re-verde com janela (G2: sem mudança de semântica de invalidação/reverse).
- **Gate por slice (G1–G8):** cada PR de slice lista quais garantias poderia tocar e o comando exato re-executado (`cargo test -p rocksdb-compat`, suites de reopen fail-closed, codec fuzz) no corpo do commit.
- **Telemetry:** bench JSON ganha campo `changelog_interval` no report do compat; compare passa a exibir `min_ratio` vs floor 0.1 quando peer real.
- **Documentation:** este RFC + tabela atualizada em `rocksdb-compat.md` + linha em `open-items.md`.
- **Screenshots:** backend-only.

## Out of scope

- Concorrência multi-thread no compat (gap #9 da matriz TiKV — `ConcurrentDb` não é wired aqui).
- Per-CF options, ingest, compaction filters e demais gaps L da matriz TiKV.
- Paridade com RocksDB **fdatasync** (referência rotulada; o gate 2× é same-class `F_FULLFSYNC`).
- Descer Pedra de `File::sync_all` para `sync_data` (relaxaria G1 neste Mac).
- Claims distribuídos/campo: o par é single-node, single-client, lab.
- Thread de background no core (regra em pé: debounce é inline e determinístico; forço de store via `pedra maintain`).
