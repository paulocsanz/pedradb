# RFC-0196 — Meter-first: re-split quieto + publish unification condicionada (apply_mc4) + a pilha de meters

**Next:** [0197](0197-ratio-curve-same-class.md) (curva de razão same-class por escala — a previsão determinística que dá a cada meter daqui o seu hat; nomeia o 100M write 0,417 hat)

**Status:** closing — adjudicado 2026-09-11 (P0.3/P1.4/P2.3 pagos, P1.3 2/3, P2.1 parcial; P0.1 re-bloqueado por wipe datado; P1.1/P1.2/P2.2 deferred com custo)
**Updated:** 2026-09-11
**ID:** 0196
**Parents:** [0195](0195-scan-readahead-bounded-cache.md) (GET-side aterrissado; este é o próximo dono),
[0194](0194-leftover-bounded-cache-25m.md) (P2.1 publish unification é carregado para cá como P0 condicionado),
[0192](0192-write-cycle-forecast.md) (o kernel de previsão que nomeia publish; `qps_hat_error_permille`),
[0189](0189-lider-janela-lenta.md) (P0.3 publish skip — prova que fills==0 já não paga; fills>0 paga),
[0185](0185-coluna-a-dropin-1x-tudo.md) (floor-cut: hints por cache — a base do mecanismo P0)
**Peer:** RocksDB default `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`). G1 1c não é win.
Sync-peer não é win. Darwin = DIAG. Linux 3-run quieto = cartaz.

> **Tese:** todo corte **incondicional** com dono telemétrico nomeado já está
> aterrizado (write: guard 0190, wal_write 0193, leftover pread 0194; GET:
> scan WILLNEED 0195) — a telemetria restante diz que o maior pedaço do ciclo
> serial pós-ticket é o **publish (820 ns do CS de 2 050 ns = 40%)** nas
> shapes com leitura (`fills>0`), exatamente a fatia que o skip do 0189 P0.3
> NÃO cobre. O veto "publish-unification-before-resplit" permanece: o corte
> só aterrissa **depois** que o re-split quieto (0192 P1.1) nomear publish
> > ~0,4 µs/op em fills>0. Este RFC é **meter-first por desenho**: P0.1
> re-medir (o gate decidindo a data), P0.2 o corte condicionado no tamanho
> do buraco (apply_mc4, 0,47× Linux), P0.3 o meter com/sem. Se o gate seguir
> fechado, P0.1 fecha `blocked` datado e P0.2 fica `non-condition` datado —
> como o 0194 P2.1 ficou; nenhum número é fabricado. A regra ≥20%: o modelo
> diz CS 2 050 → ~1 230 ns (−40% na seção serial; qps_hat ×1,67 teto
> estrutural, nunca promessa) no buraco apply_mc4 — o maior corte
> model-nomeado não-vetado que existe.

## Background (números datados; meter Linux bloqueado — re-check 19:33, `2026-09-10-host-gate-blocked-meter.md`)

- **Kernel de previsão vivo** (entry-point consistente 2× hoje,
  `scale-model write --fixture linux-quiet --leaders 4 --guard 0`):
  `cut=wal_write cs_ns=3290 qps_hat=173913`; vista ticket pós-0193:
  `ticket_cut=publish ticket_cs_ns=2050 ticket_qps_hat=278784`. O próximo
  dono WRITE pelo modelo é **publish** — e o pin é **labeled-stale**
  (pre-0193; re-pin bloqueado no gate).
- **0189 P0.3 aterrizado**: publish pula os 2 RMWs de epoch quando
  `read_tls_fills == 0` (shapes só-escrita). Nas shapes **com leitura**
  (apply_mc4, ycsb_c, ycsb_f: fills>0) o publish paga por desenho — não é
  bug, é a invalidação correta dos caches de resposta.
- **0185 floor-cut aterrizado**: fast path já pula as 4 rodadas de Mutex
  quando os caches nunca foram preenchidos (hints com latch). O que SOBRA
  em fills>0 é o custo **com caches vivos**: dirty_points/point/count/
  last_prefix sob Mutex + 2–3 atômicos + bumps por publicação — o alvo do
  0194 P2.1 (epoch compartilhado + latches por cache).
- **Erro do modelo**: único ponto computável −15‰ (pre-0190, mesma perna);
  pós-0193 não computável até P0.1 rodar. O hat é teto estrutural, não cartaz.
- **mins absolvido** (`2026-09-10-rfc0189-p22-mins-nao-dispara.md`: 0,58 µs
  flat) — não volta ao ranking como dono.

## Problems This Solves

- **Problem:** o re-split quieto (0192 P1.1) é a lacuna #1 da telemetria
  há dois RFCs — sem ele o pin envelhece, `ticket_cut=publish` é
  modelo-sobre-pin-velho, e TODO corte write-side aterrizado (0190/0193/
  0194) segue sem número. Este RFC promove a re-medição a P0 — ela é o
  pré-requisito de qualquer ganho write honesto que sobra.
- **Problem:** apply_mc4 (0,47× Linux / 0,488 Darwin DIAG) é a maior célula
  com dono model-nomeado cujo corte ainda não existe: publish epochs vivos
  em fills>0. O mecanismo (0194 P2.1: um epoch compartilhado + latches por
  cache, estendendo 0189 P0.3 + hints do floor-cut) está desenhado e
  esperando evidência.
- **Problem:** sem este RFC, o próximo ciclo repetiria "mecanismo sem
  meter" — o oposto da disciplina de escala. Aqui o meter VEM PRIMEIRO
  (P0.1) e o corte é condicionado ao veredito dele.

## Proposed Solution

1. **P0.1 — re-split quieto pós-0193** (`PEDRA_WRITE_PHASE_STATS` no guest
   quieto, 3-run, STOP/CONT warm10): re-pina o fixture `LINUX_QUIET_0189_P01`
   (enc 350 / wr 890 / guard 2230 / mlock 380 / mins 580 / publish 820 /
   grp 270 / lock_wait 2460 ns → números novos), `name_cut` decide o dono
   write real, e `qps_hat_error_permille` vira computável pós-0193 (a
   aritmética já está no kernel, 3 testes). **É o desbloqueio de tudo.**
2. **P0.2 — publish unification (condicionada a P0.1 nomear publish >
   ~0,4 µs/op em fills>0)**: um epoch compartilhado por Db + latch por
   cache (extensão do 0189 P0.3: `read_tls_fills` generalizado para
   point/count/last_prefix — um `AtomicBool` por cache que só liga no
   primeiro fill e nunca desliga). Writer com todos os latches falsos:
   UM bump de epoch, zero Mutex (o caminho fills==0 já prova o envelope
   F198/F204). Reader: primeiro fill de cada cache claima seu latch ANTES
   do load de epoch (o handshake seqlock do 0189 P0.3, reusado por cache).
   Sem unsafe novo em core. Gates anti-regressão nomeados: ycsb_c
   (leitura) e deps_cache_overwrite_mc4 (fills==0 não regride).
3. **P0.3 — meter apply_mc4 com/sem o corte** (mesmo protocolo: 3-run
   quieto, peer `sync=false`, STOP/CONT warm10, min-of-3): a regra ≥20% é
   sobre este número, no tamanho do buraco.
4. **P1 — a pilha de meters empilhados** (todos bloqueados no mesmo gate;
   qualquer um vira cartaz quando P0.1 destravar o host): 0195 P0.4
   (prefix 100M @4GiB com/sem a condição + regressão point-get quente),
   0193 P0.5 (overwrite 10k gate 0185 P0.3 3/3 ≥1,0; kvrocks_set_mc50;
   apply_mc4 pre-P0.2), 0194 P0.4 (15M+25M leftover), ycsb_b (primeiro
   Linux 3-run — BALANCE_SHAPES).
5. **P2 — atrás dos meters**: U-cells lote 3-run (ycsb_c 0,909; qs_neg
   0,808; point_select 0,430; wbwi 0,410; flink 0,522; venice 0,735;
   arango 0,003; pipelined 0,807 — Darwin DIAG, regra anti-overfit), Grid B
   (10–100× dataset, compaction on) no corte vencedor, ycsb_f rmw restante
   (0,766 run2 quieto 09-09 pre-0193; DIAG 0,530).

## Ranking — todo buraco Linux same-class <1× (número = último datado, rotulado; nenhum novo medido hoje)

| # | célula | número (data, rótulo) | dono/atribuição | ataque | banda |
|---|---|---|---|---|---|
| 1 | apply_mc4 same-class | 0,47× (Linux 0183) / 0,488 (Darwin DIAG 09-09) | publish epochs vivos (fills>0 — skip 0189-P0.3 é fills==0); modelo: 820 ns do CS 2050 pós-ticket (40%) | **P0 deste RFC** (unification condicionada ao re-split) | **P0** |
| 2 | prefix **100M @4GiB** | **0,70×** (cartaz; 09-10) | scan bounded-cache `walk_all`, `dominant=pread` — **corte 0195 aterrizado** (P0.1–P0.3 done hoje; telemetria `scan_readahead=` no latch) | meter 0195 P0.4 (gated 19:33) | P1.1 |
| 3 | overwrite_mc4 **25M @4GiB** | 0,557× 3/3 (quieto 09-09) | write-ciclo (0193 ✓) + pread leftover (0194 ✓) — aterrizados **sem meter**; piso floor-cut 4,2 µs/op | meter 0194 P0.4 (15M+25M) | P1.2 |
| 4 | kvrocks_set_mc50 | 0,37× (cartaz) | convoi serial; 0193 modelo +21% em L=50 (hat) | meter 0193 P0.5 | P1.3 |
| 5 | overwrite_mc4 10k | min 0,883 (trim7+8, pre-0193) | `wal_write` in-lock — 0193 P0 ✓ aterrizado | gate 0185 P0.3 no meter (3/3 ≥ 1,0) | P1.3 |
| 6 | ycsb_f_mc4 | run2 0,766 (quieto 09-09); DIAG 0,530 | metade rmw: espera membro→WAL+apply líder (0193 ✓ corta; número pre-0193); intern/TLS pago (P0.80) | meter 0193 P0.5; rmw restante P2.3 | P1.3/P2.3 |
| 7 | ycsb_b_mc4 | sem Linux 3-run (Darwin p99 0,060 DIAG) | cauda GET — sem número Linux | medir primeiro (BALANCE_SHAPES) | P1.4 |
| 8 | 25M/2M/100k get-side U | point_select 0,430; wbwi 0,410; flink 0,522; venice 0,735; qs_neg 0,808; pipelined 0,807; ycsb_c 0,909; arango 0,003 (todos Darwin DIAG) | U — regra anti-overfit: nenhum mecanismo sem Linux 3-run | lote U-cells 3-run | P2.1 |

Por que o P0 é este e não outro: (a) é o único corte **não-vetado** cujo
dono é nomeado pela previsão determinística (`ticket_cut=publish` — o
kernel vivo, 2× consistente hoje) e por telemetria datada (WRITEPHASE
floor-cut: publish 0,51–0,97 µs/op medido no piso Linux); (b) a regra do
veto é respeitada **por estrutura** — o corte é P0.2, DEPOIS do re-split
P0.1, e só aterrissa nomeado; (c) escala no tamanho do buraco (apply_mc4
25M-equiv, não micro 100k); (d) o mecanismo reusa envelopes provados
(0189 P0.3 seqlock handshake ×3 testes; hints floor-cut; F198/F204);
(e) não contradiz o kernel — é o que o kernel nomeia.

## Delivery slices (mandatory)

### P0 — re-split + publish condicionada + meter do corte

- [ ] **P0.1** Re-split quieto pós-0193 (0192 P1.1): 3-run no guest quieto,
      re-pin do fixture, `name_cut` pós-guarda-pós-ticket, erro do modelo
      (`qps_hat_error_permille`) computável — status: `blocked`
      (re-bloqueado 2026-09-11 com razão NOVA: o gate está ABERTO, mas a
      wiring de fatia fina do 0192 P0.2 foi apagada pelo `git reset --hard`
      de sessão paralela em 2026-09-10 23:49 — o tree vivo tem
      `WritePhaseStats` com as 6 fatias RFC-0159 e o render WRITEPHASE
      pré-kernel; o kernel `write_cycle_kernel.rs` sobreviveu intacto,
      18/18 @ `b959428a`. Reconstruir a instrumentação é fatia do 0192,
      pré-requisito do re-pin; verificado in-tree 2026-09-11)
- [ ] **P0.2** Publish epoch unification: um epoch compartilhado + latches
      por cache (estende 0189 P0.3 + floor-cut hints), tests nomeados
      (`publish_unified_skips_cache_mutexes_until_each_first_fill`,
      `publish_unified_fill_is_monotonic_under_puts` por cache,
      `publish_unified_ycsb_c_no_regress`, `fills_zero_path_unchanged`);
      **só aterrissa se P0.1 nomear publish > ~0,4 µs/op em fills>0** —
      status: `non-condition` (2026-09-11: o disparador P0.1 segue bloqueado
      pelo wipe datado acima; e a célula-alvo apply_mc4 está PAGA 1,086×
      min-of-3 sem o corte — `findings/2026-09-11-p201r2-mc4/` — condição
      moot; só revive se uma célula fills>0 <1× voltar ao board)
- [x] **P0.3** Meter apply_mc4 com/sem P0.2, 3-run quieto, peer
      `sync=false`, min-of-3, regra ≥20% no número do buraco — status:
      `done` SEM o corte (P0.2 non-condition): **apply_mc4 = 1,0859×
      min-of-3** (1,0859/1,1160/1,3308; pedra 7,7k–9,3k vs rocks 6,5k–8,6k
      qps; imagem p201r2 digest `sha256:0ec55b38…`, mesmo boot, 3 rounds
      quiet, `PEDRA_PARITY_ASYNC=1`, `ROCKS_PARITY_CLIENTS=4` nos dois
      engines, peer do mesmo round) — a célula 0,47× do 0183 está PAGA
      acima do floor pelo merge por eixo de cliente 0201 P0.3 (`f7b2c20f`)
      + clamp P0.1 (`b2b0295b`). NOTA forense 2026-09-11: a imagem medida
      NÃO continha o seam off-lock do 0193 (verificado: `write_all_at*`/
      `reserve_frame` ausentes de f7b2c20f e de todo o histórico em
      `crates/` — o P0 do 0193 foi perdido pré-commit;
      `findings/2026-09-11-wipe-forense/`); nenhum corte P0.2 foi
      necessário (`findings/2026-09-11-p201r2-mc4/`)

### P1 — a pilha de meters (mesmo gate; P0.1 destrava o host para todas)

- [ ] **P1.1** Meter 0195 P0.4: prefix 100M @4GiB com/sem a condição
      WILLNEED + regressão point-get quente (lookup_100, get_hit 25M,
      point_select) — status: `deferred` com custo (2026-09-11: gate
      aberto — o bloqueio de host caducou; a perna é pesada: dataset 100M
      @4 GiB, ~horas de guest-build + bench por braço, e nenhuma célula
      viva do board depende dela; reabrível pelo inventário
      `findings/2026-09-11-gargalos-inventario/`)
- [ ] **P1.2** Meter 0194 P0.4: 15M **e** 25M @4GiB leftover overwrite —
      status: `deferred` com custo (mesma adjudicação do P1.1; pernas
      15M+25M @4 GiB; dono no inventário 2026-09-11)
- [ ] **P1.3** Meter 0193 P0.5: overwrite 10k (gate 0185 P0.3, 3/3 ≥ 1,0),
      kvrocks_set_mc50, apply_mc4 pre-P0.2 — status: `partial` (2026-09-11,
      2/3 pagas): **kvrocks_set_mc50 = 1,678× min-of-3**
      (1,7343/2,1451/1,6782; A/B p201q digest `sha256:18babf02…`, mesmo
      boot, peer `sync=false` — `findings/2026-09-11-p201o-sweep/`) e
      **apply_mc4 = 1,0859×** (P0.3 acima); overwrite 10k
      (`ROCKS_YCSB_RECORDS=10000`) segue sem número Linux — carregada na
      onda de meter do RFC-0209
- [x] **P1.4** ycsb_b primeiro Linux 3-run (BALANCE_SHAPES) — status:
      `done` pelo sweep p201o (3-run quiet, peer `sync=false`, coluna
      async): **ycsb_b single = 1,088 min / 1,276 med** — acima do floor.
      A variante `_mc` não existe no harness (`run_clients` emite
      a/f/c/unif_mcN; ycsb_b nunca teve suíte mc) — não há shape a medir
      (`findings/2026-09-11-p201o-sweep/`)

### P2 — atrás dos meters

- [ ] **P2.1** U-cells lote 3-run (ycsb_c, point_select, wbwi, flink,
      venice, qs_neg, pipelined, arango) — status: `partial` (2026-09-11):
      a família ycsb+unif FOI medida no sweep p201o 3-run quiet (pagas:
      ycsb_b 1,088, b_unif 1,394, c 2,729, c_unif 3,238, d 1,122, e 7,976;
      buracos agora medidos em Linux: ycsb_a 0,605, ycsb_f 0,804,
      deps_scan 0,831 — dono RFC-0209/inventário); qs_neg, point_select,
      wbwi, flink, venice, arango, pipelined seguem DIAG-only (anti-overfit:
      sem Linux 3-run nenhum mecanismo)
- [ ] **P2.2** Grid B anti-overfit (10–100× dataset, compaction on) no
      corte vencedor — status: `deferred` (custo: grid 10–100× com
      compaction ligada = pernas mais caras do board; precisa de um corte
      vencedor vivo primeiro — nenhum corte novo aterrizou desde a escrita;
      dono no inventário 2026-09-11)
- [x] **P2.3** ycsb_f rmw restante (pós-meter 0193; dono medido
      rmw-get-bytes: metade put do rmw) — status: `done` (meter,
      2026-09-11): **ycsb_f_mc4 = 0,2947× min-of-3** (0,2947/0,6338/0,3195;
      pedra 146k–189k vs rocks 247k–640k; `findings/2026-09-11-p201r2-mc4/`)
      e single 0,804 min / 0,956 med (sweep p201o) — buraco REAL, dono
      reatribuído ao ataque async 1-op do RFC-0209 (buffer WAL user-space)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | re-split quieto pós-0193 + erro do modelo | blocked (wipe da wiring 0192, datado 2026-09-11) | verificado in-tree: `WritePhaseStats` 6 fatias RFC-0159; kernel intacto `b959428a` | 2026-09-11 |
| P0.2 | p0 | publish unification (condicionada ao P0.1) | non-condition (disparador bloqueado + apply_mc4 paga sem o corte) | `2026-09-11-p201r2-mc4/` | 2026-09-11 |
| P0.3 | p0 | meter apply_mc4 com/sem | done (sem corte: 1,0859× min-of-3) | `findings/2026-09-11-p201r2-mc4/` | 2026-09-11 |
| P1.1 | p1 | meter 0195 P0.4 (prefix 100M) | deferred (custo; gate aberto) | inventário 2026-09-11 | 2026-09-11 |
| P1.2 | p1 | meter 0194 P0.4 (15M/25M) | deferred (custo; gate aberto) | inventário 2026-09-11 | 2026-09-11 |
| P1.3 | p1 | meter 0193 P0.5 (10k/mc50/apply) | partial (mc50 1,678× + apply 1,0859× pagas; 10k → RFC-0209) | `2026-09-11-p201o-sweep/` + `2026-09-11-p201r2-mc4/` | 2026-09-11 |
| P1.4 | p1 | ycsb_b Linux 3-run | done (single 1,088 min; `_mc` não existe no harness) | `findings/2026-09-11-p201o-sweep/` | 2026-09-11 |
| P2.1 | p2 | U-cells lote 3-run | partial (ycsb-família medida; qs/wbwi/flink/venice/arango/pipelined DIAG-only) | sweep p201o + inventário | 2026-09-11 |
| P2.2 | p2 | Grid B | deferred (custo; atrás de corte vencedor) | inventário 2026-09-11 | 2026-09-11 |
| P2.3 | p2 | ycsb_f rmw restante | done (meter 0,2947× min; buraco real → dono RFC-0209) | `findings/2026-09-11-p201r2-mc4/` | 2026-09-11 |

## Acceptance Criteria

- **Tests (P0.2 quando disparar)**
  - `publish_unified_skips_cache_mutexes_until_each_first_fill` — com
    caches vivos o publish paga UM bump e zero Mutex; primeiro fill por
    cache claima e o caminho daquele cache volta ao envelope de hoje.
  - `publish_unified_fill_is_monotonic_under_puts` (por cache) — a prova
    seqlock do 0189 P0.3 generalizada: putter vs filler, valor observado
    nunca regride; pós-último-put o get final vê o final.
  - `publish_unified_ycsb_c_no_regress` — shape de leitura não regride
    (estrutural: com latches ligados o caminho é o de hoje + 1 compare).
  - `fills_zero_path_unchanged` — o caminho fills==0 do 0189 P0.3/floor-cut
    não muda de envelope.
- **Telemetry / Analytics**
  - P0.1 re-pina o fixture e publica `qps_hat_error_permille` pós-0193
    (perna Linux quieta); `PEDRA_IO_ADVISE_STATS` segue carregando
    `leftover_advise=` + `scan_readahead=` na mesma linha.
- **Documentation**
  - Este RFC; o deep-dive `findings/2026-09-10-telemetry-deep-dive.md`
    (seção 1bis) como insumo vivo do ranking.
- **Screenshots**
  - Backend-only — n/a.

## Out of scope

- Publicar publish unification ANTES do re-split nomear publish (o veto
  vira estrutura: P0.2 só existe pós-P0.1).
- Levers pagas/refutadas: intern/TLS (P0.80), grouping/linger, WAL-shard,
  DONTNEED/WILLNEED incondicional (Fire 118 / v69), lock-through-WAL,
  skiplist TCB (0190 P1.1 non-condition), mins híbrido (absolvido por
  medição, `2026-09-10-rfc0189-p22-mins-nao-dispara.md`).
- Mecanismo novo em célula U sem Linux 3-run (regra anti-overfit).
- Ligar telemetria default (0169). Darwin como cartaz. G1 1c como win.
  Sync-peer como win. Meter 100k como prova do buraco 100M/apply.
- Implementar as fatias deste RFC no ciclo 0195 (este documento é o
  próximo ciclo).
