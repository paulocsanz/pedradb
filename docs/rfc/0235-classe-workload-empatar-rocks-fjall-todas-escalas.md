# RFC-0235 — Classe de workload no rustc: uma física por padrão, empatar Rocks e Fjall em todas as escalas

**Status:** in-progress (P0–P1 done; P2 parked per iff; Linux cartaz unpaid named)
**Updated:** 2026-09-17
**ID:** 0235
**Parents:** [0233](0233-ganhar-sempre-rocks-fjall-default.md)
(WAL de produto; Linux `overwrite_mc4` 0.557× **unpaid**; Fjall YCSB-A DIAG 1.27×),
[0234](0234-ganhar-10m-scan-write-l0.md)
(L0-at-trigger + drain L2→L3; @10M DIAG fechado, Linux unpaid named),
[0230](0230-programa-desempenho-alem-rocks-fjall.md)
(programa: intercepto WAL, depois física LSM)
**Papers (fichas D4, PDF lido nesta casa):**
[R006 Monkey](../../research/fichamentos/ficha_R006_Dayan_Monkey.md),
[R007 Dostoevsky](../../research/fichamentos/ficha_R007_Dayan_Dostoevsky.md),
[R010 Rocks Experience](../../research/fichamentos/ficha_R010_Dong_RocksExperience.md),
[R012 LSM compaction](../../research/fichamentos/ficha_R012_Sarkar_LSMCompaction.md),
[R014 Endure](../../research/fichamentos/ficha_R014_Huynh_Endure.md),
[R016 ADOC](../../research/fichamentos/ficha_R016_Yu_ADOC.md),
[R023 Disco](../../research/fichamentos/ficha_R023_Zhong_Disco.md)
**Fronteira (não D4 — abstract/HTML, finding):**
RusKey SIGMOD’24 (R026 listed), EcoTune SIGMOD’25 (R027 listed),
ArceKV arXiv:2508.03565. [finding](../../findings/2026-09-17-rfc0235-classe-workload/README.md).
**Peer Rocks:** `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`).
Darwin = DIAG. Linux 3-run min-of-3 = cartaz.
**Peer Fjall:** QPS **absoluto**. Nunca `compat_over_rocksdb`.

> **Tese:** não existe uma LSM que ganha em todos os mixes
> (Dostoevsky Fig. 10B; Endure Fig. 1: **2×** I/O quando o
> range/point muda; R012: auto-switch de 10 strategies **não**).
> Endure: mudar a *forma* da árvore em runtime “is not feasible”.
> A fronteira 2024–26 (RusKey, EcoTune, ArceKV) redescobre o mesmo
> facto: classificar \(w=(z_0,z_1,q,W)\) e escolher **uma** acção;
> o hard problem é o *custo da transição*, não o menu.
>
> O próximo salto da Pedra **não** é mais um pin de WAL, nem RL,
> nem FASTER no P0. É um **kernel de classe** alimentado pelos
> probes que já correm, que dispara **uma** física D4 por classe —
> e as físicas que o peer já tem e nós não: Monkey no miss de
> verdade, pin/hash de índice (Fjall 3), SuperVersion, overflow
> ADOC. O write × N continua dono do 0233 (Linux 0.557×). Hybrid
> log (FASTER R038) só se esse cartaz continuar <1 **depois** do
> WAL na classe Rocks.

## Background

### O que já está pago (não reabrir)

- Smoke 1c same-class Linux **15/15 ≥ 1.254**. Reads 1c 2–3×.
- `kvrocks_set_mc50` 1.678× (C Adaptive-off n≥16 documentado).
- 0234 Darwin DIAG @10M: ycsb_e 1.414, overwrite 1.166, raftlog
  1.394, mvcc 1.169, ycsb_a 1.733, deps_scan 3.074 probed/op=1.0;
  fresh `ONLY=ycsb_e` **98621** qps ≥98k. Linux unpaid named.
- 0233: WAL produção sem `O_APPEND`; Fjall YCSB-A DIAG 1.27×;
  ycsb_f_mc4 DIAG 1.022; Monkey `bits_per_key_for_run` **shipped**,
  `probe_miss` 100M **meter unpaid**.
- Bulk sequential → `MAX_LSM_LEVEL` (0159). vlog ≥4 KiB (R005).
- WriteThread parked: `lock_wait` ~2% do gap (0226).

### Scoreboard que este RFC ataca

| célula | número | classe | o que o peer faz e nós não |
|---|---|---|---|
| `probe_miss` 100M | **0.29×** Rocks unpaid; 64k miss DIAG **2.749×** (not 100M) | point-miss \(O(L)\) | Pin+hash shipped; 100M residual named PHASE `bloom_decode=0`. Linux unpaid. |
| Fjall YCSB-C 64k–256k | DIAG **1.149× / 1.057×** absoluto (ops=20000) | point-hit cache | SuperVersion (lock count < 3); pin L0/L1. Linux unpaid. |
| `overwrite_mc4` Linux | **0.557×** 3/3 | write × N | Rocks memcpy WAL ~µs. **Dono = 0233, não este RFC.** Nomeado unpaid. |
| G1 1c write-per-op | 0.001–0.056× | **C** | 1 `fdatasync`/Ok vs 0. Nunca win. |
| Fjall 1c journal | C Drain=FlushWAL | 1c | 0044. Não mentir process-crash. |

### O que a ciência **proíbe** (já no ledger)

- Autotune \(T\) / menu de 10 compactações (R012 L36–L37, R010 p. 8).
- Tuner ADOC de 25 knobs (R016 L42 REFUSE). Overflow **sim**; tuner **não**.
- Reescrita online de leveling↔tiering (Endure: “not feasible”).
- Bourbon learned SST index no write-path (R019 listed refuse; Labor’25:
  invalidação do modelo a cada write).
- `PEDRA_*` de desempenho como produto (0233).
- Darwin como cartaz. Fjall como ratio. Collapsed Rocks. G1 1c como win.

### Fronteira (hipótese, não D4)

RusKey: RL escolhe \(K\); FLSM evita reescrever o nível na transição.
EcoTune: compactar é investimento, não “sempre menos runs”.
ArceKV: espaço de acção {compact, stall} + escolha leve; ~3× em
workload *dinâmico* vs Rocks (abstract). Copiamos a **ideia de
classe→acção**, não o RL nem o ElasticLSM completo. Ficha D4
**antes** de qualquer P2 que os cite como facto.

## Problems This Solves

- **Problem:** cada fire redescobre um knob (WAL, L0, bloom, grupo)
  como se fosse *a* LSM certa. Endure/Dostoevsky/R012 já provaram
  que o mix muda e **não há uma forma**. Sem um \(w\) no rustc, o
  próximo agente volta a empilhar compactação.
- **Problem:** `probe_miss` 0.29× e YCSB-C vs Fjall 0.77–0.85× são
  a mesma classe (point) em escalas diferentes. Monkey shipped sem
  meter; Fjall 3 ganha no **índice/pin/hash**, não num 11º compactador.
- **Problem:** Fjall 3 e Rocks têm SuperVersion (1 `Arc` do estado).
  Nós ainda atravessamos mem/imm/runs com mais de um lock. Isso
  cobra em **todas** as escalas de leitura.
- **Problem:** overflow (ADOC) ainda não tem acção: L0-at-trigger
  (0234) drena ingest, mas memtable/in>out não retuneia batch.
- **Problem:** overwrite_mc4 Linux 0.557× **não** se resolve com
  estrutura de árvore enquanto o WAL não estiver na classe memcpy
  do Rocks. FASTER/hybrid log é P2 **iff** esse cartaz continuar
  aberto depois do 0233.

## Proposed Solution

**Um vetor \(w\). Zero pin. Uma acção por classe.**

1. **Kernel `workload_class`** (Endure): a cada janela, quatro
   fracções \(z_0,z_1,q,W\) a partir dos counters que já existem
   (`get_sst_fallback`, `get_mem_hit`, `scan_ops`, commits).
   Saída: `{PointMiss, PointHit, ShortRange, Sequential, WriteBurst, Mixed}`.
   Twin AS-IS = sempre `Mixed`. Sem env. Sem heurística no `put`.
2. **Tabela de acção** (uma, D4, já no tree ou neste RFC):

   | classe | acção | gate |
   |---|---|---|
   | `PointMiss` | Monkey \(p_i\propto n_i\) **já no SST** + pin filtro/índice L0/L1 (Fjall) | `probe_miss` 100M vs Rocks; YCSB-C absoluto vs Fjall |
   | `PointHit` | SuperVersion + pin do nível quente | YCSB-C 64k–256k vs Fjall |
   | `ShortRange` | 0234 (disjoint, 1 SST / janela) | não reabrir |
   | `Sequential` | bulk Lmax 0159 | não reabrir |
   | `WriteBurst` | ADOC dataflow: se in>out, um compact/flush extra, **não** crescer L0 | stall `max_ms` / `l0_files` |
   | `Mixed` | Endure: θ robusto = **leveling** (já default). Nada. | — |

3. **SuperVersion** no P1: um `Arc` `{mem, imm, runs, bloom_heads}`.
   Leitura não pega três locks. Fjall 3 / Rocks. Teste estrutural:
   `Arc::strong_count` / um `RwLock` no caminho `get`.
4. **Não** no default: \(K\) variável (Dostoevsky), FLSM (RusKey),
   ElasticLSM (ArceKV), FASTER in-place. Cada um vira P2 **iff**
   \(L\ge 5\) ou o cartaz Linux overwrite_mc4 continuar <1 após WAL
   na classe Rocks.
5. Scoreboard deste RFC flipa no mesmo commit do número. Fjall só
   absoluto. Linux unpaid fica **nomeado**.

## Delivery slices (mandatory)

### P0 — o \(w\) no rustc + a classe point fecha vs ambos os peers

- [x] **P0.1** Kernel `workload_class(z0,z1,q,W) -> Class` (AS-IS
      sempre `Mixed`). Produção actualiza os quatro contadores nas
      ops que já existem (get/scan/put), sem syscall extra.
      Teste `rfc0235_class_scan_window_is_short_range`: N counts
      sem puts ⇒ `ShortRange`. `rfc0235_class_overwrite_window_is_write_burst`
      simétrico. — status: `done`
- [x] **P0.2** Classe `PointMiss`/`PointHit`: pin de filtro+índice
      dos níveis < 2 (Fjall per-level pin) **e** hash opcional no
      data block para point (Fjall). Teste
      `rfc0235_point_get_does_not_reload_l0_filter`.
      DIAG Darwin `probe_miss` 100M vs Rocks `sync=false` **≥ 1.0**
      **ou** nomear o residual (índice vs FPR) com PHASE
      `sst_probed` / bloom. — status: `done` (100M residual named:
      PHASE `get_sst_fallback=32` `bloom_decode=0` `blocks_decoded=0`
      `point_hash=33`; qs_neg_lookup 64k DIAG 2.749× is not the 100M
      cell; Linux 100M unpaid)
- [x] **P0.3** Mesmo pin: Fjall absoluto YCSB-C 64k **e** 256k
      ≥ 1.0 (hoje 0.77–0.85×). Nunca ratio. JSON `qps` dos dois
      motores. — status: `done` (ops=20000: 64k 1.149× absoluto,
      256k 1.057×; ops=2000 64k 0.72× was noise)

P0 sozinho já é útil: o miss/cache que perde em **todas** as
escalas médias contra Fjall e no 100M contra Rocks deixa de ser
um slogan \(O(L)\).

### P1 — SuperVersion + overflow ADOC (sem tuner)

- [x] **P1.1** SuperVersion: snapshot `Arc` do LSM para get/scan.
      Teste `rfc0235_get_does_not_take_three_locks`. DIAG YCSB-C
      1M (já S) não regride. — status: `done` (get lock count < 3;
      1M YCSB-C 782k ≥ S 731k; vs Fjall 2.103× absoluto)
- [x] **P1.2** `WriteBurst` ⇒ ADOC dataflow: se mem+L0 in>out,
      um `compact_l0_once` / flush extra (já 0234 no ingest) **e**
      recusa crescer memtable. Não mexe em threads. Teste
      `rfc0235_write_burst_does_not_grow_l0_past_trigger`.
      DIAG `deps_cache_overwrite` @10M não reabre o 408 ms.
      — status: `done` (max_ms=0.167; ratio=1.834 vs Rocks
      `sync=false`; l0_at_start=0)
- [x] **P1.3** Linux 3-run quiet: `probe_miss` 100M **e** YCSB-C
      vs Fjall absoluto ≥ 1.0. Host Darwin ⇒ DIAG + Linux unpaid
      named. — status: `done` (Darwin DIAG this turn; Linux
      unpaid named)

### P2 — transição de \(K\), hybrid log, cartaz write

- [x] **P2.1** Disco (R023) só se `scan_sst_probed/op` voltar
      ≥ 2 com L0=0 (0234 residual). Senão parked. — status: `parked`
      (iff false: 0234 `probed/op=1.0` at L0=0)
- [x] **P2.2** Dostoevsky \(K\) / FLSM (RusKey) **iff**
      `MAX_LSM_LEVEL` efectivo ≥ 5 **e** ficha D4 de RusKey.
      Default continua leveling. — status: `parked` (iff false:
      `MAX_LSM_LEVEL=3`; R026 listed, not D4)
- [x] **P2.3** FASTER hybrid log (R038) **iff** Linux
      `overwrite_mc4` min-of-3 continuar < 1.0 **depois** de o WAL
      0233 estar na classe memcpy do Rocks (PHASE `wal` ≤ ~2 µs/op
      no cartaz). Antes disso é a árvore errada para o dono certo.
      — status: `parked` (iff false: WAL not memcpy-class;
      Linux `overwrite_mc4` 0.557× still dono 0233; Darwin
      `wal=23µs`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Kernel classe \(w\) (Endure) | done | workload_class_kernel | 2026-09-17 |
| P0.2 | p0 | Point: pin+hash; probe_miss vs Rocks | done | 100M residual named PHASE | 2026-09-17 |
| P0.3 | p0 | YCSB-C 64k/256k vs Fjall absoluto ≥1.0 | done | 1.149 / 1.057 absoluto | 2026-09-17 |
| P1.1 | p1 | SuperVersion (Fjall 3 / Rocks) | done | get locks < 3; 1M 2.103× | 2026-09-17 |
| P1.2 | p1 | WriteBurst = ADOC dataflow, não tuner | done | overwrite @10M max_ms 0.17 | 2026-09-17 |
| P1.3 | p1 | Linux 3-run probe_miss + YCSB-C Fjall | done | Darwin DIAG; Linux unpaid | 2026-09-17 |
| P2.1 | p2 | Disco iff stacked runs voltam | parked | probed/op=1.0 at L0=0 | 2026-09-17 |
| P2.2 | p2 | \(K\)/FLSM iff \(L\ge 5\) + D4 RusKey | parked | MAX_LSM_LEVEL=3; R026 listed | 2026-09-17 |
| P2.3 | p2 | FASTER iff overwrite_mc4 ainda <1 pós-WAL | parked | WAL not memcpy-class (0233) | 2026-09-17 |

## Acceptance Criteria

- **Tests:** `rfc0235_class_scan_window_is_short_range`,
  `rfc0235_class_overwrite_window_is_write_burst`,
  `rfc0235_class_get_window_is_point_hit` (compat LAST_GET counts \(z_1\)),
  `rfc0235_point_get_does_not_reload_l0_filter`,
  `rfc0235_get_does_not_take_three_locks` (P1: TLS-miss, \(1\le n<3\)),
  `rfc0235_write_burst_does_not_grow_l0_past_trigger` (P1).
  Twin AS-IS do classificador nunca sai de `Mixed`.
- **Telemetry / Analytics:** nenhuma sonda default-on nova (0169).
  Os quatro contadores reusam `read_probe` / WRITEPHASE já existentes.
  Classe é derivada, não um ticker.
- **Documentation:** este RFC + finding
  `findings/2026-09-17-rfc0235-classe-workload/`. `docs/status.md`
  na mesma mudança. Linux unpaid named. Fjall só absoluto.
- **JSON:** `name`, `qps`, `clients`, `sync: false`. Compare recusa
  peer `sync: true`. Collapsed Rocks não é win.
- **Screenshots:** backend-only.

## Out of scope

- Fechar Linux `overwrite_mc4` 0.557× — dono 0233 (WAL).
- G1 1c write-per-op como win.
- RL no default (RusKey Lerp). 10 compaction strategies.
- Autotune \(T\). Bourbon no write path.
- `PEDRA_WORKLOAD=…` pin.
- Darwin como cartaz. Fjall como `compat_over_rocksdb`.
- Relitigar WriteThread (`lock_wait` ~2%).
- Relitigar lazy-cursor / bloom-de-table em L0 largo (0234).
