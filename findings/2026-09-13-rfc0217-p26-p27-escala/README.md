# RFC-0217 P2.6/P2.7 — decomposição em escala (@10M): dono do scan e dono do write

**Data:** 2026-09-13 | **Caixa:** Darwin local (DIAG — nunca claim/cartaz) |
**Binário:** rebuild @HEAD com probes `0eb0f25e` (target scratch
`p26-target`), braço compat, coluna async `PEDRA_PARITY_ASYNC=1`,
`ROCKS_YCSB_RECORDS=10000000` (knob global — deps também escala),
quiet-gated (5-min loadavg×100 < 400), `PEDRA_WRITE_PHASE_STATS=1` na
perna write.

## P2.6 (scan-at-scale) — dono nomeado com número: dívida L0 + `SstCountCursor::settle` eager

**deps_scan @10M run 2 (quiet, binário com ssu/smg):** p50 0,2107ms.
Probe `deps_scan_probe`:

| métrica | valor | leitura |
|---|---|---|
| `setup_ns/op` (ssu) | **231,21µs** | **97% do custo** — setup, não merge |
| `merge_ns/op` (smg) | 4,75µs | merge k-way é barato |
| `sst_count` | 57 | 57 tables |
| `l0_files` | **54** | dívida de L0 (trigger=4) |
| `mem_entries` | 6.267.008 | memtable gigante pós-seed |
| `blocks_decoded` | 17.067 (**3,4/op**) | block loads reais por op |
| `sst_probed/op` | 4,23 | ~4 tables L0 por janela de 25 keys |
| cache hit/miss | 5268/17067 | 76% miss |

**Mecanismo:** janela de 25 keys sobrepõe ~4 tables L0 largas (keys
aleatórias do seed uniforme); `SstCountCursor::new` → `settle()` carrega
o **primeiro bloco no construtor** (db.rs `SstCountCursor`, dentro da
região cronometrada como setup) → 4 loads de bloco/op ≈ 230µs. A
**doença-raiz é a dívida de L0**: o worker de compaction (rocksdb-compat
`spawn_compact_worker`) pula o drain L0-at-trigger sempre que
`commit_inflight > 0` ou `recently_multi` — sob seed sustentado
single-writer o L0 cresce sem limite (54 files neste run; 773 no run
write10m abaixo).

**ycsb_e in-suite @10M:** colapso a 8,77ms p50 (0,001× vs peer) NÃO é
escala pura — fresh (ycsb_e sozinho, mesmo @10M) mede **9,2–9,5µs p50**
(qps 98,7k). É estado pós-legs a–d. `decomp3` (a..e in-order, mesmo
processo) em andamento para reproduzir; hipótese viva adicional:
rebuild do point_ord invalidado em bloco pelos puts de a/d (contra-evida:
probe fresh deps leg → `ord_builds=0`; falta probe pós-leg ycsb).

**Ataques ranqueados:** (A) drain L0-at-trigger **bounded** mesmo sob
`commit_inflight` (1 `compat_compact_once` por tick em vez de
`continue`) — mata a dívida na origem; (B) lazy cursor head — REJEITADO
(k-way min-head precisa de todos os heads); (C) bloom de table —
REJEITADO (tables L0 largas contêm keys aleatórias); (D) = (A)
estrutural: compactar L0 durante ingest.

## P2.7 (write-at-scale) — decomposição PHASE da célula g1 @10M single

**write10m (ycsb_a + deps_mvcc/raftlog/cache_overwrite @10M, PHASE on,
10.629.512 commits):**

| fase | µs/commit | % do serial |
|---|---|---|
| prepare (lock+stage+encode) | 0,096 | 1,0% |
| **wal (write() off-lock)** | **4,74** | **50,7%** |
| mem (apply) | 1,55 | 16,6% |
| publish | 0,06 | 0,6% |
| **flush_check** | **2,69** | **28,8%** |
| lock_wait | 0,00 | — |

- **wal 4,74µs/commit** = o voo async medido (syscall `write()` por
  commit; p/ referência do P0.4: Darwin está ABAIXO da fatia de
  quiescência 20µs → janela-≤-voo colapsa a off aqui por construção;
  no ext4 do p149 é o dono do mc4 — p201r2: write() por op no bypass).
- **flush_check 2,69µs/commit = dono novo**: 31% do caminho de escrita
  em escala que não existia no sweep 1M (a validação de débt/L0 por
  commit escala com o estado). Ataque P2.7: baratear o check
  (epoch/versão de imutabilidade em vez de varredura por commit).
- `deps_raftlog` phasesΔ (split, primeiros 7 commits pós-rotação):
  prepare=0,34 wal=**46,13**(!) mem=3,36 — rotação de WAL paga
  preallocate/first-write caro; steady-state volta a 4,74.
- `deps_cache_overwrite` p50 6,7µs mas **max 408ms** (stall de flush —
  válvula write_pressure L0 com 773 files em voo); qps cai por stall,
  não por op.
- Probe `mvcc_latest_split`: `mvcc_ns_last`=153,6ms/3000 ops =
  **51,2µs/op** no "latest" com 773 L0 — a mesma dívida L0 do P2.6
  batendo no caminho point/get (get_sst_fallback 4272, 60% miss mem).
  `ord_builds=0` (contra-evida p/ hipótese point_ord no ycsb_e).

## Arquivos

- `scan10m/` — deps_scan run1 (load alto) + run2 (quiet) com probes
- `write10m/` — perna PHASE (JSON + log com WRITEPHASE totals)
- `ycsbe10m/` — fresh ycsb_e @10M (9,5µs p50)
- Logs brutos: scratch `p26-decomp2/` (copiados aqui)

## Próximo (rev. 3)

1. ~~decomp3 (a..e in-order)~~ — cancelado (usuário matou a task; o
   dono já está nomeado pela rev.2/rev.3).
2. ~~Ataque (A) drain L0 bounded~~ — refutado pela rev.2 (solo-async
   auto-drena; o dono era o flush do seed adiado, atacado pelo settle).
3. **P2.1 (retarget 2026-09-14):** lazy-first-block refutado (k-way
   precisa dos heads). Dono = settle que só dormia. **Landed:**
   `DB::compact_l0_once` + settle do bench drena L0 ativamente
   ( Pedra ≡ Rocks `wait_for_compact`).
4. **P1.1 RFC-0223: flush fora do commit** (worker bounded) — split
   rev.3: work 99,85% × gate 58ns.
5. p26r3b-mc50x (3-arm) decide a pergunta do mc50 no DIAG; oficial =
   gate Linux.

**Veredito (P2.6):** dono = dívida L0 (worker pula drain sob
commit_inflight) × settle eager do SstCountCursor (setup 97%).
**Veredito (P2.7):** dono = wal write() 4,74µs (async, esperado —
ataque é o P0.4/grouping no meter oficial) + flush_check 2,69µs
(ataque novo, fatia própria). DIAG Darwin; ratios oficiais = e4b.

## Correção rev. 2 (2026-09-13, mesmo dia): o dono P2.6 não é o
skip `commit_inflight` — é o trabalho de flush do SEED adiado para
dentro da janela medida

Probe de refutação (`rfc0217_p26_l0_debt_probe`, release, Darwin,
solo-async via `put` 4KiB, 1.194.927 ops ≈ 4,6 GiB em ~13 s):

```
during: l0 oscila 0→4 (max 4), parked=0, inflight=0 SEMPRE, multi=false
idle:   l0 1→0 em ≤1,5 s
```

- O bypass async (solo) **não incrementa `commit_inflight`**; o
  lone-sync segura o Db write lock pelo fdatasync (inobservável via
  `with_read`); grupo contínuo ⇒ `recently_multi` ⇒ o guard já existia.
  Ou seja: o skip quase nunca é o portão que mata o drain — o regime
  solo **se auto-drena** (branch RFC-0039 P2.2).
- Dono corrigido: o seed dos deps (10M records × 2 versões, flush por
  CF a 256 MiB) deixa L0 files não compactados — Rocks compacta durante
  o próprio seed (background), Pedra parka/defere e os 54 files estão
  vivos nos PRIMEIROS ops do scan (setup 231 µs/op × 4 tables/janela).
  O bench não tinha settle entre seed e fase medida.
- Fix no harness (mesmo commit desta rev.): **settle pós-seed default**
  (`ROCKS_PARITY_SETTLE=0` para A/B): compat = `flush()` + espera
  bounded 30 s até `l0 < L0_COMPACTION_TRIGGER`; rocks = `flush()` +
  `wait_for_compact`. E **simetria de memtable**: o lado Rocks agora
  honra o mesmo `ROCKS_PARITY_COMPAT_MEMTABLE`/256 MiB default (antes
  ficava no default 64 MiB — toda suíte >64 MiB flusheava Rocks dentro
  da janela e não o Pedra: assimetria pró-Pedra, ex. kvrocks_set_mc50
  ~100 MiB).
- O `settle` eager do `SstCountCursor` (1º bloco no construtor)
  segue real (3,4 blocks/op, 76% miss) — segunda metade do dono, ainda
  aberto.
- Pendente: re-meter deps_scan/ycsb_e @10M com settle (DIAG) e no gate
  Linux; kvrocks_set_mc50 1,678 fica **config-suspeito** até re-run
  com a simetria.

## Correção rev. 3 (2026-09-13, pipeline `p26r3`): settle funciona
## mecanicamente mas o dono do setup SOBREVIVE — é a largura das
## tables, não o nível

Pipeline v3 (`p26r3`, braços A/B consecutivos sob load externo ~13,
sem espera-quiet; DIAG — p50s de janelas de 40–70ms sob load espetado
são incomparáveis, os counters mecânicos abaixo é que fecham):

| braço | l0 | mem_entries | tables/op | blocks | setup share |
|---|---|---|---|---|---|
| settle-ON (**INCOMPLETO**: deadline 30s sob load) | 14 | **0** | 3,6 | 726 | 98,7% |
| settle-OFF | 47 | 6.265.728 | 4,2 | 762 | 97,0% |

- **Settle funciona**: `flush()` esvazia o memtable (6,27M→0) e o drain
  reduz L0 54→14 — mas **não completa** sob load 13 (log: "deps settle
  INCOMPLETE after 37.1s — debt carries into the timed window";
  `engines.rs` deadline 30s). Em máquina quieta/gate completa.
- **O dono estrutural sobrevive ao settle**: tables sondadas/op e
  blocks decodificados quase não mudam entre braços (3,6 vs 4,2; 726
  vs 762). A contagem de tables sobrepostas por janela de 25 keys é
  função da **largura das tables** (seed uniforme → toda table L0 OU L1
  cobre o keyspace inteiro), não do nível. O `SstCountCursor` continua
  pagando o 1º bloco por table sobreposta (~3,5/op) dentro do setup.
  ⇒ **P2.1 (lazy-first-block / head-by-index) fica justificado por
  dados** — a condição do RFC-0223 ("se o setup continuar dono
  pós-settle") está cumprida no lado estrutural.
- **P2.7 split (WRITEPHASE `7f2758d4`, seed 625k commits = 10M keys ×
  16/commit, 8 flushes × 256MiB ≈ 2GiB):**
  `flush_check` total 24.740,9ms = **flush_work 24.704,6ms (99,85%)** +
  **gate puro 36,3ms = 58ns/commit**. O 2,69µs/commit do write10m era
  **flush work raro diluído** (8 eventos de ~3,1s), não um gate caro.
  ⇒ ataque P1.1 = **mover flush para fora do commit** (worker bounded,
  interface parked-debt RFC-0216), NÃO epoch no gate (58ns não paga).

  **Diagnóstico P1.1 rev.2 (arqueologia pós-split): o "flush work"
  in-commit NÃO é I/O de SST.** O bench abre por
  `rocksdb_compat::DB::open_cf` (engines.rs:35) que seta
  `defer_auto_compact(true)` (lib.rs:2086) e sobe compact + flush
  workers (lib.rs:2089) — a escrita de SST já é off-commit. Por
  eliminação (stage_flush_imm é O(1); await_flush_debt vive antes do
  submit, fora do maybe_auto_flush), os 8×3,1s são o
  **`take_family`** (ramo physical-CF: `push_parked_unflushed(taken)`,
  db.rs:11078): partição da memtable por família — O(n) alocando ~MiB
  de nós BTree **sob a write-lock** (~2–3M entries/evento). O caminho
  O(1) já existe no ramo global: swap via `stage_flush_imm` (db.rs:6121)
  + o worker particiona/materializa. **Ataque landed (RFC-0223 P1.1,
  2026-09-14):** kernel `dominant_family_stage_plan` (fam ≥ 3/4 ⇒
  `StageWholeMem` via `stage_flush_imm`; abaixo ⇒ `PartitionFamily`;
  imm ocupado cai na partição). Integração no ramo defer de
  `maybe_auto_flush`. Re-meter DIAG `flush_work_ms` / cartaz = e4b.
- mc50 A/B 1ª passada **inconclusiva** sob load (compat 37k–236k qps
  entre rounds; braço Rocks 64MiB consistentemente mais rápido que
  256MiB — 151/156/168k vs 107/142/101k — sinal direcional de que o
  shape antigo era MAIS difícil pro Pedra, não mais fácil); rerun
  3-arm intercalado `p26r3b-mc50x` rodando.
- **mc50x (rerun 3-arm ×7 intercalado, load 11–12, ranges ±10%) FECHOU
  a pergunta**: compat max 247,7k/med 215,6k; rocks256 max 180,9k/med
  170,2k; rocks64 max 180,0k/med 165,1k ⇒ **rocks256 ≈ rocks64** (o
  "64MiB mais rápido" da 1ª passada era espeto de load) e compat ganha
  **1,267–1,370× (sym) e 1,306–1,376× (asym)** — mesma vitória nas
  duas configs, **sem artefato de config**. Suspeita sobre o cartaz
  1,678× Linux rebaixada; oficial = gate (e4b). DIAG Darwin.
- Artefatos: `scan10m/p26r3-{settle,nosettle}/` + logs `scan10m-
  {settle,nosettle}.log`.

**Veredito P2.6 rev.3:** settle = higiene necessária (mata dívida de
memtable e o grosso do L0) mas **não suficiente** — o dono do setup
(largura de table × 1º bloco eager) persiste em qualquer nível;
veredito de p50 oficial continua no gate. **Veredito P2.7 final:**
dono = flush work in-commit raro (99,85%) × gate 58ns; ataque =
flush off-commit.

## Correção rev. 4 (2026-09-14, pipeline `p223`, HEAD P1.1+P2.1)

DIAG Darwin — nunca cartaz. Peer `ROCKS_PARITY_SYNC=0`.

### P2.1 settle ativo — COMPLETE

`deps settle 37.1s (L0 drained)` (p26r3 era INCOMPLETE L0=14).

| | p26r3 settle-ON (poll) | p223 settle-ON (`compact_l0_once`) |
|---|---|---|
| l0 | 14 | **0** |
| level1 | 12 | 17 |
| sst_count | 28 | 22 |
| tables/op | 3,6 | **2,0** |
| blocks/200ops | 726 | 441 |
| p50_ms | 0,2814 (load) | **0,1493** |
| setup share | 98,7% | 97,9% |

Drain funciona. Setup continua dono (~98%) com 2 tables/op — janela de
25 keys ainda paga 1º bloco por table sobreposta. Cartaz = e4b.

### P1.1 stage O(1) — NÃO colapsou `flush_work` (imm ocupado)

WRITEPHASE seed 625k commits / 8 flushes: `flush_check_ms=22128.1
flush_work_ms=22096.3` (99,85%, **2,76s/evento**) — mesmo dono. Causa:
`stage_flush_imm` retorna false quando `imm` está ocupado (flush
worker atrás no seed) e o trampolim caía no `take_family` O(n).
**Fix no mesmo commit:** família dominante + imm ocupado estaciona a
memtable inteira na fila parked (O(1) `mem::replace`), nunca
`take_family`. Teste `rfc0223_dominant_family_stages_whole_mem`
atualizado.

### qs_neg_lookup 100k ×3 intercalado (família miss-path)

| r | pedra qps | rocks qps | ratio | p50 µs |
|---|---|---|---|---|
| 1 | 3.670.561 | 1.785.908 | 2,055 | 0,3 vs 0,5 |
| 2 | 3.618.703 | 1.820.616 | 1,988 | 0,3 vs 0,5 |
| 3 | 3.760.135 | 1.819.498 | 2,067 | 0,3 vs 0,5 |

**min=1,988 med=2,055.** DIAG 100k, **não** paga `probe_miss` 0,29× @100M
(RFC-0161). Bloom real rejeita miss neste tamanho. Oficial = gate.

`deps_cache_overwrite_mc4` **não rodou**: `ROCKS_PARITY_ONLY` no
`run_deps` 1c não seleciona o shape `_mc4` (saiu `apply_batch_mc4` +
`raftlog_mc4`). Rank-1 overwrite continua o número Linux 0,557×.

Artefatos: `p223/qs/r{1,2,3}-{compat,rocks}.json`, `p223/scan/`.
