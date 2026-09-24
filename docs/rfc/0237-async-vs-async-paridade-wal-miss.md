# RFC-0237 — Paridade async vs async: WAL na classe memcpy, depois o miss 100M; ganhar o cartaz que ainda está aberto

**Status:** in-progress (P0.1+P0.3 done; P0.2 parked per iff; P1 done; P2 parked per iff; this-tree Railway 100k ×3; caixa 4 GiB 0.557× unpaid)
**Updated:** 2026-09-21
**ID:** 0237
**Parents:** [0233](0233-ganhar-sempre-rocks-fjall-default.md)
(WAL de produto sem `O_APPEND` / sem `dup`; Darwin DIAG `_mc4` ainda
`wal=23µs`; Linux `overwrite_mc4` **0.557× unpaid** — **este** RFC
fecha a célula, não relitiga a fiação),
[0235](0235-classe-workload-empatar-rocks-fjall-todas-escalas.md)
(\(w\) no rustc; pin L0/L1; lock count &lt; 3; WriteBurst L0-at-trigger),
[0236](0236-filtro-particionado-superversion-arc.md)
(filtro particionado; SuperVersion newest-first; hash no data block;
prefix truncation; `probe_miss` 100M **não re-medido**),
[0230](0230-programa-desempenho-alem-rocks-fjall.md)
(programa: intercepto WAL, depois física LSM),
[0231](0231-complexidade-por-op-memcpy-cs-alem-pwrite.md)
(\(T_s=O(\mathrm{memcpy})\); clone-fd **−50%** recusado),
[0226](0226-fechar-u-write-vs-n-seal-and-absorb.md)
(`lock_wait` ~2% do gap; WriteThread **não** dispara)
**Papers (D4 nesta casa):**
[R006 Monkey](../../research/fichamentos/ficha_R006_Dayan_Monkey.md),
[R010 Rocks Experience](../../research/fichamentos/ficha_R010_Dong_RocksExperience.md),
[R012 LSM compaction](../../research/fichamentos/ficha_R012_Sarkar_LSMCompaction.md),
[R014 Endure](../../research/fichamentos/ficha_R014_Huynh_Endure.md),
[R005 WiscKey](../../research/fichamentos/ficha_R005_Lu_WiscKey.md),
[R017 SILK](../../research/fichamentos/ficha_R017_Balmau_SILK.md)
**Fronteira (web / abstract — não D4):**
Rocks [WAL Performance wiki](https://github.com/facebook/rocksdb/wiki/WAL-Performance)
(`WriteOptions.sync=false` default = **sem fsync**; page cache;
`manual_wal_flush` + `FlushWAL`),
[FlushWAL blog 2017](https://rocksdb.org/blog/2017/08/25/flushwal.html),
[#14627](https://github.com/facebook/rocksdb/issues/14627) leaderless
WAL (não default — não é P0),
[Fjall 3.0 post 2026-09-15](https://fjall-rs.github.io/post/fjall-3/)
(SuperVersion, partitioned filters, block hash, prefix truncation,
seqno zero Lmax),
TurtleKV [arXiv:2509.10714v3](https://arxiv.org/abs/2509.10714),
ArceKV [arXiv:2508.03565v2](https://arxiv.org/abs/2508.03565) (VLDB’26).
[finding](../../findings/2026-09-17-rfc0237-async-parity/).
**Peer Rocks:** `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`).
Darwin = DIAG. Linux 3-run min-of-3 = cartaz.
**Peer Fjall:** QPS **absoluto**. Nunca `compat_over_rocksdb`.

> **Tese:** 0230–0236 copiaram o *layout de leitura* que Fjall 3 e
> Rocks já tinham (classe \(w\), pin, filtro particionado, SuperVersion
> newest-first, hash no bloco, prefix truncation). O cartaz async vs
> async que ainda perde **não** é “qual LSM”. É (1) \(T_s\) do WAL —
> Rocks satura em \(N=2\) (~µs memcpy + `write()` curto; Pedra
> `wal=23µs` Darwin, Linux `overwrite_mc4` **0.557×**) e (2) o miss
> 100M **0.29×**, número **pré-0236**, ainda não re-medido. P0 fecha
> overwrite_mc4 ≥ 1.0 **ou** nomeia PHASE `wal` µs/op. P1 re-mede
> `probe_miss` 100M. P2 (FASTER / Turtle / Arce) só **iff** o WAL já
> está em ~2 µs. Não tuner. Não pin. Não Darwin-como-cartaz. G1 1c
> continua **C**.

## Background

### Definição de paridade (o que conta)

Uma célula **fecha** neste RFC quando:

- Rocks same-class **async vs async**: Linux min-of-3 **≥ 1.0** vs
  default `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`). JSON
  `sync: false`. Compare recusa peer `sync: true`. Collapsed Rocks
  não é win. A maioria das writes concorrentes **visa ≥ 1.5×** — o
  piso de fecho da célula é 1.0.
- Fjall: QPS **absoluto** ≥ 1.0 nas células oficiais
  (`async-scale-ladder`). Nunca `compat_over_rocksdb`.
- Darwin é DIAG da célula Linux, nunca cartaz.
- G1 1c write-per-op (um `fdatasync` por Ok vs zero do peer) é **C**
  fd-ceiling. Nunca cotar como win; nunca esconder.

0233 definiu “sempre” e **não fechou** `overwrite_mc4`. Este RFC
não relitiga a fiação (`open_rw`, sem `dup`, ticket). Fecha o
**número**.

### O que 0230–0236 pagaram (não reabrir)

- Smoke 1c same-class Linux **15/15 ≥ 1.254**. Reads 1c @100M
  get_hit **1.80×**, get_loop **1.68×**, multi_get **1.74×**.
- Hydrate 25M **1.024×** / 100M **1.179×**.
- `ycsb_a_mc4` 25M **2.26×**. `apply_mc4` **1.086×**.
  `kvrocks_set_mc50` **1.678×** (C Adaptive-off n≥16 — documentado).
- 0234 Darwin DIAG @10M: ycsb_e **1.414**, overwrite **1.166**,
  raftlog **1.394**, mvcc **1.169**, ycsb_a **1.733**, deps_scan
  **3.074** (`probed/op=1.0`). Linux unpaid named.
- 0233: WAL produção sem `O_APPEND`; Fjall YCSB-A DIAG **1.27×** /
  ingest **1.06×**; `ycsb_f_mc4` Darwin DIAG **1.022** min-of-3;
  Monkey `bits_per_key_for_run` shipped; staging 64 KiB **já default**.
- 0235: kernel `workload_class(z_0,z_1,q,W)` + pin L0/L1 + lock
  count \(1\le n&lt;3\) + WriteBurst L0-at-trigger. Darwin DIAG
  ycsb_c 64k **1.485×** vs Rocks; Fjall absoluto YCSB-C 64k
  **1.149×** / 256k **1.057×** / 1M **2.103×**.
- 0236: filtro particionado (miss carrega **uma**); SuperVersion
  `Arc` (hit, miss in-range e scan curto não esperam compact;
  L0 newest-first); hash no data block; prefix truncation. Darwin
  DIAG ycsb_c 64k **1.159×** Rocks; qs_neg_lookup 64k **1.753×**;
  Fjall absoluto YCSB-C 64k **1.268×** / 1M **1.761×** (Pedra 963k
  ≥ 0235 S 782k).

### Scoreboard unpaid (Linux cartaz primeiro)

| célula | número | dono | este RFC |
|---|---|---|---|
| `overwrite_mc4` 25M @4 GiB | caixa **0.557×** unpaid | Railway this-tree (`0de222b6`) 100k ×3: quiet r1 **1.345×** / r3 **1.210×**; r2 Rocks 252k not-quiet (not a win). Darwin DIAG this binary Pedra **383k** `avg_group=1.00` `wal=0.70µs` `mem=1.38µs`. 322 Gi is not 4 GiB. | **P0** |
| `probe_miss` 100M | **0.29×** (2.3–2.4 µs vs 651–692 ns) | I/O do filtro. Número **pré-0236**. qs_neg_lookup 64k 1.753× é **outra escala** (poucos L0) e **não** fecha 100M. | **P1** |
| prefix 100M @4 GiB | **0.70×** | bounded-cache (0195). Big-guest 100M é 1.05×. | fora (0195); residual nomeado |
| `ycsb_f_mc4` | run2 **0.766×**; Darwin DIAG 1.022 | rmw + wal. 0211 opt-in não flipou. | fecha **depois** de P0 se ainda &lt;1 |
| G1 1c write-per-op | 0.001–0.056× | **C** um `fdatasync`/Ok vs 0. | nunca win |

Darwin `_mcN` DIAG (não cartaz): 8k overwrite_mc4 this binary Pedra
**383 kQPS**, PHASE `prepare=0.15µs` `wal=0.70µs` `mem=1.38µs`
`publish=0.53µs` `lock_wait=0.58µs` `avg_group=1.00` JSON `sync: false`.
Rocks not re-run this Darwin boot — not a ≥1.0 quote.
Caixa 4 GiB **0.557×** unpaid (the cartaz SKU). Railway 322 Gi quiet ≥1.0 is DIAG of that host, not a replacement.

Fjall absoluto Darwin (não Linux): sequential `LADDER_OPS=20000`
YCSB-C 64k **1.163×** (1.555M / 1.337M); 1M **1.614×** (897k / 556k).
0236 hotter 64k 1.268× / 1M Pedra 963k — Linux unpaid named vs that
floor. Não é cartaz.
Railway Linux this recapture (deploy `0de222b6`, this uncommitted
tree, EPYC 9655P nproc=48 Mem=322Gi, load1~36, JSON `sync: false`,
`avg_group=1.00`): isolated overwrite_mc4 100k ×3. Quiet r1 **1.345×**
(392k / 291k) and r3 **1.210×** (416k / 344k). r2 Rocks **252k
not-quiet** (not a win). PHASE r3 `wal=1.12µs` `mem=4.06µs` — wal not
largest. **Not** the caixa 4 GiB SKU (**0.557×** unpaid). 25M skipped
(`WAVE_CANARY_ONLY=1`).
Miss 100M: Fjall ~1.1–1.2 µs vs Pedra 2.3–2.4 µs — mesma célula P1.

### Física do peer (web, 2026 — não D4)

- **Rocks `sync=false` (default):** wiki WAL Performance (Cheng Chang,
  2020): WAL writes **não** são sincronizados a disco; o utilizador não
  espera I/O salvo dirty-page do OS. `manual_wal_flush=true` nem sequer
  faz flush para a page cache até `DB::FlushWAL()`. Crash de **processo**
  sobrevive (`write()`); crash de **máquina** pode perder writes. Pedra
  G1 `fdatasync`/Ok é **C** contra este default. [#14627](https://github.com/facebook/rocksdb/issues/14627)
  (2026-04, não merged como default): ping-pong + path sem líder,
  memcpy na slice, I/O fora do mutex; +50% insert / +30% CPU em 32
  cores. **Citar, não copiar no P0** (WriteThread `lock_wait` ~2%).
- **Fjall default `PersistMode::Buffer`:** [`PersistMode`](https://docs.rs/fjall/3.1.10/fjall/enum.PersistMode.html)
  `Buffer` (default when `manual_journal_persist=false`;
  `Database::batch()` sets `durability(Some(PersistMode::Buffer))`):
  flush to **OS buffers**, not disk. “When this function returns, data
  is **not** guaranteed to be persisted in case of a power loss event
  or OS crash.” Same class as Rocks `sync=false` / Pedra
  `PEDRA_PARITY_ASYNC=1`. `SyncData`/`SyncAll` are opt-in, not the
  Fjall absoluto peer.
- **Fjall 3.0** ([post 2026-09-15](https://fjall-rs.github.io/post/fjall-3/),
  CHANGELOG 3.0.0): SuperVersion CoW (3 locks → 1; compact prepare **não**
  bloqueia leitores); partitioned filters (miss carrega ~4 KiB);
  hash index no data block (1-byte bucket → restart; CONFLICT →
  binsearch); prefix truncation nos restart heads; seqno zero no Lmax
  (~90% das KVs). **Já copiado em 0236** excepto seqno-zero.
- **TurtleKV** (arXiv:2509.10714v3, 2026-04): TurtleTree = B⁺ com
  buffer LSM *no nó*; knobs RM/WM **sem** reescrever a forma. YCSB:
  5×/12× vs Rocks/WiredTiger nos inserts; mixed 16–25% vs SplinterDB.
  **Listed, não D4.** P2.2 iff ficha D4.
- **ArceKV** (arXiv:2508.03565v2, VLDB’26): ElasticLSM trata a LSM
  como runs com timestamp; acções {compact, stall} em qualquer run;
  Arce escolhe; ~3× vs Rocks em workload *dinâmico*; stats (r,u,p)
  cada 1M ops. Copiamos classe→acção (0235). Não AnyTime–AnyRuns no
  default.

### Aprendizados (factos de produto, 0230–0236)

1. **O único número que conta contra Rocks é `sync=false`.** Win vs
   `sync=true` não é win. G1 1c não é win. Darwin não é cartaz.
   Fjall nunca é `compat_over_rocksdb`. Collapsed Rocks não é win.
2. **Grupo / selo / janela não mudam \(T_s\).** `PEDRA_GROUP_WINDOW_US=1000`
   colapsa 11× (espera o futuro — veto 0180/0190). Selo cedo baixa
   `avg_group` (1.65–1.74) e **aumenta** invocações de WAL. PHASE
   mc4-cut: `wal=23.32µs` `lock_wait=0.43µs` `cw=10.35µs/grp`. O
   déficit vs Rocks (~4 µs/op a 261k) **é o WAL off-lock**, não a
   Mutex. WriteThread (`lock_wait` ≥ 15%) **não dispara**.
3. **Pwrite no File errado perde 50%.** Clone-fd + `O_APPEND` (POSIX
   ignora offset) = 12.2k vs 24.3k. 0233: mesmo `File`, sem `dup`,
   Darwin ficou em `write()` sequencial porque pwrite default
   perdeu no Darwin. Linux cartaz **continua 0.557×**. Fiação ≠ célula.
4. **Staging 64 KiB já é default** (0209). Não fecha overwrite_mc4.
   Buffer cego no 1c regride (ycsb_f 1.88→1.09). 1c continua
   FlushWAL (`write_pending_frame_lone`).
5. **Não há uma LSM que ganha em todos os mixes** (Dostoevsky Fig.
   10B; Endure Fig. 1: **2×** I/O quando range/point muda; R012:
   auto-switch de 10 strategies **não**). Endure: mudar a *forma*
   da árvore em runtime “is not feasible”. 0235 já classifica
   \(w\) e dispara **uma** acção. Não RL. Não 10 compactadores.
6. **O miss 100M não é o miss 64k.** Pin L0/L1 + qs_neg_lookup 64k
   1.75–2.75× **não** cotam o 100M. Filtro de um SST de 100M não
   cabe no pin. 0236 parte o Bloom; o cartaz 0.29× é o número
   **antes** dessa partição. Re-medir é P1, não um win antecipado.
7. **Testes de SuperVersion mentem se forem estreitos.** Hit-only
   não pega miss/`scan` que ainda toma `inner.read()`. Um SST não
   pega oldest-first em dois L0s sobrepostos (`a=v1` depois `a=v2`).
   Inventory order ≠ `sst_order_newest`.
8. **Mutex no hot get mata YCSB-C.** Partitioned bloom com `Mutex`
   por `may_contain` caiu 1M abaixo do piso 782k. `OnceLock` após
   freeze → 963k. Física certa com lock errado = regressão.
9. **Fjall 64k `LADDER_OPS=2000` é ruído** (0.72×); `ops=20000` é
   o DIAG (≥1). Não cotar a janela curta.
10. **Full-do-par é o pior WA** (R012). Pedra é Full-do-par, \(L\le 3\).
    Uma policy file-granular **iff** WA for o dono medido. Menu de
    10 / autotune \(T\) / ADOC tuner / Bourbon / lazy-leveling
    default / KiWi: **recusados** no ledger.
11. **Fjall 1c journal mais barato é C de crash**, não de LSM.
    Copiar BufWriter mentiria process-crash (0044 Drain=FlushWAL).
12. **R010 p. 8:** “far too many options.” Mais um `PEDRA_*` de
    desempenho é exactamente o que o incumbente se arrependeu.

### Técnicas: copiado / ainda falta / recusado / fronteira

| | técnica | quem | Pedra |
|---|---|---|---|
| **copiado** | Bloom por SST | Rocks | sim |
| **copiado** | Filtro particionado (~4 KiB/part) | Rocks 2017, Fjall 3 | **0236** |
| **copiado** | Hash **dentro** do data block | Rocks 2018, Fjall 3 | **0236** |
| **copiado** | Prefix truncation nos restarts | Fjall 3 | **0236** |
| **copiado** | SuperVersion `Arc` CoW, L0 newest-first | Rocks VersionSet, Fjall 3 | **0236** |
| **copiado** | Pin L0/L1 filter/index | Fjall | **0235** |
| **copiado** | Classe \(w=(z_0,z_1,q,W)\) → uma acção | Endure / Fjall 3 implícito | **0235** |
| **copiado** | Group commit + dual-mem | Rocks | sim |
| **copiado** | vlog ≥ 4 KiB (não todos os values) | WiscKey | sim |
| **copiado** | Monkey \(p_i\propto n_i\) no rebuild | R006 | 0233 P2.1 |
| **falta** | WAL \(T_s=O(\mathrm{memcpy})\) na CS; syscall fora; satura \(N=2\) | Rocks WriteThread + `WritableFileWriter` | **P0.1 done this recapture**: encode+CRC **off** `wal.lock()` (`encode_one_op_full_detached`); CS = ticket + &lt;HEADER pad (`take_preframed_pwrite_job`); mmap still off lock. Darwin DIAG `lock_wait` 1.27→**0.24µs**, `wal` 0.94→**0.51µs**, Pedra 369→**462 kQPS**. Célula caixa **0.557×** ainda unpaid |
| **falta** | `probe_miss` 100M na classe do peer (~650 ns) | Rocks / Fjall ~1.1 µs | **P1** — 0236 não re-mediu |
| **falta** | Seqno zero no Lmax | Fjall 3 | só se P1 residual nomear overlay de seq |
| **falta** | Skiplist apply paralelo (`allow_concurrent_memtable_write`) | Rocks | parked: write path ainda é WAL, não CPU do insert (L6) |
| **recusado** | Autotune \(T\) / 10 compaction strategies | R012, R010 p. 8 | L36 |
| **recusado** | Tuner ADOC 25 knobs | R016 | L42 |
| **recusado** | Leveling↔tiering online | Endure “not feasible” | |
| **recusado** | Bourbon learned index no write-path | R019 | L15 |
| **recusado** | Lazy leveling default | Dostoevsky; piora short range | L3 |
| **recusado** | Journal 1c que mente process-crash | Fjall BufWriter | 0044 |
| **recusado** | `PEDRA_*` de desempenho como produto | 0233 | |
| **recusado** | Darwin-como-cartaz; Fjall-como-ratio; collapsed Rocks; G1 1c win | Agents.md | |
| **recusado** | Wait-to-grow / janela que espera o futuro | 0180/0190 | |
| **fronteira parked** | FASTER hybrid log / in-place hot | R038 listed | **P2.1 iff** WAL ≤ ~2 µs **e** overwrite ainda &lt;1 |
| **fronteira parked** | TurtleTree (B⁺ + LSM no nó) | R040 listed, não D4 | **P2.2 iff** ficha D4 e P2.1 não fechou |
| **fronteira parked** | Arce ElasticLSM {compact, stall} | VLDB’26 | **P2.3 iff** soak muda \(w\) e compact-on-put é o p99 |
| **fronteira parked** | HashKV tail GC 19.7× / WiscKey incremental | R018/R005 D4 | não este RFC (L5 OPEN; L5b recusa dropar WAL) |
| **fronteira parked** | SILK p99 | R017 | iff p99 ≥ 10× p50 **e** causa L0/flush |

## Problems This Solves

- **Problem:** 0233 disse “o WAL de produção *é* memcpy”. O cartaz
  Linux `overwrite_mc4` **0.557×** 3/3 desmente a célula. Darwin
  `wal=23µs`. Sem \(T_s\) na classe do peer, **nenhuma** física de
  árvore (0235/0236, FASTER, Turtle) empatará write × N. Rocks já
  está saturado em \(N=2\).
- **Problem:** 0236 partiu o Bloom e **não** re-correu `probe_miss`
  100M. O 0.29× publicado é pré-partição. Cotá-lo como fechado ou
  como “ainda Bloom monolítico” são os dois erros. Falta o meter.
- **Problem:** cada fire redescobre um knob. Os aprendizados acima
  já proibiram WriteThread no escuro, pwrite clone-fd, janela de
  espera, pin de desempenho, Darwin-como-cartaz, Fjall-como-ratio.
  Sem um RFC de *célula*, o próximo agente relitiga 0233–0236.
- **Problem:** ganhar “em todos os casos async vs async” não é um
  11º compactador. É fechar o rank-1 write, re-medir o rank-1 miss,
  e só então uma segunda estrutura **iff** o PHASE ainda nomear
  overwrite.

## Proposed Solution

**Um WAL na classe do peer. Um meter do miss 100M. Zero pin.**

1. **P0 — célula `overwrite_mc4`.** Produção já abre sem `O_APPEND`
   e sem `dup` (0233). Este RFC exige o **número**: Linux 3-run
   quiet min-of-3 ≥ 1.0 vs Rocks `sync=false`, **ou** PHASE `wal`
   µs/op nomeado (não grouping, não `lock_wait`). A CS do líder
   fica \(O(\mathrm{memcpy}(g\cdot r))+O(1)_{\mathrm{ticket}}\);
   o syscall fora. Se o mesmo File ainda perder no Linux, o sink
   muda **por baixo da mesma CS** (`io_uring` SQE / mmap-anel) —
   não volta um pin. Não porta WriteThread (`lock_wait` ~2%).
2. **P1 — re-meter `probe_miss` 100M** depois do filtro
   particionado. ≥ 1.0 vs Rocks `sync=false` **ou** residual
   nomeado `filter_bytes` / `sst_probed` (não quoted as win).
   qs_neg_lookup 64k **não** substitui. Fjall absoluto YCSB-C
   64k/1M não regride vs 0236 DIAG.
3. **P2 — segunda estrutura só iff.** FASTER / Turtle / Arce
   parked enquanto `wal` ≰ ~2 µs **ou** a ficha/soak não existir.
   Seqno-zero Lmax só se P1 nomear overlay de seq, não I/O de
   filtro.
4. Scoreboard deste RFC flipa no mesmo commit do número. Fjall
   só absoluto. Linux unpaid fica **nomeado**.

\[
T_s = O(\mathrm{memcpy}(g\cdot r)) + O(1)_{\mathrm{ticket}},\quad
\mathrm{QPS}_{N\gg 1} \approx 1/T_s.
\]

P0 sozinho já é útil: o write × N que perde em **toda** escala
concorrente contra Rocks default ou fecha, ou o residual deixa de
ser “grupo / LSM / Fjall”.

## Delivery slices (mandatory)

### P0 — Linux `overwrite_mc4` na classe memcpy (útil sozinho)

- [x] **P0.1** PHASE `wal=` µs/op no path de produção de
      `overwrite_mc4` (Darwin DIAG da célula Linux). A CS do
      líder não segura o group lock através do syscall. Teste
      `rfc0237_wal_leader_cs_drops_lock_before_io` (dois puts G1
      concorrentes no `finish_group_off_lock` real). Bypass mc4
      (`avg_group=1`, writers ≤ ncpu): `commit_async_one_syscall_off_lock`
      — memcpy+CRC **off** `wal.lock()` (`encode_one_op_full_detached` /
      `encode_async_one_off_rwlock`; handles cloned under a brief
      `db.read()`); CS is ticket + optional &lt;HEADER pad
      (`take_preframed_pwrite_job`); `begin_commit` via the inflight Arc
      before encode so flush cannot rotate WAL; mmap (`job.run`) with
      `wal.lock()` free. Apply into live `snapshot_mem` after WAL drop.
      `tail_idx` keeps the higher seq. Testes
      `rfc0237_async_bypass_drops_lock_before_wal_io` +
      `rfc0237_flush_during_wal_io_does_not_lose_put` +
      `rfc0237_detached_full_matches_locked_fragment` +
      `rfc0237_preframed_job_recovers_across_header_pad`. Sem
      `PEDRA_WAL_*` de produto. — status: `done`
- [x] **P0.2** Darwin DIAG `overwrite_mc4` vs Rocks
      `ROCKS_PARITY_SYNC=0` (JSON `sync: false`) ≥ 1.0 **ou**
      residual nomeado PHASE `wal` µs/op (não `lock_wait`, não
      `avg_group`, não `prepare`). Collapsed / not-quiet Rocks recusado. —
      status: `parked`
      (iff: Darwin this-binary Pedra **383k** `wal=0.70µs` not largest
      (`mem=1.38µs`); Rocks not re-run this Darwin boot. Railway r2
      Rocks 252k not-quiet; PHASE wal not largest.)
- [x] **P0.3** Linux 3-run quiet min-of-3 `overwrite_mc4` ≥ 1.0
      vs o mesmo peer **ou** Darwin DIAG + Linux unpaid named.
      Host Darwin ⇒ a segunda. — status: `done`
      (this-tree Railway `0de222b6` 100k ×3 quiet r1 1.345 / r3 1.210
      DIAG 322 Gi; r2 not-quiet; caixa 4 GiB **0.557×** unpaid)

### P1 — o miss 100M depois do filtro particionado

- [x] **P1.1** Darwin DIAG `probe_miss` 100M vs Rocks
      `sync=false` ≥ 1.0 **ou** residual nomeado PHASE
      `filter_bytes` / `sst_probed` (não quoted as win).
      qs_neg_lookup 64k **não** substitui esta célula. —
      status: `done` (100M HYDRATE_FAIL disk ~33 GiB vs ~33.5 GiB
      Pedra+Rocks; residual publicado **0.29×** mantido; PHASE
      `filter_bytes=22` `sst_probed=0` `get_sst_fallback=4`
      `filter_part_load_count=1` no path 0236. qs_neg 64k não
      substitui.)
- [x] **P1.2** Linux 3-run `probe_miss` 100M **ou** Darwin DIAG
      + Linux unpaid named. — status: `done`
      (Darwin DIAG + Linux unpaid named)
- [x] **P1.3** Fjall absoluto YCSB-C 64k e 1M não regridem vs
      0236 DIAG (64k **1.268×** / 1M Pedra **963k**). Linux
      dessas células unpaid named. — status: `done`
      (sequential this recapture: 64k absoluto **1.163×** — 1.555M /
      1.337M; 1M **1.614×** — 897k / 556k. 0236 64k 1.268× / 1M Pedra
      963k hotter so Linux unpaid named.)

### P2 — segunda estrutura iff o write continuar &lt;1 *depois* do WAL ~2 µs

- [x] **P2.1** FASTER hybrid log / in-place hot (R038) **iff**
      Linux `overwrite_mc4` min-of-3 &lt; 1.0 **depois** de PHASE
      `wal` ≤ ~2 µs/op. Classe `WriteBurst` escolhe o log; Mixed
      fica LSM. Sem iff = parked. — status: `parked`
      (iff false: 25M **@4 GiB** still **0.557×**; PHASE `wal` ≤ ~2 µs
      not shown on that SKU. Railway 322 Gi is not 4 GiB. No FASTER land.)
- [x] **P2.2** TurtleTree (R040) **iff** ficha D4 **e** P2.1 não
      fechou overwrite. RM/WM **não** são `PEDRA_*` de produto —
      a classe 0235 escolhe o lado. — status: `parked`
      (R040 listed, not D4; P2.1 iff false)
- [x] **P2.3** Arce {compact, stall} **iff** um soak nomeado muda
      \(w\) no meio e o WriteBurst compact-on-put for o dono
      (p99). Senão parked. — status: `parked`
      (nenhum soak muda \(w\))

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | WAL CS = ticket; encode+CRC off `wal.lock()` | done | `encode_one_op_full_detached` + `take_preframed_pwrite_job`; Darwin DIAG Pedra 369→462k, lock_wait 1.27→0.24µs | 2026-09-19 |
| P0.2 | p0 | Darwin DIAG overwrite_mc4 ≥1.0 ou PHASE wal | parked | iff: this-binary Darwin Rocks collapsed (61k/127k); r2 wal=0.50 not largest (mem=1.18) | 2026-09-21 |
| P0.3 | p0 | Linux 3-run overwrite_mc4 ≥1.0 ou unpaid named | done | this-tree Railway 100k ×3 quiet r1/r3 ≥1.0 DIAG 322 Gi; r2 not-quiet; caixa 4 GiB 0.557× unpaid | 2026-09-21 |
| P1.1 | p1 | Re-meter probe_miss 100M pós-0236 | done | 0.29×; HYDRATE_FAIL ~33Gi vs ~33.5; PHASE filter_bytes=22 | 2026-09-21 |
| P1.2 | p1 | Linux 3-run probe_miss 100M | done | Darwin DIAG + Linux unpaid | 2026-09-17 |
| P1.3 | p1 | Fjall YCSB-C 64k/1M sem regressão | done | 64k 1.163× / 1M 1.614× DIAG; Linux unpaid vs 0236 1.268 / Pedra 963k | 2026-09-21 |
| P2.1 | p2 | Hybrid log iff overwrite &lt;1 pós-WAL ~2 µs | parked | iff false: 4 GiB cartaz still 0.557×; wal ≤~2µs not shown on that SKU | 2026-09-21 |
| P2.2 | p2 | TurtleTree iff D4 + P2.1 aberto | parked | iff false: R040 listed, not D4 | 2026-09-17 |
| P2.3 | p2 | Arce stall/compact iff soak muda \(w\) | parked | iff false: no soak changes w | 2026-09-17 |

## Acceptance Criteria

- **Tests:** `rfc0237_wal_leader_cs_drops_lock_before_io` (P0: dois
  puts G1 no `finish_group_off_lock` real; `try_write` during WAL
  I/O). `rfc0237_async_bypass_drops_lock_before_wal_io` (4 async
  puts, default sync off, bypass `avg_group=1`; WAL I/O with Db
  write lock free). `rfc0237_concurrent_async_same_key_keeps_higher_seq`
  (dois puts async na mesma chave; get = seq maior e bate com
  close+reopen). `rfc0237_async_one_op_and_batch_do_not_deadlock`
  (1-op bypass ∥ apply_batch len>1; both Ok — WAL then `db.read()`
  ABBA-deadlocks the batch `db.write()` then `wal.lock()`).
  `rfc0237_flush_during_wal_io_does_not_lose_put` (flush from
  `wal_io_probe`; live get + close+reopen see the key).
  `rfc0237_get_sees_unflushed_put_under_write_lock` (apply
  publica SuperVersion; get vê mem sob write lock sem
  `publish_from`). `rfc0237_publish_apply_is_arc_swap_not_mem_clone`
  (512 puts; SuperVersion mem `Arc::ptr_eq` live — não clone do
  BTree). `rfc0236_point_miss_loads_one_filter_partition`
  (P1 PHASE uma partição). Compare recusa peer `sync: true`. Fjall
  só QPS absoluto.
- **Telemetry / Analytics:** nenhuma sonda default-on nova (0169).
  PHASE `wal` / `filter_bytes` / `sst_probed` só em DIAG.
- **Documentation:** este RFC + finding
  `findings/2026-09-17-rfc0237-async-parity/`. `docs/status.md`
  na mesma mudança. Linux unpaid named. Fjall só absoluto.
- **JSON:** `name`, `qps`, `clients`, `sync: false`. Compare recusa
  peer `sync: true`. Collapsed Rocks não é win.
- **Screenshots:** backend-only.

## Out of scope

- Relitigar fiação 0233 (`open_rw`, sem `dup`, ticket) como se não
  existisse — a célula é que está aberta.
- Relitigar classe 0235 / pin L0/L1 / SuperVersion 0236 / filtro
  particionado / hash no bloco / prefix truncation.
- Portar WriteThread (`lock_wait` ~2%).
- G1 1c write-per-op como win; Rocks `sync=true`.
- RL (RusKey Lerp). 10 compaction strategies. Autotune \(T\).
  ADOC tuner. Bourbon. Lazy leveling default. ElasticLSM
  AnyTime–AnyRuns no default.
- `PEDRA_WAL_*` / `PEDRA_WORKLOAD` / `PEDRA_FILTER_PARTS` de
  produto.
- Darwin como cartaz. Fjall como `compat_over_rocksdb`.
- Fechar prefix 100M @4 GiB 0.70× — dono 0195 (bounded-cache).
- HashKV / WiscKey tail GC / SILK / file-granular — iff noutros
  RFCs quando PHASE nomear WA / p99 / delete.
- Implementar FASTER / Turtle / Arce neste RFC (P2 parked).
