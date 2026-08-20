# RFC-0045: escrita concorrente real — fechar `mc50` ≥5× e `lock_prewrite` ≥1× na coluna async

**Status:** draft
**Updated:** 2026-08-20
**Parents:** [0044](0044-async-class-5x-rocks.md) (coluna async/async, piso 5×),
[0040](0040-fsync-always-beats-rocks-async.md) (group commit + bypass),
[AGENTS.md](../../AGENTS.md) (coluna oficial ≠ coluna async)

## Background

- Estado medido (árbitro quieto P2.1, load 9.4, 2026-08-20):
  `kvrocks_set_mc50` **2.02** (307 k vs 152 k, peer são; p50 Pedra 0.6–1.3 µs
  vs Rocks 128–366 µs) — **ganhamos 2×, não perdemos**; o que falta é o piso
  5× do RFC-0044. `deps_lock_prewrite` **0.94 med** (runs 1.40/0.94/0.77) —
  único shape < 1.
- Merge de líder já foi testado e **falsificado** (5 rounds pareados:
  mediana **0.19× vs bypass**, `findings/rfc0044-p1/kvrocks-merge/`) — líder
  único é ponto de agendamento com 50 threads / 12 CPUs. Não é o caminho.
- Fato estrutural (fonte confirmado): `Db::commit_async_ops` recebe
  `&mut self` — o caller (`ConcurrentDb`) segura o **write lock do Db
  inteiro** durante `prepare_write_ops` (interning/encode/`Bytes::from`) +
  append WAL + `apply_ops_owned` (insert BTree) + `publish_sequence`. Com 50
  escritores, tudo isso serializa; o Rocks serializa só o append do WAL e
  insere na memtable **em paralelo** (write group + skiplist concorrente).
- `deps_lock_prewrite` é single-thread (batch de 64 entradas: 32 puts no CF
  `lock` + 32 no CF `default` com chave MVCC key+ts). A perda de 6% **não**
  é concorrência; mecanismo desconhecido (hipóteses: roteamento por CF por
  entrada, encode de chave mvcc, allocs por op). Número antes de teoria.

## Problems This Solves

- **Problem:** mc50 2.02 < piso 5× da coluna async (P0.5 do RFC-0044 segue
  `doing`); mecanismo do gap ainda não identificado (handoff já falsificado).
- **Problem:** `deps_lock_prewrite` 0.94 é o único shape perdendo de 12+ do
  v0 — sem profile, qualquer fix é chute.
- **Problem:** serializar encode+mem+publish no lock do Db é o gargalo
  estrutural da escrita multi-thread; o Rocks prova que dá para serializar
  só o WAL.

## Garantias invariáveis

| # | Garantia | Este RFC |
|---|---|---|
| G1 | `fdatasync` antes do Ok (default) | intocada — só o caminho async/bypass muda |
| G2 | CRC fail-closed / never silent-wrong | encode fora do lock produz os mesmos bytes (teste de equivalência) |
| G5 | fence se sync falha | intocada |
| G6 | sem thread no core | intocada — nenhum worker novo no core |
| G8 | coluna async nunca vendida como "batemos o Rocks" | o RFC fecha piso do 0044, não o cartaz 0041 |

## Proposed Solution

1. **P0 = medir, não teorizar:** profile do mc50 (onde vão os ns do
   escritor: espera do lock, `prepare_write_ops`, append, mem-apply,
   `publish_sequence`) e do `deps_lock_prewrite` (CF routing, key encode,
   allocs). Gate: o mecanismo do P1 é escolhido pelo número do P0.
2. **Encurtar a seção crítica do bypass:** encode/prepare **fora** do write
   lock (scratch por writer; o lock cobre só atribuição de seq + append WAL
   + publish), formato Rocks (serializa o WAL, não o writer inteiro).
3. **Se P1 não fechar:** memtable com insert concorrente (append WAL
   serializado, apply em paralelo — shape skiplist do Rocks) e/ou reteste do
   `PEDRA_ASYNC_GROUP=1` em caixa quieta (a falsificação foi em caixa suja).

## Delivery slices (mandatory)

### P0 — profile pareado + microbench de decisão

- [ ] **P0.1** Instrumentação por fase do `commit_async_ops` (lock wait /
      prepare / wal / mem / publish) atrás de feature flag de bench; nenhum
      custo no caminho default — status: `todo`
- [ ] **P0.2** Microbench pareado N∈{4,12,50} threads × {bypass atual,
      encode-fora-do-lock protótipo} na mesma caixa quieta; número decide o
      P1 — status: `todo`
- [ ] **P0.3** Profile do `deps_lock_prewrite` (perf/Instruments ou contadores
      por fase); classificar os 6% (CF routing vs key encode vs allocs vs
      ruído — runs 1.40/0.94/0.77 sugerem variância alta) — status: `todo`

### P1 — seção crítica curta (o que o P0 mandar)

- [ ] **P1.1** `prepare_write_ops` fora do write lock com equivalência de
      bytes do WAL (teste `encode_offlock_matches_lock_path`); lock cobre
      seq+append+publish — status: `todo`
- [ ] **P1.2** Fix do mecanismo dominante do lock_prewrite apontado pelo
      P0.3; alvo ≥ 1.0 na árbitro quieto 3× — status: `todo`
- [ ] **P1.3** Remesura quieto 3× (P2.1 bar: load < 10): mc50 e
      lock_prewrite; sem regressão em SET/GET/pipeline/E (mediana e p50) —
      status: `todo`

### P2 — concorrência de memtable (só se P1 não fechar 5×)

- [ ] **P2.1** Protótipo memtable de insert concorrente (WAL serializado,
      apply paralelo; `tail_ord` por shard); A/B pareado vs BTree atual —
      status: `todo`
- [ ] **P2.2** Reteste `PEDRA_ASYNC_GROUP=1` em caixa quieta (falsificação
      0.19× foi a load 14–50; se o líder ganhar quieto, reabrir como opção) —
      status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | instrumentação por fase do bypass | todo | — | 2026-08-20 |
| P0.2 | p0 | microbench N-threads encode-offlock | todo | — | 2026-08-20 |
| P0.3 | p0 | profile lock_prewrite | todo | — | 2026-08-20 |
| P1.1 | p1 | prepare fora do lock + teste equivalência | todo | — | 2026-08-20 |
| P1.2 | p1 | fix lock_prewrite ≥ 1.0 | todo | — | 2026-08-20 |
| P1.3 | p1 | remesura quieto 3× sem regressão | todo | — | 2026-08-20 |
| P2.1 | p2 | memtable insert concorrente | todo | — | 2026-08-20 |
| P2.2 | p2 | reteste async-group quieto | todo | — | 2026-08-20 |

## Acceptance Criteria

- **Tests:** `encode_offlock_matches_lock_path` (bytes idênticos ao caminho
  atual, incl. interned v2); suítes async existentes re-verdes sem editar
  asserção (`async_concurrent_writers_recover`,
  `async_and_sync_concurrent_writers_recover`, crash-after-sync-put G1);
  adversarial FailingEnv re-verde.
- **Telemetry:** finding `findings/rfc0045-*/` com profile P0 + árbitro
  quieto 3× (P2.1 bar) para mc50 e lock_prewrite; colunas `sync=false` dos
  dois lados; "not official" no README se dirty.
- **Documentation:** este RFC + linha no status do 0044 (P0.5/P2.2
  referenciam este doc).
- **Screenshots:** none — backend-only.

## Out of scope

- Trocar o peer ou a coluna oficial (0041 intocado; cartaz segue G1 vs
  `sync=false`).
- Thread no core (G6).
- Merge de líder como default (falsificado 0.19×; só reavaliar via P2.2
  quieto).
- Arena/skip-list C++ (só entra se P2.1 saturar e o piso ainda faltar).
