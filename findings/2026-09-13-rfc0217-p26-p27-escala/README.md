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

## Próximo

1. decomp3 (a..e in-order) → confirmar reprodutibilidade do 8,77ms e
   capturar probe no estado exato pós-a..d (ord_builds / scan counters).
2. Ataque (A): drain L0 bounded no `spawn_compact_worker` +
   re-meter deps_scan/ycsb_e/mvcc_latest.
3. P2.7: flush_check 2,69µs/commit — decompor o check e atacar
   (epoch-based).

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
