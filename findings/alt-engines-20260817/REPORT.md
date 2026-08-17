# PedraDB × engines alternativas — relatório aprofundado (2026-08-17)

**Pergunta do dono:** "como estamos em relação à performance de pedradb? não só o nosso
benchmark, mas ao fazer um swap transparente pelo rocksdb-compat e medir os benchmarks que
já existem em sistemas open source que usam rocksdb e comparar — relatório aprofundado com
diversas implementações e alternativas e os valores."

**Resposta curta (valores medidos, mesma máquina, mesmo schedule):**

1. Na classe de durabilidade comparável (fdatasync por write antes do Ok), **Pedra está em
   0.79–0.97× RocksDB nos shapes com escrita e 2.6× mais rápido em leitura pura**
   (direto, `Db` single-client). O gate oficial de paridade (via `rocksdb-compat`,
   11 shapes) registra **min_ratio 0.902**.
2. Na classe assíncrona (WAL sem sync por op), **Pedra não compete por design (G1)**: fjall
   default e sled default chegam a 449–520k qps no YCSB-A; rocks sync=false ~280k. Esse é o
   preço declarado da garantia, não um bug.
3. **"Per-op sync" no macOS tem duas subclasses incompatíveis** que os benchmarks cruzados
   misturam sem querer: `fdatasync(2)` de libc (Pedra, RocksDB — ~30–50 µs) vs
   `File::sync_data()` do std, que no Apple é **`fcntl(F_FULLFSYNC)`** (~3–5 ms no APFS
   deste laptop). fjall `persist(SyncData)` e redb `Immediate` caem na segunda — os ~341
   e ~420 qps deles não são "engine lento", é uma barreira de disco muito mais forte
   (e, em APFS, a única que garante durabilidade em power loss sem UPS).
4. Swap transparente de **sistemas open source reais** via `rocksdb-compat` ainda não é
   possível para TiKV et al. (ingest, properties, statistics, Titan, iteradores de prefixo);
   o caminho honesto hoje é (a) o parity suite que já roda pela face compat e (b) programas
   pequenos rust-rocksdb. Detalhe na seção 6.

---

## 1. Metodologia

- **Mesmo schedule do bench de paridade oficial**: 4096 records pré-carregados, 2000 ops,
  payload 1000 B, zipfian θ=0.99, xorshift seed derivado por shape, `ykey(i) = "ycsb/{i:06}"`.
  YCSB-A (50% read / 50% insert-new-key), YCSB-C (100% read), YCSB-F (50% RMW read+put).
  Harness: [`main.rs`](main.rs) neste diretório (commit junto); raw: `results-run{1,2,3}.txt`.
- **3 runs completas** (run1 sob load de máquina; run2/3 mais limpas; redb só na run3 —
  bug do path na run1/2). **Mediana por célula** é o número do relatório; variância
  anotada quando bimodal.
- Único cliente, chamada síncrona por op — sem group commit, sem paralelismo: mede o custo
  do caminho crítico por op, não throughput de frota.
- Máquina: MacBook (Apple Silicon), APFS, `~/.cargo` local. Números absolutos de qps
  **não são comparáveis entre sessões** (o gate oficial viu rocks deprimido a 16k em run
  com load; aqui rocks-A mediu 37.9k). Razões dentro da mesma run são o dado honesto.

## 2. Valores medidos (mediana de 3 runs; qps; p50/p99 no raw)

| engine (classe) | ycsb_a | ycsb_c | ycsb_f |
|---|---:|---:|---:|
| **pedradb-core (sync, fdatasync)** | **29 896** | **2 355 019** | **33 956** |
| rocksdb (sync=true, fsync-class) | 37 907 | 899 247 | 35 114 |
| rocksdb (sync=false, async WAL) | 279 750 | 836 820 | 267 769 |
| fjall 3.1.9 (default, async journal) | 449 623 | 1 822 254 | 427 762 |
| fjall 3.1.9 (persist SyncData/op, F_FULLFSYNC no macOS) | 341 | 912 513 | 371 |
| sled 0.34 (default, async) | 432 378 | 994 530 | 1 062 652 † |
| redb 2.6.3 (Immediate, run única) | 420 | 626 673 | 344 |
| redb 2.6.3 (Eventual, run única) | 2 010 | 666 158 | 2 956 |

† sled-F é bimodal nesta máquina (1.11M / 153k / 1.06M nas 3 runs) — flusher em background
atravessa o caminho; mediana não descreve bem.

**Razões Pedra/Rocks dentro da mesma classe (fdatasync/fsync por op):**

| shape | razão (mediana) | leitura |
|---|---:|---|
| ycsb_a | **0.79×** | Pedra ~26% menos qps em insert-heavy sync |
| ycsb_c | **2.62×** | Pedra 2.6× mais rápido (tudo cabe na memtable/SST cache; leitor sem lock de write) |
| ycsb_f | **0.97×** | paridade prática em RMW sync |

No gate oficial (face `rocksdb-compat`, `Mutex<Db>`, 11 shapes, MC=1): **11/11 ≥ 0.5,
min_ratio 0.902** (RFC-0037 P2.3; `a` chegou a 4.08 com rocks deprimido na melhor run).
As duas medições são faces diferentes do mesmo motor: direto vs wrapper.

## 3. Classes de durabilidade — o que cada "sync" realmente é (evidência de código)

| engine | knob | syscall real no macOS | custo típico aqui |
|---|---|---|---|
| pedradb-core | `OpenOptions::sync` | **libc `fdatasync(2)`** direto (`pedradb-posix`, RFC-0036: escolhido para casar com RocksDB/TiKV) | ~30–50 µs |
| rocksdb | `WriteOptions::set_sync(true)` | `fsync`/`fdatasync` libc (PosixEnv) | ~30–50 µs |
| fjall | `persist(PersistMode::SyncData)` | **std `File::sync_data` → `fcntl(F_FULLFSYNC)`** (jornal/writer.rs:227) | **~3–5 ms** |
| redb | `Durability::Immediate` | sync_all-class por commit (2 barreiras: dado + commit) | ~5+ ms |
| sled | default | async (fsync em batch pelo flusher) — **excluído dos benches síncronos do próprio fjall 3.0 por "does not reliably fsync"**, issue spacejam/sled#1351 | — |

Implicações honestas:

- **fjall(sync/op)=341 qps vs rocks(sync)=37.9k qps não é comparação de engine** — é
  F_FULLFSYNC vs fdatasync. Em Linux (produção) `fdatasync` é a barreira real e as duas
  linhas ficariam na mesma classe; em APFS, `fsync` comum não garante persistência em
  power loss sem F_FULLFSYNC (fsync(2) man page da Apple). Pedra escolheu a classe do
  peer (RFC-0036) para que a paridade vs RocksDB seja medida a barreira igual.
- **Pedra não tem modo async** (G1: fdatasync antes de todo Ok). A lacuna 30k → 450k qps
  do YCSB-A é o preço da garantia default; a pergunta "quer knob de async?" é dono-de-produto,
  não engenharia.

## 4. Por que ycsb_c é 2.6× a favor da Pedra

4096 × 1000 B ≈ 4 MB de working set: tudo cabe em memtable/imutáveis na RAM. O caminho de
read da Pedra (memtable → imutáveis → SST cache, sem atravessar lock de write no read) é
mais curto que o do rocksdb 0.22 default (options default do harness — sem bloom tuning,
block cache default). É uma vantagem real neste regime (dataset pequeno), **não** uma
declaração de superioridade de leitura em geral — o gate oficial tem shapes de scan em que
Pedra está a 0.90×.

## 5. Benchmarks upstream citáveis (verificados na fonte)

- **fjall 3.0 (2025-10-10)**, methodology persistida em
  [`alt-engine-sources-20260817/fjall-3-article.md`](../alt-engine-sources-20260817/fjall-3-article.md):
  YCSB A/B-like, 10M KVs, 16B keys/100B values, cache 2 GB, mimalloc, engines fjall/LMDB
  (heed)/rocksdb/redb/sled/SQLite. Números estão em PNGs (não citáveis como texto), mas a
  metodologia cita: (a) sled **excluído** dos benches síncronos por fsync não confiável;
  (b) LMDB/SQLite sem checksum de página nas leituras (levemente mais rápidos por isso —
  mesma troca que o audit da Pedra sempre apontou).
- **RocksDB** não publica benchmark YCSB oficial reprodutível único; as referências da Meta
  (db_bench fillrandom/overwrite) são auto-medidas com options tunadas por hardware —
  citável como contexto, não como paridade.
- **Não encontrei** benchmark público terceiro comparando pedradb-like engines no macOS/APFS
  com classes de sync separadas — o §3 deste relatório é o diferencial.

## 6. Swap transparente via `rocksdb-compat`: estado real

O que existe hoje (`crates/rocksdb-compat`, 1.4k linhas): `open_default`/`open_cf`, put/get/
delete (±CF), `delete_range_cf`, `write(WriteBatch)` atômico, point+iterator em `snapshot()`,
`flush`, `compact`; CFs emulados por prefixo de chave; `Mutex<Db>` por trás. **O parity suite
oficial já roda por essa face** — esse é o "swap transparente" medido (gate 0.902).

O que **impede** plugar TiKV/et al. hoje (docs/rocksdb-compat.md): ingest externo
(`SstFileWriter`), compaction filters, properties/statistics, iteradores de prefixo,
Titan/BlobDB, `delete_files_in_range`. Programas rust-rocksdb pequenos (ex.: caches,
queues, config stores) já trocam `rocksdb = { package = "rocksdb-compat" }` e rodam.

Alternativas para "medir sistemas open source reais", em ordem de custo/honestidade:

1. **Proxy benches no parity suite** (feito): shapes `deps_*` são extraídos de padrões reais
   de uso TiKV (raftlog append, apply batch, mvcc latest, scan) e já rodam contra rocks
   verdadeiro pela face compat.
2. **Programas rust-rocksdb pequenos reais** via swap de dependency + FailingEnv (o
   propósito original do crate): mede o *custo do swap*, não o sistema inteiro.
3. **TiKV inteiro**: exige fechar a lista de API acima (engenharia de semanas, P1 do
   roadmap de compat) — não é um benchmark, é um produto.

## 7. Conclusões e próximos passos

- **Posição atual:** dentro da única classe onde garantia é comparável, Pedra entrega
  0.79–0.97× rocks em writes sync e >2× em reads (dataset residente), com gate oficial
  11/11 ≥ 0.5 (min 0.902) pela face compat. A meta ≤2× mais lento está **alcançada e
  medida**; o gap estrutural restante é a inexistência da classe async (por design).
- **Decisão em aberto para o dono:** adicionar knob de async-WAL (quebra G1 default?
  vira opt-in documentado?) — é o único caminho para competir com fjall/sled no regime
  "perde último ms de writes em crash aceitável".
- **Reprodutibilidade:** harness + raw results commitados neste diretório; schedule
  idêntico ao parity oficial; rodar `cargo run --release` no crate copiado.
