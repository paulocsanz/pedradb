# RFC-0201 — Eixo cliente: drain completo no pipeline 1-op e spin ciente de oversubscription

**Status:** open (P0 implementado; meters P1 blocked no gate datado; P2.1
deferral com número)
**Next:** desbloquear o host gate ⇒ P1.1 meter mc50 (confirma/refuta o mecanismo
composto no buraco 0,37×)
**Updated:** 2026-09-10
**ID:** 0201
**Parents:** [0185](0185-coluna-a-dropin-1x-tudo.md) (o alvo-produto: coluna A ≥1× em **tudo**),
[0197](0197-ratio-curve-same-class.md) (a curva que ranqueou o buraco; ranking linha 5),
[0193](0193-write-off-lock-pwrite-ticket.md) (o ticket/off-lock que o drain completo multiplica),
[0189](0189-ciclo-lider-janela-lenta.md) (o spin adaptativo que ganha a janela cega),
[0190](0190-apply-concorrente-despark.md) (lanes + guard-off — o envelope que invalidou o cap-8 do 0044),
[0041](0041-2x-rocks-default.md) (o piso registrado da coluna same-class)
**Peer:** RocksDB default `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`). G1 1c não é win.
Darwin = DIAG. Linux 3-run quieto = cartaz. Previsão é **hat** com erro nomeado — nunca cartaz.
Nota (0192 P0.4): ganho de QPS realizado projeta na média calibrada
(lognormal p50/p99, `findings/2026-09-10-write-forecast-why-it-missed.md`);
o número 0,37× desta célula é **pre-pipeline** (era 0178/0183/0184) — a
árvore atual não mediu mc50; todo ganho aqui é **hat** até o meter P1.1.

> **Tese:** o pior buraco write Linux nomeado sem corte aterrizado é
> `kvrocks_set_mc50` (0,37×, cartaz) — e a árvore atual tem **dois
> mecanismos cegos à escala do cliente** exatamente no caminho default que
> essa célula bate (writes 1-op async concorrentes **sempre** vão ao
> pipeline LfStack, `submit_after_begin` ~1079; `ASYNC_GROUP_DEFAULT=false`
> só afeta batches multi-op e o merge do WriteThread):
> (1) o **cap de drain 8** vira cada geração de 50 waiters em ~7 convoys
> seriais (`ceil(50/8)`), cada um pagando `wal.lock` + hop de reserva +
> encode + `pwrite` próprios;
> (2) o **spin adaptativo é cego a oversubscription** — 46 followers em
> PAUSE em 4 vCPU competem com o líder que precisava daquele CPU; a regra
> adaptive-mutex ("spin só enquanto o dono roda") exige park quando
> writers > ncpu. O corte composto (drain completo até um ousocap de
> misuse + park imediato sob oversubscription) não adiciona **nenhuma**
> espera — drena apenas waiters já enfileirados — então é distinto do
> veto de 0180/0190 ("qualquer espera para crescer grupos") e do merge
> mpsc do WriteThread falsificado (funil single-leader, mediana 0,19×).

## Background — o buraco e o caminho (fatos de código, 2026-09-10)

- **O caminho default do 1-op concorrente é o pipeline.** Receita oficial
  do cartaz isolado (`findings/2026-09-09-overwrite-mc4-linux-p149b/serial.md`)
  roda `PEDRA_PARITY_ASYNC=1 ROCKS_PARITY_SYNC=0` — sem flag de grupo; o
  shape kvrocks_set é 1-op `e.put` em loop. Em mc50 (50 clientes), todo put
  passa por `commit_async_one_pipeline`.
- **Cap 8 é artefato de shape morto.** `ASYNC_GROUP_MAX_MEMBERS=8`
  (`concurrent.rs` 425) nasceu do uncapped-herd do 0044 (p50 ~200 µs,
  WriteThread-merge era). O envelope mudou: LfStack (0190), write off-lock
  por ticket (0193), lanes paralelas (0190 P0.3), take-all reverso (trim7/8).
  G1 já é uncapped (fd-sharing). Em mc50 o cap custa ~7 convoys por geração,
  cada um com o próprio ciclo de lock+reserva+encode+pwrite.
- **O spin não sabe quantos CPU existem.** `wait_wake`
  (`PIPELINE_SPIN_WINDOW=1024` PAUSE, `BASE_WINDOWS=3`, `NO_PROGRESS=16`)
  gira enquanto o heartbeat do líder avança — correto em L≈cores (mc4:
  −17% p50, v5), suicida em L≫cores: o ciclo do líder não avança porque os
  followers estão queimando os CPU que ele precisaria.
- **Dimensionamento (HAT):** 0,37× ≈ 893 µs/op contra ~10–15 µs por ciclo
  saudável de 50 ops; o colapso atribuído a spin-herd + convoi de relink +
  park/unpark herd. Alternativas eliminadas com número
  (`findings/2026-09-10-write-forecast-why-it-missed.md`, 3bis/4bis do
  deep-dive): publish unification ≈ +1–2% (conflação do pin 820 ns/op
  pré-group-apply); fora-de-fase = framework do cliente (invariante entre
  engines); fdatasync não é pago na coluna async; termo de disco do 100M
  tem dono aterrizado desmedido (0194/0195).

## Problems This Solves

- **Problem:** a célula 0,37× (pior razão write Linux nomeada) não tem
  corte aterrizado — o ranking 0197 a delega ao meter, mas o mecanismo
  que a árvore já expõe (cap + spin) nunca foi atacado.
- **Problem:** o pipeline paga custo O(convoyos) que cresce com L: em mc4 é
  invisível, em mc50 é o shape dominante — "sempre em escala" exige que o
  caminho default degrame como O(1) por geração, não O(L/8).
- **Problem:** spin sob oversubscription é anti-adaptive-mutex: o próprio
  spin do follower remove o CPU do líder que o completaria.

## Proposed Solution

1. **P0.1 drain completo com ousocap**: o líder do pipeline drena **todos**
   os waiters já enfileirados (também no async — G1 já era uncapped por
   fd-sharing), limitado por um cap duro de misuse
   (`PIPELINE_DRAIN_MAX_MEMBERS=256`; acima disso, relink como hoje).
   Caminho WriteThread multi-op (`async_group_take`, cap 8) **inalterado**.
   Kernel puro `client_axis_kernel.rs`: `pipeline_drain_cap`,
   `drain_convoy_count` (o custo serial que o corte remove), AS-IS twin
   `pipeline_drain_cap_as_is` (= min(queued, 8)).
2. **P0.2 spin ciente de oversubscription**: `wait_wake` consulta
   `spin_policy(writers, ncpu)` — `Spin` se `writers ≤ ncpu` (comportamento
   0189 preservado: mc4 continua girando), `Park` imediato se
   `writers > ncpu` (pula as janelas de spin; o loop de park continua
   checando estado antes de parkar). `writers` = `inflight` do pipeline
   (leitura Relaxed), `ncpu` = `available_parallelism` em `OnceLock`.
   Kernel puro + AS-IS twin (`Spin` sempre — o dente 0189). Park por
   política não conta no tripwire `parks_without_silence` (flag
   test-only `parked_by_policy`).
3. **P1 meters quando o gate abrir**: mc50 3-run quieto (o buraco nomeado),
   apply_mc4/overwrite re-split (0192 P1.1/P1.2) na mesma janela, ycsb_f
   (0193 P0.5) — cada um devolve `qps_hat_error_permille`.
4. **P2 deferrals com número**: preallocate WAL off-lock (4,1 s/25 s no
   hydrate 15M, cauda p99.9+ não p99 — não é P0 sem medir a cauda no
   meter); grid B re-ancora (0196 P2.2).

## Delivery slices (mandatory)

### P0 — corte composto no pipeline default

> **Adendo 2026-09-11 (re-aberto):** o wiring abaixo foi aterrado e verificado
> em 2026-09-10 23:42 (7/7 testes verdes, scratch `named-tests.log`) contra a
> geração de árvore daquele dia. Às **23:49:31** um `git reset --hard` da
> sessão paralela (composição-registrada, entre os commits dela 23:30/23:57)
> destruiu todo o wiring não-commitado em arquivos rastreados
> (`concurrent.rs`, `lib.rs`); só sobreviveram os arquivos untracked (kernel,
> findings, RFCs). A geração viva não contém nem o cap-8 (`lead_pipeline_group`
> não existe; o líder faz `pending.drain(..)` completo) nem o spin 0189
> (`wait_wake` não existe) — o gargalo mc50 na árvore viva é o **bypass herd**:
> `PEDRA_ASYNC_GROUP` default off ⇒ 50 writers no rwlock de escrita, release
> injusto (acorda 49, 48 re-parkam), `PEDRA_WRITE_SPIN` default 0. O corte
> re-alveja esse caminho após o meter de atribuição; o kernel puro foi
> re-registrado no `lib.rs` em 2026-09-11 (4/4 testes verdes).

- [x] **P0.1** `client_axis_kernel.rs`: `pipeline_drain_cap` (min(queued,
      256), piso 1), `drain_convoy_count` (ceil-div), AS-IS twin
      `pipeline_drain_cap_as_is` (min(queued, 8)); wiring no líder de
      grupo vivo (`lead`: drenagem inicial limitada pelo kernel — o loop
      de absorb já dobrou o resto no mesmo frame de grupo desde a
      geração atual; o clamp é o piso de misuse); teste de integração
      `rfc0201_full_drain_groups_exceed_as_is_cap` (50 writers 1-op
      pinados no merge + ciclos esticados (payload 1 MiB — o stall 1-shot
      do protocolo pelo caminho real) ⇒ grupo médio > 8, impossível sob
      cap 8; todos os puts visíveis) — status: `done (2026-09-11; kernel
      4/4 + wiring + integração; nota: o bypass também conta como batch
      no `write_group_stats`, por isso o teste pina o merge)`
- [x] **P0.2** `oversubscription_spin_policy(writers, ncpu)` + AS-IS twin
      (`Spin` sempre) — kernel aterrado com testes (caixas + AS-IS). O
      wiring original (`wait_wake` do pipeline 0189) foi varrido pelo wipe
      de 2026-09-10 23:49 e **não existe na árvore viva**: nenhum follower
      gira (o bypass dá `PEDRA_WRITE_SPIN=0` = park direto; o follower de
      grupo espera em canal). A decisão de eixo que o kernel codifica está
      implementada estruturalmente pelo P0.3: oversubscrito (writers >
      ncpu) ⇒ merge ⇒ park-em-canal (pinado por
      `rfc0201_auto_async_merge_oversubscribed_herd`); encaixado (writers
      ≤ ncpu) ⇒ bypass sem spin — status: `done por absorção no P0.3
      (2026-09-11; kernel + twins verdes; alvo `wait_wake` extinto —
      nota datada em findings/2026-09-11-p201o-sweep/)`

### P1 — meters do eixo cliente (blocked no host gate)

- [x] **P0.3** (re-land pós-meter, 2026-09-11) `async_merge_policy(writers,
      ncpu, forced)` no kernel + wiring: `PEDRA_ASYNC_GROUP` vira pin
      explícito (`Option<bool>`) e o default é a regra cliente-eixo —
      escritores async concorrentes mergeiam num frame de grupo **iff
      `writers > ncpu`**; abaixo disso mantêm o bypass (formato Rocks;
      regime da falsificação 0044 intacto). Base: meter de atribuição
      2026-09-11 (`findings/2026-09-11-p201-meter-atribuicao/`) — mc50:
      merge **1,52× min-of-3 / 2,10× mediana** vs bypass 0,96×; o colapso
      0,37× era o fair handoff (0,33–0,38×). Testes: kernel
      `rfc0201_async_merge_policy_boundary_and_pins` + AS-IS twin;
      integração `rfc0201_auto_async_merge_oversubscribed_herd` (queued>0,
      amortização), `rfc0201_auto_async_bypass_when_writers_fit_cpus`
      (queued==0, batches==submits), `rfc0201_async_group_env_pin_overrides_axis`.
      A/B serial: mesmas 23 falhas do baseline, +5 verdes; musl exit 0 —
      status: `done (meter final P1.1 pendente)`
- [ ] **P1.1** meter Linux final (cartaz): sweep de regressão group vs
      default nas 20 formas (imagem `p201o`, 3 rounds quiet, mesmo boot) +
      A/B antes/depois do corte com env limpo (default = auto) — status:
      `in_progress` (sweep DONE e adjudicado 2026-09-11,
      `findings/2026-09-11-p201o-sweep/`: os 10 flags paired são todos de
      formas single-threaded/read-only — braços código-idênticos, ruído da
      caixa (noise floor 0,561 entre braços idênticos); a única forma onde
      os braços divergem mecanicamente é mc50: 1,034→**1,894** min-of-3;
      A/B pós-corte `p201q` auto-vs-pin0 rodando)
- [ ] **P1.2** re-split quieto pós-0193 + apply_mc4/overwrite na mesma
      janela (0192 P1.1/P1.2) — status: `blocked`
- [ ] **P1.3** ycsb_f mc4 (0193 P0.5) e o eixo `ratio_hat(L)` do 0197 P1.4
      com o CS re-medido — status: `blocked`

### P2 — deferrals com número

- [ ] **P2.1** preallocate WAL off-lock (`reserve_space` fallocate
      64 MiB): 4,1 s/25 s no perfil hydrate 15M, cauda **p99.9+** (não
      p99) — deferral até um meter mostrar a cauda como dono do buraco —
      status: `deferred (número registrado; dono não é p99)`
- [ ] **P2.2** grid B (compaction on) re-ancora o eixo cliente — status:
      `blocked` (0196 P2.2)

## Ranking — onde este P0 ataca (herdado do 0197, linha 5)

| # | célula | número (data, rótulo) | dono/atribuição | ataque | banda |
|---|---|---|---|---|---|
| 1 | kvrocks_set_mc50 | 0,37× (cartaz, pre-pipeline) | **este RFC**: cap-8 convoi + spin-herd (4bis deep-dive) | P0.1+P0.2 aterrizados; meter P1.1 | **P0** |
| 2 | apply_mc4 same-class | 0,47× (Linux 0183) | 0193 aterrizado; publish = polimento (+1–2%) | meter P1.2 | P1 |
| 3 | overwrite 100M @4GiB | 0,417 hat (0197) | disco (84% do déficit) — 0194/0195 aterrizados | meter 0197 P1.1 | P1 |
| 4 | ycsb_f_mc4 | 0,766 run2 | 0193 corta a espera rmw | meter P1.3 | P1 |
| 5 | prefix 100M | 0,70× (cartaz) | 0195 aterrizado | meter 0195 P0.4 | P1 |

Por que este P0 e não um meter: o host gate está fechado (load1 ≥ 8, guest
`Unknown host` — re-check 22:24) e o skill `otimizar` veda meter com o gate
falhado; o precedente 0192/0193/0194/0195 é exatamente isto — mecanismo +
kernel + testes aterrizzam com o gate fechado, meter fecha blocked-datado.
O ganho composto é **hat** (o 0,37× é pre-pipeline; a árvore atual não
mediu mc50) e o meter P1.1 é o único caminho para virar cartaz.
**(2026-09-11: gate re-adjudicado e ABERTO — o Linux é o serviço caixote
`linux-gate-p149b`, não OrbStack local; o meter P1.1 está rodando.)**

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | drain completo pipeline (ousocap 256) + kernel | done | kernel + clamp no `lead` + `rfc0201_full_drain_groups_exceed_as_is_cap`; A/B serial limpo | 2026-09-11 |
| P0.2 | p0 | spin ciente de oversubscription + twins | done (absorvido no P0.3) | kernel + twins verdes; `wait_wake` extinto pelo wipe; decisão de eixo implementada pelo P0.3 (oversubscrito ⇒ merge ⇒ park-em-canal) | 2026-09-11 |
| P0.3 | p0 | merge assíncrono cliente-eixo (`writers > ncpu`) + pin env | done | kernel + wiring + 5 testes; A/B serial limpo; base = meter de atribuição 2026-09-11 | 2026-09-11 |
| P1.1 | p1 | sweep regressão 20 formas + A/B antes/depois pós-corte | in_progress | sweep `p201o` DONE+adjudicado (10 flags = ruído código-idêntico; mc50 1,034→1,894); A/B `p201q` (auto vs pin0) rodando | 2026-09-11 |
| P1.2 | p1 | re-split quieto + apply/overwrite | blocked | 0192 P1.1/P1.2 | 2026-09-10 |
| P1.3 | p1 | ycsb_f + ratio_hat(L) | blocked | 0193 P0.5 / 0197 P1.4 | 2026-09-10 |
| P2.1 | p2 | preallocate WAL off-lock | deferred | 4,1 s/25 s (p99.9+; hydrate 15M) | 2026-09-10 |
| P2.2 | p2 | grid B re-ancora | blocked | 0196 P2.2 | 2026-09-10 |

## Acceptance Criteria

- **Tests**
  - `rfc0201_drain_cap_full_below_misuse_floor` — `pipeline_drain_cap(50)
    == 50`; `pipeline_drain_cap(1_000) == 256` (misuse floor).
  - `rfc0201_drain_convoy_count_as_is_vs_full` — mc50: AS-IS
    `drain_convoy_count(50, 8) == 7` vs novo `(50, 256) == 1`.
  - `rfc0201_spin_policy_both_sides` — `Spin` quando writers ≤ ncpu
    (inclui ncpu=1/writers=1 e writers=ncpu exato); `Park` quando
    writers > ncpu; AS-IS twin sempre `Spin`.
  - `rfc0201_full_drain_groups_exceed_as_is_cap` — stall 1-shot do líder +
    50 writers 1-op: `pipe_group_ops/pipe_groups > 8` (impossível sob
    cap 8) e todos os puts visíveis pelo caminho de leitura real.
  - `rfc0201_oversubscribed_followers_park_immediately` — N = ncpu+8 com
    heartbeats vivos: `pipe_parks ≥ 1` e `parks_without_silence == 0`
    (park por política, não por silêncio).
  - `rfc0201_spin_preserved_when_writers_fit_cpus` — N = ncpu: zero
    `parks_without_silence`, puts visíveis (o lado mc4 do 0189 intacto).
  - Regressão viva: `adaptive_spin_absorbs_slow_leader` e
    `adaptive_spin_parks_when_leader_stalled` (0189) continuam verdes.
- **Serial A/B:** failure set de `cargo test -p pedradb-core --lib --
  --test-threads=1` igual ao baseline pré-mudança (extras só com
  provenância).
- **Musl:** `cargo check -p pedradb-core --target x86_64-unknown-linux-musl`
  exit 0.
- **Parity:** nenhuma linguagem de win sem Linux 3-run quieto; o ganho
  mc50 permanece **hat** até P1.1; peer sempre `sync=false`.
