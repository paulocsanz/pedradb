# RFC-0043: 2× RocksDB **default** nos benches de alto nível + catálogo que só cresce

**Status:** in-progress
**Updated:** 2026-08-25
**Parked (quiet remesure):** remaining P-slices need a quiet 3× host; dirty sandbox numbers are not the official floor (AGENTS.md peer `sync=false`).
**Parents:** [0041](0041-2x-rocks-default.md) (piso 2.0 nas 16; 1c write continua teto),
[0042](0042-0.8x-rocks-default-write-shapes.md) (group/catch-up; não apaga 1c),
[AGENTS.md](../../AGENTS.md) (peer = `sync=false`)
**Children:** [0044](0044-async-class-5x-rocks.md) (5× same-class async; G1 intocado)

## Background

- Dependentes reais (TiKV raftstore, Quicksilver) **não** fazem 1 `put` +
  espera Ok a 200 k/s. Mandam `WriteBatch`, seek MVCC, scan curto, ou
  write raro em lote e get no working set.
- O harness já modela isso na suíte `deps` + YCSB C/E. Head3 quieto
  ([rfc0041-p11/head3](../../findings/rfc0041-p11/head3/)):

| banda | shapes | mediana |
|---|---|---|
| HL ≥ 2.0 | `apply_mc4` 2.79, MVCC 2.34, E 2.12 | piso ok |
| HL &lt; 2.0 | C 1.80, scan 1.79, `raftlog_mc4` 1.79, apply 1c 1.30, raftlog 1c 0.99 | fecháveis (CPU) |
| canário 1-op | A/F/ow 1c e `_mc4`, B/D, `cache_overwrite` | teto `1/t_fd`; **ficam no relatório** |

- RFC-0041 pede 2.0 nas **16**. A/F/ow 1c (e A/F `_mc4` mesmo com grupo
  perfeito de 4) estão acima de um fd/Ok nesta caixa. Este RFC **não**
  resolve isso e **não** apaga essas linhas.
- RFC-0042 mediu sob load ~42 — **não** é mapa oficial. Remesuras daqui
  são caixa quieta, ou o README declara o load e não compara com head3.

## Problems This Solves

- **Problem:** o cartaz de produto mistura 1-op impossível com apply/MVCC
  já ≥ 2.0; o min_ratio das 16 esconde o que o dependente sente.
- **Problem:** não há regra para **adicionar** benches (QS, lock/prewrite,
  mixgraph) sem a tentação de remover o que falha.
- **Problem:** `FLOOR=2.0` hoje não liga no `SYNC=0`; quem já passou não
  está gated.

## Proposed Solution

1. Conjunto **HL** (alto nível) com gate 2.0 vs peer `sync=false`.
   Conjunto **canário** (1-op) sempre no `compare_report.json`, nunca no
   gate deste RFC.
2. **O conjunto de shapes do compare só cresce.** Shape nova que fica
   &lt; 2.0 entra na status table como `todo` e permanece no JSON.
3. Ciclo: medir HL₀ → fechar ≥ 2.0 → acrescentar HL₁ → repetir.
4. `ROCKS_PARITY_GATE_SHAPES` (já no compare) = HL; default do script
   `SYNC=0` liga `FLOOR=2.0` só nesse subset.

## Conjuntos

### HL₀ (gate 2.0 deste RFC)

`deps_apply_batch`, `deps_apply_batch_mc4`, `deps_raftlog`,
`deps_raftlog_mc4`, `deps_mvcc_latest`, `deps_scan`, `ycsb_c`, `ycsb_e`.

Critério: o dependente agrupa escrita **ou** o caminho quente não é 1
put + Ok.

### Canário (relatório; fora do gate 0043)

`ycsb_a`, `ycsb_a_mc4`, `ycsb_b`, `ycsb_d`, `ycsb_f`, `ycsb_f_mc4`,
`deps_cache_overwrite`, `deps_cache_overwrite_mc4`.

### Como adicionar um bench

1. Mesmo `Engine` trait, mesmo seed/zipf, dois adapters (compat + rocks).
2. Nome estável no `summarize`; entrada **nova** no array de
   `rocks_parity_compare.rs` (append; nunca delete).
3. Proveniência no RFC + README do finding (post TiKV / blog QS / paper).
4. 3 runs, peer `sync:false`, linha na status table — mesmo se &lt; 2.0.
5. Se a escrita for 1-op, a shape vai para canário, não para o lixo.

QS não é open source: mixes `qs_*` são *inspirados* nos números
publicados (working set 1–20%, ~99/1, ~10× negative lookup, write em
lote), não replay de produção.

### HL₁ (P2.1 — shapes no runner; remesura pendente)

| shape | proveniência | contrato |
|---|---|---|
| `qs_hot_get` | QS v2 working set 1–20% | 99% get / 1% WriteBatch ≥32 no hot set |
| `qs_neg_lookup` | QS ~10× negative lookups | get miss além do keyspace |
| `qs_batch_write` | QS root write em lote | 1 WriteBatch / op |
| `deps_lock_prewrite` | TiKV prewrite (lock+default) | WriteBatch multi-CF, sem commit |

Suíte `qs` é **opt-in** (`ROCKS_PARITY_SUITE=ycsb,deps,qs`) para o
prefixo de 16 shapes continuar comparável. `deps_lock_prewrite` vive
na suíte `deps` (compare só cresce).

### HL₂ (P2.2 / P2.6 — no runner 2026-08-19)

Suítes opt-in (não misturam as 16 oficiais). Mixgraph-like / WBWI /
compaction filter / ingest SST: suíte `rocksapi` (P2.7; remesura 3×
continua parked). Fora: cluster TiKV 3 nós (peer Montanha).

| suíte | shapes | banda |
|---|---|---|
| `nebula` | `nebula_get_neighbors`, `nebula_insert_edge` | HL |
| `streaming` | `flink_window_state`, `kafka_changelog_flush` | HL |
| `ceph` | `bluestore_omap_write` (host-sync), `bluestore_omap_read` | HL |
| `solana` | `solana_shred_append`, `solana_trailing_read` | HL |
| `arango` | `arango_doc_crud`, `arango_traversal` | HL |
| `venice` | `venice_fanout_get` (32 gets/op), `rockstore_widecol_rw` | HL |
| `oxigraph` | `oxigraph_spo_lookup`, `oxigraph_triple_put` | HL (Oxigraph 0.5.x ainda RocksDB) |

Mais nas suítes já existentes: `kvrocks_blob_set` (16 KiB, canário
grande), `surreal_tx_rmw_mc8` (HL, conflito OCC). Compare
`peer_anomalies` marca run suja se o peer fizer rmw mais rápido que put.

### HL₃ (P2.3 — Kvrocks + MyRocks no runner; remesura pendente)

Catálogo completo (quem usa Rocks por dentro, que bench existe, o que
bloqueia): [rocksdb-dependents-benchmarks.md](../rocksdb-dependents-benchmarks.md).
Suítes `kvrocks` e `myrocks` são **opt-in**
(`ROCKS_PARITY_SUITE=ycsb,deps,kvrocks,myrocks`). Compare só cresce.

| shape | proveniência | contrato | banda |
|---|---|---|---|
| `kvrocks_get` | redis-benchmark GET | 1 get | canário |
| `kvrocks_set` | redis-benchmark SET | 1 put | canário |
| `kvrocks_pipelined_set` | Redis pipeline → WriteBatch | 1 batch ≥ `ROCKS_DEPS_BATCH` | HL |
| `kvrocks_scan` | Redis SCAN COUNT=25 | scan curto | HL |
| `kvrocks_set_mc50` | redis-benchmark default `-c 50 -t set` | 50 clientes SET, sem pipeline | HL |
| `myrocks_point_select` | sysbench oltp_point_select | 1 get PK | canário |
| `myrocks_read_only` | sysbench oltp_read_only | scan curto PK | HL |
| `myrocks_write_tx` | sysbench write-tx / tpcc stmt | 1 WriteBatch = N rows | HL |
| `linkbench_mix` | LinkBench (Meta; 55% GET_LINKS_LIST) | scan+get+batch+delete | HL |

1-op GET/SET/point_select ficam no relatório (teto `1/t_fd`); não vão
pro lixo. Mixes são *inspirados* nos benches publicados, não replay de
produção.

Peer Rocks destas suítes = **default do DB de cima**, não um async
forçado: Kvrocks `write_options.sync no`; MyRocks
`flush_log_at_trx_commit=1` (sync no commit); Surreal
`SURREAL_DATASTORE_SYNC_DATA=every` (fsync após a txn). Cartaz oficial
(16 shapes) continua vs Rocks `sync=false`. JSON
`peer_policy=host-default` quando MyRocks/Surreal entram.

### HL₄ (P2.4 — SurrealDB OCC no compat; remesura pendente)

`OptimisticTransactionDB` + `Transaction` no `rocksdb-compat` (Pedra
`OccTransaction`; rust-rocksdb shape). SurrealDB `kv-rocksdb` chama
`transaction_opt` com `set_snapshot(true)` + `WriteOptions.sync=false`
([fonte](https://github.com/surrealdb/surrealdb/blob/main/surrealdb/core/src/kvs/rocksdb/mod.rs)).
Suíte `surreal` opt-in. Compare só cresce.

| shape | proveniência | contrato | banda |
|---|---|---|---|
| `surreal_tx_get` | crud-bench / SurrealQL SELECT | begin + get + commit | HL |
| `surreal_tx_put` | crud-bench CREATE | begin + 1 put + commit | canário |
| `surreal_tx_rmw` | SurrealQL UPDATE | begin + get + put + commit | HL |
| `surreal_tx_scan` | crud-bench scan | begin + scan 25 + commit | HL |
| `surreal_tx_batch` | crud-bench insert lote | begin + N puts + commit | HL |

OCC + compile-shape rust-rocksdb que o `kv-rocksdb` deles importa
(`open_cf_descriptors`, `ReadOptions`, `raw_iterator_opt` seek/next,
`property_int_value`, `flush_opt`/`flush_wal`, `compact_range_opt`,
Options tunables = no-op). Ainda **não** é build do SurrealDB: UDT
timestamp / prefix extractor de verdade (bloom). Peer Rocks =
`OptimisticTransactionDB` (`sync=false`).

### P2.5 — programa 0.80 → >1.0 (suítes opt-in; 2026-08-18)

Meta do usuário: nas suítes novas, shape já ≥ 0.80 vai para > 1.0;
shape < 0.80 vai para 0.80 (peer = default do DB de cima). Corte de
CPU no commit (o `fdatasync` é fixo, G1):

1. `get_at` com fast-path no point cache quando
   `snap.seq == published_seq` (double-checked): leitura OCC rmw vira
   cache hit em vez de andar mem + SSTs.
2. `apply_batch_occ` valida o write-set **por referência** a partir dos
   `ops` (sem clonar chaves) e só anda o read-set quando houve publish
   depois do snapshot; `OccTransaction::commit` deixou de montar
   `check_keys` antecipado.
3. `kvrocks_set_mc50`: SET 1-cliente vs peer async é canário **físico**
   (1 fd por Ok; mesma classe dos canários 0041, teto ~`1/t_fd` —
   0.80 vs async não existe com G1). No redis-benchmark default
   (`-c 50`), o write group compartilha 1 fd entre quem espera; é a
   shape onde 0.80+ é alcançável **mantendo** mais durabilidade.

Baseline sujo (load ≫ ncpu; run4, não oficial): `kvrocks_get` 1.99,
`kvrocks_scan` 7.44, `kvrocks_pipelined_set` 0.47 (regrediu com o
flush pós-seed extra — revertido; run3 sem ele = 1.23),
`kvrocks_set` 0.14, `surreal_tx_get` 222, `surreal_tx_scan` 26,
`surreal_tx_put` 0.88, `surreal_tx_batch` 0.90 (run3 = 1.23),
`surreal_tx_rmw` 0.74 (run3 = 1.76). Remesura isolada quieta pendente.

Run5 (após o código P2.5; load 184–234, direcional):
`surreal_tx_put` **1.065** e `surreal_tx_batch` **1.537** cruzaram 1.0;
`kvrocks_pipelined_set` **0.813** (recuperado do 0.47 do run4);
`kvrocks_get` 1.61. Abertos: `surreal_tx_rmw` (0.593 — peer anômalo
nesta run: rmw 5009/s > put 3072/s do próprio peer é incoerente;
lado compat ok com 2973 ≈ put 3273) e `kvrocks_set_mc50` (0.373 com
avg_group 6.21 ops/fd sob load; caixa quieta deve agrupar mais).
Raw: `findings/rfc0043-new-suites-dirty/run5-*/`.

### Relação com 0041 / 0042

- **0041** continua dono do piso 2.0 **nas 16**. 1c write (P1.2) fica
  `todo` (teto `1/t_fd`). Este RFC não “resolve” 0041.
- **0042** (catch-up / default 0 / encode direto) revalida em caixa
  quieta se o default 0 piorou `raftlog_mc4` / apply. Knob permanece.

## Garantias invariáveis

| # | Garantia | Este RFC |
|---|---|---|
| G1 | `fdatasync` antes do Ok | intocada |
| G2 | visibilidade = lookup / range_at | intocada |
| G4 | adversarial sem editar asserção | re-verde |
| G6 | sem thread no core | intocada |
| G8 | peer = Rocks `sync=false`; mediana ≥3 | o RFC |
| — | shape existente não some do compare | **este RFC** |

## Delivery slices (mandatory)

### P0 — mapa + gate no que já passa

- [x] **P0.1** RFC + status viva (este doc) — status: `done`
- [ ] **P0.2** Remesura 16 × 3 quieta em `findings/rfc0043-p0/` (parked: quiet remesure);
      `medians.txt`; peers `sync:false` — status: `todo`
- [ ] **P0.3** Script `SYNC=0` (parked: quiet remesure): `GATE_SHAPES` = HL que P0.2 mostrou ≥ 2.0
      + `FLOOR=2.0`. Compare ainda lista as 16. — status: `doing`
      (script já gata `apply_mc4,mvcc,ycsb_e` no head3; confirma após P0.2)

### P1 — HL₀ inteiro ≥ 2.0

- [ ] **P1.1** `deps_raftlog_mc4`, `ycsb_c`, `deps_scan` ≥ 2.0 (parked: quiet remesure; CPU /
      leitura; ~+12%) — status: `doing` (TLS hit `&self`; WAL one
      `encoded_len`. Official ainda head3 1.79. Remesura só load ≪ ncpu)
- [ ] **P1.2** `deps_apply_batch` e `deps_raftlog` 1c ≥ 2.0 — status: `todo` (parked: quiet remesure)
- [ ] **P1.3** Remesura `findings/rfc0043-p1/` (parked: quiet remesure); gate = HL₀; canários
      presentes — status: `todo`

### P2 — catálogo cresce

- [ ] **P2.1** Suíte `qs` (parked: quiet remesure; `qs_hot_get`, `qs_neg_lookup`, `qs_batch_write`)
      + `deps_lock_prewrite` no runner/compare; 3 runs
      `findings/rfc0043-p2/`; quem &lt; 2.0 fica — status: `doing`
      (shapes no runner + compare + testes verdes; remesura 3× bloqueada
      por load ~55 — não gravar mediana suja)
- [ ] **P2.2** HL₂ (parked: quiet remesure): mixgraph-like / WBWI / compaction filter / ingest
      no runner. Shapes Nebula/Flink/Ceph/Solana/Arango/Venice/Oxigraph
      saíram para P2.6. Sem cluster TiKV. — status: `doing` (API: P2.7;
      remesura 3× parked)
- [ ] **P2.3** Suítes `kvrocks` + `myrocks` (parked: quiet remesure; 8 shapes) no runner/compare;
      catálogo em `docs/rocksdb-dependents-benchmarks.md`; 3 runs
      `findings/rfc0043-p2.3/`; quem &lt; 2.0 fica — status: `doing`
      (shapes + testes verdes; remesura 3× bloqueada por load ≫ ncpu)
- [ ] **P2.4** `OptimisticTransactionDB` (parked: quiet remesure) no compat + suíte `surreal`
      (5 shapes); 3 runs `findings/rfc0043-p2.4/`; quem &lt; 2.0 fica —
      status: `doing` (API + testes; remesura bloqueada por load ≫ ncpu)
- [ ] **P2.5** Programa 0.80 → >1.0 (parked: quiet remesure) nas suítes opt-in: fast-path cache no
      `get_at`, validação OCC por referência, `kvrocks_set_mc50`
      (redis-benchmark default). Alvo: put/batch/rmw ≥ 0.80, quem já
      passou > 1.0; remesura isolada quieta — status: `doing`
      (run5 sujo: put 1.065 / batch 1.537 / pipeline 0.813 já no alvo;
      rmw e set_mc50 abertos; quiet 3× pendente)
- [x] **P2.6** Catálogo expande + empatar os 3 abertos: catch-up
      `active≥16` cap 1× fd_ema (mc50); `peer_anomalies` no compare
      (rmw > put do peer = run suja); `surreal_tx_rmw_mc8` (Busy retry);
      `kvrocks_blob_set` (16 KiB); suítes opt-in `nebula` / `streaming` /
      `ceph` / `solana` / `arango` / `venice` / `oxigraph`. Oxigraph
      0.5.x (2026) ainda é RocksDB. Remesura 3× pendente — status: `doing`
      (OCC agora entra no write group; run8 sujo: rmw_mc8 0.65 / set_mc50
      0.64 / rmw 0.93 / put 1.08; quiet 3× pendente)
- [x] **P2.7** Suite opt-in `rocksapi` (API, not remesure): `mixgraph_like`,
      `wbwi_read_your_writes`, `compaction_filter_drop`, `ingest_sst` no
      runner/compare + testes verdes. `DB::compact_with_filter`. Remesura
      3× continua no P2.2 parked — status: `done`
      (`rocksapi_suite_on_compat_engine`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC + status viva | done | este doc | 2026-08-18 |
| P0.2 | p0 | remesura 16×3 quieta | todo | findings/rfc0043-p0 | 2026-08-18 |
| P0.3 | p0 | GATE_SHAPES+FLOOR no HL que já passa | doing | tikv_ycsb_parity_v0.sh | 2026-08-18 |
| P1.1 | p1 | raftlog_mc4 / C / scan ≥ 2.0 | doing | last-1024 + get() key-only; remesura pendente | 2026-08-18 |
| P1.2 | p1 | apply/raftlog 1c ≥ 2.0 | todo | — | 2026-08-18 |
| P1.3 | p1 | remesura p1; gate HL₀ | todo | findings/rfc0043-p1 | 2026-08-18 |
| P2.1 | p2 | qs + lock_prewrite; quem falha fica | doing | runner+compare; remesura pendente | 2026-08-18 |
| P2.2 | p2 | HL₂ remesure | todo | parked quiet remesure; API → P2.7 | 2026-08-25 |
| P2.7 | p2 | suite `rocksapi` (mixgraph/WBWI/filter/ingest) | done | `run_rocksapi` + `compact_with_filter` | 2026-08-25 |
| P2.3 | p2 | kvrocks + myrocks + catálogo | doing | runner+compare; remesura pendente | 2026-08-18 |
| P2.4 | p2 | SurrealDB OCC + suíte surreal | doing | compat txn + runner; remesura pendente | 2026-08-18 |
| P2.5 | p2 | 0.80 → >1.0 (get_at cache, OCC ref, mc50) | doing | run5 sujo: put 1.07/batch 1.54/pipeline 0.81; rmw+mc50 abertos | 2026-08-18 |
| P2.6 | p2 | catálogo P2 + catch-up mc50 + peer_anomalies | doing | shapes no runner; remesura quieta pendente | 2026-08-19 |

### Medianas (preencher com P0.2; até lá = head3)

Peer = RocksDB default `sync:false`; Pedra `fdatasync` antes do Ok (G1).
Caixa quieta, 3 runs, mediana das razões por run
([rfc0041-p11/head3](../../findings/rfc0041-p11/head3/)).

| shape | HL? | compat qps | rocks qps | head3 (r1, r2, r3) | falta p/ 2.0 |
|---|---|---:|---:|---|---|
| deps_apply_batch_mc4 | sim | 8,256 | 3,812 | **2.788** (1.30, 2.82, 2.79) | — |
| deps_mvcc_latest | sim | 656,437 | 274,823 | **2.342** (2.39, 2.34, 1.62) | — |
| ycsb_e | sim | 250,138 | 117,883 | **2.121** (2.69, 2.12, 0.93) | — |
| ycsb_c | sim | 2,542,776 | 1,410,769 | 1.796 (1.82, 1.80, 1.60) | +11.4% |
| deps_raftlog_mc4 | sim | 24,558 | 14,427 | 1.792 (1.64, 2.10, 1.79) | +11.6% |
| deps_scan | sim | 469,204 | 261,789 | 1.790 (1.79, 1.63, 1.79) | +11.7% |
| deps_apply_batch | sim | 5,893 | 4,664 | 1.297 (1.30, 1.45, 0.54) | +54.2% |
| deps_raftlog | sim | 5,784 | 7,591 | 0.994 (0.99, 0.35, 1.42) | ~2× (fd/Ok) |
| ycsb_a / f / ow 1c e `_mc4`, B, D | canário | — | — | 0.23–0.57 | teto `1/t_fd` (G1) |

3/16 ≥ 2.0 na mediana; 0/16 nas **todas** as 3 runs. p25–p33 são
code-only (sem remesura quieta): a tabela oficial continua head3.

## Acceptance Criteria

- **Tests:** shapes novas têm teste de schedule no
  `rocksdb-parity-bench` (mesmo n nos dois engines); adversarial
  `rocksdb-compat` verde **sem** edição de asserção.
- **Telemetry:** `findings/rfc0043-p0|p1|p2/` com
  `compare_report.json` ×3, peers `sync:false`, `medians.txt`. Gate 2.0
  só no HL. Canários e HL &lt; 2.0 continuam no JSON.
- **Documentation:** este doc; status table = JSONs. backend-only.
- **Screenshots:** none — backend-only.

## Out of scope

- 2.0 em canário 1-op (teto G1). RFC-0041 P1.2 / 0042.
- Apagar shape para o `min_ratio` subir.
- Cluster TiKV / `go-ycsb` 3 nós / sysbench TiDB (peer Montanha, não Rocks).
- Workers KV. Peer `sync=true`. Largar G1. Thread no core.
- `FLOOR=2.0` em *todas* as 16 (0041 P2.3).
