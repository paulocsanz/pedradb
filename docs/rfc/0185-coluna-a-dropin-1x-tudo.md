# RFC-0185 — Coluna A drop-in ≥1× em tudo (Linux 3/3)

**Status:** draft
**Updated:** 2026-09-08
**ID:** 0185
**Parents:** [0041](0041-2x-rocks-default.md) (floor 1× drop-in),
[0043](0043-high-level-2x-expanding-benches.md) (catálogo só cresce),
[0054](0054-close-official-gaps.md) (compat default async),
[0062](0062-launch-readiness-remaining-gaps.md) (S4 min>1.0 1c 17/17),
[0163](0163-anti-overfit-benchmark-breadth.md) (Grid B: overwrite_mc4 **0.557×**),
[0180](0180-overwrite-mc4-gt1x.md),
[0182](0182-same-boot-write-path-harnesses.md),
[0183](0183-teto-apply-serial-e-1c.md)
**Peer:** RocksDB default `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`).
G1 não é win. Sync-peer não é win. Fjall absoluto, nunca ratio.
Darwin = DIAG. Linux 3-run = cartaz.

> P0 é o screenshot que mata o anúncio: `overwrite_mc4` Linux.
> “Tudo” = conjunto **G_A** abaixo, **min de 3 rounds quietos > 1.0**,
> não mediana com named loss. Catálogo `COMPARE_SHAPES` é P2 e
> **não se apaga** para subir `min_ratio` (RFC-0043).

## Background

Coluna **A** = drop-in `rocksdb-compat` default: WAL `write()` por
commit, **sem** `fdatasync` no Ok (`PEDRA_PARITY_ASYNC=1`). Mesma
classe de crash-de-processo do Rocks default. O class-fix
`88e2a63` (2026-08-30) apagou o staging 64 KiB — a coluna A **é**
esse `write()` por commit; restaurar staging está fora.

O que já passou e o que um clone reproduz amanhã:

| recorte | número | vive? |
|---|---|---|
| YCSB + deps 1c Linux, staging | 17/17 min **1.014** (`deps_raftlog` r3), 2026-08-25 P04 | **não** — pré-class-fix |
| YCSB + deps 1c Linux, class-fix | 16/17 ≥1; `deps_raftlog` mediana **0.94** (0.968/0.942/**0.623**) | o piso 1c vivo |
| `ycsb_a_mc4` 25M Linux | **2.26×** 3/3 (0163 Grid B) | S |
| `ycsb_f_mc4` 25M Linux | mediana 1.47; **run2 0.766×** | W — “sempre” falha |
| `overwrite_mc4` 25M Linux | **0.557×** 3/3 | U rank 1 |
| `apply_mc4` G1 | **2.79×** Linux head3 | **outra coluna** |
| `apply_mc4` same-class | Darwin DIAG **0.48×**; sem Linux 3-run no floor 15/15 | U |
| Darwin overwrite_mc4 isolado | mediana 1.002 (1.002 / **0.816** / 1.032) | overfit; 0180 P2.1 ainda todo |
| `kvrocks_set_mc50` | **0.37×** Adaptive off n≥16 | **C** — documentar, não “ganhar” |

p50 do raftlog class-fix **empatou** (~11 µs). O 0.623 é stall de
cauda (max 1.52 ms), não motor 40% mais lento. Isolated empty-DB
já era ≥1× no P04. A suíte arranca raftlog com ~264 k entradas de
leftover (apply+seed). Cortes Darwin 2026-09-02 existem;
**remesura Linux 3-run do guest não rodou**.

0180 gastou P0.3–P0.38 em grouping/linger Darwin. `avg_group≈4`
**não** paga o 0.557 Linux. Caixa não re-correu depois do 0180.
Alavanca viva no mapa: leftover+L0 p50 vs Rocks quieto ≳260 k
(p50 ~11.5 µs), não mais um knob de grupo.

## Problems This Solves

- **Problem:** “17/17 ≥1×” cita P04_PASS, que mede uma coluna A
  que deixou de existir (staging).
- **Problem:** mediana 1.002 com 1/3 a 0.816 passou a “ganhámos
  overwrite”. Um clone Linux imprime 0.557×.
- **Problem:** `ycsb_f_mc4` mediana 1.47 esconde o round 0.766.
  “Sempre ≥1×” exige **min** dos 3, não mediana.
- **Problem:** `apply_mc4` G1 2.79× é vendido no lugar da coluna A.
- **Problem:** o catálogo RFC-0043 cresceu (dezenas de mc4 Darwin
  DIAG, vários ≪1×) sem Linux 3-run e sem um G_A congelado.

## Proposed Solution

1. Congelar **G_A** (22 shapes). Gate = Linux 3 rounds quietos,
   `min(r1,r2,r3) > 1.0` em **cada** uma, peer `sync: false`,
   Rocks overwrite_mc4 ≳260 kQPS (collapsed ≠ win).
2. Fechar primeiro a célula que o crítico corre: `overwrite_mc4`.
   Cortes = leftover+L0 / hold do `write()` / apply serial
   (0183). **Não** grouping, **não** skiplist sem despark 0183.
3. Depois: raftlog 1c pós-class-fix, `ycsb_f_mc4` 3/3, `apply_mc4`
   same-class (ou C nomeado pelo 0183).
4. Catálogo: cada `COMPARE_SHAPES` fora de G_A ganha Linux 3-run
   ≥1.0 **ou** C. Nunca delete. p50 melhor e QPS <0.1× é stall
   de harness/cauda (ex. `arango_traversal_mc4` 0.003) — conserta
   o stall, não “ganha” encolhendo o trabalho.

### G_A — must-win (anúncio)

União do prefixo oficial 16 (RFC-0041) com o gate Linux P04 (17
shapes 1c). 22 nomes, append-only neste RFC:

```
ycsb_a ycsb_b ycsb_c ycsb_d ycsb_e ycsb_f
deps_apply_batch deps_mvcc_latest deps_scan deps_raftlog
deps_cache_overwrite deps_lock_prewrite
ycsb_a_mc4 ycsb_f_mc4
deps_cache_overwrite_mc4 deps_apply_batch_mc4 deps_raftlog_mc4
kvrocks_get kvrocks_set kvrocks_scan kvrocks_pipelined_set
kvrocks_blob_set
```

Fora de G_A (P2 / C): `kvrocks_set_mc50`, prefix 100M @ 4 GiB,
`ycsb_b_mc4` e o resto de `COMPARE_SHAPES`. G1 1c write-per-op
é **outra coluna** — nem G_A nem C desta.

### Critério de round (não relitigar)

| conta | não conta |
|---|---|
| Linux 4 vCPU caixote AMD, 3 rounds, `rm` entre shapes | Darwin como cartaz |
| `min` dos 3 > 1.0 em **cada** G_A | mediana com named loss |
| `ROCKS_PARITY_SYNC=0`, JSON `sync: false` dos dois lados | peer `sync: true` |
| Rocks overwrite_mc4 quieto ≳260 k | Rocks collapsed (90–157 k) |
| WAL `write()` por commit (class-fix) | staging 64 KiB / skip `write()` |

Caixa bloqueada: Darwin vs Rocks da **mesma** célula é DIAG, e o
corte de produção continua. DIAG não fecha o slice.

## Delivery slices (mandatory)

### P0 — must ship first (o screenshot 0.557× some)

- [x] **P0.1** Este RFC + G_A + critério 3/3 min — status: `done`
- [x] **P0.2** `COLUMN_A_SHAPES` no harness = G_A; compare/entrypoint
      gateia **min de 3 rounds > 1.0** nesse conjunto; recusa peer
      sync e Rocks overwrite collapsed; teste nomeado.
      — status: `done`
- [ ] **P0.3** `deps_cache_overwrite_mc4` Linux 3/3 **min > 1.0**
      quieto vs Rocks default (≳260 k). Filho de [0180](0180-overwrite-mc4-gt1x.md)
      P1.1 com o critério deste RFC (não mediana 1.002). Cortes:
      leftover+L0 / lock_hold / apply; **não** grouping 0180.
      Corte leftover+L0 in-tree (`idx_prefix` one-slash, park foreign,
      skip L0 materialize while `recently_multi`). WAL `write()` off
      the Db write lock on the mc4 bypass **reverted**: quiet DIAG
      0.243× p50 48 vs 12 µs (reacquire tax). After revert, isolated
      Darwin DIAG 0.291× vs Rocks quiet 303 k (p50 9.5 vs 11.2 µs;
      p99 577 vs 39 µs). This-fire DIAG 0.364× vs Rocks 219 k (above
      collapsed, below ≳260 k; p50 9.9 vs 15.1; p99 504 vs 69; max 7.4 vs
      0.19 ms). Linux source-upload p185-3/p185-4:
      `all containers failed before first start`.
      — status: `todo`

### P1 — next wave (o resto de G_A que ainda mata “sempre”)

- [ ] **P1.1** Linux 3-run HEAD das 17 shapes 1c (class-fix).
      Scoreboard vivo; substitui P04_PASS como citação.
      — status: `todo`
- [ ] **P1.2** `deps_raftlog` 1c Linux 3/3 min > 1.0 no caminho
      class-fix (stall/cauda/leftover da suíte; p50 já empatou).
      Skip se P1.1 já passar. **Não** 2×. **Não** reverter class-fix.
      — status: `todo`
- [ ] **P1.3** `ycsb_f_mc4` Linux 3/3 min > 1.0 (mata o 0.766).
      — status: `todo`
- [ ] **P1.4** `deps_apply_batch_mc4` coluna A Linux 3/3 min > 1.0
      **ou** C nomeado por [0183](0183-teto-apply-serial-e-1c.md)
      P0.3 (mem-apply ≥15% ⇒ despark 0055 P1.1; senão teto serial).
      G1 2.79× **não** fecha este slice.
      — status: `todo`
- [ ] **P1.5** `deps_raftlog_mc4` Linux 3/3 min > 1.0 ou C.
      — status: `todo`

### P2 — later (catálogo + tetos + escala que o crítico corre)

- [ ] **P2.1** Registo in-tree de C da coluna A: hoje
      `kvrocks_set_mc50` Adaptive-off n≥16. Cada C leva mecanismo
      + número + “não é win”.
      — status: `todo`
- [ ] **P2.2** Cada `COMPARE_SHAPES` fora de G_A: Linux 3-run ≥1.0
      ou C. Famílias Darwin ≪1× (traversal 0.003, `ycsb_b_mc4` 0.060,
      wbwi 0.41, point-select 0.43, rockstore 0.566, flink 0.522)
      primeiro. Nunca apagar shape. Stall de harness (p50 ok, wall
      300×) conserta-se no runner, não no motor.
      — status: `todo`
- [ ] **P2.3** prefix 100M @ 4 GiB ≥1.0 vs Rocks default **ou** C
      bounded-cache (hoje 0.70× caixa; guest grande 1.05×).
      Harness scale, não YCSB 1024.
      — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC + G_A + 3/3 min | done | este ficheiro | 2026-09-08 |
| P0.2 | p0 | COLUMN_A_SHAPES + gate 3-run min | done | `column_a.rs` + `rocks-parity-column-a` | 2026-09-08 |
| P0.3 | p0 | overwrite_mc4 Linux 3/3 min>1.0 | todo | leftover+L0 landed; Linux bake fail | 2026-09-08 |
| P1.1 | p1 | 17×1c Linux HEAD class-fix | todo | — | 2026-09-08 |
| P1.2 | p1 | raftlog 1c 3/3 min>1.0 class-fix | todo | — | 2026-09-08 |
| P1.3 | p1 | ycsb_f_mc4 Linux 3/3 min>1.0 | todo | 0182 P1.2 | 2026-09-08 |
| P1.4 | p1 | apply_mc4 coluna A 3/3 ou C 0183 | todo | 0183 P0.3 | 2026-09-08 |
| P1.5 | p1 | raftlog_mc4 Linux 3/3 ou C | todo | — | 2026-09-08 |
| P2.1 | p2 | registo C coluna A | todo | mc50 | 2026-09-08 |
| P2.2 | p2 | COMPARE fora de G_A Linux 3-run | todo | 0043 | 2026-09-08 |
| P2.3 | p2 | prefix 100M 4 GiB ≥1.0 ou C | todo | 0178 P1.2 | 2026-09-08 |

## Acceptance Criteria

- **Tests**
  - P0.2: const G_A = 22 nomes; teste `rfc0185_column_a_shapes_are_the_gate`;
    compare com um round G_A <1.0 exit 2; JSON peer `sync: true` exit 2
    (já existe); Rocks overwrite collapsed marca anomalia.
  - P0.3: finding Linux 3-run, `min_ratio > 1.0`, os 3 rounds ≥1.0,
    `sync: false`/`false`, Rocks qps ≳260 k. Teste de produção do corte
    que fechar (não `group_profile`).
  - P1.1: `compare_report.json` 17 shapes, 3 rounds, finding substitui
    a citação P04_PASS neste RFC.
  - P1.4: ou 3/3 >1.0 coluna A, ou frase C na tabela P2.1 com o número
    do 0183 (mem-apply fracção).
- **Telemetry / Analytics**
  - Nenhuma tabela deste RFC lidera com G1 vs async nem com Darwin.
  - `compat_over_rocksdb` por shape; `min` dos 3 rounds no finding.
- **Documentation**
  - este RFC; linha em `docs/status.md`; 0062 nota que o floor 1c 17/17
    é histórico e o cartaz “sempre ≥1×” vive aqui.
- **Screenshots**
  - backend-only.

## Out of scope

- Coluna B (ambos `sync=true`) e coluna C (G1 vs Rocks async).
- 2× em raftlog 1c (p50 empatado).
- Reverter o class-fix / repor staging 64 KiB.
- Grouping/linger 0180 como alavanca de P0.3.
- Skiplist concorrente sem 0183 P0.3 despark.
- Merge Adaptive n≥16 (`kvrocks_set_mc50`).
- Apagar shape de `COMPARE_SHAPES` para subir `min_ratio`.
- Darwin como cartaz. Peer `sync=true` como vitória. Fjall como ratio.
- `crates.io` (0062 P2.4). Layout SST C++. Intel.
- WARM 100M em 4 GiB. `PEDRA_BULK_CHUNK_BYTES=4MB`.
