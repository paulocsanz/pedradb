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

- [x] **P0.1** Instrumentação por fase do `commit_async_ops` (`PEDRA_WRITE_PHASE_STATS`)
      + lock-wait no bypass — status: `done` (d5549d5)
- [x] **P0.2** Microbench N-threads (probe `multiwriter_probe`; bench-shaped
      50×2000) — status: `done`
      (**veredito**: hold 1.6 µs = wal 0.8 + mem 0.5 + publish 0.25 +
      prepare 0.13; wait médio 175 µs; 1c 1.05 M vs 50c 266 k; spin
      falsificado; **P1.1 falsificado como alavanca** — prepare é 8% do hold)
- [x] **P0.3** `deps_lock_prewrite` — status: `done`
      (**isolado 2.28/2.26/2.09 — a shape ganha ≥2×; o 0.94 do árbitro é
      efeito do contexto da suíte v0** (mesmo processo/db depois das demais
      shapes). P1.2 vira bissecção de contexto)
      Finding: `findings/rfc0045-p0/` (dirty box, números de mecanismo)

### P1 — o que o P0 deixou vivo (P1.1 original falsificado)

- [ ] **P1.1** ~~prepare fora do write lock~~ — status: `done (negativo)`
      (P0.2: prepare = 0.13 µs de hold 1.6 µs = 8%; mover não move o qps.
      Registrado como negativo com número; não implementar)
- [ ] **P1.2** Bissecção do contexto da suíte que flipa `deps_lock_prewrite`
      (2.2× isolado → 0.94 in-suite): rodar prefixos crescentes do v0 antes
      da shape até o sinal virar; profile do caso flipado — status: `todo`
- [ ] **P1.3** Remesura quieto 3× (P2.1 bar): mc50 e lock_prewrite; sem
      regressão em SET/GET/pipeline/E — status: `todo`

### P2 — concorrência de memtable (o alvo medido do 5×; promoted)

- [ ] **P2.1** Memtable apply fora da seção crítica (hold 1.6 → ≤1.1 µs;
      ceiling 640 k → 900 k): per-writer staging + apply paralelo pós-WAL
      (shape Rocks) ou estrutura concorrente; A/B pareado vs BTree atual —
      status: `todo`
- [ ] **P2.2** Handoff sem park-convoy (wait 175 µs vs hold 1.6 µs é o 2.4×
      entre 266 k e o ceiling): fila leader-follower **sem catch-up wait**
      (diferente do merge falsificado 0.19×, que esperava); protótipo
      env-gated — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | instrumentação por fase do bypass | done | d5549d5 | 2026-08-20 |
| P0.2 | p0 | microbench N-threads (probe) | done | hold 1.6 µs, wait 175 µs; spin falsificado | 2026-08-20 |
| P0.3 | p0 | lock_prewrite isolado vs in-suite | done | 2.2× isolado / 0.94 in-suite | 2026-08-20 |
| P1.1 | p1 | prepare off-lock | done (negativo) | 8% do hold; P0.2 | 2026-08-20 |
| P1.2 | p1 | bissecção do contexto v0 | todo | — | 2026-08-20 |
| P1.3 | p1 | remesura quieto 3× sem regressão | todo | — | 2026-08-20 |
| P2.1 | p2 | memtable apply fora da seção crítica | todo | alvo medido: hold ≤1.1 µs | 2026-08-20 |
| P2.2 | p2 | handoff sem park-convoy | todo | wait 175 µs é o 2.4× do gap | 2026-08-20 |

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
