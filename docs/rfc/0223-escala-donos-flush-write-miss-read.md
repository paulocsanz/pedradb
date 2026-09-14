# RFC-0223: escala — dívida de flush no write-path, miss-path no read

**Status:** draft
**Updated:** 2026-09-13

## Background

- @10M (DIAG Darwin quiet, PHASE, binário `0eb0f25e`): perna write
  10.629.512 commits — **wal 4,74µs/commit (50,7%)**, **flush_check
  2,69µs (28,8%)**, mem 1,55µs; `l0_files=773` na janela medida, stall
  408ms (finding `2026-09-13-rfc0217-p26-p27-escala`).
- `deps_scan` @10M p50 0,2107ms com **setup 231,21µs/op (97% do custo)**:
  54 L0 files vivos do seed (flush por CF a 256MiB diferido), merge k-way
  4,75µs (barato), 3,4 blocks decodificados/op com **76% de cache miss**.
- Fixes landed e **ainda sem re-meter**:
  - settle pós-seed default (`bf40c5e8`) — drain bounded de L0 após seed;
  - simetria de memtable 256MiB no lado Rocks (`bf40c5e8`) —
    `kvrocks_set_mc50 1,678×` (p201q, Linux 3-run min-of-3) é
    **config-suspeito** até re-run;
  - split flush gate × flush-work no PHASE (`7f2758d4`) —
    `flush_events`/`flush_work_ns` ainda sem número;
  - bloom real em bulk SSTs (RFC-0160 P1.6) — **`probe_miss 0,29×**
    (2,3–2,4µs vs 651–692ns, RFC-0161 100M 3-run) nunca re-medido
    com o fix.
- Gate `linux-gate-p211z` pending (região brasil, host low-disk; 6 gates
  antigos deletados 2026-09-13 liberando ~43GiB declarados). A imagem já
  embute a onda e4b (P0.4/P0.5 3-run) e boota sozinha quando o host
  aceitar o deploy.
- fjall (única campanha oficial
  `findings/2026-09-04-fjall-official-guest`): miss-path ~2× pior que
  fjall (2,3–2,4µs vs 1,1–1,2µs); @100M hydrate 123,7s vs 198,8s
  (Pedra ganha).

## Problems This Solves

- **Problem:** o write-path em escala gasta 28,8% em `flush_check` sem
  decomposição — o ataque certo (epoch no gate × mover flush para fora do
  commit) depende de saber se a média 2,69µs é gate caro por commit ou
  flushes raros diluídos.
- **Problem:** o read-path perde na miss-path (0,29× vs Rocks; ~2× vs
  fjall) e o fix já está in-tree há semanas sem número novo.
- **Problem:** dois números publicados foram medidos sob harness
  assimétrico/pré-settle (mc50 com Rocks a 64MiB; scans @10M com dívida
  de seed) — re-meter antes de qualquer claim novo.

## Proposed Solution

- P0: completar os re-meters (pipeline local quiet: settle A/B + mc50
  simetria A/B + split gate×work) e o e4b no gate; flipar findings/RFC
  com números no mesmo commit.
- P1: atacar o dono que o split nomear no `flush_check`; re-meter
  oficial do `probe_miss` com bloom real.
- P2: residual do `deps_scan` pós-settle (settle eager do
  `SstCountCursor` se continuar dono) e campanha miss-path vs fjall.

## Delivery slices (mandatory)

### P0 — must ship first (meters com os fixes landed)

- [x] **P0.1** re-meter local quiet: `deps_scan` @10M settle ON × OFF
  (A/B `ROCKS_PARITY_SETTLE`) — valida o fix P2.6 (baseline OFF:
  p50 0,2107ms) — status: `done (DIAG mecânico)` — pipeline `p26r3`
  rodou SEM quiet (v2 cancelado esperando; load externo ~13): p50s de
  janelas 40–70ms incomparáveis sob load, mas counters fecham — settle
  funciona (memtable 6,27M→0, L0 54→14) e **não completa** sob load
  (deadline 30s, log "INCOMPLETE after 37.1s"); **dono do setup
  sobrevive**: tables/op 3,6 vs 4,2 e blocks 726 vs 762 entre braços —
  largura de table, não nível. P50 oficial = gate. Finding
  `2026-09-13-rfc0217-p26-p27-escala` rev.3.
- [x] **P0.2** re-meter local quiet: `kvrocks_set_mc50` 3 rounds
  simétrico 256MiB × 3 rounds shape antigo (Rocks 64MiB via
  `ROCKS_PARITY_ROCKS_MEMTABLE`) — decide se 1,678 era artefato de
  config — status: `done (DIAG: NÃO é artefato)` — 1ª passada
  inconclusiva (load espetou 5×); rerun 3-arm intercalado ×7
  (`p26r3b-mc50x`, ranges ±10%): rocks256 ≈ rocks64 (max 180,9k vs
  180,0k; med 170,2k vs 165,1k) e compat 215,6k med ⇒ ratio sym
  1,267–1,370 × ratio asym 1,306–1,376 — **mesma vitória nas duas
  configs**; suspeita rebaixada, número oficial = gate (P0.4)
- [x] **P0.3** decompor `flush_check` com o split `7f2758d4`
  (`flush_events`/`flush_work_ns`): nomear gate-only mean × work mean
  @10M — status: `done (DIAG)` — seed 625k commits/8 flushes:
  **work 24.704,6ms = 99,85% × gate 36,3ms = 58ns/commit**. O
  2,69µs/commit era flush work raro diluído.
- [ ] **P0.4** e4b no gate 3-run (P0.4 janela-≤-voo, P0.5 PHASE,
  P1.4 linkbench, P2.2/P2.3 cartaz, mc50 oficial) — status: `todo`
  (blocked: `linux-gate-p211z` pending — sem capacidade em `brasil`)

### P1 — next wave (ataque condicionado ao P0)

- [x] **P1.1** ataque ao dono do `flush_check` — DECIDIDO pelo split
  P0.3: é **work** (99,85%; gate 58ns/commit não paga otimização).
  Diagnóstico rev.2: I/O de SST já é off-commit; custo in-commit =
  `take_family` O(n) sob write-lock. **Código landed**: kernel
  `dominant_family_stage_plan` (fam ≥ 3/4 do total ⇒ `StageWholeMem`
  O(1) via `stage_flush_imm`; abaixo ⇒ `PartitionFamily`; AS-IS
  sempre partição; imm ocupado cai na partição). Integração no ramo
  defer de `maybe_auto_flush`; testes
  `dominant_family_stage_plan_on_live_dominant_stages` +
  `rfc0223_dominant_family_stages_whole_mem`. Re-meter p223: stage
  O(1) **não** colapsou `flush_work` (8×2,76s) — imm ocupado caía no
  `take_family`; **rev.3** estaciona a memtable inteira na fila parked
  quando imm está cheio. Cartaz = e4b. — status: `done (código, rev.3)`
- [ ] **P1.2** `probe_miss` re-meter oficial no gate com bloom real
  (RFC-0160 P1.6 in-tree); se <1,0 persistir, fatia de tuning de bloom
  datada no mesmo commit do finding — status: `todo` (blocked: gate).
  DIAG 100k `qs_neg_lookup` ×3 intercalado (família miss): min 1,988
  med 2,055 vs Rocks `SYNC=0` (p50 0,3 vs 0,5 µs) — **não** paga o
  0,29× @100M.

### P2 — later / polish

- [x] **P2.1** `deps_scan` residual pós-settle: lazy-first-block
  **refutado** (k-way min-head precisa de todos os heads; tables de
  keyspace inteiro sobrepõem a janela de 25 keys — adiar o `settle()`
  do cursor só move o custo de setup→merge). Dono restante = settle
  que só dormia no worker (p26r3: 30 s de poll, L0=14 INCOMPLETE).
  **Código landed:** `DB::compact_l0_once` + settle do bench faz o
  drain L0→L1 (equivalente Pedra do `wait_for_compact`); teste
  `compact_l0_once_drains_below_trigger`. Re-meter p223 DIAG:
  settle **COMPLETE** 37,1s L0=0 (era INCOMPLETE L0=14); p50 0,149ms;
  tables/op 3,6→2,0; setup ainda 97,9%. Cartaz = e4b. — status:
  `done (código + DIAG)`
- [ ] **P2.2** campanha miss-path vs fjall (probe_miss par) na régua
  oficial de guest — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | deps_scan @10M settle A/B (pipeline local) | done (DIAG mecânico) | `p26r3` rev.3 | 2026-09-13 |
| P0.2 | p0 | mc50 simetria A/B 256×64 (pipeline local) | done (DIAG: não é artefato) | `p26r3b-mc50x` ×7 intercalado | 2026-09-13 |
| P0.3 | p0 | split flush gate×work @10M | done (DIAG) | `7f2758d4` + `p26r3`: work 99,85%, gate 58ns/commit | 2026-09-13 |
| P0.4 | p0 | e4b gate 3-run | todo | blocked p211z | 2026-09-13 |
| P1.1 | p1 | dominant family O(1) stage (`take_family` fora do commit) | done (código, rev.3) | p223: imm ocupado ainda pagava take_family 8×2,76s; agora park whole mem | 2026-09-14 |
| P1.2 | p1 | probe_miss re-meter oficial (bloom real) | todo | blocked gate | 2026-09-13 |
| P2.1 | p2 | settle drena L0 (compact_l0_once; lazy-cursor refutado) | done (código + DIAG) | p223: COMPLETE 37,1s L0=0 p50 0,149ms tables/op 2,0; setup 97,9% | 2026-09-14 |
| P2.2 | p2 | miss-path vs fjall oficial | todo | — | 2026-09-13 |

## Acceptance Criteria

- **Tests:** cada ataque com teste unitário próprio; suites
  `pedradb-core`/`rocksdb-compat`/`rocksdb-parity-bench` sem novos
  vermelhos (baseline atual: compat 96/3 pré-existentes).
- **Telemetry:** PHASE (`flush_events`/`flush_work_ns`) e `read_probe`
  em todo meter; findings com probe JSON anexado.
- **Documentation:** finding datado por fatia no mesmo commit
  (`findings/2026-09-*-…`); tabela U-cells e `docs/status.md`
  atualizadas com o número novo — célula <1,0 vira fatia datada.
- **Screenshots:** none — backend-only.

## Out of scope

- parked-debt plan e interface do worker de compaction (RFC-0216, outra
  frente): aqui só consumimos a interface, não redesenhamos.
- group window / janela-≤-voo (já coberto por P0.3b/P0.4 do RFC-0217;
  veredito = e4b).
- Darwin = DIAG, nunca cartaz; nenhum número deste RFC é claim sem o
  3-run Linux quiet.
