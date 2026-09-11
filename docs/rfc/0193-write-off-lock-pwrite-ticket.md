# RFC-0193 — Write fora do wal.lock: pwrite por ticket nas células multi-cliente

**Status:** in-progress (P0.1–P0.4 landed; P0.5 parcialmente pago 2026-09-11 — mc50 1,678× e apply_mc4 1,086×; célula 10k → RFC-0209)
**Updated:** 2026-09-11
**ID:** 0193
**Parents:** [0189](0189-ciclo-lider-janela-lenta.md) (P1.1 design aprovado — este RFC executa P1.2/P1.3),
[0190](0190-apply-concorrente-despark.md) (apply desparkado; o que sobrou do ciclo é o WAL),
[0192](0192-write-cycle-forecast.md) (kernel `name_cut` que nomeia o corte),
[0185](0185-coluna-a-dropin-1x-tudo.md) (P0.3 gate: overwrite_mc4 3/3 min > 1.0),
[0183](0183-teto-apply-serial-e-1c.md), [0055](0055-rocks-write-pipeline.md)
**Peer:** RocksDB default `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`). G1 1c não é win. Sync-peer não é win. Darwin = DIAG. Linux 3-run quieto = cartaz.

> **Tese:** depois do despark do apply (0190) e do seqlock do publish (0189 P0.3),
> a maior fatia serial medida do ciclo do líder é o **syscall `write()` dentro do
> `wal.lock()`** (`wr=0,89 µs/op` no Linux quieto, 0189 P0.1). O kernel 0192 no
> estado pós-0190 nomeia `cut=wal_write`. O corte é o desenho já aprovado do 0189
> P1.1: **frame do grupo em buffer próprio + ticket de ordem + `pwrite` off-lock**,
> WAL byte-idêntico, ordem de arquivo = ordem de ticket. O ganho modelado cresce
> com o número de líderes enfileirados — é o ataque que escala com L.

## Background (todos os números são de findings datados)

### O ciclo pós-0190 (Linux p149b quieto, rank-1 cell 222 k, ns/op)

Fatia (0189 P0.1) | valor | estado pós-cortes
---|---:|---
wal `enc` | 350 | dentro do wal.lock
**wal `wr` (syscall)** | **890** | **dentro do wal.lock — maior fatia restante**
mem `guard` | 2 230 | **~0** (0190 P0.2; Darwin 0,05–0,09)
mem `mlock` | 380 | lane curta (0190 P0.3)
mem `mins` | 580 | Darwin pós-lanes 0,80 (hipótese — remeter decide)
`publish` | 820 | 0189 P0.3 seqlock aterrizado (testes verdes)
`grp` | 270 | epilogue refutado (0,27 somado)
`lock_wait` | 2 460 | derivado — ciclo dos outros líderes, não é corte

Kernel 0192 (`write_cycle_forecast`, fixture menos-guarda):

```
write-cycle leaders=4
cut=wal_write cut_as_is=lock_hold
cs_ns=3290 cs_as_is_ns=2200
lock_wait_hat_ns=2467 lock_wait_as_is_ns=500
cycle_ns=5750 qps_hat=173913 off_wr_qps_hat=238095
```

`off_wr_qps_hat` = +36,9% no rank-1 (L=4); +21% em L=50 (210 437). Teto do modelo
(líderes fully-serialized), não cartaz; o wait **medido** pode ser menor que o hat
(no fixture pré-0190 o hat era pessimista: off 123 426 < qps 125 313). O meter decide;
o modelo só ranka.

### Quem paga este corte (células oficiais, mesmas classes)

| célula | último número quieto/cartaz | por que o mesmo corte paga |
|---|---|---|
| overwrite_mc4 10k | min **0,883** (trim7+8; quieto 222 k; gate 0185 P0.3) | CS −27% (3 290→2 400 ns); modelo +37% ⇒ acima da paridade 260 k |
| kvrocks_set_mc50 | **0,37×** | 50 líderes enfileirados na MESMA seção serial; wait hat = (L−1)/L·CS escala com L |
| apply_mc4 same-class | **0,47×** (Darwin pós-0190 DIAG 0,488) | o grupo aplica desparkado; o WAL do líder ainda é serial |
| overwrite_mc4 25M | **0,557×** 3/3 | fase de write do 25M é o mesmo piso de ciclo (mapa: Pedra precisa 4,2→1,8 µs/op) |

## Problems This Solves

- **Problem:** o syscall `write()` (0,89 µs/op) está na seção crítica que todo
  líder multi-cliente serializa — Pebble não paga isso (o `write()` é cópia em
  buffer; o syscall não é a CS — `findings/2026-09-09-wal-shard-vs-pebble-pipeline.md`).
- **Problem:** o gate 0185 P0.3 (3/3 min > 1.0) está a 0,883 com o dono nomeado e
  sem corte restante no apply (despark aterrizado).
- **Problem:** mc50 (0,37×) é a pior perda same-class e seu dono é convoi na
  seção serial — cada µs de CS vale ×(L−1)/L para cada líder enfileirado.

## Proposed Solution

Executar o desenho aprovado do 0189 P1.1 (nenhum desenho novo aqui):

1. **Seam posicional**: `EnvFile::write_all_at(buf, at)` — posix `pwrite`
   stateless; default portável seek/write/seek-back (fallback correto, não
   paralelo); capability `positional_writes()` no open. Sem capability ⇒
   caminho de hoje (fallback ativo).
2. **Frame do grupo fora do lock**: `Wal::encode_group_frame(&ops, &mut out)`
   — mesma fragmentação/header/CRC de `encode_write_op_batches`+
   `write_pending_frame`; bytes idênticos por construção.
3. **Ticket de ordem**: sob `wal.lock()` só `reserve_frame(len)` (devolve
   base=`reserved_to`, avança reserva, cuida do prealloc). Dois cursores:
   `reserved_to` (alocação) vs `position()` (escrito — ledger verificado/sync).
   `write_all_at(frame, ticket)` fora do lock. Ordem no arquivo = ordem dos
   tickets, syscall concorrente.
4. **Drenagem**: `inflight_writes: AtomicUsize` no `Wal`; `Db::sync`/close
   esperam `inflight == 0` antes do barrier.
5. **Durabilidade inalterada**: `pwrite` ANTES do Ok (mesma classe
   processo-crash do Rocks default `sync=false`); crash com write em voo =
   buraco de zeros ⇒ recuperação para no header/CRC = cauda rasgada de hoje;
   grupo além do buraco descarta inteiro.

## Ranking — todo ataque em escala (nenhum omitido)

Linux same-class <1× (as 7 do board /otimizar):

| # | célula | número | ataque | fatia deste RFC |
|---|---|---|---|---|
| 1 | overwrite_mc4 10k | min 0,883 | write off-lock (P0) | **P0** |
| 2 | overwrite_mc4 25M @4 GiB | 0,557× 3/3 | fase write = ciclo (P0 compõe); leftover/L0 pread = I/O | **P0 compõe** + P2.1 |
| 3 | kvrocks_set_mc50 | 0,37× | convoi na seção serial (P0; remeter decide lane/skiplist depois) | **P0** + P1.2 |
| 4 | apply_mc4 same-class | 0,47× | mesma CS; remeter pós-P0 | **P0** + P1.2 |
| 5 | prefix 100M @ 4 GiB | 0,70× | GET/scan bounded-cache (lado de leitura — fora deste RFC) | P2.2 (adiado com número) |
| 6 | ycsb_f_mc4 | mediana 1,47; run2 0,766 | rmw put (`template.to_vec+put`) | P2.3 |
| 7 | ycsb_b_mc4 | sem Linux 3-run (Darwin 0,060 cauda) | cauda GET p99 | P2.4 (medir antes) |

U-cells DIAG sem Linux 3-run (ycsb_c 0,909; qs_neg 0,808; point_select 0,430;
wbwi 0,410; flink 0,522; venice 0,735; arango_traversal 0,003; pipelined 0,807…):
P2.4 — nenhuma vira P0 sem Linux 3-run (anti-overfit; regra da skill).

Por que o P0 é este e não outro: é o único corte com (a) dono medido no Linux
quieto (kernel `name_cut=wal_write`), (b) desenho aprovado datado, (c) ganho que
CRESCE com L (mc50), (d) compõe em 4 das 7 células. Alternativas ranqueadas:
skiplist TCB (0190 P1.1) exige remeter pós-P0 (`mins` Darwin é hipótese, não
gatilho); leftover I/O precisa da caixa Linux; cauda GET precisa de Linux 3-run.

## Delivery slices (mandatory)

### P0 — o corte (executa 0189 P1.2)

- [x] **P0.1** Captura de igualdade ANTES de qualquer linha: rodada de shapes
      com dump do WAL (`wal-before.bin`) arquivada em `findings/` — gate `cmp`
      byte-idêntico pós-corte — status: `done`
      (`findings/2026-09-10-rfc0193-p01-wal-before/wal-before.bin`,
      sha256 `831d94d6…95c5d`; pós-corte idêntico)
- [x] **P0.2** Seam `EnvFile::write_all_at` + capability `positional_writes()`
      (posix pwrite; default portável) com teste nomeado — status: `done`
      (`env::tests::positional_write_seam_is_pwrite_on_unix` + fallback test)
- [x] **P0.3** `Wal::encode_group_frame` (buffer do grupo) + `reserve_frame`
      (ticket, dois cursores) + drenagem `inflight_writes` em sync/close;
      fallback sem capability = caminho de hoje — status: `done`
      (writer unit tests; `framed_group_len_at` multi-record; board
      contiguity/drain tests)
- [x] **P0.4** Líder real (`lead_pipeline_group` e caminho verificado/G1) troca
      encode+write in-lock por reserve sob lock curto + `pwrite` off-lock;
      `off_lock_write_order_survives_two_leaders` + `cmp` byte-idêntico +
      suíte crash/reopen/torn verde — status: `done`
      (byte-idêntico `$S/wal-after/`; serial 896/21 = baseline exato;
      regressão de fence corrigida: erro de append entra no plano de fence)
- [x] **P0.5** Meter de gate: overwrite_mc4 (0185 P0.3: 3/3 min ≥ 1.0 vs Rocks
      ≳260 k quieto), kvrocks_set_mc50, apply_mc4 antes/depois; Linux p149b
      quieto (STOP/CONT warm10); guest fora ⇒ Darwin DIAG + blocked, nunca
      cartaz — status: `partial` (adjudicado 2026-09-11, gate aberto):
      **2/3 células PAGAS** — kvrocks_set_mc50 **1,678× min-of-3**
      (1,7343/2,1451/1,6782; A/B p201q digest `sha256:18babf02…`, mesmo
      boot, peer `sync=false`; pré-corte o mesmo braço media 0,668×) e
      apply_mc4 **1,0859× min-of-3** (p201r2; `findings/
      2026-09-11-p201o-sweep/`, `findings/2026-09-11-p201r2-mc4/`);
      overwrite 10k (`ROCKS_YCSB_RECORDS=10000`, gate 3/3 ≥ 1,0) segue sem
      número Linux — carregada na onda de meter do RFC-0209. Cross-target
      musl check exit=0 `$S/xcheck-linux-musl.txt`

### P1 — composição e próximo dono

- [x] **P1.1** Variante io_uring ordenada (0189 P1.3): SQEs encadeados por
      ticket (`IOSQE_IO_LINK`), um submitter; fallback = pwrite P0 — status:
      `closed-by-verdict`
      (`findings/2026-09-10-rfc0193-p11-iouring-verdict.md` — sem host Linux
      para medir; prêmio condicional ao meter do P0; cross-target check verde)
- [x] **P1.2** Remeter Linux quieto pós-P0 com o kernel 0192: `name_cut` decide
      o próximo dono (`mins`/`mlock` ⇒ re-avaliar skiplist TCB 0190 P1.1;
      publish residual ⇒ 0189; erro do modelo `off_wr_qps_hat` vs medido
      nomeado no finding, não escondido) — status: `blocked` (perna Linux,
      re-bloqueado 2026-09-11: a wiring de fatia fina do 0192 P0.2 foi
      apagada pelo `git reset --hard` de sessão paralela em 2026-09-10
      23:49 — tree vivo tem `WritePhaseStats` 6 fatias RFC-0159, render
      pré-kernel; kernel intacto 18/18 @ `b959428a`; reconstrução é fatia
      do 0192, pré-requisito do re-pin deste P1.2) + `done` (vista local)
      (`findings/2026-09-10-rfc0193-telem-ticket-view.md` — kernel ganhou a
      vista ticket pós-P0: `ticket_cut=publish ticket_cs_ns=2050
      ticket_qps_hat=278784` no pin guard=0 L=4; fixture NÃO re-pinado,
      rotulado; erro do modelo não computável sem meter)
- [x] **P1.3** Perna 25M write-phase: o mesmo corte no piso 4,2 µs/op
      (`findings/` floor-cut-package); I/O leftover é P2.1, não este — status:
      `blocked` (mesma razão datada 2026-09-11 do P1.2: re-pin exige a
      wiring 0192 re-construída; `2026-09-10-rfc0193-p05-meter-blocked.md`
      para o histórico do gate)

### P2 — ataques nomeados, adiados com número

- [x] **P2.1** Leftover/L0 bounded-cache no 25M (lado I/O: pread de SST
      leftover durante c/; keep-newest 0173 P2.4 já aterrizado) — 0,557× —
      status: `deferred` com número (`p05-meter-blocked`)
- [x] **P2.2** prefix 100M @ 4 GiB (0,70×) — scan bounded-cache; Linux 3-run
      primeiro — status: `deferred` com número (`p05-meter-blocked`)
- [x] **P2.3** ycsb_f rmw (run2 0,766; DIAG 0,530) — `template.to_vec+put` no
      caminho rmw — status: meter `done` 2026-09-11: **ycsb_f_mc4 0,2947×
      min-of-3** (0,2947/0,6338/0,3195; pedra 146k–189k vs rocks 247k–640k;
      `findings/2026-09-11-p201r2-mc4/`) + single 0,804 min / 0,956 med
      (sweep p201o) — buraco REAL; dono reatribuído ao RFC-0209 (ataque
      async 1-op; dono medido `2026-09-09-rmw-get-bytes.md`)
- [x] **P2.4** Cauda GET / U-cells DIAG (ycsb_b 0,060, ycsb_c 0,909, qs_neg,
      point_select, wbwi, arango, flink, venice) — Linux 3-run antes de cortar;
      nenhum P0 persegue cauda 100k — status: `deferred` (`p05-meter-blocked`)
- [x] **P2.5** Grid B anti-overfit (carrega 0190 P2.2 blocked): dataset
      10–100× com compaction ligada no corte vencedor — status: `blocked`
      (`p05-meter-blocked`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | wal-before.bin gate | done | `findings/2026-09-10-rfc0193-p01-wal-before/` (cmp pós-corte idêntico) | 2026-09-10 |
| P0.2 | p0 | EnvFile::write_all_at + capability | done | `env::tests` seam tests | 2026-09-10 |
| P0.3 | p0 | frame do grupo + ticket + drenagem | done | `wal` writer/board unit tests | 2026-09-10 |
| P0.4 | p0 | líder real off-lock + byte-idêntico | done | `off_lock_write_order_survives_two_leaders`; serial 896/21 = baseline | 2026-09-10 |
| P0.5 | p0 | meter 3 células (gate 0185 P0.3) | partial (mc50 1,678× + apply 1,0859× pagas 2026-09-11; 10k → RFC-0209) | `2026-09-11-p201o-sweep/` + `2026-09-11-p201r2-mc4/` | 2026-09-11 |
| P1.1 | p1 | io_uring ordenado | closed-by-verdict | `2026-09-10-rfc0193-p11-iouring-verdict.md` | 2026-09-10 |
| P1.2 | p1 | remeter: kernel decide o próximo dono | blocked (Linux; wipe 0192 datado 2026-09-11) / done (vista local) | `2026-09-10-rfc0193-telem-ticket-view.md` | 2026-09-11 |
| P1.3 | p1 | perna 25M write-phase | blocked (mesma razão do P1.2) | wipe 0192 datado 2026-09-11 | 2026-09-11 |
| P2.1 | p2 | leftover/L0 I/O 25M (0,557) | deferred (número) | `p05-meter-blocked` | 2026-09-10 |
| P2.2 | p2 | prefix 100M (0,70) | deferred (número) | `p05-meter-blocked` | 2026-09-10 |
| P2.3 | p2 | ycsb_f rmw (0,2947 min medido) | done (meter; buraco real → dono RFC-0209) | `2026-09-11-p201r2-mc4/` + `2026-09-09-rmw-get-bytes.md` | 2026-09-11 |
| P2.4 | p2 | cauda GET / U-cells (medir Linux) | deferred | `p05-meter-blocked` | 2026-09-10 |
| P2.5 | p2 | Grid B anti-overfit | blocked | `p05-meter-blocked` (carrega 0190 P2.2) | 2026-09-10 |

## Acceptance Criteria

- **Tests**
  - `off_lock_write_order_survives_two_leaders` — dois líderes, reserve/write
    intercalados; bytes no arquivo na ordem de encode (ticket).
  - `cmp` wal-before/after.bin idêntico (a captura do P0.1 é o gate).
  - Suíte crash/reopen/torn verde (`async_ok_write_wal_without_fsync_survives_reopen`,
    `rfc0055_pipeline_four_writers_async_visible`, `rfc0055_writethread_join_eight_writers_visible`,
    `rfc0185_pipeline_group_single_record_recovers`, `rfc0190_*`).
  - Suíte serial `-p pedradb-core`: zero falhas novas vs baseline.
- **Telemetry / Analytics**
  - WRITEPHASE pós-corte: `wr` sai do hold (`hold=` ≈ enc; `wr=` medido
    off-lock); kernel 0192 `name_cut` re-ranka no mesmo dump.
  - Cada onda: perna Linux p149b quieta (STOP/CONT warm10) em `findings/`;
    veredito min-of-3 honesto (KEEP só com mecanismo + suíte limpa; collapsed
    Rocks = refuse).
- **Documentation**
  - 0189 P1.2/P1.3 apontam para cá quando P0.4/P1.1 aterrizar; finding datado
    por onda; `docs/status.md` na mesma mudança.
- **Screenshots**
  - Backend-only — n/a.

## Out of scope

- Grouping/linger/hold-open (0180 permanente). G1 1c como win. Sync-peer.
  Fjall gate. Deletar shape. Tocar o floor RFC-0041.
- Skiplist TCB (0190 P1.1) — gated no remeter P1.2, não aqui.
- Telemetria default-on (0169). Implementar 0192 P1/P2.
- `async_wal`/WAL-shard (refutado: um log, Rocks serializa no log writer —
  wal-shard finding). Cauda GET (P2.4 mede primeiro).
