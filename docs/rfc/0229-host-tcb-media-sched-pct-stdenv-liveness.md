# RFC: 0229 — Host TCB: mídia, `∀π`, PCT, StdEnv, liveness — o que Pedra paga, o que é experimento, o que nunca fecha

**Status:** done
**Updated:** 2026-09-16
**ID:** 0229
**Parents:** [0069](0069-liveness-claim-es-axioms-fail-closed.md),
[0070](0070-pct-depth-not-forall-schedules.md),
[0078](0078-fsync-ok-not-media-proof.md),
[0079](0079-tcg-guest-claim-fail-closed.md),
[0053](0053-ironfleet-years.md) (Y3.3 bounded liveness sob quórum-vivo),
[0056](0056-one-hundred-percent-delivery.md) (StdEnv injectável),
[0187](0187-teorema-experimento-tcb.md) (circuito; P2.2 TCG e P2.3 N=4 ainda `todo`),
[0220](0220-escada-de-composicao-encadeia-os-atomos-dual-unfold.md) (P2.3 PCT planta),
[0227](0227-prova-de-produto-put-get-recover-replica.md) (produto D1/R1/T1/C1; estas cinco ficaram TCB)
**Peer:** inalterado — RocksDB default `sync=false`. Este RFC não toca parity.

> Flipar `media_durable_admitted`, `lock_interleavings_admitted` ou
> `forall_schedules_admitted` para `true` **não** resolve nada: prova um
> enunciado falso. Liveness sem quórum-vivo / ES-1/2/3 **é** falsa
> (Stateright já refuta). O caminho honesto é o mesmo do IronRSL e do
> CrashMonkey: pagar o lado Pedra como teorema, aprofundar o experimento
> onde o ∀ do host é impossível, e deixar o TCB escrito.

**Frase permitida no fim deste RFC** (relativa ao TCB, nunca absoluta):
*Pedra não arredonda `fdatasync` Ok a mídia, PCT a `∀π`, StdEnv a spec
de FS, nem `elect_all` a liveness. O lado Pedra (Env honesto, alfabeto
de locks, grant do harness, ES nomeados) está no prover. O host
continua fora.*

**Frases recusadas:** “o fsync está provado”, “PCT cobre todos os
schedules”, “StdEnv é o disco”, “eleição é live”, “somos seL4”.

## Background

RFC-0227 fechou D1/R1/T1/C1 sobre o put/get/recover/replica-read que o
rustc liga. Cinco buracos do **host** ficaram de fora, já donos de RFCs
de honestidade (kernels que devolvem `false`):

| buraco | kernel vivo | residual | o que o AS-IS arredonda |
|---|---|---|---|
| `fdatasync`/mídia | `media_durable_admitted(_)` = `false` | `R-fsync-lie` / `R-tcg-guest` | `fsync_ok` ⇒ mídia |
| host `∀π` | `lock_interleavings_admitted()` = `false` | `R-group-glue` | publish gate verde ⇒ ∀ futex |
| PCT harness | `forall_schedules_admitted(_)` = `false` | `R-pct` | `pct_depth ≥ 2` ⇒ ∀ schedules |
| StdEnv | `env.rs` = POSIX real | ledger TCB | campanha Disk = prova de VFS |
| liveness sem quórum-vivo | `liveness_admitted(es1,es2,es3) = es1∧es2∧es3` | `R-es` | `elect_all` Ok ⇒ live |

Já pago no lado Pedra (não reabrir):

- D1 sob Env honesto + `crash_legal` (0227); `fsync_promotes_pending = os_honest` (0078); `fdatasync_rc ≠ 0` não é Ok (0073); `env_no_invented` / `env_honest_sync` (0214).
- Alfabeto de lock-client N=2, lock-order×rotate, ciclo 2PL, N-way OCC (0227).
- PCT d=2 default, d=3/d=4 campanha, 15 seeds pinadas, coverage floor 15/15, exaustivo N≤3 = 66 (0187 P0).
- Zero `Db<StdEnv>` hardcoded em API de biblioteca (0056 P2.3); `world_stdenv_diff_replay` Mem↔Disk (0157).
- Stateright: eventualidade **refutada** sem ES-1/2/3; bounded liveness sob axioma quórum-vivo (0053 Y3.3).

IronRSL (Hawblitzel et al., SOSP'15): liveness só *if the network is
eventually synchronous for a live quorum*. Sem esse axioma, nenhum
consenso é live (FLP). CrashMonkey+ACE (Mohan et al., FAST'19): B³
encontra bugs de crash-consistency até em FS **verificado** (FSCQ) —
prova do modelo ≠ mídia. CM-IOCov (SYSTOR'25) alarga inputs; continua
experimento, não teorema de drive.

## Problems This Solves

- **Problem:** “como fechar o TCB” é lido como “flipar o `false`”.
- **Problem:** TCG / `F_FULLFSYNC` / N=4 já são fatias 0187 P2 e
  continuam `todo`; PCT planta 0220 P2.3 continua `todo`; ninguém
  nomeia o buraco DST-dentro-das-threads (`parking_lot` park fora do
  harness).
- **Problem:** liveness sem ES é **falsa** — “resolver” seria
  mentir.

## Proposed Solution

Cada buraco decompõe em **três camadas**. Este RFC paga só a camada
Pedra (teorema) e agenda a camada experimento. A camada host **não
muda de classe**.

```text
teorema Pedra     experimento (campanha)     TCB host (never)
────────────────  ─────────────────────────  ────────────────────────────
Env honesto,      TCG power-cut,             contrato POSIX fdatasync,
crash_legal,      F_FULLFSYNC,               firmware, drive que mente
D1 0227           CrashMonkey no rasto POSIX
────────────────  ─────────────────────────  ────────────────────────────
alfabeto lock     PCT d↑, N=4, TSan          Linux futex / scheduler
N=2…N, grant      0220 P2.3 planta
token no park
────────────────  ─────────────────────────  ────────────────────────────
Mem↔Disk replay,  FailingEnv<StdEnv>         StdEnv = VFS do host
allowlist StdEnv  crash-injection 0187 P0.3
────────────────  ─────────────────────────  ────────────────────────────
liveness = ES1∧   TCP real / swarm L28       fairness da rede;
ES2∧ES3; bounded  (campanha)                 “maioria viva para sempre”
sob quórum-vivo
```

**O análogo FDB/seL4 que realmente reduz `∀π`:** o Flow do FDB *é* o
scheduler. Pedra ainda estaciona em `parking_lot`/`Condvar` (OS
threads). Pagar um **grant token** — cada park/unpark do write-group
passa pelo World — torna o interleaving das *threads Pedra* enumerável
sem provar o Linux. `lock_interleavings_admitted` continua `false`
(futex). `forall_schedules_admitted` continua `false` (threads fora do
token).

**O análogo IronFleet da liveness:** não provar sem axioma. Provar
`ES-1 ∧ ES-2 ∧ ES-3 → eventual election` no extract (hoje é
Stateright). Sem os três, o teorema é `¬live` — já medido.

**O análogo CrashMonkey da mídia:** não flipar `media_durable_admitted`.
Correr B³ sobre o rasto POSIX das barreiras Pedra (experimento; dono
0187 P2.2). Achar um bug de ext4/APFS é bug do FS, não prova de Pedra.

## Delivery slices (mandatory)

### P0 — must ship first (honestidade congelada + mapa do grant)

- [x] **P0.1** Freeze das admissions: checker `--selftest` recusa
  corpo `media_durable_admitted` / `lock_interleavings_admitted` /
  `forall_schedules_admitted` ≠ `false`, e recusa
  `liveness_admitted` ≠ `es1 && es2 && es3`. Mutante in-memory (padrão
  product-floor). Job no verification-gates se outros `check_*.py`
  estão lá. — status: `done`
  (`scripts/check_host_tcb.py` + job `host-tcb`; `--selftest` 8/8)
- [x] **P0.2** Inventário dos park/mutex de `ConcurrentDb` que **não**
  passam por grant do World (write-group `Condvar`, flush-debt,
  L0 stall, `RwLock` write). Teste nomeado: a lista publicada == grep
  vivo; sítio novo sem linha = vermelho. — status: `done`
  (`scripts/ratchet/concurrent_park_sites.tsv` sleep=3 wait_for=2 recv=1;
  `rfc0229_os_park_sites_inventoried`)
- [x] **P0.3** RFC-0220 P2.3 absorvido: planta PCT d=2 que o d=3 acha
  (cadeia-3 já medida ~1e-3/seed) **nomeada como PLANTA**, não teorema.
  `forall_schedules_admitted` intocado. — status: `done`
  (`pct_chain3_row_is_plant` + TSV `pct_plants.tsv` kind=plant;
  `planted_chain3_found_by_pct_d3`;
  `rfc0229_pct_chain3_row_is_plant_not_theorem`)

### P1 — next wave (lado Pedra no prover)

- [x] **P1.1** Grant token no primeiro park (write-group wait): o
  harness decide o wake; AS-IS = `std` park. Teorema: o token lineariza
  com o alfabeto de lock-client. Não extrai `concurrent.rs`. — status:
  `done` (`write_group_wait_grant` + `granted_block`;
  `rfc0229_write_group_wait_grant_vs_as_is`;
  `write_group_wait_grant_linearizes_unfolds_alphabet`)
- [x] **P1.2** Alfabeto de lock-client N=3 (acquire-write / flush /
  submit / publish + um passo extra já no handler). `lock_alphabet_*`
  cresce; `lock_interleavings_admitted` continua `false`. — status:
  `done` (`lock_alphabet_linearizes_n3`; put matches write-submit-publish)
- [x] **P1.3** Lean: `liveness_admitted true true true` dual-unfold do
  kernel + corolário do modelo `tcp_node_model` *sob* ES-1/2/3
  (enunciado com as três hipoteses; sem elas o goal é `false`).
  Stateright continua o dente de refutação. — status: `done`
  (`liveness_admitted_under_es123`;
  `tcp_node_eventual_election_under_es`;
  `tcp_node_eventual_election_refused_without_es`)
- [x] **P1.4** Allowlist StdEnv: grep de `StdEnv` fora de bins / tests /
  default de type-param / wrappers Env = o TSV pinado. Ilha nova =
  vermelho. — status: `done` (`check_stdenv_allowlist.py` 2/2)

### P2 — later (experimento; donos já existentes não se duplicam)

- [x] **P2.1** Nightly TCG power-cut por barreira + `F_FULLFSYNC`
  Darwin — **absorve 0187 P2.2**, hunt não gate.
  `media_durable_admitted` continua `false`. — status: `done`
  (`scripts/rfc0229_tcg_fullfsync_hunt.sh`; world-nightly; residual_no_guest)
- [x] **P2.2** Replay CrashMonkey-class do rasto POSIX das barreiras
  Pedra (B³; CM-IOCov se o runner aguentar). Experimento de FS, não
  de Pedra. — status: `done`
  (`scripts/rfc0229_crashmonkey_barrier_replay.sh`; residual_no_crashmonkey)
- [x] **P2.3** Exaustivo N=4 com poda — **absorve 0187 P2.3**. Continua
  ∀ no bound, não `∀π` do host. — status: `done`
  (`run_exhaustive_pruned`; 47 nodes / 16 leaves; `forall_schedules_admitted(4)` false)
- [x] **P2.4** Grant token nos parks restantes do inventário P0.2
  (flush-debt, L0 stall). Residual nomeado se Charon recusar o glue.
  — status: `done` (`granted_sleep` flush_debt / l0_stall / fence_drain)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | freeze admissions + liveness AND | done | `check_host_tcb.py` | 2026-09-16 |
| P0.2 | p0 | inventário park sem grant | done | `concurrent_park_sites.tsv` | 2026-09-16 |
| P0.3 | p0 | PCT d=2 planta (absorve 0220 P2.3) | done | `pct_chain3_row_is_plant` | 2026-09-16 |
| P1.1 | p1 | grant token write-group | done | `write_group_wait_grant` | 2026-09-16 |
| P1.2 | p1 | lock alphabet N=3 | done | `lock_alphabet_linearizes_n3` | 2026-09-16 |
| P1.3 | p1 | Lean liveness sob ES-1/2/3 | done | Membership.lean | 2026-09-16 |
| P1.4 | p1 | allowlist StdEnv | done | `check_stdenv_allowlist.py` | 2026-09-16 |
| P2.1 | p2 | TCG + F_FULLFSYNC (0187 P2.2) | done | `rfc0229_tcg_fullfsync_hunt.sh` | 2026-09-16 |
| P2.2 | p2 | CrashMonkey rasto POSIX | done | `rfc0229_crashmonkey_barrier_replay.sh` | 2026-09-16 |
| P2.3 | p2 | exaustivo N=4 (0187 P2.3) | done | `run_exhaustive_pruned` | 2026-09-16 |
| P2.4 | p2 | grant tokens restantes | done | `granted_sleep` | 2026-09-16 |

## Acceptance Criteria

- **Tests**
  - P0.1: `--selftest` apanha media/lock/forall body `true` e
    liveness sem AND; live GREEN com os corpos actuais.
  - P0.2: `concurrent_park_sites.tsv` exact-count; sítio `sleep`/
    `wait_for`/`recv` novo = vermelho; `rfc0229_os_park_sites_inventoried`.
  - P0.3: planta d=3 encontrada, d=2 miss; `forall_schedules_admitted(3)`
    continua `false`; kind TSV = `plant` (nunca theorem).
  - P1.1: cargo no production wait; Lean unfold do token × alfabeto.
  - P1.3: lake do módulo; `liveness_admitted false _ _ = ok false`;
    com os três `true`, o modelo admite eventualidade no bound.
  - P1.4: ilha `StdEnv` nova fora da TSV → vermelho.
  - P2.*: nightly / hunt; nunca gate de PR; nunca flip de admission.
- **Telemetry / Analytics** — none — sinal binário CI; campanhas
  nightly.
- **Documentation** — este RFC; linha em `docs/status.md`; finding
  `findings/2026-09-15-host-tcb-five/`; 0187 P2.2/P2.3 e 0220 P2.3
  apontam para cá quando absorvidos.
- **Screenshots** — none (backend/CI-only).

## Out of scope

- Flipar `media_durable_admitted` / `lock_interleavings_admitted` /
  `forall_schedules_admitted`.
- Provar Linux, rustc, firmware, ECC, `parking_lot`, io_uring ring.
- Dump de `db.rs` / `concurrent.rs`.
- Liveness **sem** ES-1/2/3 / quórum-vivo (enunciado falso).
- Montanha FDB recipes. Perf / Rocks parity. Re-pin Aeneas/Charon
  sem widen-sem-`sorry`.
- “Somos seL4” / “sem bugs” / “o fsync está provado”.
