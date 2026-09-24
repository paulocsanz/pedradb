# RFC-0044: ≥ **5×** RocksDB **async** na mesma classe (não é o cartaz G1)

**Status:** in-progress
**Updated:** 2026-08-30 (async contract corrected — per-commit `write()`, see "Correction 2026-08-30" below)
**Parked (quiet remesure):** remaining 5× async slices need a quiet 3× host; dirty sandbox is not the official floor.
**Parents:** [0041](0041-2x-rocks-default.md) (cartaz = Pedra G1 vs Rocks `sync=false`),
[0043](0043-high-level-2x-expanding-benches.md) (catálogo que só cresce),
[AGENTS.md](../../AGENTS.md)

## Background

- O **cartaz de produto** não muda: Pedra `fdatasync` antes do Ok vs Rocks
  default `WriteOptions.sync=false`. 1-op put nessa coluna é teto `1/t_fd`.
- Pedido extra: na **mesma classe** (os dois sem fsync no Ok), cada shape
  async ≥ **5×** o Rocks async. Isso mede o LSM/CPU, não o syscall de
  durabilidade.
- Knob: `PEDRA_PARITY_ASYNC=1` no binário **compat** (e
  `set_write_sync(false)` nas suítes cujo host já é async: kvrocks, ycsb,
  deps, qs, nebula, …). Default de `OpenOptions.sync` continua `true`.
- Compare JSON: `compat.sync=false` + durability
  `async-wal (PEDRA_PARITY_ASYNC=1; … NOT G1, not official)`.
- Lab sujo
  ([rfc0043-pedra-async-dirty](../../findings/rfc0043-pedra-async-dirty/)):

Contrato async = encode + `write()` aos **64 KiB** (Rocks file writer),
sem `fdatasync`. Lab sujo `findings/rfc0044-p1/kvrocks-64k/` + `ycsb-64k/`.
Acked em userspace 1 MiB = **inválido**. `write()` em todo o put era mais
estrito que o Rocks e empatava o qps.

## Correction 2026-08-30 — async contract is per-commit `write()` (class fix)

Two claims registered above are wrong (found during the public-repo
durability audit; full record in
`docs/rocksdb-vs-pedradb-guarantees.md` §2.5):

1. **"`write()` em todo o put era mais estrito que o Rocks" — false.**
   RocksDB default (`manual_wal_flush=false`, `include/rocksdb/options.h:1341`
   v9.4.0) flushes the WAL to the OS **per record**
   (`db/log_writer.cc:187-191`, `if (!manual_flush_) dest_->Flush()`).
   Per-commit `write()` is the same class, not stricter.
2. **"empatava o qps" — does not reproduce.** Local A/B on `lone_async_1c`
   (`fsync_amortization`, dirty macOS box, 2000 puts, 3 runs/side):
   64 KiB staging ≈ 545k ops/s vs per-commit `write()` ≈ 311k — the
   staged buffer bought ~1.75× (~42%) on the single-client write-per-op
   shape, and left acked bytes userspace-resident on process crash.

**Decision (user, 2026-08-30): fix the guarantee, not the disclaimer.**
`ASYNC_WAL_BUFFER` staging is deleted from the WAL; every async commit
path (`commit_async_ops`, `commit_async_one`, both group paths) calls
`Wal::write_pending_frame()` — encode + `write()` — before `Ok`. Async
is now process-crash-equivalent to RocksDB default (power loss still
loses on both; G1 is the fsyncing column). P0.4 is reverted by this
change; the ratio tables measured with staging are stale until the CHV
re-measure of this column.

**CHV outcome (2026-08-30,
`findings/2026-08-30-linux-p149-async-classfix-chv/`):** with the class
fix, the same-class column measures **8/17 ≥ 3×, min 0.94
(deps_raftlog)** vs the staging-era P1.8 hold (12/17, min 1.054). Reads
unchanged; write shapes down 1.5–2×. The RFC-0041 floor (1.0) is
breached by deps_raftlog — open product decision (re-baseline / recover
raftlog / revert); the fix stays on explicit user order.

| shape | Pedra | Rocks | ratio | ≥5×? |
|---|---:|---:|---:|:---:|
| `kvrocks_scan` | 1.02 M | 20 k | **50.0** | **sim** |
| `kvrocks_pipelined_set` | 131 k | 12 k | **11.3** | **sim** |
| `kvrocks_get` | 3.53 M | 0.89 M | 3.97 | não (peer GET baixo) |
| `kvrocks_set` | 717 k | 186 k | 3.86 | não |
| `ycsb_e` | 346 k | 89 k | 3.89 | não |
| `kvrocks_set_mc50` | 200 k | 59 k | 3.38 | não |
| `ycsb_a` | 757 k | 347 k | 2.18 | não |
| `ycsb_d` | 1.61 M | 840 k | 1.92 | não |
| `ycsb_c` | 2.75 M | 1.48 M | 1.86 | não |
| `kvrocks_blob_set` | 50 k | 31 k | 1.62 | não |
| `ycsb_f` | 295 k | 239 k | 1.24 | não |
| `ycsb_b` | 1.23 M | 1.01 M | 1.21 | não |

Melhor janela da sessão (load ~14, `kvrocks-l14/`): SET **5.66**,
mc50 **9.24** (peer doente), scan 14.0, GET 4.22, blob 3.03,
pipeline **0.96** (cauda de `write()` no disco sujo; p50 segue 5× melhor).

- Mecânica já na árvore: `commit_async_ops` faz **`write()` antes do Ok**,
  sem `fdatasync` (não acked em userspace); compact worker **não** acorda
  por put; CF `default` raw se a suíte não precisa de CFs nomeadas;
  memtable do bench 256 MiB (sem flush na janela medida);
  WAL v2 omite payload internado repetido *dentro do mesmo batch*;
  `insert_many`; `get_probe` sem `to_vec`.

## Problems This Solves

- **Problem:** o 1-op put vs Rocks async misturava G1 com “o motor é
  lento”. Sem coluna same-class o buraco do `fdatasync` esconde o CPU.
- **Problem:** write-group + catch-up sem fsync só adiciona espera
  (`set_mc50` ~0.5–0.7). Rocks é N threads + um lock.
- **Problem:** pipeline já >1 mas ≪5× — 32× memcpy/BTree vs
  `WriteBatch` C++.
- **Problem:** sem RFC, `PEDRA_PARITY_ASYNC=1` parece default de produto.

## Como fechar 5× (mapa, não wish)

Piso = `Pedra_qps / Rocks_qps` na **mesma** run quieta, peer `sync=false`.
Não usar Rocks doente. Números abaixo = Pedra vs Rocks saudável (`kvrocks/` + x5b ycsb).

| shape | hoje (kvrocks3 vs Rocks saudável) | Pedra precisa | o que falta |
|---|---:|---|---|
| scan | **50×** | — | KeyOnly + TLS; fecha |
| pipeline | **11.3×** | — | intern+v2+64 KiB; fecha |
| SET 1c | **3.86×** | +30% | same-run sujo `kvrocks-merge/` **6.91** (1.47 M vs 212 k) — arbitro é a quieta 3× (P2.1) |
| mc50 | **3.38×** | +48% | merge de líder testado: **0.19× vs bypass** (A/B 5 rounds, `kvrocks-merge/`); handoff de lock é o teto |
| blob | **1.62×** | 4×16 KB no buffer | same-run `kvrocks-merge/` 2.30 — copies de 16 KB dominam |
| A / F | 2.18 / 1.24 | put+get | F ainda RMW |

Não fecha (e não se mente):

- 5× GET **copiando** 1 KB para o cliente a 5.5 M qps (~5.5 GB/s) — o canary é lookup (`get_probe`), como Rocks `get_pinned`.
- 5× 1-op **G1** vs Rocks async — teto `1/t_fd` (RFC-0041).
- Merge de escritores async num líder único — A/B pareado
  (`findings/rfc0044-p1/kvrocks-merge/`): **0.19× vs bypass**. Em 50
  threads / 12 CPUs o líder é ponto único de agendamento. O código fica
  atrás de `PEDRA_ASYNC_GROUP=1` (default off) para reteste em caixa
  quieta; o produto é o bypass (N threads + um write lock, formato Rocks).
- Arena / skip-list C++ — só se coalesce+harness saturarem BTree (out of scope).

## Proposed Solution

1. **Duas colunas, dois cartazes.** G1 vs Rocks default = RFC-0041.
   Async vs async = **este RFC**, piso **5.0**, nunca vendido como “batemos
   o Rocks.”
2. Async 1-op / batch: `commit_async_ops` (lock, encode, mem, stage WAL).
   Group commit fica para quem `do_sync=true`.
3. WAL async = Rocks `sync=false`: encode no frame, `write()` aos
   **64 KiB** (`ASYNC_WAL_BUFFER` = `writable_file_max_buffer_size`).
   Não 1 MiB. Não ops por encode. Tail &lt; 64 KiB até o próximo flush /
   close. Sem `fdatasync`. Blocos físicos continuam 32 KiB.
4. Gate: `ROCKS_PARITY_RATIO_FLOOR=5` nas shapes da coluna async quando
   `PEDRA_PARITY_ASYNC=1`. Compare recusa misturar G1 com “5× oficial.”
5. O que já ≥5× **fica** no relatório; o que falta entra como `todo`,
   não some.

## Garantias invariáveis

| # | Garantia | Este RFC |
|---|---|---|
| G1 | `fdatasync` antes do Ok **no default** | intocada; só o knob de bench |
| G6 | sem thread **no core** | intocada (worker compact já é compat) |
| G8 | peer oficial = Rocks `sync=false` | o cartaz 0041; esta coluna é extra |
| — | shape existente não some | **0043** |

## Delivery slices (mandatory)

### P0 — coluna existe + mc50 ≥ 5× (já no código)

- [x] **P0.1** RFC + status viva (este doc) — status: `done`
- [x] **P0.2** `PEDRA_PARITY_ASYNC=1` + `set_write_sync` no compat;
      WAL `write_pending_frame`;
      teste `async_puts_are_in_wal_without_fsync` — status: `done`
- [x] **P0.3** `commit_async_ops` (sem write-group no async 1-op/batch);
      compact não notifica por put — status: `done`
- [x] **P0.4** buffer async 64 KiB (Rocks file writer), não 1 MiB —
      status: `reverted` 2026-08-30 (class fix: per-commit `write()`,
      see Correction above; acked bytes must not sit in userspace)
- [ ] **P0.5** `kvrocks_set_mc50` ≥ 5.0 async/async — status: `doing` (parked: quiet remesure; 2.02 vs peer são)
      (**veredito quieto: 2.02** vs peer são 152 k — os 3.4–9.2
      anteriores eram Rocks doente sob carga (55 k); merge rejeitado
      0.19×; o gap é outro mecanismo, não handoff de grupo)

### P1 — o resto do kvrocks ≥ 5×

- [x] **P1.1** `kvrocks_pipelined_set` ≥ 5.0 — status: `done`
      (**FECHA rearm7 quieta 2026-08-23, `findings/2026-08-22-rearm7/`,
      HEAD d343044, gate load<10 ×2, pernas 8–9**: rounds
      **5,10/6,04/5,45 — 3/3 ≥5**, med 5,67; peer 48/40/43 k estável.
      Fecha pelo padrão 3/3-em-toda-condição. Histórico:
      reaberto pelo árbitro quieto: **straddle 4.22–5.86** entre
      janelas — 11.3 @ 64 KiB e 5.86 @ 2M load ~100, mas 4.22 @ 2M
      quieto; p50 4.0 µs vs 23 µs segue ~6× melhor. Não é ≥5 estável.
      **Renovação 2026-08-22 (P0.4 clean, 3× longa quieta, árvore
      commitada)**: 4,95/5,38/6,24 — **mediana 5,38 ≥5 alcançada**,
      mas um run na linha; curta 5,04 med. Não é 3/3 — segue `doing`.
      **Re-arm 2026-08-22 (`findings/2026-08-22-p13-rearm/`, commit
      312e354, load subiu a 12–15 mid-run — evidência)**: rounds
      2,33/5,34/5,31 (r1 compat 105k vs 235k/224k nas outras — perna
      anômala), longa **5,00 exato** (compat 187k ≡ 186k da quieta
      P0.4; peer 37,4k vs 34,7k — peer ~8% mais rápido). Compat
      inalterado pelo fix P1.3 (esperado — batch writes); o alvo
      continua straddle na linha)
- [ ] **P1.2** `kvrocks_set` / `kvrocks_blob_set` ≥ 5.0 — status: `doing` (parked: quiet remesure)
      (**rearm8 quieta 2026-08-23, `findings/2026-08-23-rearm8/`**: SET
      **4,40/4,68/4,48 — 3/3 <5** (rearm7: 5,14 med mas 2/3; straddle real,
      mediana agora abaixo); blob **1,64/1,79/1,70** — saiu de 0,99 para
      ~1,7 (p50 compat 0,0 vs rocks 10,2 µs; máx 11–14 ms é o próprio
      rocks). Nenhum fecha.
      (**2026-08-23 WiscKey wiring:** `set_enable_blob_files` /
      `set_min_blob_size` no longer no-ops — spill ≥4 KiB to `VALUES.vlog`.
      Vlog `append` was `sync_all` per record (would tank async); now 64 KiB
      `write()` buffer, G1 one fsync/commit before the WAL pointer, async
      no `fdatasync`. Parity harness default `min_blob=4096`
      (`ROCKS_PARITY_MIN_BLOB=0` restores inline). A/B dirty
      `findings/2026-08-22-tcg-vs-pmu/blob_ab.txt`: blob 16 KiB
      **9.1 k → 11.2 k qps (+22%)**; GET 1 KiB unchanged (~33 M); SET
      paired A/B 103 k vs 94 k (disk dirty, not a spill — 1 KiB < 4 KiB).
      **Does not close 5×**: `sample` still shows `write()` of 16 KiB
      dominates; vlog moves the payload out of the WAL record, not off
      disk. Blob 5× remains physics unless Rocks BlobDB is the peer.)
      (**rearm7 quieta 2026-08-23**: SET **5,20/5,74/5,42 — 2/3, med 5,14**
      (r1 4,20); blob **0,99 med** vs peer são. SET na linha, blob longe.
      SET **cruza na quieta longa: 5.41** @ 2M quieto, p50 0.4 µs vs
      2.5 µs; curta 4.61. Blob **2.02** quieto — longe; copies de 16 KB
      dominam. **Renovação 2026-08-22 (P0.4 clean, 3× longa quieta)**:
      SET **5,45/5,18/16,47 — 3/3 ≥5, mediana 5,45** (o 16,47 é run
      com rocks deprimido); curta 4,70 med; blob 2,68 med — blob segue
      longe, slice continua `doing` por ele. **Re-arm 2026-08-22
      (`findings/2026-08-22-p13-rearm/`, load 12–15)**: SET longa 4,65
      (compat 1,97M→1,61M, peer plano — assinatura de load; a quieta
      re-armada decide), rounds 4,44/4,56/4,60; blob 1,47–1,80)
- [x] **P1.3** `kvrocks_get` ≥ 5.0 — status: `done`
      (**rearm8 quieta 2026-08-23, `findings/2026-08-23-rearm8/`, `wal-prealloc`
      6c49252**: **5,465/6,074/5,710 — 3/3 ≥5, med 5,58 — FECHA**; a rodada
      que faltava no rearm7 chegou com folga; peer estável 2,37–2,44 M,
      compat 13,3–14,7 M. Bateria OFICIAL: gate 2× load<10, watchdog pico 8,
      12 pernas load 7–8, peers `sync:false` `rocks-default`.
      **rearm7 quieta 2026-08-23**: **5,96/6,06/4,86 — med 5,96, 2/3** (r3
      compat 15,4 M→12,1 M, peer estável); pelo padrão 3/3 **não fecha**,
      mas é a 1ª bateria quieta com mediana ≥5 — próxima bateria decide.
      Histórico: straddle: 20 M ops 4.4–5.5; 2 M load ~100 **5.50**; 2 M quieto
      **4.61** com Rocks são a 2.5 M — p50 0.0 µs vs 0.4 µs. Não é ≥5
      estável; janela curta 1.61. **Renovação 2026-08-22 (P0.4 clean,
      3× longa quieta, árvore commitada)**: 4,74/4,71/4,71 — 3/3
      tight; o straddle é **real, não load**; curta 3,23 med.
      **Engenharia 2026-08-22 (`findings/2026-08-22-p13-get-tls-cache/`)**:
      perfil `sample` do loop GET → a TLS `LastGetTable` errava ~85%
      (1024 slots × probe-2 a load 1.0 = thrash de evicção; a leitura
      caía no `AnswerCache` ~20 ns/op) e o `prepare()` limpava 1024
      slots a **cada write publicado**. Redesenho: época por slot
      (invalidação O(1)), 2048 slots × probe-8, inserção prefere slot
      stale, `get()`/`contains()` compartilham a tabela, `cache_epoch_base`
      (fix C1/C1b, mesma mecânica da árvore paralela RFC-0048) fecha o
      cross-instance. A/B mesma caixa: GET 200 M 11,06→15,80 M qps
      (**+43%**), hit 95,6%; ycsb a/c/f **+51/57/61%** (fim do clear
      O(N) por read-after-write). Projeção vs peer ~400 ns: ~6,0×.
      **Re-arm 2026-08-22 (`findings/2026-08-22-p13-rearm/`, bateria
      P0.4-método no commit)**: rounds **5,98/6,28/6,46 — 3/3 ≥5**
      (compat 7,6–7,8M → 14,2–15,2M +90%, peer estável 2,3–2,4M) e
      longa **7,04**. Porém o gate quieto passou a 8,85 e o load subiu
      a 12–15 no meio da bateria — pela norma P0.4 é **evidência, não
      recorde oficial**; composição conservadora só de números
      controlados (compat quieta 11,8M × fator A/B 1,30 ÷ peer quieta
      2,49M) dá **≥6,1×**. **Re-arm 4 (2026-08-22,
      `findings/2026-08-22-p13-rearm4/`)**: rounds **6,00/6,30/6,66 —
      3/3 ≥5, med 6,30** (compat 14,9–15,2M, peer 2,36–2,48M estável)
      e longa 2M **6,49**; watchdog flagou load 10,79 mid-battery →
      **evidência (terceira confirmação suja consecutiva)**. **Re-arm 5
      (2026-08-22, `findings/2026-08-22-p23-rearm5/`, HEAD `28a3d59`
      com o write-path fix)**: rounds a load 7,7–8,5 (sob a barra):
      **6,18/7,07/6,70 — 4ª confirmação 3/3 ≥5**; a longa desta bateria
      rodou sob load 12–15 e é inválida (watchdog 14,86). Fechamento
      oficial aguarda caixa quieta (rearm6, sem mudanças de código)

### P2 — YCSB + quiet 3×

- [x] **P2.1** Remesura 3× quieta `findings/rfc0044-p2/` — status: `done`
      (**árbitro executado a load 9.4**: fecham ≥5 consistentes **E
      (10.5–12.8)** e **scan (7.4)**; SET cruza na longa quieta (5.41);
      GET/pipeline straddle 4.2–5.9; mc50/blob ~2.0; F 1.66; A–D
      2.9–3.8; `deps_lock_prewrite` 0.94. Piso ≥5 para todos **não**
      alcançado — registrado sem maquiagem em `rfc0044-p2/quiet/`)
- [ ] **P2.2** ycsb A–F ≥ 5.0 na coluna async — status: `doing` (parked: quiet remesure)
      (**rearm8 quieta 2026-08-23, `findings/2026-08-23-rearm8/`**: **E
      5,18/5,38/5,49 — 3/3, 2ª bateria quieta consecutiva (med 5,44)** —
      consolidado; **D 4,69/4,56/4,84 — 3/3 <5**; **A 3,47/4,48/4,64** — o
      artifact r1 do rearm7 **sumiu** (máx r1 0,078 ms com a pré-alocação
      WAL; era stall de extent 3,1 ms), r1 segue ~30% abaixo de r2/r3
      (warmup, não stall), med 4,48; B 3,33 / C 4,23 / F 3,80.)
      (**rearm7 quieta 2026-08-23**: **E re-confirma 5,08/5,75/5,38 — 3/3**;
      D **5,19/5,42/4,72 med 5,19 (2/3)**; A r2/r3 **5,13/4,93** com r1
      =0,43 artifact (stall 3,1 ms *dentro* da perna — ver
      `findings/2026-08-22-rearm7/`: extensão de extent APFS a cada 8 MiB
      de WAL, mesmo mecanismo do deps_raftlog 0,35×; fix pré-alocação WAL
      em curso); B 3,91 / C 4,33 / F 3,69. Histórico:
      quieto 3×: **E fecha 10.5 med (3/3 ≥5 em toda condição) — P2.2
      parcial E fechado**; A 2.88 B 2.93 C 3.84 D 3.02 F 1.66 não
      fecham no wall (p50/p99 do F 1.5×/22× melhores). Cliff do E no 2M
      mapeado e fixado no produto (CountCache); `auto_reclaim` opt-in.
      **Nota (2026-08-21, RFC-0046 P0): o cliff do E também fecha
      estruturalmente por retention — o default do kernel agora é
      `Window(24 h)` bounded + archive (versões velhas saem do SSD,
      scan frio volta a O(live set + janela)), não só via CountCache
      no caminho quente. Re-árbitro oficial: RFC-0046 P0.4.**
      **Fechado pelo P0.4 (2026-08-22, `findings/rfc0046-p04/clean/`):
      sem regressão do default novo — controle `04c7aa2`≡`edfa132`
      prova o lado compat estável no arco todo; E async 5,88 med
      (3/3 ≥5) contra um peer ~1,8× mais rápido que 20/ago.**
      `ycsb-longwindow/` + `rfc0044-p2/quiet/`.
      **Re-arm 2026-08-22 (`findings/2026-08-22-p13-rearm/`, commit
      312e354 — load 12–15, evidência)**: **F 3,33 med** (era 1,40;
      compat 0,78M→1,54–1,80M com peer estável — o fix P1.3), **A
      3,65 med** (compat 1,9M→2,4–3,0M; r2 0,61M vítima de spike);
      E 4,94 e C 3,95 caem −15–17% no lado compat com peer plano —
      assinatura de load, não do fix (A/B controlado: c +57%);
      oficiais aguardam a bateria quieta)
      **Write path (2026-08-22, `findings/2026-08-22-writepath-fold-gc/`)**:
      perfil do ycsb_a achou o fold do park em O(n²) (`Vec::insert(0)`
      por versão em chave quente) queimando um núcleo inteiro + versões
      sem GC (RSS 15,7 GB p/ ~100 MB vivos). Fix: `VecDeque` +
      absorb oldest-first + GC por floor de snapshot-list (compat ON,
      paridade rust-rocksdb; core OFF). A/B mesma caixa 20M ops:
      CPU user da suíte **−93%** (1 073–1 109 s → 71 s), pico RSS
      **−62%** (15,7→5,9 GB), **d +16,8%, b +14,8%, f +7,1% (3/3)**,
      c +3,3%; a/e planos (limitados por fdatasync, drift ±6% da caixa
      troca o sinal entre rodadas). **Re-arm 5 (2026-08-22,
      `findings/2026-08-22-p23-rearm5/`, HEAD `28a3d59`, rounds a load
      7,7–8,5)**: primeira bateria com **a/d/e ≥5 na mediana** —
      E 5,82 / D 5,45 / A 5,40 (era 4,94/~3,0/3,65 no rearm4);
      C 4,70, B 4,03, F 3,73 sobem mas não fecham; longa contaminada
      (load 12–15, watchdog) — bateria NON-OFFICIAL, evidência.
      Oficiais = rearm6 quando a caixa aquietar
- [x] **P2.3** Script: `PEDRA_PARITY_ASYNC=1` + `FLOOR=5` **não** é o
      default do `tikv_ycsb_parity_v0.sh` — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC + status viva | done | este doc | 2026-08-19 |
| P0.2 | p0 | knob async; write() no Ok | done | sem userspace ack | 2026-08-19 |
| P0.3 | p0 | `commit_async_ops` | done | sem group no async | 2026-08-19 |
| P0.4 | p0 | buffer async 64 KiB | done | `ASYNC_WAL_BUFFER` | 2026-08-19 |
| P0.5 | p0 | set_mc50 ≥ 5× async | doing | quieto: **2.02 vs peer são** — não fecha; 3.4–9.2 eram peer doente | 2026-08-20 |
| P1.1 | p1 | pipeline ≥ 5× | **done** | **rearm7 quieta 3/3 ≥5 (5,10/6,04/5,45), med 5,67** — fecha | 2026-08-23 |
| P1.2 | p1 | set / blob ≥ 5× | doing | BlobDB knobs + vlog 64 KiB async buffer shipped; A/B blob +22% vs inline, not 5× (`write()` of 16 KiB remains). rearm8: SET 4,40/4,68/4,48 3/3 <5 (straddle); blob 1,64/1,79/1,70 (era 0,99) — nenhum fecha | 2026-08-23 |
| P1.3 | p1 | get ≥ 5× | **done** | **rearm8 quieta 3/3 ≥5 (5,465/6,074/5,710), med 5,58 — fecha** | 2026-08-23 |
| P2.1 | p2 | quiet 3× | **done** | **árbitro @ load 9.4**: E 10.5 e scan 7.4 fecham; SET 5.41 longo; resto 0.94–3.8 | 2026-08-20 |
| P2.2 | p2 | ycsb A–F ≥ 5× async | doing | rearm8: E 3/3 (2ª consecutiva, med 5,44); D 4,56–4,84 3/3 <5; A artifact r1 sumiu (med 4,48); B 3,33/C 4,23/F 3,80 | 2026-08-23 |
| P2.3 | p2 | script não default 5× | done | tikv_ycsb_parity_v0.sh | 2026-08-19 |

## Acceptance Criteria

- **Tests:** `async_puts_are_in_wal_without_fsync`;
  `async_ok_write_wal_without_fsync_survives_reopen`;
  `async_tail_below_64kib_recovers_on_close_without_fsync`;
  `async_concurrent_writers_recover`;
  `async_and_sync_concurrent_writers_recover`;
  `interned_pipeline_batch_recovers_after_close`;
  `v2_reuses_interned_value_bytes`;
  `fragment_encoded_matches_scratch_path_bytes` (inclui interned);
  crash-after-sync-put (G1) continua a recuperar;
  `kvrocks_suite_on_compat_engine`;
  `put_batch_same_interns_and_reads_back`.
- **Telemetry:** `findings/rfc0043-pedra-async-dirty/` (sujo) e, no P2,
  `findings/rfc0044-p2/` com `compare_report.json` ×3,
  `compat.sync=false`, `rocks.sync=false`.
- **Documentation:** este RFC; README do finding diz **not official**.
- **Screenshots:** none — backend-only.

## Out of scope

- Vender ratio async/async como “batemos o Rocks.”
- 5× no 1-op **G1** vs Rocks async (teto `1/t_fd`; RFC-0041 P1.2).
- Trocar o peer oficial para `sync=true`.
- Arena/skip-list C++ (follow-up se P1.1–P1.3 saturarem memcpy+BTree).
- Buffer async 1 MiB / Ok sem encode (rejeitado: pior que Rocks).
