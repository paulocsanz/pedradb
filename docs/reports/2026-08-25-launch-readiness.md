# Relatório: prontos para lançar e chocar o mundo?

**Data:** 2026-08-25
**Veredito:** **não.**
**RFC que fecha o que falta:** [RFC-0062](../rfc/0062-launch-readiness-remaining-gaps.md)
**Peer oficial (não relitigar):** RocksDB default, `WriteOptions.sync=false`, `ROCKS_PARITY_SYNC=0`. Vitória contra `sync=true` não conta. Pedra no kernel ainda `fdatasync` antes do Ok (G1).

> **Correção de âmbito (mesmo dia):** o dono restringiu a barra a
> **`rocksdb-compat` só** — substituto estrito, só vantagem, nunca
> defeito. Intel não é meta (caixote brasil = um host AMD). A tabela
> 0.001× **não** é full-sync. Leitura vigente:
> [`2026-08-25-compat-strict-substitute.md`](2026-08-25-compat-strict-substitute.md)
> + [RFC-0062](../rfc/0062-launch-readiness-remaining-gaps.md). Este
> ficheiro permanece como arquivo das quatro perguntas originais; não
> citar Intel / TLS / seL4 como defeito do crate.

Este documento responde as quatro perguntas do dono com números no repo, não com desejo. Fonte de cada célula está citada. Onde o finding viveu só no worktree `rfc-0054-gaps`, a cópia está em `findings/` neste tree.

---

## 0. As quatro perguntas, em uma linha cada

| # | Pergunta | Resposta |
|---|---|---|
| 1 | Paridade de performance **completa** com RocksDB default (no mínimo 1×, idealmente mais / 2×)? | **Não completa.** Drop-in async: 15/15 ≥ 1.25× no Mac; Linux 13/16 ≥ 2×, mas `deps_raftlog` ainda **< 1×** (0.78–0.98). G1 1-cliente write-per-op é teto físico ≪ 1×. |
| 2 | Mais garantias que o Rocks que as pessoas realmente correm? | **Sim no contrato local default** (G1 no kernel, fence incondicional, CRC fail-closed, skip-any proibido, CORRUPTLOG, DST, `pedra verify`). **Não** em maturidade de campo, TLS-by-default, encrypt-at-rest no engine, TransactionDB pessimista, CFs físicos, nem prova seL4. |
| 3 | `rocksdb-compat` tem **as mesmas features exatamente**, só que mais rápido e mais seguro? | **Não exatamente.** Nomes rust-rocksdb 0.22 estão; layout é prefix-CF; dezenas de `set_*` compilam e **não fazem nada**; `BackupEngine` / `Checkpoint` / `TransactionDB` (2PL) **não existem** no crate. Alguns caminhos são *mais* seguros de propósito (tombstone vs unlink; skip-any ausente). |
| 4 | Prontos para lançar e chocar o mundo? | **Não.** Lab-maduro para early-adopter *com texto honesto*. Não é drop-in de campo, não é 2× em tudo, não é o C++ Rocks. Publicar o contrário quebra G8 e o próprio AGENTS.md. |

O resto deste relatório é a evidência e o mapa do que falta. O RFC-0062 fatia o fechamento.

---

## 1. O que “paridade” significa neste repo (três colunas, nunca uma)

Misturar as colunas é como o Pedra perde a conversa. Três contratos:

| Coluna | Pedra | Rocks | O que mede | Pode vender como “batemos o Rocks”? |
|---|---|---|---|---|
| **A. Drop-in same-class** (RFC-0054 default do compat) | WAL `write()`, sem fdatasync no Ok (`PEDRA_PARITY_ASYNC=1`) | `sync=false` | velocidade do motor, mesma classe de durabilidade | **Não** — é o gate de regressão, não o cartaz G1 |
| **B. Produto G1** (kernel / `set_sync(true)`) | `fdatasync` (Darwin: `F_FULLFSYNC`) **antes** do Ok | o mesmo `sync=false` | o produto: mais durável **e** o que isso custa | Só as células que passam; writes 1c não passam por física |
| **C. Sync-peer** (`ROCKS_PARITY_SYNC=1`) | G1 | `sync=true` | same-class com o Rocks que quase ninguém liga | **Nunca** “batemos o Rocks” |

Decisão de produto **2026-08-24** ([RFC-0041](../rfc/0041-2x-rocks-default.md) “Floor decision”): o **gate oficial** passou de 2× para **1×** na coluna A. Os slices de 2× continuam backlog, não shipping gate. AGENTS.md foi atualizado. O documento [pedra-vs-rocksdb-performance.md](../pedra-vs-rocksdb-performance.md) ainda fala do cartaz 2× G1 — está **stale**; não citar.

`1×` = paridade mínima. `2×` = o piso que o RFC-0041 *pedia* e que o Linux quase entrega na coluna A. “Completa” = todas as shapes oficiais, nas duas colunas A e B, em Mac **e** Linux **e** Intel, mediana ≥3 rounds quietos.

Nenhuma dessas três “completas” está verde.

---

## 2. Performance — números, não slogans

### 2.1 Coluna A (drop-in async vs Rocks default) — o gate de 1×

**Mac, bateria floor-1×** ([findings/rocks-parity-floor1x](../../findings/rocks-parity-floor1x/README.md), 2026-08-24, 15 shapes gated):

| recorte | ratio | nota |
|---|---:|---|
| mínimo | **1.254** (`deps_raftlog`) | 15/15 ≥ 1.0 — gate 1× **PASS** |
| mediano do cartaz | ~2.4–3.5 | YCSB A 3.48, B 3.28, C 2.64, F 3.28 |
| leituras quentes | 2.3–3.3 | E 2.34, C_unif 2.72 |
| `deps_scan` | 1.255 | depriorizado como meta 2× (2026-08-24) |
| `deps_mvcc_latest` | 1.438 | neste run; rearm10 quieto tinha 2.53 |

Isto é **paridade mínima no Mac, coluna A, nesta caixa**. Não é 2× em tudo. Não é G1.

**Mac, bateria quieta rearm11** ([RFC-0054](../rfc/0054-close-official-gaps.md)):

| shape | × Rocks default | estado RFC-0054 |
|---|---:|---|
| `deps_raftlog` | **1.04** (3/3 rearm9) | P0.2 fechado (>1×) |
| `deps_apply_batch` | **2.07** (3/3 rearm11) | P1.4 fechado |
| `deps_mvcc_latest` | **2.53** (3/3 rearm10) | P1.1 fechado |
| `kvrocks_blob_set` | 1.25 | P1.2 **parked** (dono 2026-08-24) |
| `deps_scan` | 1.89 | P1.3 **parked** |

**Linux/AMD 4 vCPU** (Threadripper PRO 3975WX, kernel 6.12.94-virt — [RFC-0059 substitution](../rfc/0059-substitution-anti-overindex-linux-gate.md), [linux-anti1](../../findings/2026-08-24-linux-anti1/README.md)):

| shape | Linux mediana | vs 2× | vs 1× |
|---|---:|---|---|
| YCSB A–F, lock, overwrite, mvcc, kvrocks get/set/scan/pipeline | 2.02–32.7 | PASS 13/16 | PASS |
| `deps_apply_batch` | **1.87** (isolado diag-5: 1.95) | FAIL | PASS |
| `deps_raftlog` | **0.91** (diag-5: **0.78**) | FAIL | **FAIL** |
| `deps_scan` | 2.11 | OPEN (não-gated) | PASS |
| `kvrocks_blob_set` | 1.21 | OPEN | PASS na mediana, rounds 0.85–1.4 |
| anti-overindex `ycsb_c_big` (2^20 keys) | 2.46 (−22% vs zipf) | dentro do corte 30% | PASS |

Leitura: o Mac **não** inflava a favor. Linux mantém ou melhora o piso 2× em 13 formas. Os dois buracos oficiais no Linux são write-heavy de batch (`apply` colado no 2×, `raftlog` abaixo de 1×).

**Linux `pwrite` (imposto io_uring removido)** ([linux-diag6-pwrite](../../findings/2026-08-24-linux-diag6-pwrite/README.md), fonte `17d10c4`, 5 rounds `deps_raftlog`):

| | |
|---|---|
| mediana | **0.979** (min 0.892, max 1.070; 2/5 >1×) |
| p50 | **empatado** ~10.0–10.4 µs vs Rocks 10.1–11.5 |
| p99 quieto | ~32 µs vs ~20 µs do Rocks |

O caminho de append **empatou no p50**. 2× neste shape, 1 cliente, sem mudar o insert da memtable, **não existe neste silício**. O gap que resta é p99 / memtable / encode, não o ring.

**Substituição real (SurrealDB 1.5.4 intacto, só `[patch.crates-io]`)** — Linux pós-F222 ([sub-surreal-linux-f222](../../findings/2026-08-24-sub-surreal-linux-f222/)):

| perna | × Rocks default |
|---|---:|
| point_write | 2.41 |
| point_read | 1.79 |
| doc_txn | 1.40 |
| scan | 1.33 |

4/4 >1×. Surreal v2.x (SstFileManager / Env / disk_space_manager) **não** foi exercitado.

### 2.2 Coluna B (G1 vs Rocks default) — o cartaz de produto

[findings/rocks-parity-floor1x-g1](../../findings/rocks-parity-floor1x-g1/README.md), Darwin, F_FULLFSYNC p50 ≈ **3.8 ms**:

| classe | shapes | ratio |
|---|---|---|
| leituras puras | C, C_unif, C_big, mvcc, scan | **1.13–1.99×** com durabilidade mais forte |
| qualquer shape com write 1c | A, B, D, F, apply, raftlog, overwrite, lock, E | **0.001–0.022×** |

Isto não é motor lento. É o contrato: uma barreira completa por op contra **zero** do peer. O mesmo teto no Linux com `fdatasync` 25.7 µs levou YCSB A 1c a **0.056×** (RFC-0041 P0.2) — ainda ≪ 1×. Afirmado em código: `rfc0041_one_fdatasync_cannot_hit_2x_rocks_default_ycsb_a`.

O fechamento G1 **já medido**: group commit, uma barreira por *grupo*. `deps_apply_batch_mc4` **2.788×** (head3, RFC-0041 P1.1). É exatamente o kernel provado do RFC-0057 P2.1.

### 2.3 O que “paridade completa, no mínimo, idealmente mais” exigiria

| critério | hoje | fecha? |
|---|---|---|
| Coluna A ≥ 1× em **todas** as oficiais, Mac | sim (15/15 ≥ 1.254) | já |
| Coluna A ≥ 1× em **todas**, Linux | **não** — raftlog 0.78–0.98 | P0.4 + P1.1 do RFC-0062; p50 já empatou, precisa de 3/3 >1× (cauda/H4), não de 2× |
| Coluna A ≥ 2× em **todas**, Linux | **não** — apply ~1.95, raftlog ~1× | apply talvez; raftlog 2× **recusado por física de p50** |
| Coluna A ≥ 2× scan/blob | parked pelo dono | só se des-estacionar |
| Coluna B ≥ 1× writes 1c | **impossível** sem largar G1 ou o peer | nunca neste formato; o cartaz é group-commit + tabela publicada |
| Intel (não-AMD) | **zero baterias** | P1.3 |
| 5× async (RFC-0044) | não é o cartaz; várias reads já >5× (`kvrocks_scan` 32×) | não bloqueia lançamento |

### 2.4 Recusas de “atalho 2×” (não reabrir)

Não fecham o cartaz RFC-0041 / não são o lançamento:

journal-CF como 2× (LEDGER L44), SAUCR skip-fsync, Magma/KVell/FASTER/Raft Engine/Aurora vs `sync=true` (L45–L50 FALSE), TRIAD/SpanDB-SPDK (L79/L80), skiplist-in-arena como milagre, io_uring pipeline no write (já falsificado: o imposto era `submit_and_wait`; pwrite empatou p50).

---

## 3. Garantias — mais fortes, iguais, mais fracas

Fonte canônica: [rocksdb-vs-pedradb-guarantees.md](../rocksdb-vs-pedradb-guarantees.md), [robustness-nine-axes.md](../robustness-nine-axes.md), [RFC-0060](../rfc/0060-field-and-hardware-residuals.md), [RFC-0061](../rfc/0061-residuals-sel4-ironfleet.md).

### 3.1 Onde o Pedra é mais forte (default ou tipo)

| # | Garantia | Pedra | Rocks default |
|---|---|---|---|
| G1 kernel | `OpenOptions.sync=true` — Ok ⇒ WAL fsyncado | ligado | `WriteOptions.sync=false` — Ok pode estar só no page cache |
| Fence | `DurabilityFenced` incondicional, resultado incerto **tipificado** | sempre | depende de `paranoid_checks`; com `false`, continua escrevendo no meio falho |
| WAL mid-corrupt (kernel) | CRC → `Err` no open, fail-closed | default kernel | `kPointInTimeRecovery`: abre, descarta o sufixo, **segue** |
| Skip-any | **ausente** (G2) | — | `kSkipAnyCorruptedRecords` existe |
| CORRUPTLOG | 3º evento recusa open em **qualquer** modo | sim | sem equivalente |
| Compat PIT | prefixo servido **com relatório** (`last_recovery_report`) | RFC-0047 | PIT do Rocks é log-only no default |
| `delete_file_in_range` | tombstone + compact; **nunca** unlink de SST | mais seguro | unlink de ficheiro |
| CAS | `put_if_absent` / `put_if_eq` no kernel | first-class | TransactionDB / Merge |
| DST | FailingEnv + World + BitFlip + Miri/TSan/ASan + TCG smoke | produto | FaultInjectionTest de teste, não seam |
| Scrub | `pedra verify` percorre SST/vlog/WAL/MANIFEST/CURRENT/history/backup | RFC-0060 | `verify_checksums` no read path; sem scrubber de produto equivalente neste crate |
| Formal | kernels extraídos, twins, freeze TCB, residual publicado | RFC-0056/0061 | nenhum |

### 3.2 Onde somos iguais (contrato)

Ok com `sync=true` ⇒ WAL fsyncado. WriteBatch/TX = um registro atômico. MANIFEST tmp+fsync+rename+sync_dir. Torn tail do WAL → prefixo limpo. LOCK de processo. Checkpoint/backup com verify. Change feed ≈ `GetUpdatesSince`.

### 3.3 Onde o Rocks (e o campo) ainda ganham

| área | detalhe | bloqueia lançamento? |
|---|---|---|
| Anos de frota | Meta + comunidade, discos maus de verdade, firmware, ENOSPC real, plug-pull | **sim** para claim GA / “chocar o mundo” como substituto de campo. **não** para preview lab. Nine-axes eixo 1: “Not programmable.” |
| TransactionDB 2PL + deadlock + WritePrepared | Pedra tem OCC + single-writer TX | sim para hosts que são TransactionDB, não Optimistic |
| Concurrent memtable / pipelined write / N imm | setters no-op; RFC-0055 mediu que 1c oficial não paga | não no cartaz 1c; sim se o host 50-writer for o cliente |
| CFs físicos, Titan, UDT, per-CF block cache | prefix `cf\0key`; knobs Titan no-op | sim para TiKV-como-produto; não para KV simples |
| Recuperação fina | severidades + auto-resume ENOSPC + quarentena de file | blast radius = DB inteiro (open-items §2.6, footgun #1) |
| TLS default / encrypt-at-rest | TLS opt-in `--features tls`; disco = LUKS ops | **sim** para Montanha GA (RFC-0021). Single-node embed menos. |
| Guest TCG required | `C2.2=residual_no_guest` enquanto não houver `PEDRA_QEMU_SSH` | residual contínuo, não shipping gate de crate |
| seL4 / IronRSL | RFC-0061: mesma **classe de claim**, não de **garantia** | frase proibida: “tão robusto quanto seL4” |

### 3.4 Duas faces, dois defaults — a armadilha de lançamento

| | kernel `pedradb_core::Db` | drop-in `rocksdb-compat::DB` |
|---|---|---|
| `sync` default | **true** (G1) | **false** (Rocks-shaped) |
| WAL recovery | FailClosed | PointInTime + relatório |
| Version GC | história (F20) + archive | `auto_reclaim=true` (perfil Rocks) |

Um binário que `use rocksdb::…` pelo alias **não** está no G1. Um que chama `Db::open` **está**. Lançar sem esta tabela na primeira página do README é mentira por omissão: o utilizador do drop-in pensa que comprou G1 e comprou o default do Rocks; o utilizador do kernel no Mac paga 3.8 ms/`F_FULLFSYNC` e acha o motor lento.

RFC-0038 **P1.1 ainda é decisão do dono**: FailClosed vs PIT-como-produto vs Evacuate-read-only (B2). A superfície está travada em dois modos nomeados (RFC-0050 P0.7). Sem esta decisão, o launch note não sabe o que recomendar a um operador com WAL a meio corrompido.

---

## 4. `rocksdb-compat` — “as mesmas features exatamente”?

**Não.** O crate é um *subset nomeado* do rust-rocksdb **0.22** sobre `ConcurrentDb`. Compila o alias. Não é o C++ Rocks. Não é Titan. Não é layout on-disk compatível.

### 4.1 O que está de verdade (observable KV)

`open` / `open_cf` / `put` / `get` / `delete` / `delete_range_cf` / `WriteBatch` atómico / snapshot / iterator janela 64 / flush / compact / `SstFileWriter` + ingest (via WAL+flush, G1) / `WriteBatchWithIndex` / compaction filter / `create_cf` `drop_cf` `list_cf` / `multi_get` / `get_pinned` / `merge` (full merge; partial ignorado) / `OptimisticTransactionDB` / `live_files` / `key_may_exist` / `resume()` tipado / blob files ligados a `VALUES.vlog`.

Prova de substituição: SurrealDB **v1.5.4** `kvs::tests` 62/62 no shim + bench 4/4 >1× no Linux.

### 4.2 O que compila e mente (no-op)

Aceites e **inertes** (RFC-0047 P2.1, RFC-0055 P2.1): `set_use_fsync`, `increase_parallelism`, `set_max_background_jobs`, `set_enable_pipelined_write`, `set_allow_concurrent_memtable_write`, `set_max_write_buffer_number`, compression/level/bloom-bits/`set_block_cache`/`set_checksum_type`, Titan (`set_blob_file_size` rotate é o único pedaço numerado), timestamps (`set_timestamp`, `set_read_timestamp_for_validation`), `set_verify_checksums`, `set_async_io`, prefix extractor, `optimize_for_point_lookup`, …

Um host que faz `opts.set_block_cache(&Cache::new_lru_cache(1<<30))` **acha** que comprou 1 GiB de block cache. Comprou um no-op. Isto é o oposto de “mais seguro”: é silent-wrong operacional (não de bits, de capacidade).

`ErrorKind::NotSupported` existe precisamente para “nunca Ok: faking these is silent-wrong”. Quase nenhum setter o usa. Usam `{}`.

### 4.3 O que é de propósito *diferente* (mais seguro, não drop-in bit-a-bit)

| Rocks | Pedra compat | porquê |
|---|---|---|
| `delete_file_in_range` unlinks SST | tombstone + compact | unlink sem tombstone = silent-wrong |
| skip-any WAL | ausente | G2 |
| `paranoid_checks=false` | sem equivalente | G2 |
| CFs = diretórios/ficheiros | prefix `cf\0key` + CFREG | engine único; scans isolados por bound |
| ingest = link SST externo | re-aplica + flush | G1; não é SST Rocks on-disk |

### 4.4 O que simplesmente não existe no crate

| API rust-rocksdb / Rocks C++ | estado | quem precisa |
|---|---|---|
| `TransactionDB` (pessimista, 2PL, deadlock) | **ausente** (só Optimistic/OCC) | TiKV-style, CRDB-style local |
| `BackupEngine` / `backup` crate API | **ausente** (ops é `pedra backup`, outro nome) | hosts rust-rocksdb de backup |
| `Checkpoint` tipo rust-rocksdb | **ausente** (core `create_checkpoint` existe, não o tipo) | hosts que chamam `Checkpoint::new` |
| `Env` plugável / `SstFileManager` / rate limiter | **ausente** | Surreal **v2.x**, TiKV |
| user-defined timestamps | setter no-op | TiKV 6+ |
| per-CF `Options` reais (cache, compaction) | um `Options` global | qualquer CF-tuning |
| wide columns / secondary instance / FIFO compaction | ausente | niche |
| JNI / C++ ABI / SST file format Rocks | recusado (clean-room) | “abre o diretório do Rocks” |

**TiKV-as-a-product** continua integração de raftstore, não um `pub fn` em falta. Não reivindicar cluster TiKV a partir da tabela de API ([rocksdb-compat.md](../rocksdb-compat.md)).

### 4.5 “Mais performático e mais seguro” no compat — quando é verdade

- **Mais rápido, coluna A, a maior parte das shapes:** sim, números §2.1.
- **Mais rápido, coluna B, 1c write:** não, e não vai ser.
- **Mais seguro que o default do Rocks:** sim no kernel G1 e no fail-closed; o *drop-in* default é a mesma classe async do Rocks (de propósito, RFC-0054 P0.0).
- **Mais seguro que um Rocks bem configurado (`sync=true`, PIT, paranoid):** no fence e no skip-any, sim; em recuperação fina e frota, não.
- **Mais seguro que o próprio setter no-op:** não — o no-op é o buraco.

---

## 5. Nove eixos — lab-maduro, não GA

[robustness-nine-axes.md](../robustness-nine-axes.md), atualizado 2026-08-24:

| eixo | shipped | residual que impede “chocar o mundo” |
|---|---|---|
| 1 Campo | DST, FailingEnv, Darwin fd class, uring soak | anos multi-tenant, firmware, plug-pull real |
| 2 Disco Linux | uring CQE, TCG blkdebug EIO | `dm-error` no host, ECC |
| 3 Disponibilidade | ENOSPC/EIO nomeados, fences | frota bg-error, compact multi-TB |
| 4 Escrita | group commit, apply após fd | não é concurrent memtable Rocks |
| 5 Cluster | majority lab, TLS opt-in | **não** multi-Raft de produção; cleartext default; sem `montanha-secure` + fleet = sem GA |
| 6 Ops | backup/PITR local | sem backup multi-região; compat sem `BackupEngine` |
| 7 WAL mid-corrupt | exactamente 2 modos | P1.1 dono (0038) |
| 8 Face Rocks | API 0.22 subset | prefix-CF, Titan no-op, sem cluster TiKV |
| 9 Disco/wire | LUKS ops; TLS se flags | **engine sem encrypt-at-rest**; TCP lab = cleartext |

Formal (RFC-0061): não somos seL4. Frase permitida: “kernels de decisão correct wrt spec; glue TCB; DST ataca o resto.” Frase proibida: “sem bugs” / “tão robusto quanto seL4”.

---

## 6. Estamos prontos para lançar?

Depende do *quê* se lança. Três produtos, três respostas.

### L0 — “substituto completo do Rocks, 2×, mais seguro, chocar o mundo”

**Não.** Falta: raftlog Linux ≥1× 3/3 e honestidade sobre o teto 2×; apply Linux ≥2×; Intel; knobs que não mintam; APIs rust-rocksdb que hosts reais chamam (`BackupEngine`, `Checkpoint`, TransactionDB se o host for esse); decisão 0038; TLS default se o cartaz for cluster; encrypt-at-rest se o cartaz for “mais seguro em disco”; anos de campo; CFs físicos / Titan se o cartaz for TiKV. Física: G1 1c write nunca 2× vs default.

### L1 — preview crate, early adopter, texto honesto

**Quase.** Já existe: alias rust-rocksdb 0.22, Surreal 1.5.4, coluna A ≥1.25× no Mac, 13/16 ≥2× no Linux, G1 reads ≥1.13×, group-commit 2.79×, DST+verify+CORRUPTLOG, formal residual publicado.

Falta **antes** de `crates.io` / anúncio: (1) matriz de claims que um jornalista não consiga deturpar; (2) classificar cada `set_*` como Wired / Inert-documentado / `NotSupported`; (3) remesura Linux oficial com o `pwrite` atual; (4) `publish = false` no shim `rocksdb` 0.21 **fica** — o nome `rocksdb` no crates.io não é nosso.

L1 **não** é chocar o mundo. É “tem um drop-in Rust, mais rápido na maior parte, mais estrito no kernel, knobs inertes listados, não é o Rocks C++”.

### L2 — produção single-node para um host nomeado (ex.: Surreal 1.5, um KV embarcado)

**Não hoje, sim depois do RFC-0062 P0+P1.** Precisa de L1 + raftlog Linux >1× 3/3 + apply Linux no piso 2× ou texto “1.95, custo real” + Intel ou “AMD only até P1.3” + 0038 decidido + Checkpoint/BackupEngine *nomes* se o host os chama + nenhum setter que enfraqueça checksum/sync em silêncio.

### L3 — Montanha / cluster GA

**Não.** RFC-0021: TLS 1.3 mTLS ainda não é default; health HTTP localhost-only é o piso; encrypt-at-rest é ops. Nine-axes eixo 5: “No GA without `montanha-secure` + fleet.” Joint consensus parked (RFC-0061 P2.3). Isto é outro lançamento.

---

## 7. O que falta, priorizado (entrada do RFC-0062)

### P0 — o que um lançamento mentiroso faria, e nós não fazemos

1. **Matriz de claims congelada** (este relatório) apontada do README / AGENTS / open-items. Sem isto, o próximo post cita 2× G1 ou “15/15 no Linux”.
2. **Inventário de knobs** Wired | Inert | NotSupported | Safer-divergent, testado. `set_verify_checksums(false)` e amigos **não** podem ser `{}`.
3. **Remesura Linux oficial** 3 rounds × shapes gated, com o `pwrite` que já está no `IoUringFile::write`. O 0.78× é pré-pwrite; o 0.979 é diag isolado, não a bateria de cartaz.

### P1 — o que torna L2 honesto e numericamente mínimo

1. `deps_raftlog` Linux **>1× mediana 3/3** (não 2×). Hipótese viva H4: flush-bg da memtable 1.6M nas 4 vCPUs; p50 já empatou.
2. `deps_apply_batch` Linux **≥2× 3/3** ou, se o diag mostrar custo real irredutível, o cartaz oficial aceita 1.95 com mecanismo nomeado (já é a leitura da diag-5).
3. **Bateria Intel** (RFC-0059: a VM é AMD; “o Mac enviesa?” está respondido; “e Intel?” não).
4. **RFC-0038 P1.1** — o dono escolhe A / B2 / B1+D. Sem escolha, o runbook de corrupção é um RFC aberto.
5. Superfície rust-rocksdb que hosts 0.21/0.22 **chamam no compile**: `Checkpoint`, `BackupEngine` wrapping `pedradb-ops`. Não inventar 2PL.
6. Banner no [pedra-vs-rocksdb-performance.md](../pedra-vs-rocksdb-performance.md) (stale 2× G1).

### P2 — o que “chocar o mundo” ainda pediria, e o que recusamos

1. TLS default no binário de cluster (`montanha-secure` vira o único binário publicado).
2. Encrypt-at-rest no engine **ou** declaração permanente “LUKS/volume, nunca no LSM”.
3. Des-estacionar scan/blob ≥2× **se** o anúncio disser “todas as shapes 2×”.
4. CFs físicos **iff** um upper DB nomeado falhar no prefixo (hoje Surreal 1.5 não falha).
5. `crates.io` de `rocksdb-compat` (nome **não** `rocksdb`) depois de P0+P1.
6. Campo: não se programa. Soak + `pedra verify` em cron é o substituto.

**Fora de escopo (não fecha lançamento, não reabrir):** journal-CF 2×, seL4, extração total, SSI, ARIES, TrueTime, Calvin sequencer, Titan, skip-any, 2× em YCSB A 1c G1, cluster TiKV, FDB field peer, Surreal v2.x neste RFC.

---

## 8. Frases permitidas / proibidas no anúncio

**Permitidas hoje (com citação):**

- “Drop-in rust-rocksdb 0.22 **subset** sobre Pedra. SurrealDB 1.5.4 substitui o crate `rocksdb` via patch, 4/4 pernas >1× vs Rocks default no Linux.”
- “Coluna same-class (async vs `sync=false`): 15/15 ≥ 1.25× no Mac; 13/16 ≥ 2× no Linux/AMD. `deps_raftlog` Linux ainda ~1× no p50.”
- “Kernel default fsync-before-Ok. Leituras G1 1.13–1.99× o Rocks default com durabilidade mais forte. Writes 1-cliente pagam a barreira; group commit fecha (apply_mc4 2.79×).”
- “CRC fail-closed, CORRUPTLOG, `pedra verify`, DST in-tree. Não é seL4.”

**Proibidas:**

- “Paridade completa de performance com o RocksDB.”
- “As mesmas features exatamente.”
- “2× em tudo.”
- “Batemos o Rocks” apontando coluna sync ou coluna A como se fosse G1.
- “Pronto para produção / GA / substituto de campo.”
- “Tão robusto quanto seL4.” / “sem bugs.”
- “TiKV drop-in.” / “abre um diretório Rocks C++.”

---

## 9. Índice de evidência

| facto | onde |
|---|---|
| peer = `sync=false` | AGENTS.md, `rocks-parity-compare` exit 2 |
| floor 1× coluna A, 15/15 ≥ 1.254 | findings/rocks-parity-floor1x, RFC-0041 FD 2026-08-24 |
| G1 1c writes 0.001–0.022× Darwin | findings/rocks-parity-floor1x-g1 |
| G1 1c A 0.056× Linux fd | RFC-0041 P0.2 |
| apply_mc4 2.788× | RFC-0041 P1.1 |
| rearm9/10/11 raftlog/mvcc/apply | RFC-0054, findings/2026-08-23-rearm{9,10,11} |
| Linux 13/16 ≥2×, raftlog 0.91, apply 1.87 | RFC-0059, findings/2026-08-24-linux-anti1 |
| pwrite raftlog mediana 0.979, p50 tied | findings/2026-08-24-linux-diag6-pwrite |
| Surreal 1.5.4 4/4 >1× | findings/2026-08-24-sub-surreal-linux-f222 |
| knobs inertes | crates/rocksdb-compat/src/lib_kernel.rs `set_*`, RFC-0047, RFC-0055 |
| nine axes lab ≠ GA | docs/robustness-nine-axes.md |
| TLS não default | RFC-0021 P2.3 |
| 0038 P1.1 aberto | RFC-0038 |
| não somos seL4 | RFC-0061 |
| scrub at-rest | RFC-0060 |

---

**Próximo passo:** executar [RFC-0062](../rfc/0062-launch-readiness-remaining-gaps.md) P0. Não anunciar. Não publicar crates.io. Não reabrir L44.
