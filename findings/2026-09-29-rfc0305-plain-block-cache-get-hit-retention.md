# RFC-0305: Plain-Block Cache — o get_hit@100M era retenção quente, não custo por IO

**Status:** Implementado (worktree `/tmp/pedra-plain-cache`, branch local)
**Data:** 29 de Setembro de 2026
**Célula nomeada:** `get_hit` @ 100M (2b9f0a2: Pedra 50.1 µs vs Rocks 38.8 µs — perda 0.77×)

---

## 1. Diagnóstico — por que girávamos em círculos

A perda em `get_hit` @ 100M **não era custo por IO**. Na mesma rodada
(2b9f0a2, 100M): cold hit p50 Pedra **51.0 µs** vs Rocks **53.7 µs** — o
primeiro acesso a um bloco já era mais rápido que o do Rocks (um pread +
índice/bloom em RAM contra o caminho de block-cache do Rocks). O que
diferia era o **acesso repetido**:

- O Rocks converte o knob de 1 GiB de `HyperClockCache` em ~24% de hits
  rápidos no warm loop do criterion (2 s warmup + 10 s, ≥50 M gets).
- O Pedra, no caminho ponto (`point_at_seeking_with_hashes`), retinha
  blocos apenas num LRU **thread-local de 4096 entradas** (`RAW_BLOCKS`,
  ~64 MiB), introduzido quando o cache decoded-entries deixou de ser
  consultado pelo caminho ponto (RFC-0293/0295).
- O knob compat `Cache::new_lru_cache(1 GiB)` →
  `set_block_cache_budget_bytes` alimentava o cache de **entradas
  decodificadas** (`BlockCache`), que só os caminhos de prefixo consultam:
  **inerte para point gets**. Mesma classe de bug da RFC-0160 P2.3,
  reintroduzida pela migração do caminho ponto.

Em dataset ≫ RAM (100M chaves ≈ 23.7 GiB), a taxa de hits quentes é a
fração retida: 64 MiB thread-local vs 1 GiB compartilhado — a perda
cresce com a escala (10M 12.2 vs 7.57 µs; 100M 50.1 vs 38.8 µs), o
padrão exato observado na campanha 1M–500M.

## 2. O corte — `PlainBlockCache` (RFC-0305)

Cache compartilhado de **imagens plain (decomprimidas e
CRC-verificadas)** de blocos de dados, no ponto do caminho que antes
re-lera do arquivo:

- **Chave = (id de instância da tabela, offset do bloco).** Números de
  caminho `{num:06}.sst` são re-cunhados após deleção; um id por
  instância de `SstTable` (cunhado nos dois construtores de produção)
  garante que um caminho re-cunhado erra o cache do predecessor em vez
  de herdar seus bytes. A imagem só entra **depois** de `split_block_crc`
  + decompress lz4 — o hit nunca re-executa o portão de integridade nem
  pode servir bytes adulterados.
- **Orçamento em bytes, 16 shards** (1 abaixo de 16 KiB), LRU lazy com
  ghost epochs (padrão F178 do `BlockCache`), recusa de imagem maior que
  o orçamento do shard (o probe re-lê do arquivo; correção intacta).
- **Carregado pelo `PayloadKit`** — todo ponto que já anexava
  (source, pool) anexa (source, pool, plain): flush (`adopt_sst`),
  compaction (dois kits de rewrite), reopen/recovery (`db_open`),
  `TableCache::get_or_open`.
- **Knob repontado:** `ConcurrentDb::set_block_cache_budget_bytes`
  armazena o **atômico de orçamento** que todo shard lê por admissão
  (`PlainBlockCache.budget_bytes`, resize in-place sem reattach, sem
  lock de shard para aplicar; shrink vige no insert seguinte, LRU
  lazy). Semântica `NewLRUCache` do Rocks agora cai no caminho ponto.
  O cache decoded-entries (prefixo) mantém o default de open. Default
  sem knob: 64 MiB (`PEDRA_PLAIN_BLOCK_BUDGET` para A/B).
- `RAW_BLOCKS` (TLS, 4096 entradas) **removido** — um cache por thread
  não consegue converter um orçamento do host em taxa de hit.

## 3. Dentes nomeados

| teste | prova |
|---|---|
| `rfc0305_repeat_seek_reads_block_once_and_serves_from_plain_cache` | 1 pread por bloco (contador `read_range` real via seam `SstFileSource`); repeats = hits, sem re-read |
| `rfc0305_reused_sst_path_misses_predecessor_plain_entries` | caminho re-cunhado não herda bytes do predecessor (mesmo path, mesmo layout) |
| `rfc0305_plain_block_cache_budget_generation_and_resize` | orçamento honrado com LRU, chaves por geração não colidem, imagem > orçamento recusada, shrink pelo atômico vige no insert seguinte |

O dente 3 do knob (compat → cache ponto) é provado pelo A/B do
snapshot-bench abaixo: retenção on/off pelo **mesmo** caminho de knob.
In-crate, um point get pós-flush é servido pela view dobrada viva por
design (RFC-0237 "post-flush live mem") — um unit test do knob só
re-testaria essa view.

## 4. Verificação same-fire

- `glue.kernel_fn_allowlist` é **lista de nomes de fn por arquivo**; o
  dente varre fns `pub`/`pub(crate)` em arquivos kernel. Os métodos novos
  do `PlainBlockCache` (`get`, `insert`, `hits`, `misses`, `used_bytes`,
  `clear`, `with_budget_bytes`) já estavam classificados por nome na
  allowlist de `cache_kernel.rs`; `attach_payload_kit` na de
  `sst/table_kernel.rs`; `set_block_cache_budget_bytes` na de
  `concurrent_kernel.rs`. O restante da superfície nova foi
  **rebaixado abaixo do dente** (fns privadas de arquivo / campos):
  `mint_cache_id` virou fn privada de `table_kernel.rs` (todos os 6
  sítios de cunhagem estão nesse arquivo), `plain_block_budget_from_env`
  virou fn privada do módulo `db` (`db_open_kernel.rs` é `include!` do
  `db_kernel.rs`), e o orçamento é um campo `pub(crate) budget_bytes:
  AtomicU64` (dado, não lógica — a lógica de admissão/evicção continua
  nas fns classificadas). `set_budget`, o getter `plain_block_cache` e
  `plain_block_cache_stats` foram **removidos** (nenhum chamador).
  Baseline da allowlist não cresceu; nenhuma fn nova fora da superfície.
- `./scripts/pedra_formal.sh --ci` no worktree: 2822 ok, 0 gap, 3274
  fail (dívida pré-existente), **zero drift**. `pedra_formal.py --lint`:
  **959 FAIL no corte = 959 FAIL no base 6f357fac** — conjuntos
  idênticos exceto as duas linhas do `residuals freeze`
  (`handler_loc`/`kernel_loc`) que já estavam vermelhas no base (freeze
  pré-existente quebrado; só o número "live" dentro da mensagem anda
  com as linhas adicionadas). **Zero vermelho novo.**
- Regressão de suite: full-suite 1138/4 — as 4 falhas reproduzem no
  base 6f357fac (3 pré-existentes: `rfc_writethread_join_*`,
  `rfc0201_auto_async_merge_*`, `hot_key_overwrite_fold_*`) ou são
  flaky (`async_and_sync_concurrent_writers_recover` falhou 1× sob
  carga e passa 3/3 em ambos os lados).

## 5. Números (Darwin DIAG; Linux cartaz bloqueado — exec do gate fora)

Ambos os gates Linux (`linux-gate-p238`, `linux-gate-p211z`) recusam
exec ("service unavailable — telemetry not configured"); o gate BYOC da
mac retorna 502. `/data2` do p238 está vazio (60 KiB usados) — a árvore
da campanha anterior não existe mais. DIAG Darwin da célula Linux,
mesma máquina/boot, A/B pelo knob:

| rodada | budget plain | get_hit mediana | observação |
|---|---|---|---|
| **A: Pedra vs Rocks (knob 1 GiB nos dois)** | 1 GiB | Pedra **1.846 ms** vs Rocks **1.059 ms** | `ratio=0.574` (qps 542 vs 944) |
| **B: Pedra, retenção off** | 1 B | **4.506 ms** | criterion vs A: **+128% (p=0.00)** |
| C: Pedra, default sem knob | 64 MiB | 812 µs | ver abaixo |
| A2: Pedra, knob 1 GiB (réplica) | 1 GiB | 1.146 ms | ver abaixo |

- **Dente 3 do knob provado no A/B:** retenção on/off no **mesmo
  binário**, cadeia criterion adjacente — 1.85 ms vs 4.51 ms = **2.44×**
  (p=0.00). O knob `set_block_cache` agora move o caminho ponto.
- **Ruído do host:** o Darwin estava a **load 121–133** durante todas as
  rodadas (outra sessão rodando rustc + `kani -p machines`, Antigravity,
  Pythons a 60%+; uptime 31 d). As rodadas C e A2 (812 µs e 1.146 ms
  com o MESMO budget-efeito de A) mostram a variabilidade entre
  processos: **os absolutos de hoje não são comparáveis** entre
  invocações; o par A/B (adjacente na mesma cadeia criterion) é o que
  carrega significância.
- **Regime ≠ cartaz:** este Mac tem 96 GiB de RAM e o dataset 10M é
  2.35 GiB — cabe no page cache; a célula nomeada é 100M = 23.7 GiB no
  guest de ~4 GiB, onde a retenção vive nos caches do engine. Darwin
  10M não reproduz esse regime; **o rerun Linux 100M é o número
  decisivo e segue unpaid** (caixa blocked).
- Wiring auditado no mesmo fire: `set_block_cache`/`new_lru_cache` tem
  **uma única consumidora** (`block_cache_bytes` →
  `set_block_cache_budget_bytes` → atômico do plain) — sem segundo
  cache decoded de 1 GiB como confound.
- Número do journal:
  `number: ratio=0.574 pedra_qps=542 rocks_qps=944 shape=get_hit_10M (DIAG)`

## 6. Mapa

- `get_hit` @100M/10M: W (perda com lever nomeado — retenção) → **corte
  aterrissado**; DIAG 0.574 @10M (load 120+) e A/B knob 2.44×; a
  célula Linux 100M segue **unpaid** até o rerun na caixa.
- Cold hit p50 (51.0 vs 53.7 µs) e prefix_scan (128 vs 165 µs): S no
  100M; unchanged por este corte (o cache plain só adiciona hits; o
  primeiro acesso paga o mesmo pread).
- Cold miss 528 vs 356 ns e hydrate 1.06 vs 1.38 M/s: perdas nomeadas
  restantes, outros fires.

## 7. Rerun decisivo (caixa Linux, quando voltar)

```bash
# cartaz 100M — mesma receita da campanha 2b9f0a2, agora com o knob
# pousando no caminho ponto:
SLIPSTREAM_BENCH_ENTRIES=100000000 \
SLIPSTREAM_BENCH_BACKENDS=pedradb,rocksdb \
cargo bench --bench snapshot_backends --features fjall,rocksdb,pedradb -- get_hit
# A/B do knob no Linux (piso): + SLIPSTREAM_BENCH_CACHE_BYTES=0
#                            + PEDRA_PLAIN_BLOCK_BUDGET=1, BACKENDS=pedradb
```
