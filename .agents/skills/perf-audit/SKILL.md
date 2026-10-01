---
name: perf-audit
description: >-
  Performance audit toolkit for PedraDB: deterministic per-phase probing
  (perf_probe with a counting allocator), call-graph analysis of `sample`
  dumps (perf_calltree.py), the engine's env-gated diagnostics inventory,
  and the bottleneck-class checklist (blocking, loops, locks, contention,
  allocation). Use before ANY performance claim or optimization: measure
  the phase, attribute the cost class, only then cut — and validate with
  the same tool on the same box. Never optimize from a full-stack
  benchmark number alone.
---

# `/perf-audit` — Auditoria Determinística de Performance

> DOUTRINA: **nenhum corte sem atribuição de fase**. Benchmark de ponta a
> ponta (100M) mede a caixa, não o engine. Atribua o custo à fase
> (apply/miss/hit/encode), à classe (blocking/loop/lock/contention/alloc)
> e ao sítio (call-tree) ANTES de editar. Depois valide com a mesma
> ferramenta. Toda medição local é repetida 3× e usa a mediana/mínimo —
> nunca uma run única.

## 1. Ferramentas (nesta ordem)

### 1.1 `perf_probe` — sonda determinística de fases (CPU-side, qualquer máquina)

```bash
cargo run --release -p rocksdb-compat --example perf_probe
PEDRA_HYDRATE_DIAG=1 cargo run --release -p rocksdb-compat --example perf_probe
```

Mede, com contador global de alocações e forma fixa (204.800 entradas,
semente fixa, batches 1024 monotônicos — o shape slipstream):

| fase | o que prova | alvo atual |
|---|---|---|
| apply µs/batch + allocs/op | caminho latched inteiro | < 60µs, ≤ 0,5 alloc/op |
| miss-out ns + allocs/op | P0 envelope pré-TLS | < 200ns, 0 allocs |
| miss-in ns + allocs/op | bloom/bloco do miss interno | < 500ns, 0 allocs |
| hit-warm µs + allocs/op | caminho de leitura quente | o alvo aberto |

`PERFPROBE_JSON` no fim serve para A/B (`antes` vs `depois` no mesmo
binário/máquina). Rodar 3× e comparar os JSONs; dispersão > 15% entre
runs = máquina contaminada (ver §4).

### 1.2 `perf_calltree.py` — call-graph de `sample` (macOS)

```bash
# durante a fase quente do bench (pegar o PID do binário certo!):
sample <pid> 10 -file /tmp/s.txt
python3 scripts/perf_calltree.py /tmp/s.txt --threads          # quem dorme vs trabalha
python3 scripts/perf_calltree.py /tmp/s.txt --blocking         # mapa de bloqueio
python3 scripts/perf_calltree.py /tmp/s.txt --top 20           # self-time (folhas)
python3 scripts/perf_calltree.py /tmp/s.txt --under write_sst  # subtree de um símbolo
python3 scripts/perf_calltree.py --diff antes.txt depois.txt --under hydrate
```

Cuidados: `pgrep -f snapshot_backends` pega o `cargo` pai — filtre pelo
hash do binário em `target/release/deps/`. O sample de 10–12s tem que
cair NO MEIO da fase (hydrate 100M dura ~85s; sample aos 20s).

### 1.3 Diagnósticos do engine (env, custo ~zero quando off)

| env | revela |
|---|---|
| `PEDRA_HYDRATE_DIAG=1` | `HYDRATEDIAG batches/debt_ms/lock_ms/apply_ms/publish_ms` no settle e no drop — o orçamento do writer |
| `PEDRA_FLUSH_STAGES=1` | `FLUSHSTAGES enc/crc/write ms por chunk` — o custo do codificador SST |
| `PEDRA_FLUSH_DIAG=1` | `FLUSHDIAG`/`COMPACTDIAG` — camadas de memória e níveis por segundo |
| `PEDRA_BULK_DIAG=1` | `BULKDIAG` — decisões de instalação bulk |
| `PEDRA_ARENA_DISABLE=1` | A/B do arena de staging (P1) |
| `PEDRA_BULK=0` | mata o fast-path bulk (controle) |

### 1.4 Bench de ponta a ponta (veredito, não diagnóstico)

`snapshot-bench` (workspace próprio), protocolo oficial: 200B, cache
256MiB, `PEDRA_STAGE_MAX_BYTES=67108864`, legs separadas. **Amostragem de
get_hit a 100M é curta (30×10s): sempre 2–3 repetições antes de chamar
regressão** — a história mostra ±35% entre runs do MESMO SHA.

## 2. Checklist de classes de gargalo (ache TODAS antes de cortar)

1. **Blocking** (thread parada): `--blocking` (nanosleep/cvwait/yield/
   recv_timeout/swtch_pri). Pergunta: quem espera o quê? (ex.: writer
   esperando write-lock segurado por install O(K)).
2. **Loop** (custo por operação cresce com K): grep alvo — `iter().
   position(`, `.windows(`, `.all(`, `.any(`, `for .* in 0..self.ssts`,
   `clone()` de coleções por operação. Verificar complexidade POR BATCH,
   não por entrada (K = nº de SSTs cresce com o DB — é o decaimento).
3. **Lock** (espera de exclusão): `lock_exclusive_slow` no sample; ler o
   que seguro o lock: tudo que está entre `write()` e `drop(g)` no core é
   crítico — I/O, clone de coleção e env::var não entram ali.
4. **Contenção** (muitos leitores brigando): `lock_shared_slow` +
   `cthread_yield` em TODOS os workers = poll por read-lock em Hz.
   Correção padrão: espelho atômico (ex.: `parked_bulk_count`).
5. **Alocação**: `perf_probe` allocs/op. Alvos: miss = 0; get quente ≤ 2
   (o `Vec` de retorno é inevitável); apply ≤ 0,5/op. Anti-padrão
   documentado: **arena por batch** (aa6d24e, 0,57 M/s) — capacidade tem
   que sobreviver ao batch; slab com rollover nunca copia acumulado.
6. **Syscall/IO**: `pwrite`/`pread` no sample; agrupar writes (4MiB
   staging já existe); fsync fora do lock SEMPRE.

## 3. Método (ordem obrigatória)

1. `perf_probe` 3× → tabela de fases + allocs (a "previsão determinística").
2. A fase fora do alvo → `sample` no meio da fase → `perf_calltree
   --threads --blocking --top` → classe + sítio.
3. Hipótese escrita em uma linha ("apply custa X porque SÍTIO faz CLASSE").
4. Corte mínimo; teste de dente (comportamento + invalidação);
   `perf_probe` 3× de novo → mesma tabela, números novos.
5. Suites (bloom/bulk/sst/compat) + push; veredito na caixa de bench com
   o env diag da fase.

## 4. Higiene de medição (senão tudo acima mente)

- Máquina com load > ~2× núcleus ou disco > 90% cheio: **medir CPU-side
  (perf_probe mediana) ainda vale; benchmark de disco NÃO vale**.
- Nunca comparar runs de máquinas/rodadas diferentes para regressão de
  µs — A/B no mesmo processo ou mesma rodada.
- `get`/`get_cf` mudou? Rodar as suítes compat INTEIRAS (checkpoint
  pegou falso-None cross-DB que o review não viu).
- Toda mudança de wake/throttle de worker é SEMâNTICA (latência visível):
  o throttle de notify_compact quebrou `auto_compact_when_sst_count…` —
  despertar é contrato, não ruído.

## 5. Estado do mapa (RFC-0306, atualizar ao cortar)

- miss-out: 146ns/0 allocs ✅ (P0 no ar)
- miss-in: ~1,7µs/0,9 allocs → **bloom particionado com Mutex por leitura
  (corrigido: parts_once pré-inicializado — revalidar probe + 100M)**
- hit-warm: ~20µs/**9,4 allocs** → próximo alvo: auditoria de allocs do
  caminho de hit (point-cache store, decode scratch, resolve, to_vec)
- apply: ~103µs/2,1 allocs por entrada → próxima: slab no BulkRun
  (push-loop O(1)/batch) + matar os 2 allocs restantes
- Teto do hydrate no harness: cliente compartilhado ~0,65ms/batch ⇒ máx
  ~1,6 M/s (ver RFC-0306 — vitória drástica de hydrate é fisicamente
  impossível; a drástica está na leitura).
