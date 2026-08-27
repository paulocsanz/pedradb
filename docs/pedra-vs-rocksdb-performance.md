# Por que o Pedra é mais rápido que o RocksDB — relatório técnico

> **Stale no cartaz (RFC-0062 P0.2, 2026-08-25).** A tabela da §1 abaixo ainda
> descreve o piso **2× G1** como “oficial (cartaz)”. Isso foi
> **re-baselined 2026-08-24** (RFC-0041): o gate é **1×** na coluna drop-in
> async vs Rocks `sync=false` (15/15 ≥ 1.254);
> G1 vira tabela de produto publicada (leituras 1.13–1.99×; writes 1c
> fd-ceiling ≪ 1×; group commit fecha — apply_mc4 2.788×). **Não** citar
> este ficheiro como “estamos 2× o Rocks em tudo”. Estado de lançamento:
> [`docs/reports/2026-08-25-launch-readiness.md`](reports/2026-08-25-launch-readiness.md)
> + [RFC-0062](rfc/0062-launch-readiness-remaining-gaps.md).

**Status:** relatório de engenharia, evidência medida 2026-08-17 → 2026-08-20; cartaz 2× G1 superseded 2026-08-24
**Escopo:** `pedradb-core` + `rocksdb-compat` (compat engine) vs RocksDB via `librocksdb` (branch upstream corrente no dia de cada medição)
**Método:** todo número cita o finding/RFC de onde veio. Árbitro de números em pé = 3 runs em caixa quieta (load 1-min < 10, P2.1). Janela curta sob carga é ruído (mesma engine varia 6×); p50 por op é o sinal estável. Números dirty estão **sempre** marcados como dirty.

---

## 0. Resumo — a resposta em um parágrafo

O Pedra não é mais rápido por um truque; são **quatro diferenças estruturais somadas**, e nenhuma delas é "pular fsync":

1. **Menos trabalho por write**: uma cópia lógica do payload até o frame do WAL (`encode_into` + scratch), `WriteOp` *move* (não clona) para a memtable, WAL v2 omite payload internado repetido, `insert_many` invalida uma vez por batch — Rocks paga serialize do `WriteBatch` + buffer do file writer + insert em skiplist, cada passo com sua cópia.
2. **Cache de respostas com invalidação cirúrgica** (`AnswerCache` por chave, `CountCache` por janela + seq observado + dirty log limitado) — o Rocks tem block cache, que cacheia *blocos*, não *respostas*; cada count dele é um iterator de verdade.
3. **`fdatasync` amortizado por grupo** (N ops / 1 fd) e compact/flush fora do caminho do Ok.
4. **Count como operação de primeira classe** (cursor emprestado, sem `Box<dyn>`, sem clone por passo) em vez de um scan disfarçado.

E o mais importante para o produto: o Pedra paga `fdatasync` antes do Ok (G1, default) e **ainda assim** é mais rápido que o Rocks que roda em produção — o default do Rocks é `WriteOptions.sync=false`, que perde writes acked em power loss. A vitória é **mais durabilidade e mais velocidade ao mesmo tempo**, nunca uma trocada pela outra.

---

## 1. O que está sendo comparado (e as regras de honestidade)

Duas colunas, dois contratos — nunca misturadas:

| Coluna | RFC | Pedra | Rocks | Piso | Vale como |
|---|---|---|---|---|---|
| **Oficial (cartaz)** | 0041 | default: `fdatasync` antes do Ok (G1) | default: `sync=false` | ≥ **2×** cada shape, mediana ≥3 runs quieta | "batemos o Rocks" |
| **Async same-class** | 0044 | `PEDRA_PARITY_ASYNC=1` (encode no frame antes do Ok, `write()` aos 64 KiB, sem fdatasync) | `sync=false` | ≥ **5×** | **nunca** "batemos o Rocks" — mede motor/CPU |

Regras que este relatório segue (AGENTS.md): peer oficial é sempre o Rocks default (`ROCKS_PARITY_SYNC=0`); `rocks-parity-compare` **exita 2** se o peer tiver `sync: true`; shape nenhum sai do catálogo (RFC-0043); remesura em caixa suja nunca é número oficial; ratio contra peer sync não é vitória.

Física do fsync nesta caixa (rfc0041-p02): `fdatasync` p50 isolado **25.7 µs** ≈ 38.9 k qps por fd. Isso é o teto de um put de 1 cliente com 1 fd por Ok — daí o group commit ser a única forma de pagar G1 e competir com async.

---

## 2. Decisão a decisão

### D1. Durabilidade no Ok

| | RocksDB | Pedra |
|---|---|---|
| Default | `WriteOptions.sync=false` (`options.h`: "Default: false") — WAL fica no page cache do SO | `OpenOptions.sync=true` — `fdatasync` **antes** de cada Ok |
| Power loss após Ok | Pode perder os últimos writes acked | Write acked sobrevive |
| fsync falha | Erro + fence **condicional** (`paranoid_checks`); com `false`, continua escrevendo sobre meio falho | Handle inteiro **durability-fenced** (`CoreError::DurabilityFenced`, resultado incerto tipificado); sem flag de escape |
| Corrupção de WAL | Default `kPointInTimeRecovery`: para no ponto e **segue** (disponibilidade) | Fail-closed: erro no open (nunca silent-wrong) |

Fonte: `docs/rocksdb-vs-pedradb-guarantees.md` (claims Rocks citados por arquivo do upstream, verificados 2026-08-17).

**Por que isso não nos desacelera:** o custo do fd é pago **por grupo**, não por chave — N writers esperando compartilham um `fdatasync` (group commit). No multi-cliente o fd desaparece do denominador (`apply_mc4` **2.788×** o Rocks default no head3, RFC-0041 P1.1). No 1 cliente 1 op ele é físico e **dizemos isso em código**: o teste `rfc0041_one_fdatasync_cannot_hit_2x_rocks_default_ycsb_a` afirma o teto; `ycsb_a` 1c está a 0.056× vs Rocks A (398 k) por pura física (`1/t_fd` ≈ 39 k), não por engine lento.

**Consequência:** o produto é "conectou, está durável" sem apertar botão, e o número oficial **não** compra velocidade com garantia. **Constraint:** shape 1-op/1-cliente com fd por Ok não fecha 2× nunca, por teorema medido; P1.2 do RFC-0041 fica `todo` para sempre nesse formato (sem largar G1/peer). **Trade assumido:** o fence é incondicional — qualquer fsync falho cerca o DB até close+reopen (blast radius inteiro; política de severidades é avaliação futura, `docs/open-items.md` §2.6).

### D2. Custo CPU por write (a guerra das cópias)

Caminho Rocks (put): copiar payload pro `WriteBatch` → serializar batch no buffer do file writer → insert na skiplist da memtable (outra cópia); pipeline de 32 = 32× isso. Caminho Pedra (RFC-0040 P0, RFC-0044 P1):

- `WriteRecord::encode_into` num **scratch reutilizado** (uma cópia lógica até o frame — o CRC precisa dos bytes; não dá para zerar e não tentamos);
- `WriteOp` **move** para a memtable depois do append (sem clone);
- **WAL v2**: valor internado repetido no mesmo batch é gravado **uma vez** (`kind | 0x80`);
- `BatchOp::put` **interna** payloads idênticos (SET/blob);
- `MemTable::insert_many`: **um** invalidate de `tail_ord` por batch, não por op.

**Número:** SET p50 **0.4–0.5 µs vs 2.5–3.0 µs** do Rocks (~6×) — 1.83 M vs 338 k qps na janela longa quieta (kvlong-2m, ratio **5.41**); SET 1c já tinha cruzado 5.66× na melhor janela suja com Rocks saudável (kvrocks-l14). Blob 16 KB mostra o contrário da moeda: **2.02×** — quando o payload é 16 KB, a cópia que sobrou domina e a vantagem estrutural encolhe (@32 KiB de buffer chega a 0.85, dois blobs enchem o bloco).

**Consequência:** a vantagem é maior quanto menor o payload relativo ao overhead fixo por op — exatamente o perfil KV (chave curta, valor ≤1 KB). **Constraint:** workloads de valor grande (blob) precisam de outro eixo (vlog/WiscKey já existe como camada; ainda não é o gargalo fechado).

### D3. Leitura: cache de *respostas* vs cache de *blocos*

Rocks: point get = memtable → L0 (todos os arquivos) →Ln com bloom, block cache LRU; `get_pinned` evita a cópia do valor. Count/scan = iterator: heap de merge sobre os níveis, seek O(níveis), puxa chave+valor.

Pedra:

- **`AnswerCache`** (ponto / último prefixo): hit devolve `borrow()`, invalidação **por chave** (dirty points com gen-bump em batch ≥32), não clear geral;
- **`CountCache`** (RFC-0044 P2.2, commit 3a722e7): entrada carrega a **janela decodificada** + o seq publicado observado **antes** do compute; no get, valida contra um **dirty log limitado** (1024 chaves, BTreeMap por chave, `overlaps_newer_than`); overflow evicta metade antiga e **aposenta** entradas cuja janela colide com a bounding-box dos evictados (conservador); range-delete/fat-apply continua clear wholesale; **um mutex** guarda entras+log (aposentadoria atômica com a eviction — sem janela de resposta pré-escrita);
- `get_probe` / `DB::contains`: o canary de GET não copia 1 KB (`to_vec`) — mesmo contrato do `get_pinned` do Rocks;
- TLS last-count para a última janela.

**Números:** GET p50 **0.0 µs vs 0.4 µs** medido; lado engine (sem o custo de 62 ns/op do harness): ~70 ns vs ~500 ns ≈ **7× real**, medido 4.5–5.2×. Janela longa quieta: 11.6 M vs 2.52 M (**4.61**); segunda janela longa independente (load ~100, dirty): 5.72 M vs 1.04 M (**5.50**). GET na janela curta da suíte: 1.61 (3.82 M vs 2.38 M) — o Rocks GET é forte em janela curta; o Pedra cresce mais com a janela (cold start do point-cache + overhead fixo comprimem o mais rápido).

**Consequência:** leitura é onde o Pedra mais separa — porque a pergunta "quantos nesta janela?" é respondida em O(1) hash quando nada relevante mudou. **Constraints:** o hit exige janela exata; write **dentro** da janela escaneada ainda invalida (correto, e é o cliff do E, D5); o dirty log é limitado (1024) e o overflow aposenta conservador (perde hit rate, nunca erra); range-delete clear wholesale.

### D4. Count sem iterator (o shape E)

O E do YCSB (95% scan de janelas curtas + 5% insert) expôs as duas filosofias:

- **Mecânica medida** (findings/rfc0044-p1 `ycsb-longwindow/`, probe no db travado): 2M ops com zipf θ=0.99 sobre 1024 chaves empilha ~140 k versões na chave mais quente. Scan frio dessa janela: **2.314 ms**; após `compact_for_reads` (3.1 s): **0.002 ms (~1000×)**. Point get na mesma chave: 2.7 µs — o caminho de ponto nunca foi o problema. O amplificador era o antigo `count_cache.clear()` **inteiro a cada publish** — os 5% de inserts do E (chaves ≥1024, fora de toda janela escaneada) limpavam o cache a cada ~19 scans ⇒ ~1.9 M scans frios.
- **Fix de produto (CountCache)**: com invalidação por faixa, o default completa os mesmos 2M ops de E a **1.35 M qps** (~1.5 s) — **82× o Rocks E da mesma janela** (16.5 k; dirty cross-run, não oficial). O Rocks na mesma janela degrada 79× do próprio B/C (1.1–1.3 M → 16.5 k): o compaction dele GC'ou a pilha, mas cada count ainda é um iterator de verdade andando SST.
- **Árbitro quieto**: `ycsb_e` **10.55 med (10.47/10.55/12.76, 3/3 ≥5 em toda condição testada, inclusive load 150)** — é consistentemente o topo da coluna async.

**Por quê:** no Rocks, count = iterator (setup ~20 µs de heap de merge + seek O(níveis) + materializar K+V do block cache). No Pedra, count quente = lock + hash + comparação de janela; count frio = cursor emprestado sem `Box<dyn>` e sem `user_key.clone()` por passo (RFC-0040 P0.3).

### D5. Retenção MVCC: história default vs GC default

| | RocksDB | Pedra |
|---|---|---|
| Default | Compaction **descarta** versões obsoletas sem pin | Auto-compact **mantém todas as versões** (F20, RFC-0009) |
| História | Só enquanto snapshot pinar (e snapshot longo segura compaction) | Sempre: MVCC, `changes(from,to]`, PITR, snapshots |
| GC | Automático | **Opt-in**: `Options.auto_reclaim` → `set_auto_reclaim` (GC pin-aware no auto-compact **e** no compact worker) ou `compact_for_reads` |
| Leitura fria | Enxuta (poucas versões por chave) | O(versões retidas) |

**Consequência dupla.** Para o produto: PITR e change-feed de graça, e `SnapshotTooOld` tipificado (GC fail-closed, nunca leitura errada). Para a performance: é a nossa **maior constraint estrutural** — overwrite quente + scan frio na janela é O(versões): o mesmo E que roda a 1.35 M com cache quente não completava (>80 min) com o cache quebrado, porque o scan frio andava ~140 k versões. `auto_reclaim` fecha essa classe (E 9 s / 235 k = **14.2×** na mesma janela suja), mas é opt-in e **nunca** entra nas colunas oficiais (a coluna oficial mede o default do produto). Nota honesta do finding: a concentração de 2000 ops/chave é artefato do bench; a suíte oficial (2000 ops total) não encosta no cliff — mas a *classe* do problema é real e documentada.

### D6. Escritores concorrentes: multi-writer C++ vs single-writer linearizável + group

Rocks: multi-writer real — `two_write_queues`, write groups paralelos, insert concorrente na memtable. Pedra: writers seriam no lock do WAL (linearizável); G1 compartilha 1 fd por grupo; no modo async, `commit_async_ops` é **bypass**: cada thread faz lock→encode→mem→stage WAL e solta (N threads + um write lock — o formato Rocks).

**Números:** `set_mc50` quieto **2.02×** (307 k vs 152 k, peer são). Os 3.4–9.2 que apareciam eram peer doente sob carga (Rocks 55 k). Testamos a hipótese "juntar escritores num líder fecha 5×" e **falsificamos**: merge líder/bypass mediana **0.19×** (5 rounds pareados, 44–106 k vs 311–636 k) — com 50 threads em 12 CPUs o líder é ponto único de agendamento (p50 merge ~200 µs vs bypass 0.6–1.3 µs). O código fica atrás de `PEDRA_ASYNC_GROUP=1` para reteste; o produto é o bypass. O gap restante (2×) é **outro mecanismo ainda aberto** (não handoff de grupo).

`deps_lock_prewrite` **0.94 med** (1.40/0.94/0.77) — o único shape abaixo de 1. É o shape mais parecido com TransactionDB pessimista do Rocks; nosso lock manager é single-writer TX + OCC.

### D7. Buffer WAL async: o contrato same-class exato

Async ≠ "ack sem encodar". Contrato Pedra (RFC-0044): encode no frame **antes** do Ok; `write()` ao kernel quando o buffer enche (64 KiB, alinhado ao file writer do Rocks); sem `fdatasync`; tail drena no flush/close. Testado e rejeitado: ack em userspace com buffer 1 MiB (**inválido** — mais fraco que o Rocks); `write()` em todo put (mais estrito que o Rocks, e só 1.29× no SET). A 64 KiB o pipeline fez **11.3×** em disco calmo (kvrocks-64k); a cauda do `write()` sob disco sujo pode decidir um wall curto (pipeline 0.96 na l14 com p50 5× melhor — o wall era 4 ms de write).

**Consequência:** crash de processo pode perder o tail <64 KiB — exatamente a classe do Rocks `sync=false`. É por isso que essa coluna existe: isola o motor da física do fsync.

### D8. Sem thread no core (G6); compact fora do Ok

Debounce inline determinístico (a cada N commits / flush / close), sem timer, sem worker no core; compact worker é do host (compat). No async, o compact **não acorda por put**. Consequência medida: SET 1c 1.67 M vs 296 k (5.66×) na janela com Rocks saudável — parte da vantagem é não ter background competindo no caminho quente. Rocks, em contrapartida, tem flush/compaction próprios e sofisticados (leveled/universal/FIFO, subcompactions, rate limiter) — feature, não defeito.

### D9. Os dois produtos

Pedra é a primitiva local (library single-process, `#![forbid(unsafe_code)]`, Rust puro; o multi-node é Montanha, outro produto). Rocks é C++ com décadas de campo. Onde o Rocks segue **na frente** (sem pinga de marketing, do guarantees doc): TransactionDB pessimista (2PL, deadlock detection, 2PC WritePrepared/Unprepared), throughput multi-writer de memtable concorrente, recuperação fina de erros (severidades/auto-resume/quarentena), e superfície de features (CFs cross-atomic, merge operators, user timestamps, wide columns, follower reads, SST ingestion). Nosso doctrine proíbe claim de paridade de maturidade.

---

## 3. Síntese causal — de onde vem cada × (árbitro quieto P2.1, load 9.4)

| Fonte da vantagem | Shapes onde aparece | Número |
|---|---|---|
| Count de 1ª classe + CountCache | `ycsb_e` | **10.55** (3/3; ≥5 até load 150) |
| Cache de resposta vs iterator | `kvrocks_scan` | **7.39** (809 k vs 109 k) |
| Cópias/alloc por write | `kvrocks_set` | **5.41** longo quieta (p50 6×) |
| Harness menor + point path | `kvrocks_get` | 4.61–5.50 straddle (p50 ~7× engine-side) |
| Interning + insert_many | `kvrocks_pipelined_set` | 4.22–5.86 straddle (p50 4.0 vs 23.2 µs) |
| RMW barato + cauda do Rocks | `ycsb_f` | 1.66 wall; **p50 1.5×, p99 22×** (3.0 vs 67.2 µs) |
| (motor empata, física decide) | `deps_lock_prewrite` | **0.94** |

Árbitro completo (v0 mediana de 3): E 10.55 · C 3.84 · deps_cache_overwrite 3.43 · D 3.02 · B 2.93 · A 2.88 · deps_scan 1.96 · deps_mvcc_latest 1.67 · F 1.66 · deps_raftlog 1.51 · deps_apply_batch 1.41 · deps_lock_prewrite 0.94. Veredito registrado sem maquiagem: **só E e scan fecham ≥5 amplamente; SET cruza na janela longa quieta; GET/pipeline straddle; o resto está 1.0–3.8.** O piso ≥5 do RFC-0044 **não** está alcançado.

Coluna oficial (0041, G1 vs default): head3 — apply_mc4 **2.788 ✓** · E 2.121 · scan 1.790 · C 1.796 · raftlog_mc4 1.792 · apply 1c 1.30 · raftlog 1c 0.99 · A/F 1c 0.056/0.073 (teto fd, afirmado em teste). O piso ≥2× para todos também não está fechado — e o A/F 1c não fecha **por física**, não por engine.

O p50/p99 é o sinal que sobrevive a qualquer caixa (mesma engine balança 6× no wall): SET 0.4–0.5 vs 2.5–3.0 µs · GET 0.0 vs 0.4 µs · pipeline 4.0–4.4 vs 22–28.5 µs · F 1.3 vs 1.9 µs (p99 3.0 vs 67.2 µs — a cauda é o write-buffer do Rocks).

---

## 4. Consequências (as duas direções)

**Para o produto:**
- Durabilidade out-of-the-box (G1 default) com performance de motor acima do Rocks que o pessoal roda — a tese do produto inteira num número (apply_mc4 2.8× já fechado no oficial; E/scan/SET/get/pipeline na coluna async com p50 5–7×).
- História é grátis: MVCC, change-feed, PITR sem configurar nada (F20).
- Fail-closed em corrupção; fence tipificado; fault injection como produto (seams Env/Clock/Rng/Host) — as garantias são testáveis sob falha.

**Custos que assumimos (e por quê):**
- Retenção default come disco e deixa scan frio O(versões) — cliff real do E; mitigação é opt-in (`auto_reclaim`) ou `compact_for_reads` operado.
- Blast radius do fail-stop: qualquer fsync falho cerca o DB inteiro até reopen — zero política de disponibilidade parcial (Rocks tem severidades/auto-resume). Posição assumida: kernel cerca, gerenciador decide.
- 1-op/1-cliente com G1 tem teto físico; a resposta é arquitetura de uso (grupos/pipeline), não tirar o fsync.
- Blob 16 KB ~2× e mc50 2.02: cópia grande e handoff de lock — eixos abertos.

---

## 5. Constraints e dívidas (lista honesta, 2026-08-20)

1. Piso 0044 ≥5× **não** fechado para todos (só E/scan amplamente; SET cruza na longa quieta; GET/pipeline straddle 4.2–5.9; mc50/blob ~2.0; F 1.66; A–D 2.9–3.8).
2. Piso 0041 ≥2× oficial **não** fechado em todas as shapes (apply_mc4 sim; A/F 1c impossível por física com G1 — registrado em teste).
3. `deps_lock_prewrite` 0.94 — único shape perdendo; lock manager single-writer TX + OCC vs 2PL do Rocks.
4. mc50: mecanismo do gap 2× ainda não identificado (merge falsificado 0.19×; não é handoff de grupo).
5. Blob: copies de 16 KB dominam; precisa do eixo vlog.
6. CountCache: dirty log 1024 (aposentadoria conservadora perde hit rate, nunca erra); range-delete = clear wholesale; write dentro da janela escaneada invalida (só GC resolve).
7. `auto_reclaim` é opt-in e nunca entra em coluna oficial (o default do produto retém tudo).
8. Medição: árbitro é 3× quieta; walls de 200–2000 ops são loteria sob carga (a mesma engine varia 6×); dirty nunca é oficial.
9. Blast radius do fence; sem TX pessimista; sem paridade de maturidade de campo (doctrine).

## 6. Fontes

- `AGENTS.md` (peer oficial, piso 0041), `docs/rfc/0041`, `docs/rfc/0044` (coluna async, piso 5×)
- `findings/rfc0044-p2/README.md` (árbitro quieto P2.1 load 9.4; hot-box; kvlong) + `quiet/` JSONs
- `findings/rfc0044-p1/README.md` (mecânica do E, per-op, merge A/B, 64 KiB) + `ycsb-longwindow/` (probe, reclaim, cache-fix)
- `findings/rfc0041-p02/README.md` (fdatasync p50 25.7 µs; mapa 0/16)
- `docs/rocksdb-vs-pedradb-guarantees.md` (defaults do Rocks citados por arquivo do upstream)
- `docs/rfc/0031` (G1–G8), `docs/rfc/0040` (encode_into/scratch/count cursor), `docs/architecture-refined.md` (papel do produto)
- Código: `crates/pedradb-core/src/cache.rs` (CountCache, 3a722e7), `db.rs` (count_in_range, invalidate), `crates/rocksdb-compat/src/lib.rs` (auto_reclaim, d9abd6f)
