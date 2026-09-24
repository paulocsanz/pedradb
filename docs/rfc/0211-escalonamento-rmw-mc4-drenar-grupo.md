# RFC-0211 — Escalonamento rmw mc4: drenar o grupo de writers no regime writers == ncpu

**Status:** in-progress (P0 done 2026-09-12 — meter p211m validou o
mecanismo, alvo ≥1,0 não fechado no min; P1.2 é a próxima fatia)
**Updated:** 2026-09-12
**ID:** 0211
**Parents:** [0201](0201-cliente-drain-cheio-spin-oversub.md) (a fronteira
`async_merge_policy`: merge só quando writers > ncpu — pagou mc50 1,678× e
deixou o regime ==ncpu no bypass), [0209](0209-wal-buffer-user-space.md)
(o meter p209b isolou o dono: buf tira a syscall do caminho e o piso
ycsb_f_mc4 sobe 0,532→0,780 sem fechar — o resto é escalonamento, não
syscall), [0196](0196-meter-first-publish-unification.md) (P2.3 mediu o
buraco 0,2947× e o reatribuiu)
**Peer:** RocksDB default `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`).
A coluna same-class (`PEDRA_PARITY_ASYNC=1`) é o gate oficial (floor
RFC-0041 = 1,0). G1 1c não é win por construção (fd antes de Ok); sync-peer
nunca é win; Darwin = DIAG; Linux 3-run quiet min-of-3 = cartaz; previsões
rotuladas **hat**. Cross-boot absoluto não é comparável (nota de
re-ancoragem p209b) — toda comparação deste RFC é same-boot.

> **Tese:** o buraco estrutural nº 1 do board medido same-boot
> (`ycsb_f_mc4` piso **0,532** nobuf / **0,780** buf, p209b 3 rounds
> quiet) é a fronteira 0201 do bypass: com writers == ncpu, cada writer
> 1-op async toma a write-lock do Db e corre o commit inteiro serializado
> (encode + write WAL + apply + publish). O pipeline drenável — líder
> drena a geração inteira, 1 frame, `write()` off-lock, apply em grupo —
> já existe, já é o caminho dos grupos sync/multi-op e está pago no regime
> oversubscribed (mc50 1,678×); ele simplesmente não alcança o regime
> ==ncpu. O ataque é estender a fronteira para o grupo async 1-op
> (rmw = get + put 1-op é a forma que afunda), decidido por kernel puro +
> meter 3-run quiet min-of-3. Sem espera por writers em lugar nenhum
> (o líder drena o que JÁ chegou — cap de misuse 0201 preservado).

## Background (números datados; mecanismo verificado in-tree 2026-09-11)

- **Mecanismo (código, não hat):** `ycsb_f_mc4` = 4 clientes × {50%
  `get_probe`, 50% rmw (`get` + `put` 1-op)}. O `put` 1-op async em
  `submit_after_begin` (concurrent.rs) com `async_merge_policy(4, 4)=
  false` vai ao bypass: write-lock do Db → `commit_async_one` inteiro
  (encode, syscall WAL, apply, publish, flush-check) → unlock. Quatro
  writers, uma lock, série completa por op. O Rocks `sync=false` paga só
  memcpy bufferizado por writer + insert paralelo.
- **Dois degraus medidos (p209b, same-boot, 3 rounds quiet, min-of-3):**
  nobuf 0,532 → buf 0,780 (syscall fora da lock: +47% no piso, dispersão
  apertada 1,882/0,791/0,532 → 0,963/1,055/0,780) — resta a serialização
  encode+apply+publish sob a write-lock.
- **Forma que afunda vs formas que não afundam (mesmo boot):** rmw mc4
  0,532; write puro mc4 (`deps_cache_overwrite_mc4`) 1,035; mix 50/50 sem
  dependência (`ycsb_a_mc4`) 1,537. O `get` do rmw entre os puts agrava a
  alternância read/write-lock e o put paga a série serial inteira.
- **O que já está pago e não se re-mede:** apply_mc4 1,0859×;
  kvrocks_set_mc50 1,678× (o pipeline drenável em ação no regime
  oversubscribed). Guardiãs nas ondas novas em nível ≥, sem contradição.

## Problems This Solves

- **Problem:** o pior buraco same-class vivo medido same-boot (0,532×
  min) é um regime de escalonamento sem dono: nem o buffer WAL (0209,
  adjudicado opt-in) nem o merge 0201 (fronteira writers > ncpu) o cobrem.
- **Problem:** os mecanismos candidatos (`PEDRA_ASYNC_GROUP=1`,
  `PEDRA_WRITE_FAIR=1`, `PEDRA_WRITE_SPIN=N`, composição com
  `PEDRA_WAL_BUFFER=1`) nunca foram medidos NA célula do buraco — sem
  número, a fronteira 0201 fica protegia por hat em ambos os sentidos.
- **Problem:** multi-op mc4 (`deps_apply_batch_mc4`, `deps_raftlog_mc4`)
  já bate ≥1 no bypass — qualquer corte que os mova de regime sem número
  é risco de regressão de guarda.

## Proposed Solution

1. **Kernel puro** `rmw_sched_kernel.rs`: `SchedDecision::{Bypass, Merge}`
   a partir de `(writers, ncpu, single_op, forced)` — merge quando
   `writers > ncpu` (regra 0201 intacta) **ou** (`single_op` ∧
   `writers ≥ 2`) (o regime novo); pin `forced` vence como hoje; twin
   AS-IS = a fronteira 0201 exata (single_op irrelevante); guardas de
   misuse (`ncpu == 0` nunca mergeia; `writers ≤ 1` nunca mergeia).
2. **Wiring opt-in** no caminho real (`submit_after_begin`): env
   `PEDRA_RMW_SCHED=1` (lido na abertura, padrão **off** = AS-IS
   byte-comportamental); `ops.len() == 1` alimenta o `single_op` do
   kernel; multi-op e G1/sync intocados; `#![forbid(unsafe_code)]`
   permanece; nenhuma telemetria nova default-on (0169).
3. **Meter no gate real** (protocolo p209b idem): same-boot, 3 rounds
   quiet (load1<2 ×2), `ROCKS_PARITY_CLIENTS=4` nos dois engines,
   `PEDRA_PARITY_ASYNC=1`, peer `sync=false` do mesmo round,
   `ROCKS_YCSB_DIST` unset, `ROCKS_YCSB_OPS=2000`; braços {env-limpo
   (âncora), `PEDRA_RMW_SCHED=1`, `PEDRA_RMW_SCHED=1`+`PEDRA_WAL_BUFFER=1`,
   `PEDRA_WAL_BUFFER=1`}; células-alvo `ycsb_f_mc4` (+ `ycsb_f` single,
   `kvrocks_set` single), guardiãs `ycsb_a_mc4`,
   `deps_cache_overwrite_mc4`, `deps_apply_batch_mc4`,
   `kvrocks_set_mc50`; telemetria `write_group_stats` +
   `write_phase_stats` capturada por braço (grupos formados de verdade,
   lock_wait vs wal). Veredito datado no RFC — vitória OU perda honesta
   (perda vira P1 nomeada, nunca silêncio).
4. **Default flip** só com meter (P1) — até lá opt-in, revertível numa
   onda, zero mudança para quem não seta o env.

## Ranking — o dono e as vizinhas (inventário rev. 2, 2026-09-11)

| # | célula | número (rótulo) | estado |
|---|---|---|---|
| 1 | ycsb_f_mc4 | **0,532** min same-boot p209b (nobuf) | **P0 deste RFC** |
| 2 | ycsb_f_mc4 braço buf | **0,780** min (p209b) | P0 (mesma fatia: residual pós-syscall) |
| 3 | ycsb_a_mc4 braço buf | 1,115 min (nobuf 1,537) | guarda do meter (regressão do buf a não propagar) |
| 4 | prefix 100M @4GiB / 25M/15M | 0,70 / 0,557 (cartazes próprios) | blocked orçamento de onda (0209 P2.2 re-adjudicado) |
| 5 | U-cells (qs, point_select, wbwi, flink, venice, arango, pipelined) | DIAG | lote Linux 3-run na mesma janela de gate (0209 P2.1) |

Por que este P0 e não outro: (a) é o piso №1 medido same-boot em dois
degraus independentes (buf/nobuf) que isolam o mecanismo; (b) o pipeline
drenável alvo já existe, está pago no mc50 e é o desenho do próprio repo —
a fatia é a fronteira, não um mecanismo novo; (c) o corte é opt-in até o
meter = risco zero ao default; (d) guardiãs multi-op ficam fora do regime
novo por construção (`single_op`).

## Delivery slices (mandatory)

### P0 — kernel + wiring opt-in + meter com veredito datado

- [x] **P0.1** Kernel `rmw_sched_kernel.rs`: `SchedDecision` + regra
      (`writers > ncpu` ∨ (`single_op` ∧ `writers ≥ 2`)) + twin AS-IS
      (fronteira 0201 exata) + guardas misuse (`ncpu == 0`, `writers ≤ 1`
      ⇒ Bypass) — testes nomeados `rfc0211_*` — status: `done` (6 testes:
      fronteira eq-ncpu, twin ≡ 0201 no grid inteiro, multi-op bypass,
      oversubscribed ambos, pin vence, degenerados)
- [x] **P0.2** Wiring no caminho real: env `PEDRA_RMW_SCHED=1` opt-in
      (padrão = AS-IS), `single_op = ops.len() == 1` alimentando o kernel,
      multi-op/G1 intocados, teste de eixo env serializado (mutex + 
      drop-guard) provando merge real via `write_group_stats` (grupos
      formados, tamanho médio ≥ 2) com env e bypass sem env — status:
      `done` (`rfc0211_env_axis_rmw_sched_forms_groups`: env-limpo
      `queued == 0`/batch por submit; env=1 `queued > 0`/amortizado;
      puts duráveis pós-reopen nos dois braços; vizinhos `rfc0201_*` 10/10;
      musl exit 0)
- [x] **P0.3** Meter caixote linux-gate (braços/células do desenho,
      3-run quiet min-of-3, telemetria por braço) + finding datado +
      veredito no RFC — status: `done` (2026-09-12T00:25:56Z, onda
      `p211m`, finding `findings/2026-09-11-rfc0211-p0-meter/`:
      `PEDRA_RMW_SCHED=1` sobe o piso do alvo 0,491→**0,836** min
      (+70%; med 0,622→1,352) com TODAS as guardiãs ≥ clean no min
      (ycsb_a +2,9%, cache_overwrite +42,5%, apply_batch +5,1%, mc50
      +17,3%) — mecanismo validado, mas **0,836 < 1,0 = perda honesta
      vs o gate**; rmwbuf não fecha (0,552); P1.2 decomposição do
      residual é a próxima fatia)

### P1 — decisão de default e residual

- [x] **P1.1** Flip do default da fronteira SE P0.3 validar: min-of-3
      ≥1,0 na célula-alvo SEM regredir guardiãs (regra ≥5%); senão
      mantém opt-in com finding datado — status: `done` (2026-09-12,
      regra do próprio RFC aplicada ao meter P0.3: alvo 0,836 min <
      1,0 com guardiãs todas ≥ clean ⇒ ramo "senão" — **opt-in
      mantido, zero mudança de default**; finding
      `findings/2026-09-11-rfc0211-p0-meter/`; revisável apenas por
      meter futuro min-of-3 ≥1,0 sem regressão de guarda)
- [x] **P1.2** Decomposição do residual do braço vencedor
      (`write_phase_stats`: lock_wait vs wal vs publish por commit) —
      nomeia o dono da próxima fatia (ou fecha com número) — status:
      `todo`

### P2 — anti-overfit e frente pesada

- [x] **P2.1** Sweep do eixo writers (mc2/mc3/mc6/mc8) no regime
      vencedor — a fronteira nova não pode ser hat além do ponto medido —
      status: `done (2026-09-13: DIAG Darwin admission-clean, min-of-3 — fronteira cruza mc3→mc6: delta 0,99×→1,97×→2,23×; guard apply_batch 0,91–1,13× passa; avg_grp 1,0→3,1 explica; meter oficial gate-blocked datado)`
- [x] **P2.2** Grid B completo (10–100×, compaction on) quando um corte
      mudar default (herda a pré-condição do 0209 P2.3) — status:
      `blocked (condicional-datado)` (2026-09-12: P1.1 aplicou o ramo
      "senão" — opt-in mantido, zero default mudado no 0211; nenhuma
      fatia restante do RFC muda default ⇒ pré-condição do Grid B
      não-satisfeita; abre apenas por meter futuro min-of-3 quiet
      ≥1,0 no alvo sem regressão de guardiãs que flip um default)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | kernel SchedDecision + twin AS-IS + guardas | done (6 testes `rfc0211_*` verdes) | este commit | 2026-09-11 |
| P0.2 | p0 | wiring PEDRA_RMW_SCHED opt-in no submit_after_begin | done (teste de eixo env no caminho real; 0201 10/10; musl 0) | este commit | 2026-09-11 |
| P0.3 | p0 | meter 4 braços × células + guardiãs, veredito datado | done (p211m 2026-09-12: alvo 0,491→0,836 min +70%, guardiãs todas ≥ clean; 0,836<1,0 = perda honesta, opt-in mantido) | findings/2026-09-11-rfc0211-p0-meter | 2026-09-12 |
| P1.1 | p1 | flip default pós-meter | done (2026-09-12: regra aplicada — min 0,836 <1,0 ⇒ opt-in mantido, zero default mudado) | findings/2026-09-11-rfc0211-p0-meter | 2026-09-12 |
| P1.2 | p1 | decomposição do residual (telemetria) | done (2026-09-13: dono do 0,836 NÃO é o escalonador — provado nos 2 extremos; dono restante = I/O serial per-commit (fdatasync da coluna paridade), confirmação Linux admission-clean gate-blocked registrada) | findings/2026-09-12-rfc0211-p12-residual | 2026-09-13 |
| P2.1 | p2 | sweep eixo writers | done (2026-09-13: DIAG Darwin min-of-3, fronteira mc3→mc6 delta 1,97×/2,23×, guard passa; meter oficial gate-blocked datado) | findings/2026-09-13-rfc0211-p21-sweep | 2026-09-13 |
| P2.2 | p2 | Grid B quando default mudar | blocked (condicional-datado 2026-09-12: P1.1 opt-in mantido, zero default mudado; abre só por meter futuro que flip default) | P1.1 acima | 2026-09-12 |

## Acceptance Criteria

- **Tests (nomeados, caminho real)** — prefixo `rfc0211_`:
  - `rfc0211_boundary_writers_eq_ncpu_single_op_merges` — a fronteira
    nova: `writers == ncpu ∧ single_op` ⇒ Merge; twin AS-IS ⇒ Bypass.
  - `rfc0211_multi_op_stays_bypass_at_eq_ncpu` — multi-op em
    writers == ncpu ⇒ Bypass nos DOIS regimes (guarda apply/raftlog mc4).
  - `rfc0211_oversubscribed_merges_in_both` — `writers > ncpu` ⇒ Merge
    com e sem o env (regra 0201 intacta).
  - `rfc0211_forced_pin_wins_both_directions` — `forced` vence a
    fronteira nova como vence a velha.
  - `rfc0211_degenerate_boxes_never_merge` — `ncpu == 0`, `writers ≤ 1`.
  - `rfc0211_env_axis_rmw_sched_forms_groups` — eixo env serializado
    (mutex + drop-guard): 4 threads × puts 1-op async com
    `PEDRA_RMW_SCHED=1` formam grupos de verdade
    (`write_group_stats`: `groups_committed > 0`, tamanho médio ≥ 2);
    sem env, `groups_committed == 0` (bypass) — mesmo workload.
  - Suíte crash/reopen/torn existente verde (o contrato de durabilidade
    não muda: o grupo drenável é o caminho dos grupos sync de hoje).
- **Telemetry / Analytics**
  - Nenhuma linha nova default (0169). O finding do meter carrega
    `write_group_stats` + `write_phase_stats` por braço (óraculo de que o
    merge aconteceu e onde foi o tempo).
- **Documentation**
  - Este RFC; inventário rev. 2 (`findings/2026-09-11-gargalos-inventario/`);
    finding do meter P0.3; `docs/status.md` nos mesmos commits dos flips.
- **Screenshots**
  - Backend-only — n/a.

## Out of scope

- Espera por writers em qualquer forma (wait-to-grow/linger — veto
  permanente 0180/0190; o líder drena o que já chegou, cap 256).
- Mover multi-op mc4 ou G1/sync de regime (guardas intocadas por
  construção do `single_op`).
- Flipar default sem meter válido (P1.1 exige min-of-3 quiet).
- Reabrir meters pesados 100M/15M/25M (blocked re-adjudicado no 0209
  P2.2 com host-check datado).
- Cotar G1 1c como win; sync-peer como win; Darwin como cartaz;
  re-medir apply_mc4/mc50 fora de guarda.
