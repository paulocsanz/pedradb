# RFC-0236 — Filtro particionado + SuperVersion Arc: a física que Fjall 3 e Rocks têm; a classe 0235 dispara

**Status:** in-progress (P0–P1 done; P2 parked per iff; Linux unpaid named)
**Updated:** 2026-09-17
**ID:** 0236
**Parents:** [0235](0235-classe-workload-empatar-rocks-fjall-todas-escalas.md)
(classe \(w\) no rustc; pin L0/L1; lock count &lt; 3; Darwin YCSB-C vs Fjall ≥1; `probe_miss` 100M **residual**),
[0233](0233-ganhar-sempre-rocks-fjall-default.md)
(WAL de produto; Linux `overwrite_mc4` **0.557× unpaid** — **não** este RFC)
**Papers (D4 nesta casa):**
[R006 Monkey](../../research/fichamentos/ficha_R006_Dayan_Monkey.md),
[R014 Endure](../../research/fichamentos/ficha_R014_Huynh_Endure.md),
[R010 Rocks Experience](../../research/fichamentos/ficha_R010_Dong_RocksExperience.md)
**Fronteira (abstract/HTML, finding — não D4):**
Fjall 3.0 (2026-01 / 2026-09 post), Rocks partitioned index/filter (2017) + data-block hash (2018),
ArceKV VLDB’26 (arXiv:2508.03565), TurtleKV (arXiv:2509.10714, R040 listed),
FASTER hybrid log (R038 listed).
[finding](../../findings/2026-09-17-rfc0236-partitioned-sv/).
**Peer Rocks:** `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`).
Darwin = DIAG. Linux 3-run min-of-3 = cartaz.
**Peer Fjall:** QPS **absoluto**. Nunca `compat_over_rocksdb`.

> **Tese:** a classe 0235 já existe. O que ainda perde em **escala**
> não é “qual LSM”; é **o que o get carrega do disco** e **o que o
> compact bloqueia**. Fjall 3 (post 2026-09-15) e Rocks default já
> têm: (1) **filtro particionado** — miss carrega ~4 KiB, não o
> Bloom inteiro; (2) **SuperVersion CoW** — compact não pega o lock
> de leitura. Nós pinámos L0/L1 inteiro e contámos locks &lt; 3
> atrás de um `RwLock&lt;Db&gt;`. `probe_miss` 100M **0.29×** é
> exactamente o filtro grande. Write × N continua dono **0233**
> (memcpy WAL); hybrid log / TurtleTree só no P2 **iff** esse
> cartaz continuar &lt;1. Não RL. Não 10 strategies. Não autotune \(T\).

## Background

### O que 0235 pagou (não reabrir)

- Kernel `workload_class(z0,z1,q,W)` + AS-IS `Mixed`.
- Pin L0/L1: `bloom_decode=0` em hit/miss DIAG; `point_hash` no path.
- Get lock count \(1\le n&lt;3\) (TLS-miss). LAST_GET conta \(z_1\).
- Darwin DIAG: ycsb_c vs Rocks **1.485×**; Fjall YCSB-C 64k **1.149×**
  / 256k **1.093×** absoluto; overwrite @10M **1.834×** max_ms 0.17.
- Disco / \(K\)-FLSM / FASTER **parked** (iffs 0235).

### Scoreboard que este RFC ataca

| célula | número | o que o peer faz e nós não |
|---|---|---|
| `probe_miss` 100M | **0.29×** Rocks; ~2× Fjall | Fjall 3 / Rocks: **partitioned filter** (4 KiB/partition). Pin L0/L1 não cabe o filtro de um SST de 100M. Monkey \(p_i\propto n_i\) já no rebuild — o miss ainda I/O do filtro. |
| Get sob compact | lock count &lt; 3, mas `inner.read()` espera o write lock | Fjall 3 / Rocks: `Arc&lt;Version&gt;` CoW; compact prepare **não** bloqueia leitores. |
| YCSB-C cache | Darwin ≥1 vs Fjall; Linux unpaid | `rustc_hash` no cache (~25 ns vs xxh3); hash **dentro** do data block (1-byte bucket → restart), não só p8 no índice. |
| `overwrite_mc4` Linux | **0.557×** 3/3 | Rocks memcpy WAL ~µs. **Dono = 0233.** Este RFC não mexe no WAL. |
| G1 1c write-per-op | 0.001–0.056× | **C** fd-ceiling. Nunca win. |

### O que a ciência **proíbe** (já no ledger)

- Autotune \(T\) / menu de 10 (R012, R010 p. 8).
- Tuner ADOC de 25 knobs (R016 L42).
- Reescrita leveling↔tiering online (Endure: “not feasible”).
- Bourbon no write-path (R019).
- `PEDRA_*` de desempenho como produto.
- Darwin como cartaz. Fjall como ratio. Collapsed Rocks. G1 1c como win.

### Fronteira 2025–26 (hipótese, não D4)

- **Fjall 3** (código + post, 2026-09-15): SuperVersion CoW; 3 locks → 1;
  **partitioned filters** (V2 já tinha index particionado); hash index
  no data block; prefix truncation; seqno zero no Lmax. Isto **é** o
  peer Rust. Copiamos o layout de leitura, não o journal 1c (C 0044).
- **ArceKV** (VLDB’26, arXiv:2508.03565): ElasticLSM = acções
  {compact, stall} em qualquer run; Arce escolhe; ~3× vs Rocks em
  workload *dinâmico*; adapta em ≤20M ops. Copiamos **classe→acção**
  (já 0235). Não o menu AnyTime–AnyRuns no P0.
- **TurtleKV** (arXiv:2509.10714, R040 listed): TurtleTree = B⁺ com
  buffer LSM *dentro do nó*; knobs RM/WM **sem** reescrever a forma.
  Até 8× write / 5× read vs Rocks no paper. Ficha D4 **antes** de P2.
- **FASTER / F2**: hybrid log, in-place no hot set. Ganha overwrite
  quando o WAL da LSM não está na classe memcpy. P2 iff 0233.

## Problems This Solves

- **Problem:** 0235 classifica PointMiss e a acção é “Monkey + pin
  L0/L1”. Em 100M o filtro de um SST de nível baixo **não cabe** no
  pin. O miss 0.29× (2.3 µs vs 650 ns) é I/O do Bloom, não FPR.
  Fjall 3 / Rocks carregam uma partição.
- **Problem:** “SuperVersion” 0235 é um contador de locks. Compact
  ainda escreve `RwLock&lt;Db&gt;` e o get espera. Fjall 3: compact
  prepare não afecta leitores. Isso cobra em **todas** as escalas
  de leitura sob ingest.
- **Problem:** a classe não pode fechar write × N. Rocks satura em
  \(N=2\) (~µs memcpy). Sem memcpy WAL **ou** uma segunda estrutura
  (hybrid log / TurtleTree), nenhuma política de compactação empatará
  `overwrite_mc4`. Esse dono fica **nomeado 0233**; P2 só se o cartaz
  continuar aberto.

## Proposed Solution

**A classe 0235 dispara **duas** físicas de layout, zero tuner.**

1. **Partitioned filter** (Rocks 2017 blog / Fjall 3): o trailer do
   SST deixa de ser um Bloom monolítico. Top-level index sempre
   residente; miss carrega uma partição (~4 KiB). Twin AS-IS = um
   filtro. Gate: `probe_miss` 100M vs Rocks `sync=false` **ou** PHASE
   `filter_bytes` / `sst_probed` nomeado (não win). Classe
   `PointMiss` **é** quem arma o path; Mixed/ShortRange não mudam.
2. **SuperVersion Arc CoW:** `Arc&lt;{mem, imm, runs, blooms}&gt;`.
   Get/scan clona o Arc (FAA). Compact instala um Arc novo; leitores
   no Arc velho não esperam o write lock. Teste: get durante
   `compact_l0` não incrementa wait no write lock. YCSB-C 1M não
   regride vs 0235.
3. **Não** no P0: hybrid log, TurtleTree, ElasticLSM AnyRuns, Ribbon
   filter, `rustc_hash` (micro 25 ns — só se PHASE do miss já for
   filtro). Cada um vira P1/P2 com iff.

## Delivery slices (mandatory)

### P0 — o miss 100M deixa de carregar o Bloom inteiro; get não espera compact

- [x] **P0.1** Kernel `filter_partition(key, nparts) -> part` (AS-IS
      sempre 0 / um filtro). SST write emite partições + top index.
      Get miss carrega **uma** partição. Teste
      `rfc0236_point_miss_loads_one_filter_partition`. — status: `done`
- [x] **P0.2** SuperVersion: `Arc` do LSM; get (hit **e** miss in-range)
      e scan curto clonam o snapshot publicado (SSTs em
      `sst_order_newest`) — não esperam o write lock de compact. Teste
      `rfc0236_get_does_not_wait_compact_write_lock` (dois L0 `a=v1`/`a=v2`).
      DIAG YCSB-C 1M não regride vs 0235 (782k). — status: `done`
- [x] **P0.3** Darwin DIAG `probe_miss` 100M vs Rocks `sync=false`
      ≥ 1.0 **ou** residual nomeado com PHASE `filter_bytes` /
      `sst_probed` (não quoted as win). Host Darwin ⇒ Linux unpaid
      named. — status: `done`

P0 sozinho já é útil: o miss que perde em **toda** escala grande
contra Rocks/Fjall deixa de ser “Bloom monolítico + RwLock”.

### P1 — hash no data block (Fjall 3) + prefix truncation

- [x] **P1.1** Hash index **dentro** do data block (1-byte bucket →
      restart; CONFLICT cai no binsearch). Não é o p8 do índice SST.
      Teste `rfc0236_point_get_uses_block_hash_or_falls_back`.
      DIAG YCSB-C 64k vs Fjall absoluto não regride. — status: `done`
- [x] **P1.2** Prefix truncation nos restart heads (Fjall 3). Teste
      estrutural: block resident size cai em keys com prefixo comum
      (`rfc0236_prefix_truncation_shrinks_block`). — status: `done`
- [x] **P1.3** Linux 3-run `probe_miss` 100M **ou** Darwin DIAG +
      Linux unpaid named. — status: `done`

### P2 — segunda estrutura (não mais uma LSM) iff o write continuar &lt;1

- [x] **P2.1** FASTER hybrid log / in-place hot (R038) **iff** Linux
      `overwrite_mc4` min-of-3 &lt; 1.0 **depois** de WAL 0233 na
      classe memcpy (`wal` ≤ ~2 µs). Classe `WriteBurst` escolhe o
      log; Mixed fica LSM. Sem iff = parked. — status: `parked`
- [x] **P2.2** TurtleTree (R040) **iff** ficha D4 e P2.1 não fechou
      overwrite. RM/WM knobs **não** são `PEDRA_*` de produto — a
      classe 0235 escolhe o lado. — status: `parked`
- [x] **P2.3** Arce {compact, stall} **iff** um soak nomeado muda
      \(w\) no meio e o stall 0235 WriteBurst compact-on-put for o
      dono (p99). Senão parked. — status: `parked`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Partitioned filter; uma partição no miss | done | filter_partition_kernel | 2026-09-17 |
| P0.2 | p0 | SuperVersion Arc CoW; get hit/miss + scan ∥ compact | done | published_sv newest-first L0 | 2026-09-17 |
| P0.3 | p0 | probe_miss 100M vs Rocks ou PHASE | done | PHASE residual; Linux unpaid | 2026-09-17 |
| P1.1 | p1 | Hash no data block (Fjall 3) | done | block hash trailer | 2026-09-17 |
| P1.2 | p1 | Prefix truncation | done | ENTRY_TRUNC_FLAG | 2026-09-17 |
| P1.3 | p1 | Linux 3-run probe_miss | done | Darwin DIAG + Linux unpaid | 2026-09-17 |
| P2.1 | p2 | Hybrid log iff overwrite_mc4 &lt;1 pós-WAL | parked | wal ≰ ~2 µs | 2026-09-17 |
| P2.2 | p2 | TurtleTree iff D4 + P2.1 aberto | parked | R040 listed, not D4 | 2026-09-17 |
| P2.3 | p2 | Arce stall/compact iff soak muda \(w\) | parked | no soak changes w | 2026-09-17 |

## Acceptance Criteria

- **Tests:** `rfc0236_point_miss_loads_one_filter_partition`,
  `rfc0236_get_does_not_wait_compact_write_lock` (P0: two L0s newest wins + in-range miss + short scan under write lock),
  `rfc0236_point_get_uses_block_hash_or_falls_back` (P1),
  `rfc0236_prefix_truncation_shrinks_block` (P1).
  Twin AS-IS do particionador = um filtro.
- **Telemetry / Analytics:** nenhuma sonda default-on nova (0169).
  PHASE `filter_bytes` / partição só em DIAG. Classe 0235 já deriva.
- **Documentation:** este RFC + finding
  `findings/2026-09-17-rfc0236-partitioned-sv/`. `docs/status.md`
  na mesma mudança. Linux unpaid named. Fjall só absoluto.
- **JSON:** `name`, `qps`, `clients`, `sync: false`. Compare recusa
  peer `sync: true`. Collapsed Rocks não é win.
- **Screenshots:** backend-only.

## Out of scope

- Fechar Linux `overwrite_mc4` 0.557× — dono 0233 (WAL memcpy).
- G1 1c write-per-op como win.
- RL (RusKey Lerp). 10 compaction strategies. Autotune \(T\).
- ElasticLSM AnyTime–AnyRuns no default.
- `PEDRA_WORKLOAD=…` pin. `PEDRA_FILTER_PARTS=…` pin.
- Darwin como cartaz. Fjall como `compat_over_rocksdb`.
- Relitigar WriteThread (`lock_wait` ~2%).
- Relitigar pin L0/L1 / classe 0235.
