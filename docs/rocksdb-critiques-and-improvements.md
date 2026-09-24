# RocksDB: críticas e oportunidades de melhoria para o PedraDB

Análise baseada em **fontes primárias**, catalogada para guiar as decisões de
design. As fontes cruas estão em `docs/references/`. Cada item abaixo cita a
origem e indica a implicação concreta para o PedraDB.

## Fontes primárias usadas

| Ref | Fonte | Tipo |
|-----|-------|------|
| [P1] | "Introducing Pebble" — Cockroach Labs, 15 set 2020 | Anúncio + rationale |
| [P2] | `pebble/docs/rocksdb.md` — "Pebble vs RocksDB: Implementation Differences" | Análise técnica detalhada |
| [D] | Dayan & Idreos, "Dostoevsky" — SIGMOD 2018 | Modelo de custo LSM, Lazy Leveling |
| [M] | Dayan & Idreos, "Monkey: Optimal Navigable Key-Value Store" — SIGMOD 2017 | Alocação ótima de Bloom filters |
| [F] | fjall-rs blog — "Announcing Fjall 2.0", set 2024 | Engine LSM em Rust puro (forbid unsafe) |

Arquivos locais (persistidos para reuso entre sessões):
- `docs/references/pebble-announcement-2020.md` — ref [P1]
- `docs/references/pebble-vs-rocksdb-differences.md` — ref [P2]
- `docs/references/dostoevsky-sigmod2018.pdf` + `.txt` — ref [D]
- `docs/references/monkey-sigmod2017.pdf` + `.txt` — ref [M]
- `docs/references/fjall-2-announcement.html` — ref [F]

> **Nota metodológica:** a pesquisa inicial usou `web_search` (indisponível por
> limite do wrapper). Corrigido: pesquisa feita via `curl` direto ao DuckDuckGo
> + fetch das fontes primárias. As fontes acadêmicas ([D], [M]) são peer-reviewed
> (SIGMOD); as de engenharia ([P1], [P2], [F]) são de times que construíram
> engines LSM em produção.

---

## 1. Complexidade e manutenibilidade

**Crítica [P1]:** RocksDB cresceu de ~30k LOC (LevelDB) para 350k+ LOC. Serve
"muitos mestres" (MyRocks, Rocksandora, CockroachDB, …), o que gera uma
surface area de configuração enorme. Bugs são poucos mas **severos** — e
navegar 350k linhas de C++ "não é exatamente divertido".

**Exemplo concreto [P1]:** bug em compaction que entrava num ciclo infinito
para uma SSTable específica, starvationando o resto da LSM
(facebook/rocksdb#4575).

**Implicação PedraDB:**
- Manter a surface area enxuta. Não reimplementar tudo — filtrar features pelo
  critério "isso serve ao caso de uso do PedraDB?". O Pebble tem ~45k LOC +
  ~45k de testes (uma fração do RocksDB) entregando a paridade que o
  CockroachDB precisava.
- `#![forbid(unsafe_code)]` já estabelecido — mantém o footprint de auditabilidade baixo.

## 2. Custo da fronteira de linguagem (a lição do cgo)

**Crítica [P1]:** a barreira Go↔C++ é "psicologicamente real". Impossibilita
profiling nativo, obriga a duplicar lógica em C++ para evitar travessias
frequentes de FFI, e dificulta debug de stack traces.

> **Relevância direta para a nossa decisão de arquitetura:** este é exatamente o
> argumento que valida a escolha **clean-room em Rust puro** em vez de tradução
> gradual via CXX. A dor do cgo é a mesma dor do CXX quando a fronteira é
> "grossa". O oráculo (CXX na API C pública) mantém a fronteira **fina** — só
> para testes —, então não herdamos esse custo.

## 3. Overhead de representação de Internal Keys

**Crítica [P2] — Internal Keys:** RocksDB representa Internal Keys na forma
**encoded** (string) na maioria das APIs internas (`InternalIterator::Seek`
recebe uma chave encoded), exigindo alocação temporária para anexar SeqNum +
Kind toda vez que um `Seek(user_key)` é convertido. O comparador às vezes
**decodifica a mesma chave múltiplas vezes** durante o processamento.

**Como melhorar [P2]:** Pebble usa um `InternalKey` struct (análogo ao
`ParsedInternalKey`) em **toda a API interna**, alocando zero. Encoding só na
fronteira mais baixa (leitura/escrita de SST).

**Implicação PedraDB:** modelar `InternalKey { user_key, seqnum, kind }` como
struct nativa desde a Fatia 2 (MemTable). Nunca circular strings encoded nas
APIs internas.

## 4. Indexed Batches incompletos

**Crítica [P2] — Indexed Batches:** o `WriteBatchWithIndex` do RocksDB **não
suporta** iteração de `Merge` nem `DeleteRange` dentro do batch, porque a lógica
exigiria duplicar o processamento que já existe no iterator normal.

**Como melhorar [P2]:** Pebble trata o batch como **mais um nível da LSM**,
dando aos records do batch seqnums temporários (`RecordOffset | (1<<55)`), e
reaproveita o `MergingIterator` existente — zero código duplicado, suporte
completo a merge/range-delete em batches.

**Implicação PedraDB:** projetar o `MergingIterator` (Fatia 7) desde o início
para aceitar um batch como nível. Reservar o high-bit do seqnum para batch.

## 5. Grandes batches → OOM / death loop

**Crítica [P2] — Large Batches:** RocksDB deixa a memtable crescer além do
tamanho configurado (arena sem limite). Um batch de 1 GB numa memtable de 64 MB
faz a memtable inflar, potencialmente estourando memória e sendo killed pelo
kernel — e no restart o batch é re-lido do WAL e re-aplicado, gerando um
**death loop**.

**Como melhorar [P2]:** Pebble usa arena de tamanho **fixo**. Batch que não
cabe vira um `flushableBatch` (ordenado uma vez) adicionado à lista de
memtables imutáveis — legível como nível da LSM. Commit retorna assim que o
`flushableBatch` é adicionado.

**Implicação PedraDB:** MemTable com arena fixa (Fatia 2). Path para grandes
batches via flushable batch na Fatia de Flush (Fatia 4).

## 6. Commit pipeline com muitos pontos de sincronização

**Crítica [P2] — Commit Pipeline:** o pipeline tradicional do RocksDB usa
batch grouping com leader/follower. O líder copia (concatena) os batches do
grupo, notifica followers, espera todos aplicarem, sobe seqnum visível,
notifica completion. São muitos waits/notifies — overhead de sincronização.

**Como melhorar [P2]:** Pebble usa uma **commit queue** lock-free
(single-producer, multi-consumer) que espelha a ordem de commit. Cada batch é
escrito individualmente no WAL (só memory copy), aplicado concorrentemente à
memtable, e publica seu seqnum removendo o prefixo já-aplicado da fila. Sem
leader, sem followers, sem group copy.

**Implicação PedraDB:** projetar o commit path (quando chegarmos a batches) com
uma fila de publicação de seqnum, não com group commit leader-based.

## 7. Range deletions desintegradas do merging iterator

**Crítica [P2] — Range Deletions:** RocksDB separa `MergingIterator` do
processamento de range tombstones (`RangeDelAggregator` é um oracle externo).
Isso **impede a otimização de pular blocos inteiros de chaves deletadas** — o
iterator percorre chave por chave dentro do range coberto.

**Como melhorar [P2]:** Pebble integra range tombstones **dentro do
`mergingIter`**: cada nível carrega um point iterator **e** um range-del
iterator. Quando um range tombstone em `Ln` cobre a chave atual (e é mais novo),
o iterator **seeka direto pro fim do range** em vez de iterar. Como um tombstone
em `Ln` é garantidamente mais novo que qualquer chave em `Ln+1`, o skip é seguro.
Posicionamento lazy dos range-del iterators só dos níveis que viram a chave
corrente.

**Implicação PedraDB:** arquitetura de iterator (Fatia 7) com range-del
embutido no merging iter desde o dia 1 — não como addon.

## 8. Pacing de flush/compaction frágil

**Crítica [P2] — Flush & Compaction Pacing:** RocksDB oferece um rate limiter
estático (`bytes/sec`) compartilhado entre flush e compaction. Frágil:
configurado baixo → stall; alto → latency spikes; precisa retuning por
hardware; não muda em runtime.

**Como melhorar [P2]:** Pebble usa **rate limiters separados** e pacing por
**invariante**, não por teto fixo:
- Flush: mantém "bytes restantes a flushear" constante ⇒ flush rate = write rate.
- Compaction: mantém "compaction debt" (bytes que precisam ser compactados)
  num nível-alvo constante ⇒ compacta só tão rápido quanto preciso.

**Implicação PedraDB:** Fatia 4 (Flush) e Fatia 6 (Compaction) com pacing por
invariante, não rate-limit estático.

## 9. Write throttling artificial prejudica latência em open-loop

**Crítica [P2] — Write Throttling:** RocksDB adiciona atrasos artificiais a
writes quando limiares como `l0_slowdown_writes_threshold` são atingidos. Em
modelos **closed-loop** (benchmark sincronizado) isso parece eliminar stalls.
Mas em **open-loop** (mundo real, arrivals independentes) o atraso artificial
**aumenta a latência sem benefício** — se writes chegam mais rápido do que o
sistema aguenta, delay artificial não resolve, só piora.

**Como melhorar [P2]:** Pebble **não** adiciona atrasos artificiais; serve
writes tão rápido quanto possível.

**Implicação PedraDB:** evitar write stalls artificiais. Se o sistema não
aguenta, satar (stall) explicitamente é mais honesto que degradar
silenciosamente.

## 10. APIs internas com indireção virtual excessiva

**Crítica [P2] — Other Differences:** RocksDB usa chamadas virtuais (vtable)
extensivamente no `InternalIterator`. Pebble minimiza indirect function calls
no hot path do iterator, elide checks de lower/upper bound por-key em long
scans, e tem API de iterator melhorada (`SeekPrefixGE`, `SetBounds`).

**Implicação PedraDB:** usar enums + match (dispatch estático) em vez de trait
objects no hot path de iterators. `SetBounds` como operação in-place no
iterator, não recriar.

---

## 11. Alocação subótima de Bloom filters (todas com o mesmo FPR)

**Crítica [M] — Monkey (SIGMOD 2017):** RocksDB (e todos os LSMs da época)
alocam a **mesma** false positive rate (FPR) para o Bloom filter de cada run,
independente do tamanho do run. Isso é matematicamente subótimo:

- O custo de pior caso de um point lookup é proporcional à **soma** das FPRs
  de todos os filtros.
- Como cada I/O para acessar um run custa o mesmo (fence pointers em RAM),
  dar mais bits ao filtro do run maior (que já é o maior) reduz pouco o FPR
  dele. Os mesmos bits alocados aos filtros dos runs menores (níveis acima)
  teriam impacto exponencialmente maior.

**Insight do Monkey [M]:** alocar FPR proporcional ao tamanho do run — os FPRs
dos níveis menores **decrescem exponencialmente**. A soma das FPRs vira uma série
geométrica que converge para uma constante **independente de L** (número de
níveis). Resultado: **shave de O(L)** no custo de lookup.

- RocksDB/LevelDB: lookup cost = O(L · e^(-M/N))
- Monkey: lookup cost = O(e^(-M/N)) — constante, não cresce com os dados.
- Monkey melhora latência de lookup em **50-80%** em experimentos.

**Implicação PedraDB:** na Fatia 3 (SST format), não usar FPR uniforme. Alocar
bits por filtro proporcional ao tamanho do run, com FPR decrescendo
exponencialmente dos níveis menores pros maiores. Isso é uma melhoria que custa
zero em tempo de execução e é puramente uma decisão de design na hora de
construir os filtros.

## 12. Merging supérfluo em todos os níveis exceto o último

**Crítica [D] — Dostoevsky (SIGMOD 2018):** esta é a crítica mais profunda ao
design do RocksDB. Todos os LSMs mainstream (RocksDB, LevelDB, Cassandra, HBase)
fazem merges de custo igual em **todos** os níveis. Mas:

> *"Merge operations from all levels of LSM-tree but the largest (i.e., most merge
> operations) reduce point lookup cost, long range lookup cost, and storage space
> by a negligible amount while significantly adding to the amortized cost of
> updates."* — [D]

Em outras palavras: ~90% dos merges que o RocksDB faz contribuem com quase nada
para reduzir lookup cost ou space amplification — só servem para inflar a write
amplification. Os gains de lookup e space vêm quase inteiramente do último nível
(o maior).

**Solução — Lazy Leveling [D]:** fazer leveling (1 run por nível) **só no último
nível**, e tiering (múltiplos runs acumulando) nos níveis menores. Isso melhora
o custo de update (menos write amplification) mantendo os mesmos bounds de point
lookup, long range lookup, e space amplification.

**Solução — Fluid LSM-tree [D]:** generalização do espaço de design que
parametriza a frequência de merge separadamente para o último nível e para os
demais. Pode assumir qualquer design existente (leveling puro, tiering puro,
bLSM, Lazy Leveling) como caso especial. "Dostoevsky" navega esse espaço
analiticamente para encontrar o tuning ótimo para dado workload e hardware, e
**estritamento domina** todos os designs existentes em performance e espaço.

**Implicação PedraDB:** na Fatia 6 (Compaction), não copiar cegamente o algoritmo
de compaction do RocksDB (leveling puro ou tiering puro). Projetar a estratégia
de compaction como algo configurável/adaptativo, com Lazy Leveling como default.
O modelo de custo do Dostoevsky (equações fechadas em [D]) pode guiar o tuning
automático.

## 13. Trade-offs não-lineares difíceis de co-tunar

**Crítica [D] + [M]:** a relação entre os knobs de design (size ratio T, merge
policy, buffer size, Bloom filter FPR) e performance é **não-linear**, e os
manuais de tuning do RocksDB são essencialmente "tente e veja". Não há um modelo
analítico que diga "para este workload e hardware, esta configuração é ótima".

**Implicação PedraDB:** incorporar as equações de custo de [D] como ferramenta
de auto-tuning, não depender só de heurísticas. O PedraDB pode expor um modelo
de custo que, dado o workload observado, recomenda (ou aplica automaticamente)
o tuning ótimo — algo que nem RocksDB nem Pebble fazem hoje.

---

## Engenharia em Rust: lições do fjall

**[F] — fjall 2.0:** o fjall é o LSM-tree KV store em Rust ativo mais maduro
que existe hoje. Lições diretamente aplicáveis:

- **`#![forbid(unsafe_code)]`** confirmado como viável e sustentável (o fjall
  inteiro é unsafe-free). Reforça a decisão do PedraDB.
- **Tipo `Slice` custom** em vez de `Arc<[u8]>` para valores — imutável,
  clonável, com futura otimização de alocação. A diferença material: o fjall
  mudou de `Arc<[u8]>` para um tipo próprio na 2.0 para poder controlar melhor
  a estratégia de alocação.
- **Key-value separation** (inspirado em BlobDB do RocksDB + Titan do PingCAP):
  valores grandes vão para um "value log" separado, reduzindo write
  amplification na LSM. É como overflow pages em B-tree.
- **Compressão per-partição** (LZ4 default, zlib opcional via `miniz-oxide`).
- **Garbage collection de valor log** em duas fases: scan + evict.

**Implicação PedraDB:** planejar desde cedo para o tipo `Slice` (Fatia 2), e
deixar a arquitetura preparada para key-value separation numa fatia futura.

---

| # | Decisão de design | Origem | Fatia |
|---|-------------------|--------|-------|
| 1 | `InternalKey` como struct, não string encoded | [P2] | 2 |
| 2 | MemTable com arena de tamanho fixo | [P2] | 2 |
| 3 | Tipo `Slice` custom para valores (não `Arc<[u8]>`) | [F] | 2 |
| 4 | Bloom filter FPR proporcional ao tamanho do run (decresce exponencialmente) | [M] | 3 |
| 5 | Batch como nível da LSM (seqnum high-bit) | [P2] | 4/7 |
| 6 | Commit pipeline com publish-queue lock-free, sem group-commit leader | [P2] | batches |
| 7 | Range tombstones integrados ao merging iterator + skip de blocos | [P2] | 7 |
| 8 | Flush/compaction pacing por invariante, não rate-limit estático | [P2] | 4/6 |
| 9 | Sem write-throttling artificial (stall explícito > degradação silenciosa) | [P2] | 6 |
| 10 | Dispatch estático (enum+match) no hot path de iterators | [P2] | 7 |
| 11 | `SetBounds` / `SeekPrefixGE` na API de iterator | [P2] | 7 |
| 12 | Lazy Leveling como estratégia default de compaction (tiering acima, leveling no último) | [D] | 6 |
| 13 | Modelo de custo analítico (equações de Dostoevsky) para auto-tuning | [D] | 6+ |
| 14 | Key-value separation preparada na arquitetura | [F] | futuro |
| 15 | Surface area enxuta: feature só entra se servir ao caso de uso | [P1] | sempre |

## Pendências de pesquisa

Concluídas nesta rodada (via `curl` direto):
- [x] ~~Survey LSM do Niv Dayan~~ → resolvido pelos papers [D] (Dostoevsky) e [M] (Monkey)
- [x] ~~Dostoevsky / Monkey~~ → baixados e analisados
- [x] ~~Engines Rust modernos~~ → fjall [F] analisado

Ainda pendentes:
- [ ] Issues abertos no facebook/rocksdb sobre compaction, amplificação, memtable
- [ ] slateDB (Rust LSM para object storage) — abordagem de log-structuring em cloud
- [ ] Críticas de hardware-consciousness (NVMe, direct I/O, io_uring)
