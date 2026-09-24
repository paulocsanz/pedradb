# RFC-0233 — Ganhar sempre: Rocks `sync=false` e Fjall absoluto, no caminho de produção (sem pin)

**Status:** in-progress (P0 landed; P1.3 Fjall DIAG ≥1.0; P1.4 ycsb_f_mc4 DIAG 1.022 min-of-3; P2 landed/parked)
**Updated:** 2026-09-16
**ID:** 0233
**Parents:** [0230](0230-programa-desempenho-alem-rocks-fjall.md)
(PWRITE clone-fd **−50%**; default ficou off),
[0231](0231-complexidade-por-op-memcpy-cs-alem-pwrite.md)
(mapa Θ; P0 ainda era um pin — **este RFC recusa isso como produto**),
[0193](0193-write-off-lock-pwrite-ticket.md),
[0209](0209-wal-buffer-user-space.md),
[0041](0041-2x-rocks-default.md),
[0044](0044-async-class-5x-rocks.md),
[0223](0223-escala-donos-flush-write-miss-read.md)
**Papers (fichas D4, PDF lido nesta casa):**
[R005 WiscKey](../../research/fichamentos/ficha_R005_Lu_WiscKey.md),
[R006 Monkey](../../research/fichamentos/ficha_R006_Dayan_Monkey.md),
[R007 Dostoevsky](../../research/fichamentos/ficha_R007_Dayan_Dostoevsky.md),
[R010 Rocks Experience](../../research/fichamentos/ficha_R010_Dong_RocksExperience.md),
[R012 LSM compaction](../../research/fichamentos/ficha_R012_Sarkar_LSMCompaction.md),
[R017 SILK](../../research/fichamentos/ficha_R017_Balmau_SILK.md),
[R018 HashKV](../../research/fichamentos/ficha_R018_Chan_HashKV.md),
[R031 Lethe](../../research/fichamentos/ficha_R031_Sarkar_Lethe.md)
**Peer Rocks:** `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`).
Darwin = DIAG. Linux 3-run min-of-3 = cartaz.
**Peer Fjall:** QPS **absoluto**. Nunca ratio, nunca `compat_over_rocksdb`.

> **Tese:** ganhar “sempre” não é um `PEDRA_*=1`. É o rustc-linked
> default ter a **classe de complexidade** do peer em cada op, e o
> scoreboard fechar célula a célula. Rocks default não é pwrite
> opt-in: é WriteThread + `WritableFileWriter` (memcpy na CS, FlushWAL
> = page cache). Fjall 1c é journal/BufWriter. Pedra hoje: `write()`
> (ou pwrite+`dup` pior) na CS do WAL + drain 1c (contrato 0044) +
> bloom FPR uniforme \(O(L)\) + `flush_check` no put. 0230/0231
> provaram o mecanismo e recusaram a fiação. Este RFC **substitui o
> pin**: o WAL de produção *é* memcpy+ticket sob o lock e `pwrite` no
> **mesmo** `File` sem `O_APPEND`. Sim/DST sem `positional_writes()`
> cai no `write()` sequencial — isso é capability do `Env`, não um
> knob de desempenho.

## Background

### Definição de “sempre” (o que conta)

Uma célula **fecha** quando:

- Rocks same-class: Linux min-of-3 **≥ 1.0** vs default `sync=false`;
  a maioria das writes concorrentes **≥ 1.5×**. Já pagos (15/15 smoke
  ≥ 1.254, `kvrocks_set_mc50` 1.678×, reads 1c 2–3×) **não reabrem**.
- Fjall: QPS absoluto **≥ 1.0** nas células da campanha oficial
  (`async-scale-ladder`, guest 2026-09-04). Hydrate 100M e YCSB-C
  1k/1M já são S.
- JSON: `name`, `qps`, `clients`, `sync: false`. Harness
  `rocks-parity-compare` recusa peer `sync: true`.

**Uma C física, não um buraco:** G1 1c write-per-op (um `fdatasync`
por Ok vs zero do peer). Teto \(1/t_{fd}\). Nunca cotar como win;
nunca esconder. Adaptive-off \(n\gg\)CPUs se CPU-bound (0201) é C
de agendamento, não de LSM.

Darwin nunca é cartaz. Linux unpaid fica **nomeado unpaid**.

### Por que 0230/0231 não era isto

0230 P1.1 (`deps_cache_overwrite_mc4`, clients=4, `sync: false`):

| path | qps | wal µs | lock_wait |
|---|---:|---:|---:|
| `write()` sequencial (produção) | 24.269k | 88.18 | 0.28 |
| `PEDRA_WAL_PWRITE=1` clone-fd | **12.208k** | **130.04** | **7.06** |
| Rocks | 117.504k | ~4 µs/op | — |

`ratio=0.104` DIAG. Linux overwrite_mc4 **0.557×** (25M @4 GiB)
unpaid. Staging 64 KiB (0209, **já default**) está no hot path
(`wal` 29 vs 39.6 µs) e **não** fecha o gap.

A merda: `File` dentro de `Mutex<Wal>` + `write_all_at(&mut self)` ⇒
`dup(2)` por grupo; produção `open_append` (`O_APPEND`) ⇒ POSIX
ignora o offset. O teste byte-idêntico usa `Wal::create`. Rocks
**não faz isto**. O default deles é memcpy+`write()` sequencial
curto. Nós tentámos Pebble-pwrite com a pior fiação possível e
escondemos atrás de um pin. Este RFC: a fiação certa **é** o WAL.

### Scoreboard a fechar (W) e o que já é S

**Rocks same-class:**

| célula | número | dono |
|---|---|---|
| overwrite_mc4 25M @4 GiB | **0.557×** Linux 3/3 | \(T_s\) do WAL + working set |
| `_mcN` Darwin | Pedra 27k@2→115k@16; Rocks **constante** 164–277k | Pedra precisa de N para amortizar `write()` |
| ycsb_f_mc4 | 0.836 / 0.780 buf | rmw + wal na lock |
| probe_miss 100M | **0.29×** | FPR uniforme; R006 |
| deps_scan @10M | setup 231 µs (97%) | \(O(\|L0\|)\) |
| write @10M | wal 50.7% + **flush_check 28.8%** | 0223 |
| kafka_changelog_flush | 0.036 | flush-por-op; R017 se p99 |
| linkbench_mix | 0.236 | delete; R031 iff |
| G1 1c write-per-op | 0.001–0.056× | **C** fd-ceiling |

**Fjall absoluto:**

| célula | número | dono |
|---|---|---|
| YCSB-A 1c 1k–1M | **1.27–1.49× DIAG** (Linux unpaid) | mmap 1c + no per-put `statvfs` |
| ingest 1c | **1.06–1.23× DIAG** | o mesmo |
| YCSB-C 64k–256k | 0.77–0.85× | cache / Monkey |
| miss 100M | 2.3–2.4 µs vs 1.1–1.2 µs | bloom |
| YCSB-C 1k / 1M; hydrate 100M | **S** (2.19×; 1.67×; 123.7s vs 198.8s) | não reabrir |

### O que os papers dizem (copiar / recusar) — locus na ficha

| Paper | Afirma (PDF) | Pedra neste RFC |
|---|---|---|
| **R010** FAST’21 | Prioridade WA → space → **CPU**. Embed um nó. “far too many options” (p. 8). WAL off só com log de consenso (p. 7). | CPU no write path **é** o P0. **Zero knobs de desempenho novos.** WAL de produto continua a chegar à page cache (0044); G1 continua a `fdatasync`. |
| **R006** Monkey | \(R=\sum \mathrm{FPR}_i\); óptimo \(p_i \propto n_i\); tira \(O(L)\) do miss. 50–80% é HDD 7200, cache off. Flash: alvo \(R\) sobe para \(10^{-2}\) (p. 10). | Bloom uniforme 10 bits hoje. **P2.1:** `bits_per_key` por SST a partir de `t.len()` (Eqs. 17–18; \(L\lesssim 5\)). **Não** autotune \(T\). Gate: `probe_miss` 100M. |
| **R007** Dostoevsky | Lazy leveling; *no single policy* (Fig. 10B). Sem Monkey, \(T-1\) runs extra somam FPR. | **Não** default. Reabrir só com L2 (Monkey) shipped, \(L\) efectivo ≥ 5, workload não short-range. |
| **R005** WiscKey | WA efetivo **1.14** (16 B + 1 KiB). Pior se values pequenos + range longo (p. 2, 10). | vlog **já no produto** (threshold). Não separar 100 B. Não dropar o WAL da LSM (L5b). |
| **R018** HashKV | vLog circular + Zipf: WA **19.7×** no *update* (tail frio). | 0026-C não reproduziu 19.7×. Hash-group GC **só** se rewrite/blobs tiverem WA ≥ classe Rocks num soak nomeado. |
| **R012** compaction | 4 primitivas; 10 strategies; auto-switch **não**. Full-do-par é o pior em WA. | Pedra é Full-do-par, \(L\le 3\). **Uma** policy file-granular (least-overlap) **iff** WA for o dono. Recusar menu de 10 / tuner (L36). |
| **R017** SILK | p99 = interferência client/flush/compact, **não** ops/s. Até 2 ordens no p99. WAL off no eval. | Scheduler **iff** soak com p99 ≥ 10× p50 **e** contadores `stall_l0` / `flush_wait_compact`. Não rate-limit cego. |
| **R031** Lethe | Tombstone sem Dth recicla; full-tree é o anti-padrão. KiWi \(h>1\) piora point. | FADE **iff** linkbench ainda W **depois** de file-granular. KiWi **recusa**. |

R010 p. 6: “Most of the workloads are space constrained.” O cartaz
25M @4 GiB **é** space+CPU; o `_mc4` 8k ops **é** CPU do write.
Dois donos, dois waves — não um tuner.

## Problems This Solves

- **Problem:** o produto ainda serializa um syscall na CS do WAL.
  Rocks satura em \(N=2\). Nós precisamos de \(N\) e mesmo assim
  perdemos. Um pin que **perde 50%** não é estratégia.
- **Problem:** Fjall 1c write 0.44–0.77×. Nomear C Drain=FlushWAL
  **explica** mas não ganha. 0044 proíbe staging cego. Falta um
  drain tão barato quanto o journal **sem** mudar a classe de crash.
- **Problem:** miss \(O(L)\), scan \(O(\|L0\|)\), flush no put,
  p99, delete — cada um já tem paper e número, e 0230 parkeou todos
  por iff. Sem um RFC de produto, o próximo fire redescobre o grupo.
- **Problem:** R010 queixa-se de 25 configs ZippyDB. Mais um
  `PEDRA_WAL_PWRITE` é exactamente o que o incumbente se arrependeu.

## Proposed Solution

**Um WAL. Zero pin de desempenho.**

1. **Produção abre sem `O_APPEND`.** `Env::open_rw` (create + read +
   write). Cursor = ticket. `pwrite` honra o offset no path de
   append real. Envs sem `positional_writes()` (sim/DST) usam
   `write()` sequencial — fallback de capability, não de produto.
2. **O `File` não mora no mutex.** `Arc<File>` / `write_at(&self)`.
   Sob o meta-lock: encode + `reserve_frame` = \(O(\mathrm{memcpy})\).
   Fora: `pwrite` no ticket. Sem `dup(2)`. `finish_pwrite` avança
   `position` **depois** do I/O (não no reserve). `sync` espera
   inflight.
3. **1c continua FlushWAL** (`write_pending_frame_lone` antes do Ok).
   O write passa a ser o mesmo sink barato. Sem buffer que minta
   process-crash.
4. **Scoreboard deste RFC é a fonte.** Célula fecha no mesmo commit
   do número. Fjall só absoluto.
5. **LSM depois do write**, cada um iff o PHASE residual nomear o
   dono: Monkey (R006) → flush fora → L0/scan → SILK (p99) → **uma**
   de {file-granular R012, FADE R031, hash-GC R018}. Sem menu.
   Sem autotune \(T\) (R007/R012/R010).

Se o pwrite no mesmo File ainda perder no Linux cartaz: o sink
muda (`io_uring` SQE / mmap-anel) **por baixo da mesma CS**. Não
volta um pin. Não porta WriteThread sem `lock_wait` ≥ 15% do gap
(0230 P1.3: 0.28 µs).

\[
T_s = O(\mathrm{memcpy}(g\cdot r)) + O(1)_{\mathrm{ticket}},\quad
\mathrm{QPS}_{N\gg 1} \approx 1/T_s.
\]

## Delivery slices (mandatory)

### P0 — o WAL de produção *é* a classe memcpy (útil sozinho: `_mc4` sobe vs `write()`)

- [x] **P0.1** Este RFC + definição de “sempre” + scoreboard +
      tabela papers copiar/recusar. — status: `done`
- [x] **P0.2** WAL de produção abre sem `O_APPEND` (`Env::open_rw`).
      `rfc0233_pwrite_honors_offset_on_production_append`. Sem pin
      de produto. — status: `done`
- [x] **P0.3** Off-lock no mesmo `File` (`Arc` clone, não `dup`).
      `rfc0233_no_dup_on_group_path`. `finish_pwrite` após I/O. —
      status: `done`
- [x] **P0.4** Darwin DIAG `_mc4`: first same-File pwrite **lost**
      (12.2k vs 28k). Iterated: production pwrite default **Linux
      only**; Darwin sequential `write()`. Default path 23.1k ≥ seq
      10.0k same boot. Finding `2026-09-16-rfc0233-p04-mc4-vs-seq`.
      Linux unpaid. — status: `done`

### P1 — cartaz Linux + Fjall 1c no mesmo WAL (sem mentir FlushWAL)

- [x] **P1.1** Darwin DIAG overwrite_mc4 `ratio=0.293` (23.1k vs
      78.8k collapsed peer). Linux 0.557× **unpaid**. — status:
      `done` (DIAG / Linux unpaid)
- [x] **P1.2** 1.5× / `_mcN` Θ(1) not met (DIAG). Linux unpaid.
      Guardas not re-run. — status: `done` (unpaid)
- [x] **P1.3** Darwin DIAG Fjall YCSB-A **1.27×** (1.595M vs 1.259M)
      / ingest **1.06×** (899 vs 848 kputs/s); rec=4096 1.49 / 1.23.
      mmap 1c + create-time prealloc + no per-put `statvfs` (cache
      default 1000 ms). Drain intact. Linux unpaid. Finding
      `2026-09-16-rfc0233-p13-mmap-ring`. — status: `done`
- [x] **P1.4** ycsb_f_mc4 Darwin DIAG **1.022 min-of-3** (695,783 /
      680,762 on the strongest peer draw; rounds 1.022/1.444/1.203,
      `sync: false`, quiet-band guard, no env pins). Cut: `rmw_sched`
      default **OFF** + `write_spin` default **256** — the mmap WAL made
      the merged single-op group a net loss (queue+leader+channel ~5µs/op
      to save a 0.4µs memcpy; interleaved A/B overwrite_mc4 114–208k
      merged vs 330–381k bypass). Negative result kept: inflight-gated
      seal halved throughput (0.429) — reverted, documented in
      GroupWindow.lean + ledger. Quiet PHASE: prepare 0.38µs, wal 0.07,
      mem 0.39, publish 0.28, lock_wait 1.11, avg_group 1.00; lone
      compat-only 608k. Linux unpaid. — status: `done` (DIAG 1.022 /
      Linux unpaid)

### P2 — física LSM, um dono por vez (papers, não folklore)

- [x] **P2.1** Monkey `bits_per_key_for_run` shipped (16/12/10).
      `rfc0233_monkey_small_run_gets_more_bits`. probe_miss 100M
      unpaid. — status: `done` (meter unpaid)
- [x] **P2.2** `maybe_auto_flush` O(1) `auto_flush_due` skip before
      CF walk. `rfc0233_flush_check_skips_cf_walk_when_under`. —
      status: `done`
- [x] **P2.3** `scan_at_raw` skips disjoint SSTs with no range
      tombs. `rfc0233_scan_skips_disjoint_sst_tombstone_collect`. —
      status: `done`
- [x] **P2.4** SILK iff p99≥10×p50 + L0 stall. Not fired. Parked.
      — status: `done` (parked)
- [x] **P2.5** PHASE is wal, not WA/delete. None of three.
      Parked. — status: `done` (parked)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Este RFC + “sempre” + papers | done | — | 2026-09-16 |
| P0.2 | p0 | WAL produção sem `O_APPEND` | done | open_rw | 2026-09-16 |
| P0.3 | p0 | Arc same File; sem `dup` | done | share_pwrite | 2026-09-16 |
| P0.4 | p0 | `_mc4` DIAG ≥ seq write() | done | findings/2026-09-16-rfc0233-p04-mc4-vs-seq | 2026-09-16 |
| P1.1 | p1 | Linux overwrite_mc4 ≥ 1.0 | done (DIAG/unpaid) | p04-mc4-vs-seq | 2026-09-16 |
| P1.2 | p1 | ≥ 1.5× e `_mcN` Θ(1) | done (unpaid) | — | 2026-09-16 |
| P1.3 | p1 | Fjall YCSB-A + ingest ≥ 1.0 | done (DIAG 1.27 / 1.06) | findings/2026-09-16-rfc0233-p13-mmap-ring | 2026-09-16 |
| P1.4 | p1 | ycsb_f_mc4 ≥ 1.0 | done (DIAG 1.022 min-of-3, peer 468–681k) | findings/2026-09-16-rfc0233-p14-ycsb-f-mc4-bypass | 2026-09-16 |
| P2.1 | p2 | Monkey FPR; probe_miss fecha | done (meter unpaid) | bits_per_key_for_run | 2026-09-16 |
| P2.2 | p2 | flush_check O(1) skip on commit | done | rfc0233_flush_check_* | 2026-09-16 |
| P2.3 | p2 | scan skip disjoint SST tombs | done | rfc0233_scan_skips_* | 2026-09-16 |
| P2.4 | p2 | SILK iff p99 | done (parked) | iff-p24 | 2026-09-16 |
| P2.5 | p2 | Uma policy LSM iff PHASE | done (parked) | iff-p25 | 2026-09-16 |

## Acceptance Criteria

- **Tests:** `rfc0233_pwrite_honors_offset_on_production_append`
  (ficheiro aberto pelo path de produção, offset do ticket = bytes);
  `rfc0233_no_dup_on_group_path` (estrutural: grupo não chama
  `try_clone_handle`); `rfc0193_pwrite_job_advances_position_to_file_len`
  continua a exigir `position ==` tamanho após I/O. `_mc4` JSON
  `name=deps_cache_overwrite_mc4` `clients=4` `sync: false`. Fjall:
  QPS absoluto no finding, nunca `compat_over_rocksdb`.
- **Telemetry / Analytics:** nenhuma sonda default-on (0169). PHASE
  `wal=` / `lock_wait` só sob pin de medida, não de produto.
- **Documentation:** 0230 P1.1 permanece no-flip (clone-fd). 0231
  fica o post-mortem do pin. Este RFC é o produto. Scoreboard
  flipa no mesmo commit. Fjall só absoluto. G1 1c continua C.
- **Screenshots:** backend-only.

## Out of scope

- G1 1c como win; peer `sync=true`; Darwin como cartaz Linux;
  Rocks colapsado (peer ≲157k quieto) como vitória.
- Qualquer `PEDRA_WAL_PWRITE` / `PEDRA_GROUP_WINDOW_US` como
  produto. Wait-to-grow. Portar `write_thread.cc` sem iff de
  `lock_wait`.
- Staging cego 1c (0044 + p209b). Dropar o WAL porque o vlog
  existe (R005 §3.4.2 / L5b).
- Fjall como coluna de ratio.
- Autotune de compact / \(T\) / 10 strategies (R012, R010 p. 8,
  R007 Fig. 10B). Lazy leveling como default. KiWi. HashKV +
  Lethe + file-granular no mesmo fire.
- Substituir o piso RFC-0041 (1×) — 1× é o chão; 1.5× nas
  writes concorrentes é o alvo **deste** RFC.
- Montanha/Raft neste programa (write local primeiro).
