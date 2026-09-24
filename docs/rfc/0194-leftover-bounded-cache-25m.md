# RFC-0194 — Leftover bounded-cache: política de páginas no 25M (o maior buraco estável)

**Status:** in-progress (P0.1–P0.3 done; meter slices blocked no gate datado)
**Updated:** 2026-09-10
**ID:** 0194
**Parents:** [0193](0193-write-off-lock-pwrite-ticket.md) (P2.1 executa aqui; o corte de write P0 compõe
na mesma célula), [0185](0185-coluna-a-dropin-1x-tudo.md) (coluna A; 25M é célula do board),
[0173](0173-bounded-cache-ram-degrade.md) P2.4 keep-newest (aterrizado), fires 118/119
(`findings/2026-09-09-leftover-page-keep.md`, `findings/2026-09-09-bounded-leftover-keep.md`)
**Next:** [0195](0195-scan-readahead-bounded-cache.md) (o gêmeo GET da política de páginas — prefix 100M)
**Peer:** RocksDB default `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`). G1 1c não é win.
Sync-peer não é win. Darwin = DIAG. Linux 3-run quieto = cartaz.

> **Tese:** o maior buraco same-class **estável** não pago é o `overwrite_mc4
> 25M @ 4 GiB` (**0,557× 3/3**, quieto 2026-09-09). O lado write-ciclo desse
> buraco acabou de ser cortado pelo 0193 P0 (aterrizado, meter bloqueado);
> o que sobra com dono nomeado é o **lado I/O**: em modo bounded-cache
> (SST bytes > warm cap) a compactação/flush pré-lê leftovers (25M:
> n_files=23; 15M: n_files=14; `dominant=pread`), e a política de páginas
> hoje não distingue hot de bounded — o Fire 118 provou que a versão
> incondicional (DONTNEED sempre) **regrediu** o hot 2M (0,795→0,681) e foi
> revertida; o Fire 119 deixou a condição correta desenhada e não aterrizada.
> Este RFC aterra essa condição. É o ataque que escala com o dataset
> (15M/25M/100M), não um micro 100k.

## Background (números datados; meter Linux bloqueado — ver `2026-09-10-rfc0193-p05-meter-blocked.md`)

### A família leftover (gerador `pedra diagnose get`, Darwin)

Store | modo | n_files | corte do gerador | pread
---|---|---:|---|---
2M @4GiB | **hot** (cabe) | 2 | indistinguishable, pread=0, block_lvl=dram | 0
15M @4GiB | bounded-cache | 14 | probe_path, walk_all | dominante
25M @4GiB | bounded-cache | 23 | probe_path, walk_all, `legal=14133 pread=13500` | **95% da composição legal**

- Fire 118 (DONTNEED incondicional no 2M hot): 0,681 — **regressão**, revertida.
  Lição anti-overfit embutida no desenho deste P0: hot NUNCA solta páginas.
- Fire 119 (a condição): `sst_page_keep_budget=0` **sse** bounded-cache (sst >
  warm cap) **E** o one-slash vivo não tem SST cobrindo. Hot mantém ram/4.
- Write-side 25M: o piso do ciclo foi medido 4,2 µs/op (floor-cut) e o 0193 P0
  o corta (modelo ticket CS 2 050 ns, pin velho — ver ressalva); o pread de
  leftover durante c/ é o dono restante nomeado (0193 P2.1).

## Problems This Solves

- **Problem:** a maior célula same-class não paga (0,557× 3/3 estável) não tem
  corte pós-0193 com desenho datado — o I/O leftover é o dono nomeado e a
  condição correta (Fire 119) nunca aterrizou.
- **Problem:** sem política de páginas, c/ de 23 SSTs no 25M evicta o conjunto
  quente (WAL/memtable/L0) do page cache — o write p50 paga em park/L0 (2M
  DIAG atribuiu "write park/L0", não GET pread).
- **Problem:** a versão ingênua já foi refutada (Fire 118) — sem a condição
  hot/bounded o corte é regressão.

## Proposed Solution (executa 0193 P2.1 — nenhum desenho novo)

1. **Orçamento de páginas**: `sst_page_keep_budget` no `OpenOptions`/estado do
   Db — default = manter (comportamento de hoje; hot 2M intacto por construção).
2. **Condição Fire 119**: orçamento 0 apenas quando (a) bytes de SST do
   one-slash vivo > warm cap (bounded-cache) E (b) nenhuma SST viva cobre a
   família one-slash. Hot (2M) nunca entra.
3. **Soltar off-lock**: após a merge de c/ consumir um range de leftover, o
   kernel de compactação chama `EnvFile::advise(DONTNEED)` pelo seam existente
   (`AdviseKind::DontNeed`, `pedradb-posix`), **fora** de qualquer lock do
   caminho de write (o 0193 ticket board já provou o padrão off-lock).
4. **Sem unsafe novo em core**: `pedradb-core` segue `#![forbid(unsafe_code)]`;
   o fadvise vive no seam posix que já existe (SAFETY.md inalterado — nenhum
   `unsafe` novo).

## Ranking — todo ataque em escala, todo buraco <1× não pago (números = último datado, rotulados)

| # | célula | número (data) | dono/atribuição | ataque | banda |
|---|---|---|---|---|---|
| 1 | overwrite_mc4 **25M @4GiB** | **0,557× 3/3** (quieto 09-09) | write-ciclo (0193 P0 aterrizado, sem meter) + **pread leftover/L0 durante c/** (Fire 119; n_files=23) | **P0 deste RFC** (composição com 0193) | **P0** |
| 2 | kvrocks_set_mc50 | 0,37× (cartaz) | convoi na seção serial; 0193 P0 aterrizado (modelo +21% em L=50); pós-P0 exige remeter | meter 0193 P0.5 → re-split decide lane/skiplist | P1.1 |
| 3 | apply_mc4 same-class | 0,47× (Linux) / 0,488 (Darwin DIAG) | mesma CS + **publish epochs vivos** (fills>0: o skip do 0189 P0.3 é só fills==0); sem split pós-0193 | meter + re-split; publish unificação é P2.1 (gated) | P1.1/P2.1 |
| 4 | prefix 100M @4GiB | 0,70× (cartaz) | GET/scan bounded-cache: probe anda pelos arquivos que cobrem (range-prune já existe em `probe_order`); nos tamanhos grandes os ranges cobrem ⇒ walk_all, `dominant=pread` | **medir primeiro** (Linux 3-run + split de preads/GET) | P1.3 |
| 5 | overwrite_mc4 10k | min 0,883 (trim7+8, pre-0193) | `wal_write` in-lock — **0193 P0 aterrizado**; gate 0185 P0.3 decide no meter | meter 0193 P0.5 (3/3 ≥ 1,0) | P1.1 |
| 6 | ycsb_f_mc4 | run2 0,766; DIAG 0,530 (09-09) | metade rmw: espera do membro pelo WAL+apply do líder (0193 P0 corta); intern/TLS **pago** (P0.80 não moveu) | meter 0193; sem mecanismo novo antes | P1.1/P2.2 |
| 7 | ycsb_b_mc4 | sem Linux 3-run (Darwin p99 0,060) | cauda GET | **medir primeiro** (Linux 3-run) | P1.4 |

U-cells DIAG sem Linux 3-run — **nenhuma vira mecanismo** (anti-overfit;
regra da skill): ycsb_c 0,909; qs_neg 0,808; point_select 0,430; wbwi 0,410;
flink 0,522; venice 0,735; arango_traversal 0,003; pipelined 0,807 — P2.3
(lote de 3-run quando o gate abrir).

Por que o P0 é este: é o único corte com (a) o maior buraco **estável** (3/3,
sem colapso do peer), (b) desenho datado com a refutação da versão errada
embutida (Fire 118→119), (c) pré-requisito aterrizado (keep-newest 0173 P2.4),
(d) composição com o 0193 P0 na MESMA célula (write-ciclo + I/O), (e) escala
com o dataset. O kernel 0192 (vista ticket, pin velho) nomeia `publish` —
**este RFC não o persegue**: o pin é pre-0189-P0.3 e o re-split quieto (P1.2)
é quem promove `publish` de modelo a dono; persegui-lo agora contradizaria o
próprio kernel (fixture não re-pinado, bloqueado, datado).

## Delivery slices (mandatory)

### P0 — a política de páginas (executa 0193 P2.1)

- [x] **P0.1** Condição + orçamento: `sst_page_keep_budget` (default manter) +
      predicado bounded-cache (sst bytes > warm cap) ∧ sem SST cobrindo o
      one-slash vivo — unidades puras testáveis — status: `done`
      (`leftover_page_kernel.rs`, 7 testes, 2026-09-10)
- [x] **P0.2** c/ merge consome leftover ⇒ `advise(DONTNEED)` off-lock só na
      condição P0.1; hot 2M bit-intocado — status: `done`
      (`leftover_advise_{fires_only_in_bounded_cache,skips_covering_one_slash,
      is_off_lock,hot_never}`, 2026-09-10)
- [x] **P0.3** Telemetria opt-in (`PEDRA_IO_ADVISE_STATS`): contadores de
      advises emitidos/pulados por razão (hot/covering/still-warm) no dump
      WRITEPHASE-adjacente — status: `done`
      (`ConcurrentDb::io_advise_line` + bench `io_advise_line`;
      `leftover_advise_counters_line_is_opt_in`, 2026-09-10)
- [ ] **P0.4** Meter: 15M **e** 25M @4GiB leftover overwrite vs quieto Rocks
      ≳263 k (NÃO 2M, NÃO 100k — 100k não tem leftover SST); Linux quieto
      STOP/CONT warm10, min-of-3; gate do guest fora ⇒ Darwin DIAG + blocked,
      nunca cartaz — status: `blocked` (gate 2026-09-10 18:32:
      guest irresolúvel + Darwin load1 36,52; `findings/2026-09-10-host-gate-blocked-meter.md`)

### P1 — meter e re-atribuir (tudo gated no host)

- [ ] **P1.1** Meter 0193 P0.5 carregado: overwrite_mc4 10k (gate 0185 P0.3
      3/3 ≥1,0), mc50, apply_mc4 — decide se o corte de write fecha o rank 5/2/3
      ou nomeia o próximo dono — status: `blocked` (mesmo gate/finding)
- [ ] **P1.2** Re-split quieto pós-0193 (`PEDRA_WRITE_PHASE_STATS=1`): re-pin
      do fixture 0192; `ticket_cut=publish` vira dono medido ou morre;
      skiplist TCB re-avaliado só por esse split — status: `blocked` (fixture
      0192 fica labeled-stale até a perna; mesmo gate/finding)
- [ ] **P1.3** prefix 100M: Linux 3-run + split de preads/GET (contagem de
      arquivos abertos por probe) ANTES de qualquer scan-cache — status: `blocked` (mesmo gate/finding)
- [ ] **P1.4** ycsb_b: Linux 3-run (cauda GET p99) — medir antes de cortar —
      status: `blocked` (mesmo gate/finding)

### P2 — nomeados, adiados com número/gate

- [ ] **P2.1** Publish epoch unification (shapes com leitura): um epoch
      compartilhado + latches por cache (estende 0189 P0.3 + hints do
      floor-cut) — **só dispara se o P1.2 nomear publish > ~0,4 µs/op em
      fills>0** — status: `non-condition` (o disparador é o re-split do P1.2,
      bloqueado no gate — não dispara; `findings/2026-09-10-host-gate-blocked-meter.md`)
      **[carregado para o [0196](0196-meter-first-publish-unification.md)
      como P0.2 condicionada — o meter-first deste corte é o próximo ciclo]**
- [ ] **P2.2** ycsb_f rmw restante (pós-meter 0193): o que sobrar da metade
      rmw depois do write off-lock — status: `blocked` (depende do meter do P1.1)
- [ ] **P2.3** U-cells DIAG: lote Linux 3-run (ycsb_c, qs_neg, point_select,
      wbwi, flink, venice, arango, pipelined) — nenhum vira P0 sem 3-run —
      status: `blocked` (mesmo gate/finding)
- [ ] **P2.4** Grid B anti-overfit (10–100× dataset, compaction on) no corte
      vencedor (carrega 0190 P2.2 + 0193 P2.5 blocked) — status: `blocked`
      (Grid B é meter Linux; corte aterrissado aguarda o grid)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | condição Fire 119 + orçamento | done | `leftover_page_kernel.rs` (7 testes) | 2026-09-10 |
| P0.2 | p0 | DONTNEED off-lock na c/ | done | `leftover_advise_*` (4 testes) | 2026-09-10 |
| P0.3 | p0 | telemetria de advises | done | `io_advise_line` (opt-in test) | 2026-09-10 |
| P0.4 | p0 | meter 15M/25M (não 2M/100k) | blocked | gate 2026-09-10 18:32 (finding datado) | 2026-09-10 |
| P1.1 | p1 | meter 0193 P0.5 (3 células) | blocked | mesmo gate/finding | 2026-09-10 |
| P1.2 | p1 | re-split quieto + re-pin 0192 | blocked (fixture labeled-stale) | mesmo gate/finding | 2026-09-10 |
| P1.3 | p1 | prefix 100M medir primeiro | blocked | mesmo gate/finding | 2026-09-10 |
| P1.4 | p1 | ycsb_b Linux 3-run | blocked | mesmo gate/finding | 2026-09-10 |
| P2.1 | p2 | publish unification (gated P1.2) | non-condition (gated) | mesmo gate/finding | 2026-09-10 |
| P2.2 | p2 | ycsb_f rmw restante | blocked (depende P1.1) | mesmo gate/finding | 2026-09-10 |
| P2.3 | p2 | U-cells lote 3-run | blocked | mesmo gate/finding | 2026-09-10 |
| P2.4 | p2 | Grid B | blocked (meter Linux) | corte aterrizado, grid gated | 2026-09-10 |

## Acceptance Criteria

- **Tests**
  - `leftover_advise_fires_only_in_bounded_cache` — env de gravação conta
    `advise(DONTNEED)`: bounded-cache emite; hot (2M, cabe) NÃO emite
    (regressão do Fire 118 virada teste).
  - `leftover_advise_skips_covering_one_slash` — SST viva cobrindo a família
    ⇒ sem advise (condição (b)).
  - `leftover_advise_is_off_lock` — advise emitido sem nenhum lock do caminho
    de write segurado (mesma família de teste do ticket board 0193).
  - Suíte serial `-p pedradb-core`: zero falhas novas vs baseline (regra A/B).
  - Hot 2M leftover DIAG antes/depois: **idêntico** (o orçamento default
    mantém páginas; o corte só existe na condição bounded).
- **Telemetry / Analytics**
  - `PEDRA_IO_ADVISE_STATS=1`: emitidos/pulados por razão (hot/covering/
    still-warm) — sem o env, custo zero no caminho quente.
- **Documentation**
  - Finding datado por onda; `docs/status.md` na mesma mudança; 0193 P2.1
    aponta para cá quando P0.2 aterrizar.
- **Screenshots**
  - Backend-only — n/a.

## Out of scope

- Implementar antes de medir: prefix scan-cache (P1.3 mede), publish
  unification (P2.1 gated no P1.2), qualquer U-cell (P2.3).
- Grouping/linger/hold-open (0180); G1 1c como win; sync-peer; intern/TLS
  (P0.80 pago); WAL-shard (refutado); skiplist TCB fora do re-split.
- Meter 100k para este corte (100k não tem leftover SST — Fire 119).
- Tocar o floor RFC-0041.
