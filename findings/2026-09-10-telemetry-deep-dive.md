# 2026-09-10 — Telemetria funda: dono por célula não paga, kernel de previsão e erro do modelo

**Pedido:** "refinar nossa telemetria e previsão determinística" (fecho do
cycle 0194; insumo do ranking 0195; **atualizado no fecho do 0195** — seção 1bis;
**re-atualizado pós-0192-P0.4/0197-P2.1** — seções 3bis/3ter;
**re-atualizado na escolha do P0 do 0198** — seção 4bis).
**Estado do gate:** meter Linux bloqueado (guest irresolúvel + Darwin load1
20–40; `2026-09-10-host-gate-blocked-meter.md`, re-check 22:24 pós-restart do
OrbStack) — todo número abaixo é o **último datado**, rotulado; nada foi
re-medido hoje.

## 1. O que a telemetria mede hoje (pós-0192/0193/0194)

| Tralha | Latch | Fatias/contadores |
|---|---|---|
| WRITEPHASE (split) | `PEDRA_WRITE_PHASE_STATS` | `wal(enc/wr) mem(guard/mlock/mins) publish lock_wait grp(walk/complete/settle)`; pós-0193 board path: `wal_encode_ns` off-lock, `wal_write_ns` = write+wait_contiguous, `wal_lock_hold_ns` = aquisição→release |
| Contadores 0192 | mesmo latch | `publish_cas_retries`, `cas_lf` (LfStack), `lane_c` (por lane, Instant só no miss) |
| Kernel 0192/0193 | zero I/O | `cut=` / `cs_ns` / `qps_hat` / `off_wr_qps_hat` / `ticket_cut` / `ticket_cs_ns` / `ticket_qps_hat`; AS-IS = `lock_hold` |
| 0194 P0.3 (novo) | `PEDRA_IO_ADVISE_STATS` | `leftover_advise=drop:N keep_hot:N keep_covered:N keep_default:N issued:N` |
| 0192 P2.1 | — | `lane_hist` no JSON do compare (deps_raftlog + loop mc) |

Sem latch: zero `Instant` no caminho quente do insert; config default do
0194 = zero bookkeeping (os 5 contadores ficam 0 — pinado por teste).

## 1bis. Lado GET (novo no 0195, mesma linha de latch)

`PEDRA_IO_ADVISE_STATS` agora imprime, na MESMA linha do
`leftover_advise=`: `scan_readahead=windows:N bytes:N blocks:N`.

- Contadores atômicos no `Db`, bump **só em miss de loader de scan em
  store bounded** (fitting/hot ⇒ zero por construção; point-get não passa
  pelo loader — testado em `scan_readahead_hot_and_point_gets_unchanged`).
- Dedupe por stream: um advise por trecho de janela
  (`w.offset >= advised_to`) — **um advise por ~64 preads** no desenho
  4-KiB, não um por bloco (a primeira versão sem dedupe re-adviseava o
  rabo do run a cada bloco — O(n²)/cap; corrigido na mesma onda, exemplo
  real: 88 blocos ⇒ 2 windows/17 127 B, cada bloco coberto 1×).
- Observável em workload real: `cargo run --release -p pedradb-core
  --example scan_readahead` (bounded vs fitting no mesmo arquivo).
- O que falta ver (gated): no Linux o `WILLNEED` vira page-cache hit — o
  contador prova que o advise foi emitido, o meter P0.4 prova que paga.

## 2. Previsão determinística — estado do pin

Fixture `LINUX_QUIET_0189_P01` (último split quieto medido, 2026-09-10
**pre-0193**, re-pin bloqueado ⇒ **labeled-stale**):
`enc=350 wr=890 guard=2230 mlock=380 mins=580 publish=820 grp=270 lock_wait=2460` ns/op.

Vista pós-0190 (`--guard 0`): `cs_ns=3290`, `cut=wal_write`,
`qps_hat=173913`, `off_wr_qps_hat=238095`. Vista ticket pós-0193:
`ticket_cs_ns=2050` (CS − wr − enc), `ticket_cut=publish` (820 ns do pin
**velho**), `ticket_qps_hat=278784` — **+60,2% sobre o qps_hat pós-0190**,
teto do modelo, não promessa.

Ressalvas vivas (não escondidas): (a) `publish=820` é pre-0189-P0.3 — o
seqlock-skip zera só em shapes fills==0; em apply_mc4/ycsb_f/ycb_c o publish
paga; (b) o próximo dono REAL decide no re-split quieto (P1.2, gated), não
no pin velho.

## 3bis. Correção do tier calibrado muda o tamanho de TODO corte de CS

`findings/2026-09-10-write-forecast-why-it-missed.md` (0192 P0.4): o CS é
**7% da média** nas pernas medidas (r2: CS 2,49 µs de 34,6 µs); o wait paga
**amplificação de variabilidade** (scv 17–24 ⇒ Kingman (1+scv)/2 ≈ 9–12×) e
43% é fora-de-fase. Consequências com número:

- **Publish unification (0196 P0.2) perdeu o tamanho que a justificava.**
  O pin `publish=820 ns/op` é da era **pré-group-apply**, quando CADA membro
  publicava (4 publishes/grupo). Hoje o publish é **por grupo**
  (`apply_async_group_shared`): o split trim7+8 (04:05Z, fills==0 pós-skip)
  mostra `publish=0,29 µs/commit`; o caminho fills>0 soma
  dirty_points-lock + invalidate_many + last_prefix clear + 2 RMWs ≈
  0,8–1,1 µs/**grupo** ⇒ ~**200–280 ns/op** em mc4. Projeção calibrada:
  +1–2% qps no buraco apply_mc4 — o "×1,67 teto" do 0196 era
  teto-contra-teto, a mesma conflation que o 0192 P0.4 corrigiu. O corte
  segue desenhado (non-condition), mas não é mais "o maior corte
  model-nomeado": é polimento.
- **Fora-de-fase (~13 µs/op na perna quieta) NÃO é dono engine**: Rocks paga
  ~11,4 µs equivalente (média 15,2 µs − ciclo 3,8 µs na perna 264k). É o
  framework do cliente/harness, invariante entre engines. O buraco ENGINE
  quieta é ≈2,9 µs/op (18,0 − 15,2): CS ~2,4 + wait ~2,5 − waits do Rocks.
- **Os 446–551 µs de cauda (a fonte do scv)**: memtable nunca flaz no
  1024×10k (64 MiB ≫ 10 MB), preallocate dispara ≤1×/segmento — sobram
  janelas de scheduler/caixa (as mesmas que balançam o min-of-3 de
  0,35→1,29). Não há corte aterrizável; é a variável que o meter controla.

## 3ter. Lado GET (0197 P2.1 aterrizado)

`GET_SIDE_ANCHORS_2026_09_10` no `ratio_curve_kernel` + render no
`scale-model ratio`: 100M em 2 caixas (prefix 700‰ cartaz; big-guest 1050‰
contraste) + point-gets DIAG 3-run (get_hit 1576‰, get_loop 1484‰,
multi_get 1562‰, prefix vlen200 1309‰ — recomputados dos logs primários
`2026-09-04-win-probe-prefix/serial.win9{,.rerun2,.rerun3}.log`). Fato novo
no ranking: **o point-get em escala é PAGO (1,48–1,58×)** — o buraco GET
restante é o padrão sequencial frio (prefix 0,70×), já com corte 0195
aterrizado esperando meter.

- **Único ponto com hat E medido na mesma perna** (pre-0190, quieto):
  hat off-lock 123 426 vs medido 125 313 ⇒ `qps_hat_error_permille` =
  **−15 ‰** (hat 1,5% pessimista — teto do modelo conservador).
- Pós-0190/pós-0193: **não computável** (meter bloqueado). A aritmética
  está no kernel (`qps_hat_error_permille`, 3 testes) — a perna de medição
  aplica o número quando o gate abrir.
- Lição registrada: o hat é **teto estrutural** (CS serial `(L−1)/L`), não
  cartaz; nunca citar `ticket_qps_hat` como ganho prometido.

## 4. Dono por célula não paga (todo buraco same-class <1×, número datado)

| # | célula | número (data, rótulo) | dono pela telemetria | estado do corte |
|---|---|---|---|---|
| 1 | overwrite_mc4 **25M @4GiB** | **0,557× 3/3** (quieto 09-09) | piso write-ciclo 4,2 µs/op (floor-cut) **cortado pelo 0193 P0** (aterrizado, sem meter) + **pread de leftover na c/** (Fire 119: n_files=23, `dominant=pread`, legal=14133/pread=13500) **cortado pelo 0194 P0** (aterrizado, sem meter) | sobra: METER (P0.4 gated) |
| 2 | kvrocks_set_mc50 | 0,37× (cartaz) | **dono 0198 P0** (seção 4bis): cap-8 relink convoy + spin-herd em oversubscription no pipeline 1-op (código: `concurrent.rs` 1321/449); 0193 P0 já corta a espera in-lock (aterrizado, sem meter) | **0198 P0 aterrizado** (drain completo + spin ciente de ncpu); meter 0193 P0.5 (P1.1 gated) |
| 3 | apply_mc4 same-class | 0,47× (Linux) / 0,488 (Darwin DIAG 09-09) | mesma CS + **publish epochs vivos** (fills>0: skip 0189-P0.3 é fills==0); sem split pós-0193 | meter + re-split (P1.1/P1.2); publish unif. é P2.1 gated |
| 4 | prefix 100M @4GiB | 0,70× (cartaz) | GET/scan bounded-cache walk_all, `dominant=pread` (range-prune do probe_order já existe; ranges cobrem ⇒ anda tudo); contraste big-guest 1,05× ⇒ déficit é I/O, não compute | **corte aterrizado no 0195** (P0.1–P0.3: kernel + WILLNEED condicionado + telemetria); sobra: METER P0.4 (gated 19:33) |
| 5 | overwrite_mc4 10k | min 0,883 (pre-0193) | `wal_write` in-lock — **0193 P0 aterrizado** | gate 0185 P0.3 decide no meter (P1.1) |
| 6 | ycsb_f_mc4 | run2 0,766 (quieto 09-09); DIAG 0,530 | metade rmw: membro espera WAL+apply do líder (0193 P0 corta a espera); intern/TLS pago e não move (P0.80) | meter 0193 (P1.1); rmw restante P2.2 |
| 7 | ycsb_b_mc4 | sem Linux 3-run; Darwin p99 0,060 (DIAG) | cauda GET | medir primeiro (P1.4) |

U-cells Darwin DIAG (nenhum mecanismo — regra anti-overfit da skill):
ycsb_c 0,909; qs_neg 0,808; point_select 0,430; wbwi 0,410; flink 0,522;
venice 0,735; arango_traversal 0,003; pipelined 0,807 — lote 3-run P2.3.

## 4bis. Escolha do P0 do 0198: eixo-cliente no kvrocks_set_mc50

Fatos de código (verificados 2026-09-10, árvore atual): writes 1-op async
concorrentes **sempre** vão ao pipeline LfStack (`submit_after_begin`,
`concurrent.rs` ~1079 — `ASYNC_GROUP_DEFAULT=false` só afeta batches
multi-op e o merge do WriteThread; a receita oficial do cartaz isolado roda
sem flag de grupo). Nesse pipeline, dois mecanismos são cegos à escala do
cliente:

1. **Cap de drain 8** (`ASYNC_GROUP_MAX_MEMBERS`, `concurrent.rs` 425,
   aplicado em 1321: `stolen.len() > 8` ⇒ `relink_from(8)` +
   `wake_as_leader`): em mc50 cada geração vira ~**7 convoys seriais**
   (`ceil(50/8)`), cada um pagando `wal.lock` + hop de reserva + encode +
   pwrite do próprio grupo.
2. **Spin adaptativo cego a oversubscription** (`wait_wake`,
   `PIPELINE_SPIN_WINDOW=1024` × `BASE_WINDOWS=3` mínimo): 46 followers em
   PAUSE em 4 vCPU competem com o líder que precisaria daquele CPU — a
   regra adaptive-mutex ("spin só enquanto o dono roda") exige park quando
   writers > ncpu; em mc4 (L≈cores) o spin fica (evidência v5: −17% p50).

Dimensionamento (HAT, rotulado): o 0,37× é número **pre-pipeline** (era
0178/0183/0184; mc50 na árvore atual não medido — meter 0193 P0.5 carrega
essa célula). 0,37× ≈ 893 µs/op vs ~10–15 µs por ciclo saudável de 50 ops;
o colapso atribuído a spin-herd + convoi de relink + park/unpark herd.

Alternativas eliminadas com número (tier calibrado, seção 3bis): publish
unification ≈ +1–2% (não-significante); fora-de-fase = framework
(invariante entre engines); fdatasync não é pago na coluna async; termo de
disco 100M tem dono aterrizado desmedido (0194/0195). **Preallocate do WAL
off-lock** fica deferral com número: 4,1 s/25 s no perfil do hydrate 15M
(fallocate/F_PREALLOCATE 64 MiB bloqueante sob o mutex do wal), cauda
p99.9+ — não p99; dono 0198 P2.

Conformidade com non-goals: drain completo **não adiciona espera** (só
drena waiters já enfileirados — distinto do veto "qualquer espera para
crescer grupos" de 0180/0190) e é distinto do merge mpsc do WriteThread
falsificado (caixa 12-CPU, mediana 0,19×, funil single-leader): aqui é
LfStack + write off-lock + lanes; a razão do cap 8 (herd uncapped do 0044,
p50 ~200 µs) era do shape WriteThread antigo. Ouso-cap duro 256 contra
misuse.

## 5. O que a telemetria NÃO vê ainda (lacunas nomeadas)

1. **Split quieto pós-0193** (0192 P1.1) — sem ele o pin envelhece e
   `ticket_cut=publish` continua modelo-sobre-pin-velho. É a lacuna #1
   (destrava P1.2/P2.1 e o erro do modelo pós-corte).
2. **Perna medido-vs-hat por célula** (0192 P1.2) — 1 ponto no total
   (−15 ‰ pre-0190); o compare JSON agora carrega `lane_hist`, mas o
   `qps_hat` por shape só vira erro nomeado com a perna Linux.
3. **ycsb_b / prefix 100M sem Linux 3-run** — atribuição de cauda GET é
   Darwin DIAG; nada decide nelas antes do meter (P1.3/P1.4).
4. GET clock (0176) segue fora deste ciclo (gémeo de write, não lacuna
   deste RFC).

## 6. Conclusão para o 0198

Com o meter bloqueado (re-check 22:24), o tier calibrado (3bis) eliminou
toda alternativa write-side "significante" que não dependa de re-medir:
publish é polimento (+1–2%), fora-de-fase é framework, disco tem dono
aterrizado. O buraco restante com mecanismo **verificável em código e
aterrizável sem Linux** é o eixo-cliente do kvrocks_set_mc50 (4bis): cap-8
convoi + spin-herd em oversubscription — dois cortes compostos no pipeline
1-op default, com kernel puro, AS-IS twins e testes de integração
determinísticos por invariante (grupo médio > 8 impossível sob cap 8;
parks ≥ 1 sob writers > ncpu com heartbeats vivos). O 0198 P0 é esse par;
P1/P2 carregam os meters gated e o deferral do preallocate WAL.
