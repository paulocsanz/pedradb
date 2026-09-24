# RFC-0226 — Fechar o U write-vs-N: seal-and-absorb no async, grupo-de-1 no bypass

**Status:** closing (P0–P2 landed or parked/C with this fire’s meter)
**Updated:** 2026-09-15
**ID:** 0226
**Parents:** [0211](0211-escalonamento-rmw-mc4-drenar-grupo.md)
(drenar o grupo no regime `writers == ncpu`; o follow-up
`seal_async_first_drain` aterrissou fora deste RFC),
[0217](0217-fechar-board-async-grupo-ucells-escala-encode-read.md)
(janela de coleta / grupo em baixa concorrência; veto wait-to-grow),
[0180](0180-overwrite-mc4-gt1x.md) (célula rank-1),
[0201](0201-cliente-drain-cheio-spin-oversub.md)
(drena o que já chegou; nunca espera para crescer),
[0055](0055-rocks-write-pipeline.md)
(WriteThread / concurrent memtable / pipelined write — parked até
número obrigar),
[0044](0044-async-class-5x-rocks.md) (classe async: `write()` por
commit, sem fd para amortizar)
**Peer:** RocksDB default `WriteOptions.sync=false`
(`ROCKS_PARITY_SYNC=0`). G1 1c não é win. Darwin = DIAG. Linux 3-run
quiet = cartaz. Finding do fire:
`findings/2026-09-14-write-degrada-n-seal-async/`.

> **Tese:** o write Pedra descreve um U em N clientes (fundo em mc2)
> enquanto o Rocks é plano ~144 k. Não é um buraco — são **três
> regimes**. O Rocks é plano porque o WriteThread (1) nunca espera
> membro futuro (self-link + snapshot da lista + absorb na seção
> serial) e (2) tem custo fixo de grupo-de-1 ≈ custo de um writer
> sozinho. Pedra (1) fazia o líder esperar para crescer o grupo
> async **sem fd para amortizar** (join 20 µs + collect 10,9 µs/grp
> contra ~3,6 µs/commit de engine — 85% da parede) e (2) entra no
> caminho de grupo mesmo quando o grupo vai ter 1 membro
> (`avg_group=1,04` em mc2), pagando Mutex+mpsc a mais que o bypass.
> O corte 2026-09-14 (`seal_async_first_drain`) pagou (1) no regime
> `writers ≤ ncpu`. Este RFC paga o resto: o falso-positivo do selo
> em mc16 (−13% nomeado), o grupo-de-1 que ainda entra em `lead()`,
> e o número Linux da célula rank-1 (0,557× ainda não re-medido com
> o corte). Não porta o WriteThread no escuro (0055 P1.3 parked).

## Background

Sweep Darwin DIAG 2026-09-14, eixo clientes, `overwrite` records=1024
ops=20k, seeds idênticos (`findings/2026-09-14-write-degrada-n-seal-async/`
§1):

| clientes | Pedra | Rocks | ratio | regime |
|---:|---:|---:|---:|---|
| 1 | 102,0k | 147,8k | 0,69 | **C** — teto-fd G1 (1 barreira/op vs 0 do peer). Linux async da mesma célula vence 1,254×. Não é este RFC. |
| **2** | **62,5k** | 143,7k | **0,43** | Fundo do U. `avg_group=1,04`: maquinaria de grupo para 1 op. |
| 4 | 82–104k | 57–141k | ~0,6–0,7 | Rank-1. Cartaz Linux **0,557×** 3/3 (pre-corte). Darwin DIAG do corte: +19% vs twin. |
| 8 | 107,3k | 134,2k | 0,80 | writers ≤ ncpu; o corte sela. |
| 16 | 43–189k | 143,9k | 0,30–1,31 | Oversubscribed (16 > 12 CPUs nesta caixa). Corte: **−13% vs twin** (177,4k vs 204,5k) — selo dispara com `submit_active ≤ ncpu` mesmo com 16 clientes. |

Rocks é plano ~144 k em N. Pedra nunca escala com N e tem variância
até 4× (P/E cores Darwin).

Mecanismo in-band (`PEDRA_WRITE_PHASE_STATS=1`, overwrite_mc4, grupo
formado `avg_group=3,29`):

```
engine ≈ 3,6 µs/commit
cw     = 10,88 µs/grp   (collect)
join   = JOIN_LOOK_US=20 µs  (não entra no cw)
```

Por grupo em mc4: ~30 µs de espera + ~4 µs de trabalho. **A espera é
~85% da parede.** `PEDRA_GROUP_WINDOW_US=1000` (coletar *mais*)
colapsa a célula 11× — esperar por membro futuro é perda pura no
async. O Rocks sela com quem já está na fila e absorve atrasados
durante a seção serial (`db/write_thread.cc`: `LinkOne` + walk +
`JoinBatchGroup`; o atrasado vira o *próximo* grupo, não faz o
atual esperar).

O que já aterrissou (2026-09-14, fora de um RFC próprio; matrícula
formal 2026-09-15):

- Kernel `group_window_kernel::seal_async_first_drain(writers, ncpu,
  any_sync, window_us)`: sela no primeiro drain iff `!any_sync ∧
  window_us==0 ∧ writers≤ncpu`. `writers` = max `submit_active` do
  batch (sinal de oversubscription, não o `active` vivo — gap ghost
  RFC-0217 P0.1b). Twin `*_as_is` = nunca sela. Env
  `PEDRA_SEAL_ASYNC=0` restaura o gêmeo. Default ON.
- Wiring em `ConcurrentDb::lead()`: o join yield-loop e o bound de
  collect saem quando a política sela. Atrasados continuam no absorb
  pós-`group_start` (shape Rocks).
- Darwin DIAG intercalado, célula rank-1: corte 9/12 pares vs twin,
  mediana 92,8k vs 77,8k (+19%). Guardiãs: `kvrocks_set_mc50` mediana
  intacta (`avg_group` ~23), G1 neutro, mc2 neutro. **mc16 −13%
  nomeado.** Linux 0,557× **não re-medido**.
- Catálogo `seal_async_first_drain` atom, `GroupWindow.lean`
  `seal_async_first_drain_fate_iff`, floor_atom 325. Skill `otimizar`:
  kernel fn nova sem linha de catálogo = failed fire.

Como o Rocks evita o U (os três fatos, não um slogan):

1. **Nunca espera o futuro.** Quem chega se liga sozinho na cauda
   (`LinkOne`, CAS). O líder sela o snapshot. O atrasado monta o
   *próximo* grupo enquanto o atual escreve.
2. **Grupo-de-1 é barato.** WriteThread de um writer = ligar-se,
   ser líder, `write()` no buffer, desligar. Sem Mutex de fila, sem
   mpsc por op, sem yield de 20 µs.
3. **No async default não há barreira.** A seção serial é memcpy no
   buffer do WAL + insert no skiplist (~µs). Grupo é otimização
   marginal, não necessidade. Pedra G1 1c *tem* barreira — teto por
   construção da coluna, não defeito (RFC-0041).

O U Pedra é (1)+(2) no caminho de grupo, não (3) — a coluna deste
RFC é same-class async (`PEDRA_PARITY_ASYNC=1`), onde Pedra também
não fsynca no Ok (errata 0217: o dono do residual mc2–mc8 **não** é
fdatasync).

## Problems This Solves

- **Problem:** o selo 2026-09-14 usa `max(submit_active) ≤ ncpu`
  como sinal de “não-oversubscribed”. Em mc16 (16 clientes, 12
  CPUs) o max do batch frequentemente cai ≤ ncpu (nem todos os 16
  estão dentro de `submit()` ao mesmo tempo) → o selo dispara →
  collect que *deveria* ficar (followers no `submit()`, spin do
  líder sobrepõe trabalho real) some → **−13% vs twin**, nomeado.
- **Problem:** mc2 continua o fundo do U (`avg_group=1,04`) mesmo
  com o selo: o submitter *entra* no caminho de grupo (gap ghost /
  `rmw_sched` / `merge_eligible`) e o líder sela um grupo de 1.
  Sela sem esperar, mas ainda paga Mutex da fila + mpsc por op +
  `lead()` — estritamente mais caro que o bypass
  (`commit_async_one` sob a write-lock). O Rocks não tem esse degrau.
- **Problem:** o cartaz rank-1 (`overwrite_mc4` Linux **0,557×**)
  não foi re-medido com o corte. Darwin +19% vs twin não é o
  número. Sem o 3-run Linux (ou DIAG Darwin da curva N se a caixa
  bloquear) o corte é hipótese sobre a célula que conta.
- **Problem:** portar o WriteThread (lista lock-free, pipelined
  write, concurrent memtable) sem número é o buraco que o 0055
  existe para impedir. Este RFC não o reabre.

## Proposed Solution

Três cortes de política no kernel de janela que **já existe**, nesta
ordem, cada um com meter vs Rocks `sync=false` no mesmo fire. A
estrutura WriteThread (0055) só abre se P1 ainda deixar ≥15% do gap
na maquinaria de grupo.

1. **Selo só no líder sozinho (P0).**
   `seal_async_first_drain` ganha `batch_len`. Sela iff a política
   atual **e** `batch_len == 1`. Primeiro drain com 2+ membros já
   é um grupo — o collect fica (oversubscription / herd no
   `submit()`). Recupera o −13% do mc16 sem desfazer o ganho do
   líder sozinho (mc4 primeiro drain frequentemente 1 por gap
   ghost). Twin AS-IS = a fn de 2026-09-14 (sela sem olhar
   `batch_len`). Default ON, `PEDRA_SEAL_ASYNC=0` continua o gêmeo
   pré-2026-09-14 (nunca sela).
2. **Grupo-de-1 cai no bypass (P1).**
   Kernel `solo_leader_bypass(batch_len, queue_len, active)`: se o
   primeiro drain é 1 e ninguém está na fila nem em `submit()`,
   `lead()` devolve o membro ao caminho `commit_async_one` em vez
   de correr a seção serial de grupo. Mata o fundo do U (mc2
   `avg_group=1,04` pagando Mutex+mpsc). Twin AS-IS = fica em
   `lead()`. Opt-in `PEDRA_SOLO_BYPASS=1` até o meter; flip de
   default só com número.
3. **Selo async sempre (P1, iff meter).**
   Dropar o teto `writers ≤ ncpu` — política Rocks pura no async:
   nunca esperar membro futuro, absorver na seção serial, em
   qualquer N. Só se `kvrocks_set_mc50` e mc16 **não** recuarem no
   meter do (1). Senão o teto fica e vira C nomeado.

Cada fn de kernel nova ou assinatura mudada atualiza catálogo,
Lean, ledger e pisos **no mesmo commit** (regra `otimizar` /
RFC-0222 P0.7). Sem linha de catálogo = fatia falhou.

## Delivery slices (mandatory)

### P0 — selo honesto + o número que conta

- [x] **P0.1** Este RFC — status: `done`
- [x] **P0.2** `seal_async_first_drain` ganha `batch_len`; sela iff
      `!any_sync ∧ window_us==0 ∧ writers≤ncpu ∧ batch_len==1`. Twin
      AS-IS = política 2026-09-14 (ignora `batch_len`). Teste nomeado
      `seal_async_first_drain_boundary`. Catálogo + Lean
      `seal_async_first_drain_fate_iff` + ledger/pisos no mesmo
      commit. Wiring em `lead()` passa `batch.len()`. — status: `done`
- [x] **P0.3** Meter vs Rocks `sync=false`, célula
      `deps_cache_overwrite_mc4` (`SUITE=ycsb` `run_clients`,
      `name=deps_cache_overwrite_mc4` `clients=4` `n=8000`).
      **Darwin DIAG** (caixa Linux blocked): curva N=1,2,4,8,16
      `_mcN` + mc4 3-round intercalado. Journal
      `number: ratio=0.160 pedra_qps=41,823 rocks_qps=260,763
      shape=deps_cache_overwrite_mc4 DIAG`. Cartaz Linux 0.557×
      unpaid. mc16 cut 114.8k ≥ twin 103.0k (−13% recuou).
      `kvrocks_set_mc50` `clients=50` `n=10000`: 228.4k vs twin
      243.2k (−6.1%). G1 1c não reivindicado. Finding
      `findings/2026-09-15-rfc0226-scale-why/`. — status: `done`

### P1 — grupo-de-1 não entra em `lead()`; selo async pleno só com número

- [x] **P1.1** Kernel `solo_leader_bypass` (batch=1 ∧ fila vazia ∧
      `active≤1` ⇒ Bypass; resto Keep). Twin AS-IS = Keep. Env
      `PEDRA_SOLO_BYPASS=1` opt-in, default off. Teste
      `solo_leader_bypass_boundary` + catálogo/Lean
      `solo_leader_bypass_fate_iff`. Wiring: `lead()` devolve o
      único membro ao `commit_async_one`. — status: `done`
- [x] **P1.2** Meter P1.1 `_mcN`: mc2-solo 30.3k vs cut 27.3k
      (ainda < twin 34.8k, 1 round); mc4-solo 58.9k vs cut mediana
      41.8k (1 round, sem 3-round vs Rocks). **Sem flip de default.**
      Opt-in fica. — status: `done` (honest no-flip)
- [x] **P1.3** Dropar `writers≤ncpu` **iff** mc16 e mc50 ≥ twin.
      mc16 sim (`deps_cache_overwrite_mc16` 114.8k ≥ 103.0k);
      mc50 não (`kvrocks_set_mc50` clients=50, 228.4k vs 243.2k).
      Teto fica, **C** nomeado. — status: `done` (parked/C)

### P2 — estrutura WriteThread só se a política não fechar o gap

- [x] **P2.1** Lista self-link / pipelined write (0055 P1.3) **iff**
      ≥15% do déficit restante em Mutex+mpsc (PHASE). PHASE
      `deps_cache_overwrite_mc4` cut r2: `wal=23.32µs`
      `lock_wait=0.43µs` — o déficit é `write()`, não a Mutex.
      0055 parked. — status: `done` (parked)
- [x] **P2.2** Yield adaptativo tipo Rocks `max_yield_usec` **iff**
      o caminho **sync** ainda pagar join/collect. G1 não re-medido
      neste fire; parked. — status: `done` (parked)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Este RFC | done | — | 2026-09-15 |
| P0.2 | p0 | Selo só se batch==1 | done | — | 2026-09-15 |
| P0.3 | p0 | Meter Darwin DIAG `_mcN` (Linux unpaid) | done | — | 2026-09-15 |
| P1.1 | p1 | `solo_leader_bypass` opt-in | done | — | 2026-09-15 |
| P1.2 | p1 | Meter P1.1; sem flip | done | — | 2026-09-15 |
| P1.3 | p1 | Selo async sempre | done (parked/C) | — | 2026-09-15 |
| P2.1 | p2 | WriteThread self-link | done (parked) | — | 2026-09-15 |
| P2.2 | p2 | Yield adaptativo no sync | done (parked) | — | 2026-09-15 |

## Acceptance Criteria

- **Tests:** `seal_async_first_drain_boundary` cobre batch==1 / batch≥2
  / sync / window / oversub / ncpu=0. `solo_leader_bypass_boundary`
  cobre (1,0,1)⇒Bypass, (1,1,1)⇒Keep, (2,0,2)⇒Keep, AS-IS sempre Keep.
  Kernel `group_window` 16/16 não regride. Caller
  `concurrent_calls_group_window_kernel` menciona a fn nova.
- **Telemetry / Analytics:** nenhuma sonda default-on (RFC-0169).
  Meter usa `PEDRA_WRITE_PHASE_STATS=1` já existente (`cw`,
  `avg_group`, fases). `PEDRA_SEAL_ASYNC=0` e `PEDRA_SOLO_BYPASS=1`
  são pins de twin/opt-in, não telemetria.
- **Documentation:** finding 2026-09-14 ganha ponteiro a este RFC;
  `docs/status.md` linha; fatia `done` no mesmo commit do código.
  Skill `otimizar` grind: P0.2/P0.3 são o próximo fire.
- **Screenshots:** backend-only.

## Out of scope

- Win vs G1 1c / `sync=true` / Darwin como cartaz / Rocks colapsado.
- Eixo keyspace (104k→64k de 1k→2M records, Rocks sobe) — memtable
  versionada, working set > cache. Dono: [0223](0223-escala-donos-flush-write-miss-read.md)
  / cache, não a pipeline de grupo.
- `PEDRA_GROUP_WINDOW_US>0` como produto (perda 11× medida).
- Concurrent memtable insert (0055 P1.1 parked; 0190 já desparkou
  por outro eixo).
- Portar `db/write_thread.cc` sem o iff do P2.1.
- Mudar o default de durabilidade (G1 continua fdatasync antes do Ok).
