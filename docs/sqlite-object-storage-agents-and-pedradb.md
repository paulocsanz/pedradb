# SQLite em object storage e PedraDB

> **Status:** relatório independente, 2026-08-12. Não é decisão de produto.
> **Correção de tese:** “vários por aí” = o mercado (Turso, Rivet, Fly/Litestream,
> mvSQLite, Cloudflare, …) tornando SQLite paralelo e horizontal em cima de
> objeto. **Não** é um relatório sobre workloads de LLM/agente. AgentFS/Willow
> aparecem só como *um* uso do mesmo mecanismo.
> **Método:** fontes primárias + código atual. O doc
> [`object-storage-as-substrate-possibility.md`](object-storage-as-substrate-possibility.md)
> (2026-08-11) **não basta** — omite a categoria SQLite-VFS + compute/storage
> split, afirma SST “ainda não existe” (já existe), e descreve Tigris de forma
> errada.
> **Não é prova de maturidade Rocks/Pebble/FDB.**

Extrações das fontes: [`references/sqlite-object-storage-primaries.md`](references/sqlite-object-storage-primaries.md).

---

## 0. Resposta em uma página

A tendência que você está vendo é real e tem **três camadas distintas**.
Misturá-las produz o diagnóstico errado.

| Camada | O que é | Quem está construindo | Paralelismo |
|--------|---------|------------------------|-------------|
| **A. SQLite VFS + storage desagregado** | SQLite intacto; páginas via VFS. Compute sem disco local (ou disco só como cache). Object storage é verdade fria e/ou WAL quente. | Turso Cloud diskless, Rivet, mvSQLite, Litestream VFS, Cloudflare DO/D1, sqlite-s3vfs/Turbolite | Dois eixos: **N bancos, 1 writer cada** (isolamento) **ou** páginas de *um* banco num KV distribuído (mvSQLite). Não é multi-writer no arquivo SQLite. |
| **B. LSM nativo em S3** | WAL/SST/manifest *são* objetos. Sem SQLite. | SlateDB, Tonbo, turbopuffer, WarpStream | 1 writer por namespace + CAS no manifest. |
| **C. Bloco/POSIX em cima de objeto** | Fingir disco. Qualquer DB (Postgres, SQLite, Pedra) abre um “volume”. | JuiceFS, CubeFS, VAST NVMe/TCP, Mountpoint, “elastic storage” de plataforma | Escala o *volume*, não o *banco*. Replica de volume continua difícil. |

**Onde PedraDB ajuda de verdade:** não como “mais um SlateDB”, e não
substituindo o SQL/SQLite na borda. Ajuda como **o KV quente / o FDB
debaixo da VFS / o motor de metadados do POSIX em objeto** — o slot que
Rivet hoje preenche com Rocks/Postgres/FDB, que mvSQLite preenche com
FoundationDB, e que JuiceFS preenche com TiKV/FDB/Redis.

**O gap mais profundo:** Pedra é um kernel POSIX de **um diretório = um
banco**, com `LOCK` exclusivo, `fsync` por commit, e Montanha ainda em
Raft in-process (RFC-0017 P0 de transporte TCP está `todo`). Os sistemas
que estão ganhando essa tendência precisam de **milhões/bilhões de bancos
como linhas num store compartilhado**, open instantâneo sem restore, e
compute que morre sem levar o disco. Isso não é um `Env` apontado para
S3. É outro contrato.

---

## 1. Por que o material local não era confiável

Verificado contra código e primários nesta sessão:

| Afirmação local (2026-08-11) | Verdade agora |
|------------------------------|---------------|
| SST/compaction “not built yet” | **Falso.** `sst/table.rs` escreve v4 (blocos 4 KiB, bloom, lz4, decode lazy). `Db::flush` / `compact` / `compact_vlog` existem. MANIFEST+CURRENT+checkpoint existem. |
| Foco em SlateDB / WarpStream / turbopuffer / Tigris | **Incompleto.** O que vários por aí estão usando para *paralelizar SQLite* é VFS + compute/storage split (Rivet, Turso diskless, mvSQLite, Litestream VFS, DO/D1) — não um LSM nativo em S3. |
| “Tigris é FDB vestido de S3” | **Errado.** Docs oficiais: FDB guarda *metadados*; o *conteúdo* do objeto vai para **block store** regional; API S3 na frente. É o padrão JuiceFS de cabeça para baixo (objeto na API, bloco no data plane). |
| Latências turbopuffer (cold 874 ms, warm 14 ms, piso ~10 ms) | **Confirmado** em <https://turbopuffer.com/docs/architecture>. Write no primário: p50 = **165 ms** / 500 kB, 1 WAL entry/s — o doc local arredondou para “até 200 ms”. |
| SlateDB WAL batch + epoch fencing + manifest ~5.6 MB | **Confirmado** no RFC-0001. Cuidado: o RFC é de quando compaction *ainda não existia*; tratar o RFC como o binário de hoje é impreciso. |
| Non-goal “object-store-first no kernel” | **Continua correto** para o path de commit local. Não implica “Pedra não tem papel nessa tendência”. |

---

## 2. O que os sistemas de SQLite+objeto *realmente* fazem

### 2.1 Como “vários por aí” tornam SQLite paralelo / horizontal

Não é um SQLite mágico com N writers no mesmo arquivo. Dois mecanismos,
às vezes combinados:

**Isolamento (N bancos).** Turso, Rivet, DO/D1, Litestream: um tenant /
Actor / conexão = um SQLite. Single-writer por arquivo (limite do
SQLite). Escala = mais compute reivindicando mais arquivos, sem mover
disco. Multi-tenant no mesmo *nó* (Turso batcha dezenas–centenas de DBs
num PUT de S3 Express).

**Páginas num KV distribuído (um ou N bancos).** mvSQLite: VFS mapeia
página → chave no FoundationDB, OCC de página, leitores/escritores em
nós distintos sem arquivo local. Um banco grande *também* paraleliza
writes finos; não só o truque “um arquivo por tenant”.

Os dois exigem a mesma coisa de infra: compute sem volume casado,
páginas sob demanda, object storage (ou um KV replicado) como verdade.

Rivet (2026-07-31), a constraint que manda:

> Zero-disk compute for elastic scaling. Machines running Actors hold no
> volumes and no local state.

Willow/mvSQLite, sobre o formato de armazenamento (o workload de agente
é só o exemplo deles):

> Making a transaction on an mvSQLite database is more like reading or
> writing an EBS volume. The more common cloud-SQLite pattern keeps a
> synchronized local file… right shape for one big database with many
> readers, and the wrong shape for a million small databases that any
> node might need to claim a moment from now.

Quem trata “SQLite em S3” como “um arquivo gigante que muitos escrevem
direto no objeto” está resolvendo o problema errado — e os VFS crús
(sqlite-s3vfs, Turbolite) assumem isso e avisam: sem locking, risco de
corrupção.

### 2.2 Seis arquiteturas, não uma

```
                    ┌─────────────────────────────────────────┐
                    │  app / tenant / Actor / DO / cell        │
                    │  SQL SQLite intacto                      │
                    └──────────────────┬──────────────────────┘
                                       │ SQLite VFS
                                       │ xRead / xWrite / xSync  (páginas 4 KiB)
           ┌───────────────┬───────────┼────────────┬──────────────┐
           ▼               ▼           ▼            ▼              ▼
     (1) disco local   (2) Litestream (3) Rivet   (4) mvSQLite  (5) páginas
         + backup          VFS RO        hot tier    → FDB         = objetos
         async S3          LTX/S3        Rocks/PG/   páginas KV    sqlite-s3vfs
                                         FDB + S3                  Turbolite
                                         frio
           │
           ▼
     (6) POSIX-em-objeto (JuiceFS / VAST / elastic volume)
         o SQLite nem sabe que embaixo é S3
```

| # | Sistema | Write path | Read path | Compute sem disco? | Multi-tenant extremo? | Fonte |
|---|---------|------------|-----------|--------------------|------------------------|-------|
| 1 | Litestream clássico | fsync local, ship async LTX → S3 | disco local | Não (primário tem o arquivo) | Não | Fly blog 2025-12 |
| 2 | Litestream VFS | não escreve (RO + PITR) | Range-GET de páginas LTX, LRU | Leitura sim | Replica de *um* banco | idem |
| 3 | Turso Cloud diskless | group-commit multi-DB → **S3 Express** (~6 ms PUT 4k); disco é cache write-through | cache local; miss = S3 Express | Sim | Sim (amortiza PUT em ~100 DBs ativos/nó) | turso.tech 2025-04-07 |
| 4 | AgentFS disaggregado | WAL local + sync lógico; S3 source of truth; blobs grandes → S3 | pull de páginas/WAL | Objetivo (ainda direção) | Por arquivo SQLite | penberg.org 2026-01-11 |
| 5 | Rivet Actors | VFS → **hot tier replicado** (Rocks/PG/FDB), *nunca* S3 no commit; chunks 256 KiB | cache in-proc → hot → rehydrate S3 | Sim | Sim (bilhões de DBs = linhas, não volumes) | rivet.dev 2026-07-31 |
| 6 | mvSQLite / Willow | VFS → FDB (OCC de página); blobs → S3 | FDB; time-travel por versão | Sim (cliente stateless) | Sim (1k agentes/processo) | github.com/losfair/mvsqlite |
| 7 | Cloudflare DO SQLite | SQLite no mesmo thread; replicate off-machine; stream p/ objeto | local, ~zero | Não (DO pinado no DC; CF provisiona na frente) | Sim, mas **teto 10 GB/DB** | blog.cloudflare.com 2024-09-26 + Rivet FAQ |
| 8 | sqlite-s3vfs / Turbolite | página = objeto (ou bloco) | Range/GET | “Sim” mas sem locking real | Ruim (1 PUT / página) | github; HN Turbolite: *experimental, may corrupt* |
| 9 | SlateDB | batch WAL-SST → objeto + CAS manifest | SST objects + cache | Sim (1 writer) | Ruim p/ bilhões de DBs pequenos (overhead de manifest/compaction *por DB*) — Rivet diz isso explicitamente | slatedb.io RFC-0001 |
| 10 | JuiceFS-class | POSIX write → cache → blocos ~4 MiB no objeto; metadata noutro DB | cache + GET | O *cliente* é stateless; o *metadata engine* não | Escala arquivos; bilhões de SQLites pequenos só se o metadata aguentar | juicefs.com |

Números de física do meio (não opiniões):

| Operação | Latência citada | Fonte |
|----------|-----------------|-------|
| EBS gp3 read/write | ~1 ms | Rivet |
| `fsync` local (ordem de grandeza) | ~2 ms | Turso diskless |
| S3 Express PUT 4 KiB same-AZ | avg 6.4 ms, p99 7 ms | Turso, 1000 samples, us-east-1 |
| S3 Express GET 4 KiB same-AZ | avg 3.8 ms, p99 4 ms | idem |
| S3 Standard PUT 4 KiB | avg 31 ms, p99 102 ms | idem |
| S3 Standard GET 4 KiB | avg 19 ms, p99 42 ms | idem |
| S3 Standard (Rivet) | read 30–45 ms, write 30–100 ms | Rivet |
| turbopuffer write 500 kB | p50 165 ms, ≤1 WAL/s | turbopuffer docs |
| turbopuffer cold 1M docs | p50 874 ms | idem |
| turbopuffer warm | p50 14 ms; consistência ~10 ms de piso | idem |
| Custo S3 Standard PUT | $0.005 / 1k | AWS via Turso |
| Custo S3 Express PUT | $0.0025 / 1k | idem |
| Storage 3× EBS vs S3 | $0.24 vs $0.023 /GB-mês (~10×) | Rivet |

Dois corolários que todo mundo que *mediu* converge:

1. **S3 Standard no caminho de commit transacional é inviável** (latência e
   conta de PUT). Ou você usa S3 Express + batch multi-tenant (Turso), ou
   você **não põe S3 no write path** (Rivet, Neon pageserver, DO).
2. **Atomicidade nativa é por objeto.** Multi-key ACID em S3 cru não
   existe. Por isso Tigris/FDB no metadata, SlateDB CAS num manifest,
   mvSQLite no FDB, Rivet no hot tier.

### 2.3 A camada C — bloco/elástico em cima de objeto

Isso é a outra metade da sua intuição, e é *ortogonal* ao SQLite-VFS.

- **JuiceFS:** metadata em Redis / TiKV / MySQL / PG / **FoundationDB**;
  dados em objeto, fatiados em chunks → slices → **blocks (default 4 MiB)**
  para PUT paralelo. POSIX / HDFS / S3-gateway na frente.
- **VAST (2025):** file + object + **block NVMe/TCP** no mesmo cluster.
  Bloco deixa de ser “o disco rápido separado” e vira protocolo sobre o
  mesmo data platform.
- **S3 Express One Zone (2023→):** objeto com latência de dígito único
  de ms e até 2M GET/s por directory bucket — o objeto *se comporta* como
  bloco barato para o hot set.
- **Plataformas (Railway Volumes vs Buckets, Fly+Tigris):** volume =
  um serviço, sem replica; bucket = N clientes. O caveat “replicas cannot
  be used with volumes” é o motivo pelo qual todo mundo que quer escala
  horizontal foge de volume.

C não torna Pedra “object-native”. C deixa Pedra **rodar sem saber**.
Útil para lift-and-shift. Inútil como tese de produto: qualquer Rocks/fjall
também roda em JuiceFS.

---

## 3. PedraDB hoje — o que o código faz (não o pitch)

Verificado em `crates/pedradb-core` nesta sessão.

### 3.1 Contrato real

```
App / SQL / DCS / store
        │
        ▼
  ordered KV + multi-key ACID   (begin/get/put/delete/range/commit)
        │
  WAL (32 KiB blocks, CRC, First/Middle/Last)
  MemTable → SST v4 (blocos-alvo 4 KiB, bloom, lz4, decode lazy)
  VALUES.vlog opcional (WiscKey)
  MANIFEST-* + CURRENT + LOCK exclusivo
        │
        ▼
  trait Env   = POSIX: create, append, seek, set_len, rename,
                sync_data/sync_all, sync_dir, read_dir
```

- Default `OpenOptions.sync = true`: `put` / `commit` só retornam depois
  de `sync_all` no WAL. `WriteOptions::no_sync` + `Db::sync` existem.
- `ConcurrentDb`: group commit Rocks-style (um fsync por grupo), dual
  memtable (flush SST sem segurar o write lock o tempo todo).
- Um diretório = um banco. `exclusive: true` (default) = arquivo `LOCK`.
- Não há API de página. Não há prefix multi-tenant. Não há object store.
- `pedradb-store` (Montanha): multi-Raft **in-process**, majority,
  `put_batch` no range, 2PC `commit_tx` no mesmo processo, DCS como
  layer. RFC-0017 P0.1–P0.4 (TCP multi-processo, kill-leader em 3 VMs,
  snapshot catch-up, cluster DST) estão **`todo`**.

### 3.2 O que isso *não* é

| Precisa o mundo SQLite+objeto | Pedra hoje |
|-------------------------------|------------|
| `xRead`/`xWrite` de página 4 KiB | KV de tamanho variável |
| Open em ms sem baixar o DB | Open = LOCK + recover WAL + carregar MANIFEST + handles |
| Bilhões de DBs ociosos = só storage | Um dir + WAL + MANIFEST + LOCK por DB |
| Compute sem estado | `StdEnv` assume filesystem local |
| Commit durável em <10 ms sem disco local | fsync local *ou* (inexistente) PUT objeto |
| Single-writer *por arquivo*, muitos arquivos no mesmo processo | `Db` é `&mut` / um `ConcurrentDb` por diretório |
| Time-travel barato (mvSQLite/Litestream) | Checkpoint = cópia de fileset, sem LTX/PITR |
| Dialeto SQLite na borda | Sem SQL no core (há `pedradb-sql` como layer — não é SQLite) |

SST ter bloco-alvo 4 KiB é coincidência de tamanho, não compatibilidade
de página. O bloco Pedra é stream de internal keys; a página SQLite é
B-tree com header, cells, pointer map.

---

## 4. Onde PedraDB se encaixa — slots, ranqueados

### Slot 1 — Hot tier sob uma VFS SQLite (o encaixe real)

Rivet já escreveu o job description. O hot tier é pluggable:
**RocksDB, Postgres, ou FoundationDB**. Precisa:

- commits de página/chunk em ms, replicados (≥3)
- `db_id ‖ chunk_id → 256 KiB` (64 páginas)
- open de um “banco” = lookup de chaves, não attach de volume
- idle = zero CPU

Pedra/Montanha *quer* ser esse objeto: Rocks-class local + TX, FDB-class
no store. Se Montanha um dia oferecer majority + ranges estáveis, uma
VFS fina (`xRead` = `get(page_key)`, `xSync` = `put_batch` das dirty
pages) é o produto. **Não precisa de S3 no kernel.** S3 entra como
tiering de chunks frios — trabalho da layer, igual Rivet.

**Gap para ocupar o slot:** ver §5.1–5.4. Hoje não dá para um Rivet
apontar para Pedra no lugar de Rocks/FDB.

### Slot 2 — Page store FDB-shaped (mvSQLite)

mvSQLite prova o padrão: SQLite intocado, VFS, páginas como KV no FDB,
OCC de página, time-travel, cliente stateless, blobs grandes fora do
B-tree.

Isso é *exatamente* a tese Montanha (RFC-0017): “apps/SQL são layers;
o store é TX+ordered keys”.

**Gap:** FDB entrega strict serializability + simulação de décadas +
limites conhecidos (5 s, 10 MB). Montanha tem 2PC in-process e P0 de
cluster `todo`. Substituir FDB aqui sem isso é marketing.

### Slot 3 — Metadata engine de POSIX-em-objeto (JuiceFS)

JuiceFS Community já aceita Redis, TiKV, PG, MySQL, **FDB**. O data
plane é objeto; o que dói em bilhões de arquivos pequenos é
metadata TX. Um Montanha maduro entra como “mais um metadata engine”,
não como o filesystem.

Útil, mas commodity: o valor está no store, não numa FUSE Pedra.

### Slot 4 — WAL export / PITR (Litestream do Pedra)

O rung 1.5 do doc anterior continua o experimento de menor blast radius:

- frames WAL imutáveis → objetos
- manifest/CAS ou LTX-like
- replica de leitura / restore sem ser o kernel

Pedra já tem seq number, WAL block format, checkpoint. Falta: formato
estável exportável, shipper, reader que não precisa do diretório POSIX.
Isso **não** torna Pedra um banco diskless. É backup + replica.

### Slot 5 — Cache quente na frente de objeto (Neon/turbopuffer)

SST+compaction+vlog **já existem** (o doc de ontem estava defasado).
Dá para imaginar: objeto = SST/vlog imutáveis; Pedra local = memtable +
block cache + WAL quente.

Ainda exige: `Env` (ou backend paralelo) que é PUT/GET/CAS, não
`rename`+`fsync`; group commit para objeto, não para `fsync`; fencing
de writer. Isso é reescrever o media layer. SlateDB *já é* esse
projeto. Occupying this slot is a competitor move, not a leverage move.

### Slot 6 — Substituir SQLite na borda

Pedra oferece KV+TX. O que esses sistemas vendem é o *dialeto e o
arquivo* SQLite (apps existentes, `LD_PRELOAD`, VFS, um `.db` que se
copia). Trocar isso por `begin/put/commit` é sair da categoria. **Não
jogar.** O valor está *debaixo* da VFS.

### Slot que *não* existe

“Apontar `StdEnv` para um bucket e ficar diskless.” `Env` exige
`seek`, `set_len`, `rename` atômico no mesmo filesystem, `sync_dir`.
Objeto oferece PUT/GET/LIST/If-Match. Implementar `Env` sobre S3 é
mentira com append-rewrite e fsync no-op — o tipo de coisa que o
próprio audit skill classifica como silent-wrong.

---

## 5. Gaps profundos (o que teria que ser verdade)

Ordenados pela distância até o Slot 1/2 — os únicos que importam.

### 5.1 Unidade de isolamento errada

Rivet: “a database is rows in a shared storage layer, not a volume plus
a filesystem plus a process.” SlateDB foi rejeitado por eles também
porque manifest+compaction *por banco* não escala a bilhões.

Pedra é volume+filesystem+process. `LOCK` + `CURRENT.log` +
`MANIFEST-*` + `VALUES.vlog` por banco. Open/recover de 10⁵ bancos
num nó é um produto diferente.

**O que faltaria:** um modo multi-tenant em que `db_id` é prefixo de
chave num único (ou poucos) Pedra, com open O(1), sem LOCK por banco,
sem MANIFEST por banco. Isso é o store, não um flag no `OpenOptions`.

### 5.2 Semântica de mídia

| POSIX `Env` | Object |
|-------------|--------|
| append + fsync | novo objeto (ou rewrite) |
| seek+overwrite | não existe (imutável) |
| rename atômico | CAS `If-None-Match` / `If-Match` (S3 ≥ 2024-08; clones mentem) |
| `sync_dir` | não tem equivalente |
| list dir | LIST paginado, caro, eventual em alguns clones |

Um `ObjectEnv` honesto não implementa `Env`. É outro trait: `put_if`,
`get_range`, `list_prefix`. O kernel teria que deixar de assumir
in-place WAL e publish-via-rename. Isso toca WAL, SST, MANIFEST,
checkpoint, vlog GC.

### 5.3 Modelo de commit vs conta de objeto

Default Pedra = 1 fsync / TX. Turso só fecha a conta porque **agrupa
dezenas/centenas de SQLites num PUT** de S3 Express. SlateDB/turbopuffer
agrupam no tempo (`flush_interval`, 1 WAL/s).

`ConcurrentDb` group-commit amortiza *um disco*. Não amortiza *N bancos
em um objeto*. Falta um flusher multi-DB consciente de request-price.

S3 Express resolve latência (~6 ms) e **não** replica cross-AZ
(SLA 99.95% one-zone). Rivet recusa S3 Express por ser AWS-only /
não self-host. Qualquer Pedra “diskless” tem que escolher o lado
Turso (Express + multi-tenant batch) ou o lado Rivet (hot KV, S3 só
frio). Não dá para ter os dois de graça.

### 5.4 Página ≠ LSM record

Uma VFS SQLite gera:

- muitas escritas de 4 KiB (amplificação: 1 byte sujo = 1 página)
- `xSync` = “essas páginas são um commit”
- readers concorrentes, **um** writer (WAL mode)

Pedra oferece TX multi-key de records. Para ser o hot tier você
mapeia `xSync` → `apply_batch` das páginas dirty — ok, o TX local
ajuda. Para ser o FDB debaixo do mvSQLite você precisa de OCC *de
página* (read-set de page ids), versões, e transações mais longas
que o timeout de rede do FDB. FDB tem timeout de 5 s; mvSQLite existe *porque*
estoura isso. Pedra local não tem o timeout de rede — vantagem
real — mas Montanha distribuído reintroduz o problema no dia em
que cruzar ranges.

### 5.5 Cluster que ainda não é cluster

Código do store: Raft in-process, `RpcMode::Queued` para DST, 2PC
in-process. RFC-0017 P0 (TCP, 3 VMs, InstallSnapshot, cluster DST)
aberto no mesmo dia deste relatório.

Sem isso, Pedra não substitui FDB no Slot 2 nem oferece o hot tier
replicado do Slot 1. “Majority” que só existe no mesmo processo não
é o durability story que Rivet exige (“triple redundancy” antes do
ack).

### 5.6 Instant start, zero restore

Litestream VFS: índice no trailer LTX (~1% do arquivo) + Range GET.
Rivet: fetch da página na hora + prefetch. Turso: cache write-through,
miss paga Express.

Pedra open: recover WAL linear, carregar version set, opcionalmente
abrir vlog. Não há page index remoto, não há lazy-open de um prefixo
de chaves de outro processo, não há “este nó nunca viu esse `db_id`”.

### 5.7 Time-travel e fork

A categoria já vende isso (mvSQLite: versão = read version FDB;
Litestream: `PRAGMA litestream_time`; Turso: cópia de um `.db`).

Pedra: MVCC de sequence local + checkpoint fileset. Sem retenção de
versões de página endereçável, sem fork barato de um prefixo, sem
API de “leia no seq S”. O sequence existe internamente; não é o
produto.

### 5.8 Multi-writer no mesmo SQLite

SQLite é single-writer por arquivo. Turso tem MVCC próprio (não
vanilla). Rivet recusa: Actor = single-writer, ponto. mvSQLite
paraleliza via OCC de *página* no FDB, não via writers no mesmo
arquivo local.

Pedra OCC (`occ.rs`) é local, um `Db`. Não há merge/transform hook.
O eixo que a categoria já resolveu é isolamento ou OCC de página —
não “N writers no mesmo arquivo POSIX”. Não prometer o terceiro.

### 5.9 Blobs grandes

Páginas 4 KiB para blobs grandes (mídia, dumps) é desastre de
amplificação. mvSQLite/Willow já mandam o blob para S3 e deixam o
SQLite com metadata — o mesmo recorte WiscKey, no objeto.

Pedra tem `large_value_threshold` + `VALUES.vlog` + `compact_vlog`
(RFC-0016). Isso é o WiscKey certo **no disco local**. Não é offload
para objeto. O vlog inteiro continua no mesmo `Env`. O gap da
categoria é “ponteiro no KV, bytes no bucket”, não mais um log local.

### 5.10 SQL / familiaridade

Layer `pedradb-sql` não é SQLite. O moat desses sistemas é o dialeto
e o arquivo (apps existentes, `LD_PRELOAD`, VFS). Pedra não deve
fingir que é um SQLite. Deve ser o *chão* de um.

### 5.11 Operação e confiança

Rivet constraint 5: “no new database to operate.” FDB/PG/Rocks já
estão lá. Pedra+Montanha é mais um stateful system. Sem soak,
sem cluster DST, sem história de silent_wrong=0 em campanha
determinística *de store*, o slot 1/2 não abre por mérito de API.

---

## 6. Como a tendência de bloco-em-objeto muda (e não muda) o quadro

**Muda o infra default.** Volumes param de escalar no momento em que
você quer replica / multi-attach / BYOC sem StatefulSet. Object +
cache local vira o disco. S3 Express e NVMe/TCP deixam o objeto
“rápido o suficiente” para muita coisa que antes exigia EBS.

**Não muda a física do Pedra kernel.** Se a plataforma te entrega um
POSIX que por baixo é JuiceFS/VAST, Pedra já roda — e também o
Rocks, o fjall, o SQLite. Zero diferenciação.

**Muda o *produto acima* do kernel.** O store que serve páginas/chunks
para um milhão de VFS, com majority e prefix isolation, *é* o
“elastic block” que as plataformas estão tentando vender — só que
com TX e ordem, não com SCSI. Tigris (FDB metadata + block data +
S3 API) é o mesmo desenho com outra roupa. Montanha, se um dia for
real, compete nesse andar, não no andar `Env`.

---

## 7. O que *não* fazer

1. **Não** implementar `Env` sobre S3 e chamar de diskless.
2. **Não** pivotar o kernel para “object-store-first”. SlateDB/Tonbo
   já ocupam; Rivet já explicou por que isso perde o commit em ms e
   o multi-tenant de bilhões.
3. **Não** substituir SQLite na borda (o produto que o mercado está
   paralelizando *é* SQLite).
4. **Não** usar o doc de 2026-08-11 como base de decisão — está
   incompleto e tem erro factual (Tigris, SST).
5. **Não** reivindicar o slot FDB/Rivet enquanto RFC-0017 P0 estiver
   `todo`.

---

## 8. O que *faria* Pedra útil — ordem honesta

Se a aposta é “o futuro é objeto e SQLite horizontal”, a sequência
que o código atual justifica:

| Ordem | Trabalho | Por quê | Não é |
|-------|----------|---------|-------|
| 1 | Kernel local correto e rápido (já a doutrina) | Hot tier sem isso é teatro | Object Env |
| 2 | **Page/chunk mapping** num único Pedra: `put_batch` de `(db, page) → 4KiB/256KiB`, open sem LOCK por db | É o contrato da VFS; testa o Slot 1 *in-process* | Distribuição |
| 3 | WAL/seq export estável (rung 1.5) para objeto, RO reader | PITR + “qualquer máquina lê o frio” | Write path em S3 |
| 4 | vlog pointer → objeto (não só `VALUES.vlog` local) | Fecha o gap de blob grande (WiscKey no bucket) | LSM-on-S3 |
| 5 | Montanha P0 de verdade (TCP, 3 VMs, majority sob kill) | Sem isso não há hot tier replicado nem FDB-slot | Mais layers SQL |
| 6 | VFS SQLite (C ou rust) em cima do store | É o produto que o mercado já fala | Reimplementar SQL |

Item 2 é o experimento barato que o repo *ainda não tem* e que
responderia, com número, se Pedra serve de page store. Sem esse
número, o resto é narrativa.

---

## 9. Veredito

A tendência é real: object storage virou o disco compartilhado;
bloco/elástico em cima de objeto está absorvendo o que era EBS;
SQLite voltou ao servidor porque **um arquivo por tenant** (e/ou
páginas num KV) paraleliza o que um cluster SQL não paraleliza —
isolamento e compute sem volume, não throughput de um B-tree.

PedraDB **não** é um sistema dessa categoria e **não** entra nela
apontando o WAL para um bucket. Entra, se entrar, como o componente
que esses sistemas ainda compram de terceiros: **um KV transacional
embutível que aguenta ser o hot tier de milhões de páginas e, mais
tarde, o FDB debaixo da VFS**.

Hoje falta o contrato de página/multi-tenant, falta o cluster de
verdade, falta export/tiering para objeto, e falta aceitar que o
SQL/POSIX continua na borda. O kernel (SST, vlog, group commit, Env
seam, TX) é pré-requisito — não é o produto dessa tendência.

O doc local de 11/08 acertou o non-goal do kernel e errou o mapa:
olhou SlateDB e não olhou Rivet/AgentFS/mvSQLite, que são o
encontro concreto de “vários AIs” + SQLite + objeto.

---

## Fontes primárias lidas (não resumo de segunda mão)

| Fonte | URL | Lida |
|-------|-----|------|
| Rivet zero-disk SQLite | https://rivet.dev/blog/2026-07-31-how-we-built-the-first-zero-disk-s3-tiered-storage-engine-for-sqlite/ | sim, texto completo |
| Turso Cloud diskless | https://turso.tech/blog/turso-cloud-goes-diskless | sim |
| AgentFS disaggregado | https://penberg.org/blog/disaggregated-agentfs.html | sim |
| Willow / mvSQLite | https://su3.io/posts/willow | sim |
| mvSQLite README | https://github.com/losfair/mvsqlite | sim |
| Litestream VFS | https://fly.io/blog/litestream-vfs/ | sim |
| SlateDB overview | https://slatedb.io/docs/design/overview/ | sim |
| SlateDB RFC-0001 Manifest | https://slatedb.io/rfcs/0001-manifest/ | sim |
| turbopuffer architecture | https://turbopuffer.com/docs/architecture | sim |
| Tigris architecture | https://www.tigrisdata.com/docs/concepts/architecture/ | sim |
| JuiceFS architecture / metadata engines | https://juicefs.com/docs/cloud/introduction/architecture/ , https://juicefs.com/docs/community/databases_for_metadata | sim (docs) |
| S3 Express | https://aws.amazon.com/s3/storage-classes/express-one-zone/ | sim (claims oficiais) |
| Cloudflare DO SQLite | https://blog.cloudflare.com/sqlite-in-durable-objects/ | parcial (post 2024) |
| Código Pedra | `env.rs`, `db.rs`, `sst/table.rs`, `wal/mod.rs`, `concurrent.rs`, `store/lib.rs`, RFC-0017 | sim |

**Não lido na íntegra nesta sessão (tratar como não-evidência):** wiki
interna do mvSQLite além do README; código-fonte Rivet/Turso; paper
QuiCK da Apple; RFC SlateDB posteriores que possam ter adicionado
compaction.

**O que *não* ficou estabelecido:** se S3 Express p99 do Turso ainda
segura em 2026 sob carga multi-tenant real; se Rivet “billions of
databases” é medido ou target; se AgentFS disaggregado saiu do post
e entrou em produção. Targets de blog ≠ capacidade medida.
