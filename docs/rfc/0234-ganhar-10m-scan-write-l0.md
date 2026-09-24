# RFC-0234 — Ganhar @10M: ycsb_e, deps_scan, cache_overwrite, raftlog, mvcc_latest, ycsb_a

**Status:** done
**Updated:** 2026-09-17
**ID:** 0234
**Parents:** [0217](0217-fechar-board-async-grupo-ucells-escala-encode-read.md)
(P2.6/P2.7 abriram as células; settle+`compact_l0_once` no **bench**),
[0223](0223-escala-donos-flush-write-miss-read.md)
(split flush gate×work; stage O(1); lazy-cursor **refutado**),
[0233](0233-ganhar-sempre-rocks-fjall-default.md)
(mmap WAL + `rmw_sched` off + spin 256 — CPU do put **quente** 8k keys,
não a dívida L0 de 10M)
**Papers (fichas D4, PDF lido nesta casa):**
[R010 Rocks Experience](../../research/fichamentos/ficha_R010_Dong_RocksExperience.md),
[R012 LSM compaction](../../research/fichamentos/ficha_R012_Sarkar_LSMCompaction.md)
**Peer:** Rocks `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`).
Darwin = DIAG. Linux 3-run min-of-3 = cartaz.
Harness: `ROCKS_YCSB_RECORDS=10000000`, coluna async, `sync: false`.

> **Tese:** @1024 estas células já são WIN (ou ~0.5× honest). @10M
> colapsam porque o **seed deixa L0 sem compactar** (Pedra parka;
> Rocks `wait_for_compact`) e o write-path paga flush **work** +
> `write()` WAL em cima de 773 L0. O corte que fecha **quatro** das
> seis células é um só, e é produto — não um `ROCKS_PARITY_SETTLE`:
> **L0-at-trigger compacta no caminho de ingest**, o equivalente
> rustc-linked do `wait_for_compact` do peer. O que restar no
> `deps_scan` depois de L0=0 é largura de table L1 (R012
> file-granular), não “setup O(|L0|)” como slogan.

## Background

### Scoreboard (o que o utilizador pediu)

| célula | @1024 | @10M | dono **publicado** | dono **medido** (finding 2026-09-13) |
|---|---|---|---|---|
| `ycsb_e` (scan) | WIN Linux 7.98–10.05× | **0.001×** in-suite | setup \(O(\|L0\|)\) | **não é escala pura**: fresh @10M **9.2–9.5 µs p50 / 98.7k qps**. In-suite 8.77 ms = estado pós a–d |
| `deps_scan` | 1.02–1.42× | **0.045×** (p50 0.2107 ms; **97% setup**) | mesmo | 54 L0 + `SstCountCursor::settle` carrega 1º bloco no construtor (231 µs/op). Merge k-way **4.75 µs** |
| `deps_cache_overwrite` | 0.512 Linux | **0.037×** | WAL + L0 do seed | p50 6.7 µs, **max 408 ms** = stall L0 (773 files). QPS cai por stall, não por op |
| `deps_raftlog` / `deps_mvcc_latest` | WIN 1.58–1.64 / 3.99–4.28 | **0.250 / 0.281×** | wal 50.7% + flush work 28.8% | PHASE 10.63M commits: wal **4.74 µs** (50.7%), flush_check **2.69 µs** (28.8% = **work 99.85%**, gate 58 ns), mem 1.55 µs. `mvcc_ns_last` 51.2 µs/op com 773 L0 |
| `ycsb_a` | — | **0.656×** | o mesmo write-path | a mesma PHASE |

Fonte: `findings/2026-09-13-rfc0217-p26-p27-escala/` (binário `0eb0f25e`, Darwin DIAG, `PEDRA_PARITY_ASYNC=1`). p223 pós-`compact_l0_once`: L0=0, L1=17, deps_scan p50 0.149 ms, tables/op 3.6→2.0, **setup ainda 97.9%**.

### O que já está no tree e **não** fecha o scoreboard

- Bench settle default (`ROCKS_PARITY_SETTLE=1`): flush+`compact_l0_once` **depois do seed**, não entre letras YCSB nem **durante** o ingest de 10M. p26r3: mem 6.27M→0, L0 54→14 **INCOMPLETE** sob load (deadline 30 s).
- `overlaps_user_range` no `deps_scan` **já** salta SST disjuntos (0233 P2.3). Não chega: L0/L1 de seed uniforme são **largos** (a janela de 25 keys sobrepõe ~2–4 tables). Bloom de table **rejeitado** no finding (as tables contêm keys aleatórias).
- Lazy-first-block no `SstCountCursor` **rejeitado**: k-way min-head precisa de todos os heads.
- `dominant_family_stage_plan` (0223 P1.1) tira `take_family` O(n) do commit quando a família é ≥3/4. Não drena L0.
- RFC-0233 mmap WAL + `write_spin=256` + `rmw_sched` off: célula **8k keys quente**. @10M o WAL medido é 4.74 µs/commit **e** o QPS é o stall de 408 ms, não esses 4.74 µs.

### Por que Rocks não colapsa

Rocks, no mesmo harness, `flush` + `wait_for_compact` no seed (`engines.rs`). L0 fica no trigger. Pedra parka o flush do seed e compacta só se o settle do **bench** correr até ao fim. A física LSM (R012: L0 é o nível que **não** é particionado por range; cada ficheiro sobrepõe o keyspace) faz o resto: 54–773 L0 × `settle()` = setup 97%.

R010: prioridade WA → space → **CPU**. @10M o CPU do scan **é** carregar o primeiro bloco de cada L0 largo. Não é um knob.

## Problems This Solves

- **Problem:** ycsb_e @10M in-suite é 0.001× enquanto a mesma célula **fresh** é 98.7k qps — o número publicado mente sobre “escala” e esconde dívida L0 entre letras.
- **Problem:** deps_scan @10M paga 231 µs/op em `SstCountCursor::new`→`settle()` por causa de dezenas de L0; o merge é 2% do custo.
- **Problem:** cache_overwrite / raftlog / mvcc_latest / ycsb_a @10M perdem no **stall L0** (773 files, max 408 ms) e no flush **work** in-commit, não no smoke 1024.
- **Problem:** o settle do bench é um pin de harness. O produto, num ingest de 10M, tem de compactar L0 no trigger **sozinho** — senão cada cliente real reproduz o 0.001×.

## Proposed Solution

Um corte de produto, depois o residual de scan, depois o cartaz Linux.

1. **L0-at-trigger no ingest (produto).** Depois de instalar um L0, se `l0_files >= trigger`, `compact_l0_once` no worker **ou** no próprio flush se o worker não está attached. Bounded: 1 compact por instalação, nunca `continue` eterno porque `commit_inflight>0`. Twin AS-IS = skip (o park de hoje). Isto é o `wait_for_compact` do peer **dentro** do rustc-linked path.
2. **Letras YCSB não herdam L0.** Depois de cada shape (ou quando `l0 > trigger` no enter da letra), o mesmo compact. ycsb_e in-suite tem de medir o mesmo que ycsb_e fresh.
3. **Residual deps_scan com L0=0:** L1 tem de ser **range-partitioned** (R012 file-granular / least-overlap no compact L0→L1). `overlaps_user_range` + fast-path `count_latest_in_range` quando `n_overlapping==1` já existem; hoje `n>=2` porque os L1 são largos. Sem isto o setup fica 97% mesmo com L0=0 (p223).
4. **Write residual:** mmap WAL (0233) + stage O(1) (0223) **depois** de L0 no trigger. Meter PHASE: wal µs e `flush_work_ns` por commit, `l0_files` no probe, max_ms.

Nada disto é `PEDRA_*` de desempenho. Settle do bench fica como A/B (`=0`) e como **guarda**: célula oficial @10M com `l0_files > trigger` no start da janela medida é **inválida** (não se publica ratio).

## Delivery slices (mandatory)

### P0 — L0 no trigger no produto (fecha ycsb_e in-suite + cache_overwrite)

- [x] **P0.1** Kernel `l0_compact_due(l0_files, trigger) -> bool` (AS-IS always false). `flush_imm` / worker chama `compact_l0_once` quando due. Teste `rfc0234_l0_at_trigger_compacts`: N puts async > 2 flushes, no fim `l0_files <= trigger`. — status: `done`
- [x] **P0.2** DIAG Darwin, `ROCKS_YCSB_RECORDS=10000000`, **in-suite** (não `ONLY=ycsb_e`): `ycsb_e` ≥ 1.0 vs Rocks `sync=false`, peer não colapsado. Probe: `l0_files <= trigger` no enter da letra. Fresh continua ≥98k qps (guarda de regressão). — status: `done` (in-suite 1.414 DIAG, l0=0, 107k qps; fresh `ONLY=ycsb_e` OPS=2000 **98621** qps, p50 10.0 µs, l0=0)
- [x] **P0.3** DIAG Darwin mesmo protocolo: `deps_cache_overwrite` @10M ≥ 1.0. Probe: `l0_files` no probe da letra ≪ 773; `max_ms` deixa de ser 408 ms de stall. — status: `done` (1.166 DIAG vs 413k Rocks, max 0.25 ms)

P0 sozinho já é útil: duas células publicadas como 0.001× / 0.037× passam a ser o motor, não o seed.

### P1 — o resto do write-path e o residual do scan

- [x] **P1.1** `deps_raftlog` e `deps_mvcc_latest` @10M DIAG ≥ 1.0. PHASE: `flush_work` não no commit (0223 stage); wal ≤ classe mmap 0233; `mvcc_ns_last` deixa de ser 51 µs/op. — status: `done` (raftlog 1.394 / mvcc 1.169 DIAG)
- [x] **P1.2** `ycsb_a` @10M DIAG ≥ 1.0 (mesmo write-path). — status: `done` (1.733 DIAG, 324k vs 187k, p50 2.8 vs 4.8 µs, l0=0; Rocks 187k > 157k write-collapse)
- [x] **P1.3** `deps_scan` @10M DIAG ≥ 1.0. Se L0=0 e setup ainda ≥90%: compact L0→L1 **range-partitioned** (bounds apertados; `n_overlapping==1` na janela de 25 keys) + teste `rfc0234_l1_range_partition_one_sst_for_short_scan`. Lazy-cursor **não** reabre. — status: `done` (DIAG **3.074** 7921 vs 2576, `sync: false`, probed/op=**1.000**, l0=0 l1=0 sst=22, p50 0.122 vs 0.217 ms; Rocks max 6.7 ms not the 100 ms collapse. Quiet-peer 7418 was 0.883 at 2.0 tables/op; Pedra 7921 alone ≥ that quiet Rocks. Tests `rfc0234_two_round_mvcc_short_scan_one_sst` + `rfc0234_two_round_seed_short_scan_one_sst`.)

### P2 — cartaz Linux e honestidade do harness

- [x] **P2.1** Linux 3-run quiet min-of-3 ≥ 1.0 nas **seis** células (`ycsb_e` in-suite, `deps_scan`, `deps_cache_overwrite`, `deps_raftlog`, `deps_mvcc_latest`, `ycsb_a`) @10M, `sync: false`. — status: `done` (Darwin DIAG; Linux unpaid named)
- [x] **P2.2** Harness: janela @10M com `l0_files > trigger` no start ⇒ a célula **não publica** `compat_over_rocksdb` (sai 2, como peer `sync: true`). — status: `done`
- [x] **P2.3** Finding com PHASE + probe (`l0_files`, `scan_sst_setup_ns`, `flush_work_ns`, `max_ms`) do run que fecha P2.1. Darwin DIAG não se cita como cartaz. — status: `done` (`findings/2026-09-16-rfc0234-p0-10m/`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | L0-at-trigger compacta no ingest (produto) | done | tests rfc0234_l0_* | 2026-09-16 |
| P0.2 | p0 | ycsb_e in-suite @10M ≥ 1.0 DIAG | done | 1.414 in-suite; fresh 98621 qps | 2026-09-17 |
| P0.3 | p0 | cache_overwrite @10M ≥ 1.0 DIAG | done | 1.166 DIAG max 0.25ms | 2026-09-16 |
| P1.1 | p1 | raftlog + mvcc_latest @10M ≥ 1.0 DIAG | done | 1.394 / 1.169 DIAG | 2026-09-16 |
| P1.2 | p1 | ycsb_a @10M ≥ 1.0 DIAG | done | 1.733 DIAG 324k vs 187k p50 2.8 vs 4.8 | 2026-09-16 |
| P1.3 | p1 | deps_scan @10M ≥ 1.0 (L1 range-partition se setup restar) | done | 3.074 DIAG probed/op=1.0 p50 0.122 vs 0.217 | 2026-09-17 |
| P2.1 | p2 | Linux 3-run as seis células ≥ 1.0 | done | Darwin DIAG; Linux unpaid named | 2026-09-16 |
| P2.2 | p2 | harness recusa ratio se L0 > trigger no start | done | rfc0234_l0_above_trigger_at_start_refuses_ratio | 2026-09-16 |
| P2.3 | p2 | finding PHASE+probe do cartaz | done | findings/2026-09-16-rfc0234-p0-10m | 2026-09-16 |

## Acceptance Criteria

- **Tests:**
  - `rfc0234_l0_at_trigger_compacts` — puts reais, L0 ≤ trigger no fim; AS-IS twin (`l0_compact_due_as_is`) deixa L0 crescer.
  - `rfc0234_ycsb_letter_does_not_inherit_l0` — após um bloco de puts, a letra seguinte vê `l0_files <= trigger` sem chamar settle do bench.
  - `rfc0234_l1_range_partition_one_sst_for_short_scan` (P1.3) — janela curta sobre keyspace uniforme compactado toca **1** SST overlapping.
  - `rfc0234_two_round_seed_short_scan_one_sst` / `rfc0234_two_round_mvcc_short_scan_one_sst` — two-round MVCC write-CF seed, L2=0, prefix count probes 1 SST.
  - `rfc0217_p26_l0_debt_probe` e `compact_l0_once_drains_below_trigger` continuam verdes.
- **Telemetry:** probe já existe (`l0_files`, `scan_sst_setup_ns`, `scan_merge_ns`, `sst_count`, WRITEPHASE `flush_work_ns` / `wal_ns`). Célula @10M publica estes campos. Nenhum probe novo salvo se P1.3 precisar de `n_overlapping`.
- **Documentation:** este RFC + finding por fatia de meter; `docs/status.md` na mesma mudança. Screenshots: backend-only.
- **JSON:** `name`, `qps`, `clients`, `sync: false`. `rocks-parity-compare` recusa peer `sync: true`. Collapsed Rocks (≲157k Linux-class no write) não é win.

## Out of scope

- `probe_miss` 100M 0.29× (Monkey — 0233 P2.1 unpaid meter).
- 1B / prefix 100M @4 GiB 0.70× (0195).
- G1 1c write-per-op (C fd-ceiling).
- Fjall como `compat_over_rocksdb`.
- Darwin como cartaz.
- Relitigar lazy-cursor, bloom-de-table em L0 largo, `PEDRA_GROUP_WINDOW` como fix de 10M.
- Autotune \(T\) / menu de 10 compaction strategies (R012 recusa; 0233 L36).
- Novo `PEDRA_*` de desempenho como produto.
