# RFC-0037: apply ≤2× vs Rocks fd + 2× Pedra on the other 10

**Status:** draft  
**Updated:** 2026-08-16  
**Parents:** [0036](0036-tikv-rocks-2x-fdatasync.md) (WAL `fdatasync`, L0-only compact, 10/11), [0016](0016-pedradb-production-robustness.md) (ConcurrentDb dual-mem / off-lock flush)

## Background

Lab `6a4267b`, 4096/2000 zipfian 1 KB, `ycsb,deps` combinada, peer Rocks `WriteOptions.sync=true` / `fdatasync`. Raw: [tikv-ycsb-0036-fdatasync](../findings/tikv-ycsb-0036-fdatasync/).

| shape | Pedra qps | Rocks fd | slower | 2× Pedra (alvo paralelo) |
|---|---:|---:|---:|---:|
| ycsb_a | 57 486 | 60 707 | 1.06× | 115 k |
| ycsb_b | 343 220 | 410 604 | 1.20× | 686 k |
| ycsb_c | 1 322 933 | 1 440 014 | 1.09× | 2.65 M |
| ycsb_d | 397 562 | 435 106 | 1.09× | 795 k |
| ycsb_e | 157 871 | 106 872 | 0.68× | 316 k |
| ycsb_f | 53 345 | 55 971 | 1.05× | 107 k |
| deps_apply_batch | 1 908 | 4 405 | **2.31×** | — (primeiro: ≤2× vs Rocks) |
| deps_mvcc_latest | 424 253 | 311 024 | 0.73× | 849 k |
| deps_scan | 212 099 | 303 195 | 1.43× | 424 k |
| deps_raftlog | 6 588 | 3 244 | 0.49× | 13 k |
| deps_cache_overwrite | 36 653 | 25 619 | 0.70× | 73 k |

Apply: p50 **166 µs vs Rocks 169 µs**. O qps cai no max **221 ms** — um `rewrite_ssts` no `put` depois do auto-flush.

### O que o compact faz hoje

`maybe_auto_flush` → `finish_flush_pipeline` → `maybe_auto_compact` → `compact_l0_into_l1` → `rewrite_ssts`:

1. `SstTable::entries_cloned()` = `materialize_entries()` decodifica **todos** os blocos e **clona** o `Vec<(InternalKey, Bytes)>` (4 L0 × ~4 MiB ≈ 16 MiB de valores 1 KB).
2. `gc_compact_entries` mete tudo num `BTreeMap` e volta a `Vec`.
3. `write_sst_entries_on` recodifica + lz4 + `fdatasync`.
4. Reopen do SST + MANIFEST.

O scan já tem merge em streaming (`SstRangeIter` / `StreamingVisibleIter`). O compact **não**.

`rocksdb-compat` envolve um `Db` single-writer em `parking_lot::Mutex`. O bench é **um cliente**. `ConcurrentDb` já escreve o SST de flush **sem** o write lock (`prepare_flush_imm` / `write_memtable_to_l0_file_num` / `install_l0_sst`); o compact **continua no lock**, no mesmo `finish_flush_pipeline`.

`Host` hoje é só Env+Clock+Rng. **Não há** fila de jobs nem `spawn`.

### Experiências já medidas (e o que cada uma prova)

| tentativa | apply | scan | o que ficou provado |
|---|---:|---:|---|
| Relabel L0→L1 no MANIFEST (sem rewrite) | **2.05×** | **2.88×** (20 ficheiros) | apply quase passa; scan precisa de poucos ficheiros |
| `auto_flush_bytes` 64 MiB (Rocks `write_buffer_size`) | 2.10× | 1.93× | mem com 70 k entradas piora o B-tree do put (p50 268 µs) |
| Fundir pares L1 até ≤10 | 3.0× | 1.60× | o merge volta para o put; apply/raftlog regridem |
| Promover 2 L0 por passo | pior apply | pior scan | mais compactes + mais ficheiros |

Conclusão mecânica: **no put, ou pagas o merge, ou pagas o scan depois.** Sem thread no core, 11/11 no cliente único só fecha se o merge no put ficar barato o suficiente (não se o deferres para o *próximo* put — esse put ainda é apply).

### Fila no ConcurrentDb / host — o que realmente faz

- **Off-lock no mesmo thread** (espelhar o flush): o `put` que disparou o flush **ainda espera** o merge. Não muda o qps single-client do apply. Ajuda *outros* clientes a fazer group-commit enquanto o merge corre — o harness atual não tem esses clientes.
- **Fila + o próximo put drena:** o compact muda de sítio no apply; o max 221 ms continua numa op do apply.
- **Worker fora do core** (`rocksdb-compat` / `pedradb-store` / processo `pedra maintain`): G6 fala do **core**. Um thread no *host* pode drenar a fila depois do Ok. Aí o apply single-client não junta o merge. Visibilidade: L0 fica até o install; G2 ok. Precisa de fence se o merge falha (G5) e de não esconder chaves (G2).
- **Host::spawn no core:** não. Isso é thread no core (G6).

Ordem honesta: **primeiro** merge em streaming no put (P0, cliente único, sem thread). **Depois** off-lock + worker no host se o P0 não chegar a 0.5 (P1/P2).

### 2× Pedra nos outros 10 — piso físico

Alvo paralelo = **2× o qps Pedra da tabela 0036**, não 2× vs Rocks (vários já são mais rápidos que o Rocks).

- **ycsb_a / ycsb_f:** p50 21–26 µs ≈ um `fdatasync`. Mix 50 % write. Teto single-client ≈ `2 / (t_fd + t_get)` ≈ 90–100 k qps. **115 k (2× A) está no/além do syscall.** 2× nestes shapes, single-client, **não** fecha sem skip de sync (quebra G1) ou **vários clientes + group commit** (`ConcurrentDb`, já existe).
- **ycsb_b / ycsb_d:** 5 % write; 2× Pedra é sobretudo cauda de write + get. Leitura já está no µs.
- **ycsb_c / mvcc:** p50 já ≤ Rocks. 2× Pedra é mutex + encode CF + cache. Teto próximo do relógio.
- **ycsb_e / deps_scan:** e já 0.68× Rocks; scan 1.43×. 2× Pedra no scan (424 k) é menos ficheiros / menos probe (20 L1 matou o scan nas experiências).
- **raftlog / overwrite:** p50 já melhor que Rocks; 2× Pedra é a mesma cauda de flush/compact que o apply.

## Problems This Solves

- **Problem:** apply 2.31× vs Rocks fd com p50 empatado — `rewrite_ssts` clona o L0 inteiro no `put`.
- **Problem:** deferir o compact para o próximo `put` não tira o pico do apply single-client.
- **Problem:** relabel / mais ficheiros passa o apply e parte o scan. Os dois tetos puxam lados opostos.
- **Problem:** “2× mais rápido” em A/F single-client choca com um `fdatasync` por write; sem dizer isso o P1 vira teatro.

## Proposed Solution

1. **P0 — merge L0 em streaming no put.** Mesmo sítio (`compact_l0_into_l1`). Sem `entries_cloned` do ficheiro inteiro: k-way pelos blocos (`decode_block` / iter já usados no scan), escrever o SST de saída à medida. G2 = o mesmo conjunto de versões. Sem thread. Sem mudar o peer.
2. **P1 — compact off-lock no ConcurrentDb** (padrão do flush: prepare / I/O / install). O core não cria thread. `rocksdb-compat` pode passar a `ConcurrentDb` *depois* do P0 medido.
3. **P1 — 2× Pedra nas leituras** (C, e, mvcc, scan) no que o split P1.1 ranquear. Sem cap por camada.
4. **P2 — worker no host** (compat/store, não `pedradb-core`) só se o apply single-client ainda >2× depois do merge streaming. **P2 — 2× Pedra em A/F** só com N clientes + group commit; o RFC **não** promete 2× A single-client.

## Garantias invariáveis

Herdadas de RFC-0036 / 0031:

| # | Garantia | Este RFC |
|---|---|---|
| G1 | WAL `fdatasync` antes do Ok | intocada |
| G2 | visibilidade = lookup / range_at | merge streaming emite as mesmas versões que `rewrite_ssts` + `gc_compact_entries` default |
| G4 | adversarial compat | re-verde **sem** editar asserção |
| G5 | fence se sync falha | merge/install falha não publica o output; L0 antiga fica |
| G6 | sem thread no **core** | P0 inline; P1 off-lock no thread do caller; worker só fora do core |
| G8 | números honestos | apply vs Rocks fd da **mesma** run; 2× Pedra vs tabela 0036, não vs um Rocks mais lento |

## Delivery slices (mandatory)

### P0 — must ship first (11/11 no cliente único)

- [x] **P0.1** RFC + Status vivo (este doc) — status: `done`
- [ ] **P0.2** `compact_l0_into_l1` em streaming: zero `entries_cloned` do SST inteiro; teste de que o conjunto visível = merge actual; adversarial sem editar asserção — status: `todo`
- [ ] **P0.3** Remesura `tikv_ycsb_parity_v0.sh` FULL_SYNC=0; `deps_apply_batch` ≥ 0.5 vs Rocks fd da run; os outros 10 não regridem do 2× Rocks — status: `todo`

### P1 — off-lock + 2× Pedra nas leituras

- [ ] **P1.1** Split de tempo no apply (flush vs `entries_cloned` vs encode SST vs MANIFEST) e no C/e/mvcc/scan (mutex / encode / ficheiros) — status: `todo`
- [ ] **P1.2** Compact prepare/write/install off-lock em `ConcurrentDb` (mesmo molde do flush); `Db` single-thread continua sem thread — status: `todo`
- [ ] **P1.3** O gargalo #1 do P1.1 nas **leituras** (C / e / mvcc / scan) até 2× o qps Pedra 0036; remesura — status: `todo`

### P2 — host worker + escritas 2× Pedra

- [ ] **P2.1** Se P0.3 apply ainda < 0.5: fila de compact drenada por thread no **compat/store** (não no core); fence + L0 visível até install — status: `todo`
- [ ] **P2.2** 2× Pedra em A/F/overwrite: harness multi-cliente + `ConcurrentDb` group commit; **não** 2× A single-client — status: `todo`
- [ ] **P2.3** Gate `ROCKS_PARITY_RATIO_FLOOR=0.5` nas 11 vs fd; tabela 2× Pedra nos 10 — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC + investigação | done | este doc | 2026-08-16 |
| P0.2 | p0 | L0 compact streaming (sem clone 16 MiB) | todo | — | 2026-08-16 |
| P0.3 | p0 | apply ≥ 0.5 vs Rocks fd (11/11) | todo | — | 2026-08-16 |
| P1.1 | p1 | split apply + leituras | todo | — | 2026-08-16 |
| P1.2 | p1 | compact off-lock no ConcurrentDb | todo | — | 2026-08-16 |
| P1.3 | p1 | 2× Pedra no gargalo #1 de leitura | todo | — | 2026-08-16 |
| P2.1 | p2 | worker host se P0 não chegar | todo | — | 2026-08-16 |
| P2.2 | p2 | 2× Pedra A/F com N clientes | todo | — | 2026-08-16 |
| P2.3 | p2 | gate 0.5 + tabela 2× Pedra | todo | — | 2026-08-16 |

## Acceptance Criteria

- **Tests:** `cargo test -p rocksdb-compat` adversarial **sem** editar asserção; teste novo: L0 compact streaming vs `rewrite_ssts` no mesmo snapshot (mesmas chaves/valores visíveis, incluindo tombstone); `auto_compact_l0_leaves_existing_l1` continua a não reescrever o L1 antigo.
- **Telemetry:** `scripts/tikv_ycsb_parity_v0.sh` (FULL_SYNC=0); finding com p50/p95/p99; apply `meets_floor` a 0.5; coluna “2× Pedra” só depois do P1.3/P2.2 com número da run.
- **Documentation:** este RFC + linha em `findings/tikv-ycsb-lab-20260815.md` no commit do P0.3. backend-only.
- **Screenshots:** none — backend-only.

## Out of scope

- Thread, timer ou `Host::spawn` dentro de `pedradb-core` (G6).
- Skip de `fdatasync` no Ok (G1).
- 2× ycsb_a/f **single-client** vs a tabela 0036 (piso do syscall).
- 2× vs Rocks `sync=false`.
- Relabel L0→L1 sem merge como caminho do 11/11 (já medido: parte o scan).
- `auto_flush_bytes` 64 MiB como default (já medido: 70 k mem piora o put).
