# RFC-0195 — Scan readahead em bounded-cache: o gêmeo GET da política de páginas (prefix 100M)

**Status:** in-progress (P0.1–P0.3 done 2026-09-10; meter slices blocked no gate datado)
**Updated:** 2026-09-10
**ID:** 0195
**Next:** [0196](0196-meter-first-publish-unification.md) (meter-first: re-split quieto + publish unification condicionada)
**Parents:** [0194](0194-leftover-bounded-cache-25m.md) (a política de páginas do lado write aterrizou; este é o gêmeo GET),
[0176](0176-modelo-matematico-de-escala.md) (GET clock — o forecast GET é o oráculo deste corte),
[0192](0192-write-cycle-forecast.md) (telemetria/latch; kernel de write NÃO é contraditado — caminho distinto),
fires 118/119 (a lição da condição: versão incondicional é regressão)
10°**Peer:** RocksDB default `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`). G1 1c não é win.
Sync-peer não é win. Darwin = DIAG. Linux 3-run quieto = cartaz.

> **Tese:** o maior buraco same-class <1× com dono nomeado **intocado** pelos
> cortes write-side aterrizados-desmedidos (0190 despark, 0193 ticket write,
> 0194 leftover advise — todos esperando o gate de meter) é o
> **prefix 100M @ 4 GiB: 0,70×** (cartaz). A atribuição é estrutural, não
> medida às cegas: todo fd de SST abre com `FADV_RANDOM` (paridade Rocks
> `set_advise_random_on_open`, correto para point gets — v69 lookup_100
> 0,87×) e **todo bloco lido é um pread de 4 KiB** (`read_range` →
> `positioned_read_exact`). Um scan sequencial em modo bounded-cache anda
> bloco a bloco **sem readahead nenhum por construção** — cada bloco paga a
20> latência fria. O grande convidado prova a decomposição: **100M big-guest
> é 1,05×** (compute em paridade ⇒ o déficit de 0,70× é padrão de I/O, não
> decode). O corte: **readahead de janela na caminhada sequencial**
> (`POSIX_FADV_WILLNEED` à frente do cursor), **condicionado** — a lição
> Fire 118/119 como desenho: só quando (a) a store está em bounded-cache
> (mesmo predicado do 0194) E (b) a caminhada é um run sequencial de blocos
> adjacentes no arquivo (run ≥ 2). Point gets e stores fitting ficam
> byte-idênticos. Alvo: **≥20% do buraco** — 0,70× → ≥0,84× no mesmo
> tamanho, min-of-3 quieto, peer `sync=false` (gate do host decidindo a
> data; sem gate ⇒ mecanismo aterriza com testes, meter fecha `blocked`
> datado — o precedente 0193/0194).

## Background (números datados; meter Linux bloqueado — `2026-09-10-host-gate-blocked-meter.md`)

- **prefix 100M @4GiB: 0,70×** (cartaz; skill rank 2026-09-10). Diagnóstico
  do gerador (`pedra diagnose get --keys 1e9 --ram 68719476736`): nos
  tamanhos grandes os ranges cobrem ⇒ **`walk_all`, `dominant=pread`**.
- **100M big-guest: 1,05×** — quando o dataset cabe na RAM, o scan Pedra
  empata com Rocks: o buraco 4 GiB é padrão de leitura, não compute.
- Caminho de leitura hoje (`sst/table.rs` / `env.rs`): `decode_block_on_file`
  → `SstFileSource::read_range(path, h.offset, 4 KiB)` →
  `CachedEnvSource` abre com `advise(Random)` e faz um pread por bloco.
  O loop de caminhada (`db.rs` `decode_block(bi)` por bloco) não emite
  readahead — nenhuma camada emite.
- **Rocks no mesmo shape:** point gets com `FADV_RANDOM`, mas o caminho de
  **iterador/scan tem readahead** (prefetch adaptivo até 64 KiB+; compaction
  lê com 2 MiB de readahead). O scan não é um point get esticado — é uma
  leitura sequencial que hoje pagamos como aleatória.
- Lição embutida (Fire 118 → 119): a versão **incondicional** (WILLNEED em
  toda leitura de bloco) recria o v69 lookup_100 (100 keys puxando 128 KiB
  cada) e é **vetada no desenho**; a condição (bounded ∧ run adjacente) é
  carregada por teste nomeado, não assumida.

## Problems This Solves

- **Problem:** o único dono nomeado por telemetria que nenhum corte
  aterrizado cobre é o pread do scan bounded-cache (write-side: guard/despark
  0190 ✓, wal_write/ticket 0193 ✓, leftover DONTNEED 0194 ✓ — todos sem
  meter; GET-side: nada aterrizado).
- **Problem:** `AdviseKind::WillNeed` existe no seam (`Env::advise`, posix
  `FileAdvise`) desde o RFC-0029 e **nenhum caminho de produção o emite** —
  o seam pago e nunca usado no lado de leitura.
- **Problem:** sem readahead, cada bloco do scan paga uma ida ao disco; com
  janela, o kernel sobrepõe as próximas N leituras — o buraco inteiro
  (0,70 vs 1,05 big-guest) é essa latência.

## Proposed Solution

1. **Kernel puro** `scan_readahead_kernel.rs` (inteiro, sem I/O):
   `scan_readahead_window(handles: &[BlockHandle], next: usize, bounded: bool,
   warm: bool) -> u64` — 0 quando não-bounded, quando fitting (hot), ou
   quando `handles[next+1]` não é adjacente a `handles[next]`; senão a
   janela em bytes cobrindo o run adjacente a partir de `next`, com teto
   **256 KiB** (64 blocos) por advise. O AS-IS (`scan_readahead_as_is`) é
   sempre 0 (o gerador de hoje).
2. **Wiring** no loop de caminhada (os sítios `decode_block(bi)` de scan em
   `db.rs`): antes de decodificar o bloco `bi` (evicted/payload-pool miss),
   se o kernel devolve janela > 0 ⇒ `Env::advise(path, offset, janela,
   WillNeed)` best-effort pelo seam existente (preferência: estender
   `SstFileSource` com `readahead` default no-op que o `CachedEnvSource`
   implementa no fd já aberto — zero open extra; fallback = `env.advise`
   por path). Fora de qualquer lock de write (caminho de leitura).
   Fitting/hot/point-get: **nenhuma linha muda** (janela 0 por construção).
3. **Telemetria** no mesmo latch `PEDRA_IO_ADVISE_STATS`:
   `scan_readahead=windows:N bytes:N blocks:N` ao lado do
   `leftover_advise=` do 0194 (uma linha de I/O por dump).
4. **Sem unsafe novo em core**: `pedradb-core` segue `#![forbid(unsafe_code)]`;
   o fadvise vive no seam posix existente (`SAFETY.md` inalterado).

## Ranking — todo buraco Linux same-class <1× (número = último datado, rotulado)

| # | célula | número (data, rótulo) | dono/atribuição | ataque | banda |
|---|---|---|---|---|---|
| 1 | prefix **100M @4GiB** | **0,70×** (cartaz; skill rank 09-10) | scan bounded-cache `walk_all`, `dominant=pread`; fds `FADV_RANDOM` + pread 4 KiB/bloco = zero readahead; big-guest 1,05× ⇒ déficit é I/O | **P0 deste RFC** (janela WILLNEED condicionada) | **P0** |
| 2 | overwrite_mc4 **25M @4GiB** | 0,557× 3/3 (quieto 09-09) | write-ciclo (0193 ✓) + pread leftover na c/ (0194 ✓) — cortes aterrizados **sem meter** | meter 0194 P0.4 (15M+25M) | P1.3 |
| 3 | kvrocks_set_mc50 | 0,37× (cartaz) | convoi serial; despark 0190 ✓ aterrizado (modelo +21% L=50); mc50 = C/blocked (0190 P1.2) até re-medir | meter 0193 P0.5 | P1.2 |
| 4 | apply_mc4 same-class | 0,47× (Linux, 0183) / 0,488 (Darwin DIAG 09-09) | mesma CS + publish epochs vivos (fills>0); skip 0189 P0.3 é fills==0 | meter + re-split quieto (publish unif. segue gated P1.2) | P1.1/P1.2 |
| 5 | overwrite_mc4 10k | min 0,883 (trim7+8, pre-0193) | `wal_write` in-lock — 0193 P0 ✓ aterrizado | gate 0185 P0.3 no meter (3/3 ≥ 1,0) | P1.2 |
| 6 | ycsb_f_mc4 | run2 0,766 (quieto 09-09); DIAG 0,530 | metade rmw: espera do membro pelo WAL+apply do líder (0193 ✓ corta); intern/TLS pago (P0.80) | meter 0193; rmw restante P2.1 | P1.2/P2.1 |
| 7 | ycsb_b_mc4 | sem Linux 3-run (Darwin p99 0,060 DIAG) | cauda GET — sem número Linux | medir primeiro (BALANCE_SHAPES) | P1.4 |
| 8 | ycsb_c_mc4 | 0,909 (Darwin DIAG, U) | U sem diagnose Linux | lote U-cells 3-run | P2.2 |
| 9 | 25M/2M/100k get-side U | point_select 0,430; wbwi 0,410; flink 0,522; venice 0,735; qs_neg 0,808; pipelined 0,807; arango 0,003 (todos Darwin DIAG) | U — nenhum mecanismo sem Linux (regra anti-overfit) | lote U-cells 3-run | P2.2 |

Por que o P0 é este: (a) único dono nomeado **não coberto** por corte
aterrizado; (b) decomposição provada por contraste (big-guest 1,05× — o
déficit é I/O, não compute); (c) o seam (`WillNeed`) existe pago e sem
chamador; (d) condição carregada com a lição Fire 118/119 (incondicional é
regressão — v69); (e) não contradiz o kernel 0192 (caminho GET distinto;
o kernel de write segue dizendo que o próximo dono WRITE decide no
re-split quieto — P1.1); (f) escala com o dataset (100M/4 GiB é o tamanho
do buraco, não um micro 100k).

## Delivery slices (mandatory)

### P0 — a janela de readahead do scan

- [x] **P0.1** Kernel puro `scan_readahead_kernel`: janela de run adjacente
      (teto 256 KiB), 0 se não-bounded/fitting/run<2 + gêmeo AS-IS —
      unidades puras testáveis — status: `done` 2026-09-10 (9 testes:
      `fitting_store_never_reads_ahead`, `adjacent_run_extends_window`,
      `non_adjacent_run_is_zero`, `window_caps_at_256_kib`,
      `cap_never_splits_a_block`, `tail_and_oob_anchors_are_zero`,
      `as_is_twin_is_always_zero`, `pair_budget_fills_the_cap`,
      `bounded_boundary_is_the_0194_warm_cap`; API aterrizada:
      `scan_readahead_window(blocks, at, bounded)` + predicado
      `scan_readahead_bounded(sst_bytes, warm_cap)`)
- [x] **P0.2** Wiring no loop de caminhada: advise `WillNeed` best-effort
      pela janela à frente do cursor (fd do `CachedEnvSource` via
      `SstFileSource::readahead`, default no-op), off-lock (miss path do
      loader, fora do mutex do block cache), um advise por trecho de
      janela (dedupe por cursor — `w.offset >= advised_to`); somente os
      dois loops forward de `scan_at_raw` (o count cursor e os walks
      reversos ficam de fora por desenho); fitting/hot/point-get
      byte-idênticos (resident-payload e ≤v4 ⇒ NONE por construção) —
      testes nomeados (`scan_readahead_fires_only_in_bounded_cache`,
      `scan_readahead_hot_and_point_gets_unchanged`,
      `scan_readahead_is_off_lock`; kernel cobre
      `scan_readahead_non_adjacent_run_is_zero` e o teto) — status:
      `done` 2026-09-10
- [x] **P0.3** Telemetria `scan_readahead=windows:N bytes:N blocks:N` sob
      `PEDRA_IO_ADVISE_STATS` — status: `done` 2026-09-10
      (`leftover_advise_counters_line_is_opt_in` estendido; exemplo real
      `examples/scan_readahead.rs`: bounded leg `windows:2 bytes:17127
      blocks:88`, fitting leg zeros)
- [ ] **P0.4** Meter de gate: prefix 100M @4GiB Linux 3-run quieto
      STOP/CONT warm10, min-of-3, peer `sync=false`, com e sem a condição;
      regressão obrigatória nos shapes point-get quentes (lookup_100,
      get_hit 25M, point_select) — gate fora ⇒ Darwin DIAG + blocked datado
      — status: `blocked` (re-check gate 2026-09-10 19:33,
      `findings/2026-09-10-host-gate-blocked-meter.md`)

### P1 — os meters empilhados (tudo gated no host; precedem qualquer P0 novo)

- [ ] **P1.1** Re-split quieto pós-0193 (0192 P1.1): `PEDRA_WRITE_PHASE_STATS`
      re-pina o fixture e `name_cut` decide o próximo dono write — status:
      `blocked` (mesmo gate/finding)
- [ ] **P1.2** Meter 0193 P0.5 carregado: overwrite_mc4 10k (gate 0185 P0.3
      3/3 ≥ 1,0), kvrocks_set_mc50, apply_mc4 — status: `blocked`
- [ ] **P1.3** Meter 0194 P0.4: 15M **e** 25M @4GiB leftover overwrite —
      status: `blocked`
- [ ] **P1.4** ycsb_b_mc4 Linux 3-run (BALANCE_SHAPES sem número) — status:
      `blocked`

### P2 — atrás dos meters

- [ ] **P2.1** ycsb_f rmw restante (0194 P2.2; dono medido rmw-get-bytes:
      metade put do rmw) — status: `todo-condicional` (atrás do meter
      P1.2; gate fechado 19:33 — `2026-09-10-host-gate-blocked-meter.md`,
      não medível)
- [ ] **P2.2** U-cells lote 3-run (ycsb_c, point_select, wbwi, flink,
      venice, qs_neg, pipelined, arango) — status: `blocked`
- [ ] **P2.3** Grid B anti-overfit (dataset 10–100× com compaction no corte
      vencedor) — status: `blocked`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | kernel da janela de readahead | done | 9 testes kernel (`scan_readahead_kernel::tests::*`) | 2026-09-10 |
| P0.2 | p0 | WILLNEED off-lock na caminhada | done | `scan_readahead_fires_only_in_bounded_cache`, `scan_readahead_hot_and_point_gets_unchanged`, `scan_readahead_is_off_lock` | 2026-09-10 |
| P0.3 | p0 | telemetria scan_readahead | done | `leftover_advise_counters_line_is_opt_in` + `examples/scan_readahead.rs` | 2026-09-10 |
| P0.4 | p0 | meter prefix 100M @4GiB | blocked | re-check gate 19:33 (`2026-09-10-host-gate-blocked-meter.md`) | 2026-09-10 |
| P1.1 | p1 | re-split quieto pós-0193 | blocked | mesma re-check/finding | 2026-09-10 |
| P1.2 | p1 | meter 0193 P0.5 (10k/mc50/apply_mc4) | blocked | mesma re-check/finding | 2026-09-10 |
| P1.3 | p1 | meter 0194 P0.4 (15M/25M) | blocked | mesma re-check/finding | 2026-09-10 |
| P1.4 | p1 | ycsb_b Linux 3-run | blocked | mesma re-check/finding | 2026-09-10 |
| P2.1 | p2 | ycsb_f rmw restante | todo-condicional (atrás do meter P1.2; gate fechado 19:33 — não medível) | `2026-09-10-host-gate-blocked-meter.md` | 2026-09-10 |
| P2.2 | p2 | U-cells lote 3-run | blocked | mesma re-check/finding | 2026-09-10 |
| P2.3 | p2 | Grid B | blocked | mesma re-check/finding | 2026-09-10 |

## Acceptance Criteria

- **Tests**
  - `scan_readahead_fires_only_in_bounded_cache` — bounded ∧ run adjacente ⇒
    janela > 0 cobrindo o run; fitting (hot) ⇒ 0 mesmo com run.
  - `scan_readahead_hot_and_point_gets_unchanged` — point get / store
    fitting: zero advises emitidos, contadores 0 (regressão v69 virada
    teste).
  - `scan_readahead_non_adjacent_run_is_zero` — run < 2 ⇒ janela 0.
  - `scan_readahead_window_caps_at_256kib` — run maior que o teto ⇒ janela
    = teto.
  - `scan_readahead_is_off_lock` — advise emitido segurando nenhum lock de
    write (o padrão do teste 0194 `leftover_advise_is_off_lock`).
- **Telemetry / Analytics**
  - `PEDRA_IO_ADVISE_STATS=1` imprime `scan_readahead=` ao lado do
    `leftover_advise=`; sem latch, zero custo no caminho de leitura.
- **Documentation**
  - Este RFC; o kernel documenta o teto 256 KiB e por quê (64 blocos ×
    4 KiB = uma janela por ~64 preads; maior que isso evicta o conjunto
    quente em 4 GiB).
- **Screenshots**
  - Backend-only — n/a.

## Out of scope

- Mecanismos write-side novos (o dono write decide no re-split P1.1 — o
  kernel 0192 segue soberano no seu caminho).
- Publish unification antes do re-split nomear publish (0194 P2.1
  non-condition). Skiplist TCB (0190 P1.1 non-condition).
- Levers pagas: intern/TLS, grouping/linger, WAL-shard, DONTNEED
  incondicional (Fire 118), WILLNEED incondicional (v69), lock-through-WAL.
- Ligar telemetria default (0169). Darwin como cartaz. G1 1c como win.
- Meter 100k como prova deste corte (o tamanho do buraco é 100M @ 4 GiB).
