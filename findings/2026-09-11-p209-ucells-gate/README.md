# P209 U-cells — lote Linux 3-run no gate (RFC-0209 P2.1): as 21 células das 7 suítes opt-in sobem ao caixote

**Data:** 2026-09-11 (p211u3 23:36–00:02Z) | **Caixa:** caixote
`linux-gate-p149b` (CHV, 4 vCPU, guest Alpine, nproc=4) | **Imagem:**
`p211u3` digest `sha256:a1c874bb…` (base `p04a`; src = árvore com o
P0.1+P0.2 do RFC-0211 commitado `4c93d2c5`)
**Protocolo:** mesmo boot, 3 rounds quiet (load1<2 ×2 20s), **uma invocação
por suíte** (isola estado cross-suíte — lição 2 abaixo), pedra
`PEDRA_PARITY_ASYNC=1` (coluna same-class) vs RocksDB default
`ROCKS_PARITY_SYNC=0` do MESMO round, `ROCKS_YCSB_OPS=2000`,
`ROCKS_YCSB_DIST` unset = uniform, `timeout 900` por invocação, 42/42
invocações rc=0 (zero U-FAIL). Suítes: qs, myrocks, streaming, arango,
venice, rocksapi, kvrocks — 21 shapes.

## Lições de protocolo (tentativas p211u/p211u2 — sem dados, registradas)

1. **p211u (digest 2d1e3730): redirect para diretório inexistente.** O
   `run_engine` redirecionava stdout para `$dest.run.log` com o diretório
   do round nunca criado — o bash falha em abrir o redirect ANTES de
   executar o binário: 6×`U-FAIL` no mesmo segundo do QUIET,
   `shapes=0`. Zero dados.
2. **p211u2 (digest e4a9d400): pendure real, diagnosticado no Darwin.**
   Com as 7 suítes NUMA invocação só, o processo pendurava após
   `linkbench_mix done`: `sample` (Darwin, reprodução local
   `SUITE=qs,myrocks,streaming`) mostra a main em
   `CompatEngine::flush` (`rocksdb-compat/src/lib.rs:3302`) — 1331/1793
   amostras em `parking_lot lock_slow` do `compact_gate` (serializa com
   o worker L0; lib.rs:3300–3308) entremeadas com `finish_flush_pipeline
   → try_rotate_wal → persist_manifest` (open/pwrite/rename/unlink por
   flush). Progresso patológico, não deadlock: o flush disputa o gate
   com a compaction de fundo sobre o DB que o myrocks acabou de encher
   (64.000 linhas de `write_tx`). **Isoladas, as suítes rodam em
   segundos** (varredura Darwin: qs/myrocks/streaming/arango/venice/
   rocksapi/kvrocks todos OK isolados; captação em
   `hang-repro/sample.txt` do scratch). Correção da onda: uma invocação
   por suíte + `timeout 900`. Efeito colateral: o build verboso lotou a
   janela de 500 entradas do console e engoliu marcadores da consulta
   armazenada (o tail ao vivo entregava) — build agora redirecionado,
   só marcadores no console.
3. **Linha truncada no transporte do console:** `myrocks_point_select`
   chegou cortada (`min=1.162 med=1`…); o min (número do veredito)
   está preservado, med/rounds dessa célula não.

## Nota de produto (DIAG, dono futuro, não é célula deste lote)

O custo do flush do compat sobre DB grande com contenção `compact_gate`
é um observável real (flush-after-myrocks patológico no mesmo
processo). Fora do escopo aqui; candidata a investigação própria se
reaparecer em escala (o `kafka_changelog_flush` 0,036× abaixo é o
parente medido em célula isolada).

## Resultado p211u3 — ratio vs rocks do MESMO round (pedra async same-class / rocks default sync=false), min-of-3

| forma | min | med | r1/r2/r3 |
|---|---|---|---|
| qs_hot_get | **1,384** | 1,416 | 1,384/1,416/3,418 |
| qs_neg_lookup | **1,345** | 1,381 | 1,381/1,345/1,381 |
| qs_batch_write | **1,763** | 1,816 | 1,881/1,763/1,816 |
| myrocks_point_select | **1,162** | — | linha truncada (lição 3) |
| myrocks_read_only | **4,123** | 4,253 | 4,253/4,371/4,123 |
| myrocks_write_tx | 0,751 | 0,822 | 0,751/0,822/0,967 |
| linkbench_mix | 0,236 | 0,268 | 0,315/0,268/0,236 |
| flink_window_state | **1,544** | 1,570 | 1,577/1,544/1,570 |
| kafka_changelog_flush | 0,036 | 0,037 | 0,037/0,037/0,036 |
| arango_doc_crud | **1,237** | 1,285 | 1,285/1,290/1,237 |
| arango_traversal | **3,346** | 3,523 | 3,523/3,744/3,346 |
| venice_fanout_get | **3,431** | 3,993 | 3,431/3,993/4,212 |
| mixgraph_like | **1,394** | 1,441 | 1,511/1,394/1,441 |
| wbwi_read_your_writes | 0,494 | 0,506 | 0,494/0,506/0,532 |
| compaction_filter_drop | 0,081 | 0,083 | 0,083/0,087/0,081 |
| ingest_sst | 0,069 | 0,070 | 0,081/0,070/0,069 |
| kvrocks_get | **4,141** | 5,201 | 4,141/10,215/5,201 |
| kvrocks_set | **2,547** | 2,775 | 2,547/6,140/2,775 |
| kvrocks_pipelined_set | **3,504** | 3,751 | 4,764/3,504/3,751 |
| kvrocks_scan | **29,125** | 32,806 | 29,125/32,806/34,324 |
| kvrocks_blob_set | **2,263** | 2,400 | 2,263/2,400/2,489 |

**15/21 células min-of-3 ≥ 1,0** (negrito); **6 perdas honestas
nomeadas**: kafka_changelog_flush **0,036** (flush-por-op no compat:
pipeline completo por flush), ingest_sst **0,069** e
compaction_filter_drop **0,081** (suíte rocksapi: APIs nativas do Rocks
— SstFileWriter/ingest e filtro de compaction — executadas via emulação
do compat), linkbench_mix **0,236** (mix scan-heavy com deletes),
wbwi_read_your_writes **0,494** (WriteBatchWithIndex emulado),
myrocks_write_tx **0,751** (tx de N updates em WriteBatch).

## Virada Darwin DIAG → Linux gate (as 7 células originais do 0209)

| célula ( Darwin DIAG → Linux 3-run min ) | |
|---|---|
| qs 0,808 → qs_hot_get **1,384** / qs_neg_lookup **1,345** | flip win |
| point_select 0,430 → myrocks_point_select **1,162** | flip win |
| wbwi 0,410 → 0,494 | segue perda (nomeada) |
| flink 0,522 → flink_window_state **1,544** | flip win |
| venice 0,735 → **3,431** | flip win |
| arango 0,003 → doc_crud **1,237** / traversal **3,346** | flip win |
| pipelined 0,807 → **3,504** | flip win |

O DIAG Darwin subestimava quase tudo (host/boot diferentes; os
absolutos cross-boot nunca foram comparáveis). Nenhuma célula win no
DIAG virou perda no gate.

## Adjudicação (veredito terminal datado)

**P2.1 do RFC-0209: done 2026-09-11/12 (onda p211u3, `linux-gate-p149b`).**
Lote anti-overfit medido: 21/21 células com Linux 3-run quiet min-of-3,
peer default `sync=false`, same-class async. 15 células ≥1,0; 6 perdas
honestas nomeadas acima (dono de fatia futura se forem atacadas; não há
claim de vitória nelas). Evidência: bloco `P211U U-cells lote…` no
console do gate (captura `p211u3-summary.txt` do scratch da sessão) +
este finding. Darwin = DIAG; Linux 3-run quiet min-of-3 = cartaz.
