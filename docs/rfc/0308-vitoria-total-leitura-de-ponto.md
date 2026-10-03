# RFC-0308: Vitória Total em Leitura de Ponto — `lookup_100` (get_loop/multi_get), `get_hit` e `cold hit` em Todas as Escalas, Inclusive >500M

**Status:** Aprovado para Implementação
**Data:** 3 de Outubro de 2026
**Base:** `b7fcde6` + RFC-0307 (P0 fix `range_deleted(key, seq, …)` e P1 flat-HIs em validação)
**Evidência-mestre:** ladder ×3 mediana (`2026-10-03-snapshot-ladder-b7fcde6-3x-full.pdf`, 54 células, code=0)

---

## 1. As três derrotas (números da ladder, mediana ×3)

### 1.1 `lookup get_loop` — derrota para o fjall em TODAS as escalas

| Escala | fjall | Rocks | Pedra | gap vs melhor |
|---|---|---|---|---|
| 1M | **167µs** | 210µs | 214µs | −28% |
| 10M | **745µs** | 664µs | 856µs | −29% (e −22% vs Rocks) |
| 25M | **926µs** | 885µs | 1.050µs | −29% |
| 100M | **2,55ms** | 3,25ms | 3,25ms | −28% |
| 250M | **3,72ms** | 3,81ms | 4,24ms | −14% |
| 500M | **5,92ms** | 21,0ms | 7,04ms | −19% |

### 1.2 `lookup multi_get` — mesmo padrão (e o Rocks tem batch REAL)

| Escala | fjall | Rocks | Pedra | gap |
|---|---|---|---|---|
| 1M | **176µs** | 182µs | 221µs | −26% |
| 250M | **3,62ms** | 4,14ms | 4,39ms | −21% |
| 500M | **6,24ms** | 22,1ms | 8,14ms | −30% |

### 1.3 `cold hit p50` — derrota para o Rocks nas escalas médias

| Escala | fjall | Rocks | Pedra | gap vs melhor |
|---|---|---|---|---|
| 1M | **2,9µs** | 11,7µs | 12,9µs | −4,4× (fjall) |
| 10M | 9,6µs | 13,3µs | 14,9µs | −55% (fjall) |
| 25M | 18,1µs | **10,3µs** | 15,6µs | −51% (Rocks) |
| 100M | 50,8µs | **40,7µs** | 48,2µs | −19% |
| 250M | 77,9µs | **44,4µs** | 54,9µs | −24% |
| 500M | 46,6µs | 65,1µs | 58,4µs | vencemos o Rocks, −25% vs fjall |

### 1.4 `get_hit` — derrota estreita e constante (a causa-raiz das outras duas)

| Escala | fjall | Rocks | Pedra | gap vs melhor |
|---|---|---|---|---|
| 1M | **1,78µs** | 2,14µs | 2,23µs | −25% |
| 10M | 7,52µs | **6,52µs** | 8,70µs | −33% |
| 25M | 9,36µs | **8,96µs** | 10,85µs | −21% |
| 100M | **27,2µs** | 34,3µs | 31,3µs | −15% |
| 250M | 43,5µs | **38,3µs** | 41,9µs | −10% |
| 500M | **47,7µs** | 238,9µs | 68,5µs | −44% |

**Leitura estrutural:** o gap do `get_hit` é ~0,45–2µs POR OPERAÇÃO e não cresce
com a escala (as curvas já são lineares pós flat-bounds) — é **glue constante por
leitura**, não complexidade. `get_loop` ≡ 100× esse glue (a conta fecha: 214µs/100 =
2,14µs ≈ get_hit@1M). `multi_get` do nosso adapter ≡ N×get (código:
`snapshot_pedradb.rs:240-247` — `keys.into_iter().map(|k| self.get(k))`), enquanto o
Rocks tem MultiGet real. `cold hit` = o mesmo caminho + first-touch. **Uma família de
causas, três sintomas.**

---

## 2. Anatomia do caminho de leitura (inventário de custo por get, com código)

O bench chama `reader.get(k)` → `db.get_cf(&cf_data, k)` (`snapshot_pedradb.rs:228`)
→ `get_cached` (`lib_kernel.rs:2918`). Custo por operação no estado quente:

| # | Passo | Código | Custo | Removível? |
|---|---|---|---|---|
| 1 | encode `cf\0key` no buffer TLS | `get_cached` ENC (2935-2943) | cópia 30B | necessária 1× |
| 2 | envelope settled check | `fast_encoded_miss` (2946) | 3 loads atômicos | ok |
| 3 | `point_tls_epoch()` + `key_tls_gen(enc)` | 2951-2952 | 2 loads + **FxHash de 30B** | o hash repete o do passo 8 |
| 4 | LAST_CF lookup | 2953 | direct-map (Slots) | barato |
| 5 | `point_cache.get(enc)` | `ConcurrentDb::get:2692` → `AnswerCache::get` | **MUTEX global + HashMap lookup A CADA GET, mesmo em MISS** | **sim — shard** |
| 6 | `inner.try_read()` + `get_after_point_miss` | 2696-2698 | RwLock read + snapshot + lookup completo | o lookup é o trabalho real |
| 7 | fill do point-cache | `db_kernel.rs:3800-3808` | lock #2 + insert (alloc) por miss | sim |
| 8 | volta compat: `LAST_CF.store` no Some | 2961 | TLS store | só em hit |
| 9 | `decode_entry` (harness) | comum aos 3 engines | — | — |
| 10 | `.to_vec()` do valor no retorno do compat | `Ok(hit.map(\|b\| b.to_vec()))` 2956/2963 | **cópia 200B + alloc POR GET** | **sim — API Bytes** |
| 11 | probe da tabela ladder | probe closure + run linear (`tables/get 1,985`) | bloom + seek overhead | **sim — merge de runs** |

Cold (pós-reopen) acrescenta, por tabela tocada no primeiro get:
- decode lazy da partição do bloom (`parts_once` OnceLock; `encoded_parts` residente
  mas páginas frias) — `FILTER_PART_LOADS` conta;
- primeira falta nas páginas do índice (`Arc<Vec<BlockHandle>>`, já em RAM);
- TLS/point-cache frios (passes 3-7 acima pagam integral).

**Onde o fjall ganha hoje:** sem codec CF (chave nua), sem camadas TLS/point-cache
com mutex global, sem probe de tabela ladder extra, blocos menores. Onde PODEMOS
ganhar: batch real (multi_get), um único probe (run merged), cache de respostas sem
mutex, sem cópia de saída, warm de first-touch.

---

## 3. Pilares

### Pilar A — `multi_get` de verdade (engine batch) [vence 1.2, ajuda 1.1]

**Design.** Novo `ConcurrentDb::multi_get_batch(keys: &[&[u8]]) -> Vec<Option<Bytes>>`:
1. **Um** `visible_sequence()` e **um** clone de SuperVersion para o lote todo.
2. **Plano de probes**: ordenar os índices por chave codificada (sort 100 elementos,
   ~1µs) → uma passada pelos runs usa o MESMO `Bound32` bisect com **busca galópica
   entre chaves consecutivas** (o cursor anda só para frente: 100 chaves ≈ 100 probes
   totais em vez de 100×log K, e o índice/bloom da tabela fica quente entre chaves
   da mesma tabela — localidade que nem o Rocks tem).
3. Resultado em `Vec` pré-alocado uma vez (reuso via parâmetro out em `multi_get_cf`).
4. Fills do point-cache em lote (um lock, N inserts) sob validade única de seq.
5. Adapter do harness passa a chamar `multi_get_cf` real (`snapshot_pedradb.rs`).

**Alternativas consideradas.** (a) Paralelizar o lote em workers — rejeitado: 100
chaves × µs não paga spawn; (b) pipelined prefetch de blocos (async) — rejeitado por
escopo: exige io_uring readv; fica registrado para RFC futura.

**Alvo:** multi_get ≤ 0,6× fjall em 1M–500M (batch + localidade), get_loop imune
(mede o caminho single), mas o Pilar B/E derrubam o get_loop junto.

### Pilar B — Um único probe por get: merge de runs no settle [vence 1.1/1.4]

**Diagnóstico:** `tables/get 1,985` — todo get paga o run bulk (biseccionado, O(log K)
flato) **mais o run ladder linear** (a tabela inicial mista, cujos bounds cobrem a
família toda → nunca rejeitável por bounds).

**Design.** No settle (e a cada install incremental quando barato), **fundir os runs
disjuntos do mesmo nível**: se `max(hi(run_i)) < min(lo(run_j))` entre dois runs do
mesmo nível com nenhum range-tombstone atravessando, eles viram UM run com by_lo/flat
concatenados (merge O(K) de arrays já ordenados — `merge_sorted_flat` kernel novo).
Pós-settle a forma do bench (ladder + bulk disjuntos) colapsa para **1 run disjunto**:
`tables/get → ~1,0`, o ladder some do caminho quente.

**Invariantes.** A fusão preserva a propriedade "pairwise disjoint em user-key space"
(a checagem de inventário nível-a-nível continua sendo o oráculo); runs com
range-tombstones que atravessam a fronteira NÃO fundem (o tombstone vive na tabela
original e a coleta por run continua correta — o skip por run do RFC-0306 P2
permanece).

**Verus:** o predicado puro `can_merge_runs(a: RunSummary, b: RunSummary) -> bool`
disjoint+no-crossing-tombstone vira kernel `no_std` com prova (mesma família do
`run_pairwise_disjoint_los` existente). Anti-vacuidade: par de mutants
(crossing-tombstone deve RECUSAR fusão; overlap deve RECUSAR) no battery M1..M7.

### Pilar C — Point-cache sem mutex global + hash único [vence 1.1/1.4]

**Design.**
1. `AnswerCache` vira **S=16 shards** por hash da chave (`lock = key_hash % 16`):
   sem contenção até 16 leitores; mesma semântica de gen/epoch por shard; `clear`
   gira gen em todos (Release) — mesma garantia F198.
2. **Hash único por get**: o FxHash do `enc` calculado UMA vez no `get_cached` e
   passado para `key_tls_gen_bucket(hash)` e para o shard do point-cache (elimina o
   segundo hash do passo 3 e o re-hash do passo 5).
3. **Cache negativo seq-guarded**: com o fix RFC-0307 P0 (validade por seq REAL),
   cachear `None` passa a ser seguro (a invalidação por dirty-points cobre a chave;
   fat/range limpa o shard-set). Bounded pela capacidade existente. Vende o
   miss-in em loops (get_loop misto tem ~0% miss com n do bench — mantém para
   workloads reais).

**Loom:** o esquema de shards + gen-bump é estado compartilhado ≤3 threads no
modelo — caso de teste Loom (leitor/escritor/limpador, ausência de resposta stale
após gen-rotate), roteando as ESTRUTURAS de produção via `sync_kernel` (política
Zero-Twin).

**Alvo:** passo 3+5 de ~150-250ns (contenção zero em bench single-thread, mas o
custo existe: 2 hashes + lock) → ~40ns.

### Pilar D — Saída sem cópia + decode-once [vence 1.4, reforça 1.1]

**Design.**
1. `get_cf_bytes(&CF, key) -> Option<Bytes>` novo no compat (e `get_cached_bytes`
   interno): retorna `Bytes` (refcount) — o LAST_CF já guarda `Bytes`; o caminho
   hit devolve SEM `.to_vec()`; o decode do harness lê por slice. Rocks também
   devolve cópia (API C) — aqui ganhamos ~50-100ns + 1 alloc/get.
2. `resolve_stored_value` já é slice; auditar a última cópia do block-decode
   (payload block → `Bytes` de saída: garantir slice do payload promovido quando
   o valor é inline no bloco — hoje `point_at_seeking` copia o valor para o
   scratch: devolver `Bytes::slice` do bloco decodificado quando o bloco está no
   plain-cache, zero cópia).

**Mutação:** os kernels de extração de valor (slice vs copy) entram no battery
mutation-fuzzer (mutante "retorna slice errado" tem que matar teste de igualdade
byte-a-byte com oráculo).

### Pilar E — Cold hit: warm de first-touch no open/install [vence 1.3]

**Diagnóstico:** cold = quente + decode da partição do bloom (OnceLock lazy) +
primeiras faltas de página (encoded_parts + índice + payload) + TLS/point-cache
frios. O Rocks "perde" menos no cold porque o filter block dele é lido no primeiro
acesso ao arquivo e fica no block cache (32MiB) — nosso equivalente é mais laziness.

**Design.**
1. **Bloom warm no open**: decodificar as partições (materializar `parts_once`) na
   abertura de cada tabela — custo linear no tamanho do filtro (~350KB/tabela a
   64MiB chunks; ~44MB no total a 100M — cabe no warm-cap RFC-0194 ¾-limite).
   Paralelizável nos workers existentes de open (FileHandleCache já é async).
2. **madvise WILLNEED** do corpo+índice das tabelas no open (bounded pelo
   warm-cap; DONTNEED do payload continua mandando no ciclo de vida).
3. **TLS pre-arm**: primeira `get_cf` materializa ENC+LAST_CF uma vez (custo fixo
   hoje pago dentro da medição cold).
4. **Contadores novos no probe** (fase `cold-first-100`): part-loads/get,
   minor-faults/get (via `task_info`/`/proc/self/stat` diff), tables/get — a
   atribuição fria vs quente vira número, não hipótese.

**Alvo:** cold hit ≤ max(fjall, Rocks) × 1,0 em 1M–250M; a 500M já vencemos o Rocks.

**DST:** o warm-to-DONTNEED-to-warm de novo introduz transições de residência —
vetor DST swarm-disco (PEDRA_SWARM_DISK=1) com shape reopen-mix-get para caçar
janelas de visibilidade do payload (regra AGENTS: claim só com disco real).

---

## 4. Escala >500M (1B, 2B)

- **Tabelas:** 1B/64MiB ≈ 1.800; 2B ≈ 3.600. Flat bounds: log₂(3600) ≈ 12 steps em
  array contíguo de 115KB (3600×32B) — cabe em L2; o custo por get é constante.
- **Metadata total:** bloom ~1,25B/chave → 1,25GB a 1B — NÃO cabe em cache; cada
  probe paga DRAM (~80-100ns) — igual ao Rocks (filter blocks dele também não
  cabem). O Pilar A (localidade de lote) é a vantagem diferencial em escala: 100
  chaves por tabela tocam a MESMA partição de bloom.
- **Envelope/checkpoint:** O(#CF) — constante.
- **Chunk size:** manter 64MiB (self-tune já validado); acima de ~4K tabelas o
  custo de publicação (rebuild O(K)) pode passar a incremental-merge do Pilar B —
  medir com o counter de publish_ns a 1B antes de mudar.
- **RAM:** orçamento efetivo cgroup (RFC-0274) continua o guardião; o bloom warm
  do Pilar E adiciona ~0,6% do dataset — dentro do warm-cap.
- **500M→1B hydrate:** 2,35 M/s não deve cair (engine steady ~130ns/op medido);
  se cair, o batch-tail do probe aponta o gate.

## 5. Tecnologias (mapeamento completo)

| Tecnologia | Onde entra | Artefato/gate |
|---|---|---|
| **Verus** | `can_merge_runs` (Pilar B), validade do cache negativo (C), predicados de recall dos fast-paths | kernels `no_std` com prova; mutants de refutação no M1..M7 |
| **Loom** | shards do AnswerCache + gen-rotate (C) | modelo ≤3 threads com estruturas reais via `sync_kernel` |
| **Stateright** | protocolo de visibilidade do warm (E): estados residency × get | modelo com `fn`s de produção nas transições |
| **Mutation fuzzer (M1..M7)** | todos os kernels novos (merge de flats, plano de batch, slice de saída, warm bounds) | score ≥98% de mortos antes do claim |
| **DST swarm-disco** | Pilar E (residência), Pilar B (runs fundidos × replay de crash) | campanha com `PEDRA_SWARM_DISK=1`, oráculo twin |
| **PCT** | multi_get concorrente com writers (A) | campaign PCT com gets/writes/flush intercalados |
| **perf_probe** | fase cold-first-100 + counters (part-loads, faults, tables/get) e fase multi_get_100 | números antes/depois por pilar, caixa |
| **Oráculo diferencial** | armadilha permanente (pre-merge check) + campanha ×20 por pilar | zero PRE-MERGE STALE |
| **rocks-parity-bench** | células get_hit/ycsb_a/ycsb_c pós-pilares (paridade oficial RFC-0041) | floor 1.0 mantido |
| **suítes** | compat 140+, bulk 55, lookup 16, get 66 por pilar; mediana ×3 na caixa | gate de publish |

## 6. Gates numéricos (caixa, mediana ×3, defaults)

| Métrica | 1M | 10M | 25M | 100M | 250M | 500M |
|---|---|---|---|---|---|---|
| get_hit vs melhor peer | ≤0,80× | ≤0,80× | ≤0,80× | ≤0,85× | ≤0,85× | ≤0,70× |
| get_loop vs melhor | ≤0,80× | ≤0,80× | ≤0,80× | ≤0,85× | ≤0,85× | ≤0,80× |
| multi_get vs melhor | ≤0,60× | ≤0,60× | ≤0,60× | ≤0,65× | ≤0,65× | ≤0,60× |
| cold hit vs melhor | ≤1,00× | ≤1,00× | ≤1,00× | ≤1,00× | ≤1,00× | manter vitória |

E sem regressão: hydrate ≥2× Rocks, settle ≤2s@500M, miss ≤150ns, prefix_scan
≤ Rocks+5% em toda a ladder.

## 7. Riscos e ordem de rollout

1. **C+D primeiro** (sem mudança de semântica; maior alvo de removede glue): shard +
   hash único + Bytes-out. Risco: API nova no adapter (baixo).
2. **B** (merge de runs): invariante de disjointness — Verus + guarda de inventário
   no settle. Risco médio; revertível (flag interna de fusão desativável).
3. **A** (multi_get batch): concorrência com writers — PCT + suítes txn.
4. **E** (cold warm): residência × DONTNEED — DST disco + contadores primeiro,
   warm gradual (bounded) depois.
5. Cada pilar = um commit + uma rodada de ladder ×3 (a campanha de 3h da caixa é o
   gate; nenhum pilar publica sem ela).

## 8. Não-negociáveis

Zero knobs (tudo default/auto); nenhuma vitória sobre estado errado (P0 do RFC-0307
é pré-requisito — já corrigido no worktree); recall nunca sacrificada por fast-path
(os fail-safes de bounds continuam como oráculo); claims só com disco real
(PEDRA_SWARM_DISK=1) e mutation score ≥98% nos kernels novos.
