# RFC-0230 — Programa de desempenho: intercepto `wal_write`, depois a física LSM, para a maioria ≫ Rocks e Fjall

**Status:** closing (P0 landed; P1.1 no-flip; P1.2 C; P1.3/P2 parked by iff)
**Updated:** 2026-09-16
**ID:** 0230
**Parents:** [0041](0041-2x-rocks-default.md) (piso 1× same-class; 2× é backlog),
[0185](0185-coluna-a-dropin-1x-tudo.md) (G_A min-of-3 > 1.0),
[0193](0193-write-off-lock-pwrite-ticket.md) (pwrite fora do `wal.lock` — wiped, desenho vivo),
[0209](0209-wal-buffer-user-space.md) (staging 64 KiB default ON; 1c drena),
[0217](0217-fechar-board-async-grupo-ucells-escala-encode-read.md) (U-cells + escala),
[0223](0223-escala-donos-flush-write-miss-read.md) (flush_check + miss-path),
[0226](0226-fechar-u-write-vs-n-seal-and-absorb.md) (selo singleton; WriteThread parked),
[0055](0055-rocks-write-pipeline.md) (concurrent memtable iff ≥15%)
**Papers (fichas D4, lidas na íntegra nesta casa):**
[R005 WiscKey](../../research/fichamentos/ficha_R005_Lu_WiscKey.md),
[R006 Monkey](../../research/fichamentos/ficha_R006_Dayan_Monkey.md),
[R007 Dostoevsky](../../research/fichamentos/ficha_R007_Dayan_Dostoevsky.md),
[R010 Rocks Experience](../../research/fichamentos/ficha_R010_Dong_RocksExperience.md),
[R012 LSM compaction](../../research/fichamentos/ficha_R012_Sarkar_LSMCompaction.md),
[R017 SILK](../../research/fichamentos/ficha_R017_Balmau_SILK.md),
[R018 HashKV](../../research/fichamentos/ficha_R018_Chan_HashKV.md),
[R031 Lethe](../../research/fichamentos/ficha_R031_Sarkar_Lethe.md)
**Peer Rocks:** `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`). G1 1c
não é win. Darwin = DIAG. Linux 3-run min-of-3 = cartaz.
**Peer Fjall:** QPS **absoluto** na campanha existente
(`findings/async-scale-ladder/`, `findings/2026-09-04-fjall-official-guest/`).
Nunca `compat_over_rocksdb` como win vs Fjall.
**Finding de síntese:** [2026-09-16-next-cut-wal-write-vs-n](../../findings/2026-09-16-next-cut-wal-write-vs-n/README.md).
**Filhos:** [0231](0231-complexidade-por-op-memcpy-cs-alem-pwrite.md)
(post-mortem do pin; clone-fd recusada),
[0233](0233-ganhar-sempre-rocks-fjall-default.md)
(produto: WAL memcpy default, sem pin; Rocks+Fjall em todas as escalas).

> **Tese:** o piso 1× vs Rocks default (15/15 smoke ≥ 1.254, RFC-0041) já
> existe. A maioria das perdas que restam **não é política de grupo** —
> é o intercepto `write()` na seção crítica (PHASE Darwin `_mc4`
> `wal=23µs`; Linux quieto `wr=890ns` no `wal.lock`) e, em escala, flush
> no commit + miss-path de bloom. Fjall ganha o 1c write porque
> journaliza e nós drenamos cada frame 1c de propósito (Rocks FlushWAL).
> Ganhar *de verdade* (maioria ≫, resto ≥) é um programa em ondas
> mapeado a papers já fichados — não um mega-corte, não portar
> `write_thread.cc` no escuro, não wait-to-grow.

## Background

### O que já está pago (não reabrir)

- Smoke 1c same-class Linux: **15/15 ≥ 1.254** (`findings/rocks-parity-floor1x/`).
  Leituras 1c já são 2–3× Rocks nesse gate. G1 leituras 1.13–1.99× com
  fd antes do Ok (`rocks-parity-floor1x-g1/`).
- Grupo em oversubscription: `kvrocks_set_mc50` 1.678× (p201q; 0223 P0.2
  diz que **não** era artefato 64 vs 256 MiB).
- Apply G1 mc4 **2.79×** — outra coluna; não cotar como same-class.
- Selo singleton (0226 P0.2): mc16 −13% recuou. WriteThread / always-seal
  **parked** (`lock_wait=0.43µs` ≪ 15%).
- Staging WAL 64 KiB default ON (0209); 1c drena (`write_pending_frame_lone`).
- Ingest SST + compaction filter caminhos nativos (0217 P1.2).
- Vlog / KV-sep já existe (WiscKey R005 no produto; limiar 4 KiB no
  harness). Hydrate 100M vs Fjall: Pedra **ganha** (123.7s vs 198.8s).

### Scoreboard vivo — perdas que este RFC conserta ou nomeia C

**Rocks same-class (cartaz ou DIAG rotulado):**

| célula | número | classe | dono (código + paper) |
|---|---|---|---|
| overwrite_mc4 25M @4 GiB | **0.557×** 3/3 Linux | **W rank 1** | `wal_write` + working set (0185/0193/0223) |
| overwrite_mc4 `_mcN` Darwin | **0.160×** DIAG 2026-09-15 (`clients=4` n=8000) | W | PHASE `wal=23.32µs` |
| ycsb_f_mc4 | 0.836 rmw / 0.780 buf / run2 0.766 25M | W | rmw + wal na lock (0211 residual) |
| apply_mc4 same-class | Darwin 0.48×; sem Linux 3-run no floor | U | 0183 teto apply; 0055 iff |
| prefix 100M @4 GiB | **0.70×** | W | 0195 aterrizado, meter blocked |
| probe_miss 100M | **0.29×** vs Rocks; ~2× pior que Fjall | W | bloom (0160 landed, **sem re-meter**); Monkey R006 |
| deps_scan @10M | p50 setup 231µs (97%) | W | L0/seed 0223 |
| write @10M PHASE | wal 50.7% + flush_check 28.8% | W | 0223 |
| kafka_changelog_flush | 0.036 min-of-3 (p211u3) | W | flush-por-op; SILK R017 se cauda |
| ingest_sst / compaction_filter | 0.069 / 0.081 (pré-nativo) | re-meter e4b | 0217 P1.2 landed |
| linkbench_mix | 0.236 | W | delete-heavy; Lethe R031 iff |
| wbwi | 0.494 | W | emulação; caminho nativo |
| myrocks_write_tx | 0.751 | W | N-updates |
| G1 1c write-per-op | 0.001–0.056× | **C** | 1 fd/op vs 0 do peer |
| kvrocks Adaptive-off n≥16 | 0.37× histórico; 1.678× paid n=50 default | **C** se CPU-bound | 0201 |

**Fjall absoluto (não ratio):**

| célula | número | classe | dono |
|---|---|---|---|
| YCSB-A 1c (1k–1M records) | 0.44–0.77× fjall | W | journal vs `write_pending_frame_lone` |
| ingest 1c | 150–300k vs 540–910k | W | mesmo |
| YCSB-C 1k×100 B | **2.19×** fjall | S | não reabrir |
| YCSB-C 64k–256k | 0.77–0.85× fjall | W | block cache / Monkey |
| YCSB-C 1M | **1.67×** fjall | S | fjall cai |
| miss-path 100M | 2.3–2.4µs vs 1.1–1.2µs fjall | W | bloom 0223 |
| hydrate 100M | Pedra 123.7s vs 198.8s | S | KV-sep |

### Papers → o que **não** copiar às cegas

| Paper | O que o texto afirma (ficha D4) | O que Pedra faz / deve fazer |
|---|---|---|
| WiscKey R005 | WA keys=10, values=1 → WA efetivo **1.14** (16 B + 1 KiB); pior se values pequenos + range longo | vlog já no produto. Não separar values de 100 B (o próprio paper recusa vitória universal). |
| Monkey R006 | FPR ∝ 1/tamanho do run; tira \(O(L)\) do lookup vazio; 50–80% latência no fork LevelDB | Bloom uniforme hoje. `probe_miss 0.29×` é o dono. Autotune \(T\) **não** é P0. |
| Dostoevsky R007 | Lazy leveling: último nível leveling, cima tiering — menos merge supérfluo | Leveled default (Rocks Experience R010: WA 16 vs tiered 4.8 vs FIFO 2.1). Trocar policy só se WA for o dono do 100M (R012 takeaway A: **não há strategy perfeita**). |
| HashKV R018 | vLog circular + Zipf: WA **19.7×** no update (WiscKey GC no tail frio) | GC do vlog incremental (0027). Iff: se PHASE/vlog mostrar WA de update ≫ load. |
| SILK R017 | p99: interferência client/flush/compact; priorizar L0→L1, preemptar alto | Não é ops/s. Só se caudas (kafka_flush, stall 408ms @10M) sobrarem após flush_check sair do commit. |
| Lethe R031 | delete-aware; tombstone persistente só no último nível | Iff linkbench/delete-heavy ainda <1× após wal_write. |
| Rocks Experience R010 | Prioridade migrou WA → space → **CPU**. Embed, um nó | CPU no write path **é** o intercepto 0193. Space-amp é 100M @4 GiB. |
| Compaction R012 | 4 primitivas; 10 strategies; trocar *pode* ganhar muito, sem auto-tuner | Não inventar a 11ª strategy no P0. |

## Problems This Solves

- **Problem:** o piso 1× 1c está pago e o board ainda perde nas células
  que um crítico corre (overwrite_mc4 0.557× Linux; `_mcN` 0.160 Darwin;
  Fjall YCSB-A 0.44–0.77×). Sem um programa, cada fire redescobre o
  grupo ou o selo.
- **Problem:** 0226 mostrou que selar cedo **baixa** `avg_group` e
  **aumenta** a taxa de `write()`. Sem \(T_{\mathrm{serial}}\) de
  memcpy-class, mais política de grupo não muda a classe Θ(1) do Rocks.
- **Problem:** 0209 está default ON e o PHASE Darwin ainda marca
  `wal=23µs` — ou o staging não está no caminho quente do grupo, ou o
  23 µs é drain/prealloc/lock. Sem A/B, 0193 e “consertar o buffer”
  ficam misturados.
- **Problem:** miss-path 0.29× vs Rocks e ~2× vs Fjall tem bloom real
  in-tree sem re-meter (0223). Monkey já diz *por que* FPR uniforme perde.
- **Problem:** “ganhar do Fjall” tratado como ratio. Fjall é absoluto;
  1c write deles é journal, o nosso 1c drena por contrato Rocks.

## Proposed Solution

1. **Scoreboard G_perf** neste RFC (tabelas acima). Cada fire flipa a
   linha no mesmo commit do número. W corta; U mede; C documenta;
   S não reabre.
2. **Onda 0 = intercepto `wal_write`.** Probe `PEDRA_WAL_BUFFER=0` vs
   default no `_mc4` real (`SUITE=ycsb` `CLIENTS=4`). Depois aterrissar
   0193 (pwrite + ticket, default off). Flip só com Linux 3-run da
   célula unpaid. Este é o único corte cujo ganho **cresce com N**
   (\((L-1)/L \cdot T_{CS}\)).
3. **Onda 1 = o que o PHASE residual nomear.** Concurrent memtable
   (0055) **iff** `lock_wait` ≥15% do gap que restar. Absorb durante o
   `pwrite` (WriteThread sem portar a lista). Re-meter Fjall 1c YCSB-A
   absoluto — se o intercepto caiu, o 0.44–0.77 mexe; senão o 1c drain
   é C nomeado vs Fjall (contrato Rocks), não um bug.
4. **Onda 2 = física LSM (papers, cada um com iff).** Monkey FPR no
   miss-path; flush_check fora do commit (0223); SILK só se p99
   restar; Dostoevsky/Lethe/HashKV só se WA/delete/vlog GC forem o
   dono medido. R012: uma strategy, não um tuner eterno.
5. Vetos permanentes: wait-to-grow; G1 1c como win; Darwin como
   Linux; Fjall como `compat_over_rocksdb`; portar WriteThread sem iff;
   staging cego em 1c (0044 + p209b).

**Alvo de produto deste RFC (não substitui o piso 0041=1×):**

- **Maioria** (write concorrente same-class + reads 1c já pagos):
  Linux min-of-3 **≥ 1.5×** Rocks default nas células W da tabela
  write (overwrite_mc4, ycsb_f_mc4, apply_mc4 same-class quando
  medida). Fjall absoluto **≥ 1.0×** no YCSB-A 1c *depois* da onda 0,
  ou C explícito (“1c drain = FlushWAL”).
- **Resto:** ≥ 1.0× Rocks min-of-3; Fjall absoluto ≥ 1.0 nas linhas
  que hoje perdem (mid working-set reads, miss-path) **ou** C.
- **C imutável:** G1 1c fd-ceiling; Adaptive-off n≫CPUs se CPU-bound.

## Delivery slices (mandatory)

### P0 — scoreboard + probe do buffer + seam 0193 (útil sozinho)

- [x] **P0.1** Este RFC + finding 2026-09-16 — status: `done`
- [x] **P0.2** A/B `PEDRA_WAL_BUFFER=0` vs default em
      `deps_cache_overwrite_mc4` (`SUITE=ycsb` `CLIENTS=4` `SYNC=0`,
      `name=deps_cache_overwrite_mc4` `clients=4`). Staging **está** no
      hot path (`wal` 29 vs 39.6 µs) mas não fecha o gap. Finding
      `2026-09-16-rfc0230-p02-wal-buffer-ab`. — status: `done`
- [x] **P0.3** Kernel 0193: `EnvFile::write_all_at` +
      `wal_ticket_kernel` (`reserve_frame`, `pwrite_off_lock`). Testes
      `rfc0193_*`. Catálogo/Lean/ledger no mesmo tree. Default off
      (`PEDRA_WAL_PWRITE=1`). — status: `done`
- [x] **P0.4** Wiring opt-in: `take_pwrite_job` (clone fd) →
      `write_all_at` fora do meta lock; byte-idêntico vs AS-IS
      (`rfc0193_pwrite_wal_bytes_match_as_is`). — status: `done`

### P1 — meter que decide default + residual da CS

- [x] **P1.1** Meter P0.4 vs Rocks `sync=false` Darwin DIAG `_mc4`:
      PWRITE 12.2k vs AS-IS 24.3k vs Rocks 117.5k (`ratio=0.104`).
      **Sem flip.** Linux unpaid. Finding
      `2026-09-16-rfc0230-p11-pwrite-meter`. — status: `done` (honest no-flip)
- [x] **P1.2** Fjall YCSB-A 1c absoluto: campanha
      `async-scale-ladder` 0.44–0.77×; pwrite é path de grupo, 1c
      drena. **C “1c drain = FlushWAL”.** — status: `done` (C)
- [x] **P1.3** Concurrent memtable iff `lock_wait` ≥15% do gap.
      AS-IS `lock_wait=0.28µs`. Parked. — status: `done` (parked)
- [x] **P1.4** Re-meter `probe_miss` 100M: não corrido neste host.
      Parked unpaid (0.29× existente). — status: `done` (parked/unpaid)

### P2 — física LSM com iff (além do intercepto)

- [x] **P2.1** Monkey FPR iff P1.4. P1.4 parked ⇒ parked. — status:
      `done` (parked)
- [x] **P2.2** `flush_check` fora do commit iff split. PHASE `_mc4`
      `flsh=0.01µs`, sem `flush_work_ns`. Parked. — status: `done`
      (parked)
- [x] **P2.3** SILK iff p99 após P2.2. Parked. — status: `done`
      (parked)
- [x] **P2.4** Uma policy LSM iff WA/delete/vlog-GC. PHASE é
      `wal_write`. Parked. — status: `done` (parked)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Este RFC | done | — | 2026-09-16 |
| P0.2 | p0 | A/B WAL_BUFFER no `_mc4` real | done | findings/2026-09-16-rfc0230-p02-wal-buffer-ab | 2026-09-16 |
| P0.3 | p0 | Kernel 0193 write_all_at + ticket | done | wal_ticket_kernel.rs | 2026-09-16 |
| P0.4 | p0 | Wiring pwrite opt-in, bytes idênticos | done | take_pwrite_job | 2026-09-16 |
| P1.1 | p1 | Meter + flip default | done (no-flip) | findings/2026-09-16-rfc0230-p11-pwrite-meter | 2026-09-16 |
| P1.2 | p1 | Fjall YCSB-A absoluto | done (C 1c drain) | async-scale-ladder | 2026-09-16 |
| P1.3 | p1 | Concurrent memtable iff lock_wait | done (parked) | — | 2026-09-16 |
| P1.4 | p1 | Re-meter probe_miss bloom | done (parked/unpaid) | — | 2026-09-16 |
| P2.1 | p2 | Monkey FPR por nível iff miss | done (parked) | — | 2026-09-16 |
| P2.2 | p2 | flush_check fora do commit iff | done (parked) | — | 2026-09-16 |
| P2.3 | p2 | SILK I/O iff p99 | done (parked) | — | 2026-09-16 |
| P2.4 | p2 | Uma policy LSM iff | done (parked) | — | 2026-09-16 |

## Acceptance Criteria

- **Tests:** `rfc0193_*` (ticket ordem, AS-IS seek/write, pwrite
  byte-idêntico, `positional_writes=false` fallback). P0.2 não é
  teste de kernel — é finding com JSON `name=deps_cache_overwrite_mc4`
  `clients=4` `sync=false`. Monkey/flush/SILK: teste nomeado no fire
  que os aterrissar.
- **Telemetry / Analytics:** nenhuma sonda default-on (0169).
  `PEDRA_WRITE_PHASE_STATS=1` e `PEDRA_WAL_PWRITE=1` são pins.
- **Documentation:** scoreboard deste RFC flipa no mesmo commit;
  0193 living table atualiza se P0.3/P0.4 forem o land dele;
  Fjall só em absoluto.
- **Screenshots:** backend-only.

## Out of scope

- Substituir o piso RFC-0041 (1×) por 2× neste RFC — 2× continua
  backlog do 0041.
- G1 1c win; peer `sync=true`; Darwin como cartaz; Rocks colapsado.
- Wait-to-grow / `PEDRA_GROUP_WINDOW_US>0`.
- Portar `db/write_thread.cc` sem o iff de `lock_wait`.
- Staging cego em 1c (0044 class-fix + p209b).
- Fjall como coluna de ratio. Quicksilver/Slipstream (job ≠ LSM embed).
- Auto-tuner eterno de compaction (R012 takeaway C: medir implicação,
  não um oráculo).
- Montanha/Raft neste programa (write local primeiro).
