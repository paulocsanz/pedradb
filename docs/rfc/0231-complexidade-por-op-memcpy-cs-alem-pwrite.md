# RFC-0231 — Complexidade por operação: \(T_s = O(\mathrm{memcpy})\), syscall fora da CS

**Status:** draft (superseded as product by [0233](0233-ganhar-sempre-rocks-fjall-default.md) — o pin `PEDRA_WAL_PWRITE` não é o caminho)
**Updated:** 2026-09-16
**ID:** 0231
**Filho de produto:** [0233](0233-ganhar-sempre-rocks-fjall-default.md)
**Parents:** [0230](0230-programa-desempenho-alem-rocks-fjall.md)
(P0.3–P0.4 aterrissaram ticket/`write_all_at`; P1.1 **no-flip** —
`PEDRA_WAL_PWRITE=1` −50% QPS no `_mc4` Darwin),
[0193](0193-write-off-lock-pwrite-ticket.md),
[0209](0209-wal-buffer-user-space.md),
[0044](0044-async-class-5x-rocks.md) (classe process-crash = FlushWAL),
[0055](0055-rocks-write-pipeline.md),
[0223](0223-escala-donos-flush-write-miss-read.md)
**Papers (fichas D4):**
[R006 Monkey](../../research/fichamentos/ficha_R006_Dayan_Monkey.md),
[R007 Dostoevsky](../../research/fichamentos/ficha_R007_Dayan_Dostoevsky.md),
[R010 Rocks Experience](../../research/fichamentos/ficha_R010_Dong_RocksExperience.md),
[R012 LSM compaction](../../research/fichamentos/ficha_R012_Sarkar_LSMCompaction.md),
[R017 SILK](../../research/fichamentos/ficha_R017_Balmau_SILK.md),
[R018 HashKV](../../research/fichamentos/ficha_R018_Chan_HashKV.md),
[R031 Lethe](../../research/fichamentos/ficha_R031_Sarkar_Lethe.md)
**Peer Rocks:** `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`).
G1 1c não é win. Darwin = DIAG. Linux 3-run min-of-3 = cartaz.
**Peer Fjall:** QPS **absoluto**. Nunca ratio, nunca `compat_over_rocksdb`.

> **Tese:** 0230 provou o *mecanismo* (ticket + pwrite) e o meter recusou
> o *custo* (`dup(2)` + `pwrite` em fd `O_APPEND`). O ganho drástico não
> é mais um `if` de grupo nem um flag de I/O: é igualar a **classe de
> complexidade** de cada op à do peer. Rocks WriteThread: memcpy na
> seção crítica, syscall fora. Fjall 1c: journal/BufWriter, sem líder.
> Pedra hoje: `write()` (ou pwrite pior) na CS do WAL + drain 1c por
> contrato FlushWAL + bloom FPR uniforme \(O(L)\) no miss. Este RFC
> escolhe **uma** arquitetura para a classe write (C: mesmo `File`,
> sem `O_APPEND`, sem `dup`) e mapeia o resto da tabela a fatias iff.

## Background

### O que 0230 mediu (não reabrir o flag)

`deps_cache_overwrite_mc4`, clients=4, n=8000, `sync: false`,
`PEDRA_SEAL_ASYNC=0`. Finding
[`2026-09-16-rfc0230-p11-pwrite-meter`](../../findings/2026-09-16-rfc0230-p11-pwrite-meter/README.md).

| path | qps | wal µs | lock_wait µs |
|---|---:|---:|---:|
| sequential `write()` (AS-IS) | 24.269k | 88.18 | 0.28 |
| opt-in pwrite (clone fd) | **12.208k** | **130.04** | **7.06** |
| Rocks `sync=false` | 117.504k | ~4 µs/op efetivo | — |

`number: ratio=0.104 pedra_qps=12,208 rocks_qps=117,504 shape=deps_cache_overwrite_mc4 DIAG`.
Linux cartaz overwrite_mc4 **0.557×** (25M @4 GiB) unpaid.

P0.2 do 0230: staging **está** no hot path do grupo (`wal` 29 vs 39.6 µs)
e **não** fecha o gap (QPS ruído 39.4k vs 38.7k vs Rocks 138k no mesmo
boot). Finding [`2026-09-16-rfc0230-p02-wal-buffer-ab`](../../findings/2026-09-16-rfc0230-p02-wal-buffer-ab/README.md).

WAL de produção abre com `Env::open_append` (`O_APPEND`). POSIX: `pwrite`
em fd `O_APPEND` **ignora o offset** e cola no EOF. Darwin: o dup+pwrite
ainda saiu mais caro que `write()`. O teste byte-idêntico
`rfc0193_pwrite_wal_bytes_match_as_is` usa `Wal::create` (sem
`O_APPEND`) — não é o path `_mc4`. `EnvFile::write_all_at` é `&mut self`;
`take_pwrite_job` faz `try_clone()` = `dup(2)` por grupo.

`_mcN` 2026-09-15: Pedra **sobe** 27k@2 → 115k@16; Rocks **constante**
164–277k para N≥2. Pedra precisa de N para amortizar um `write()` caro
e mesmo assim não alcança o intercepto. Selo 0226 baixou `avg_group` e
**aumentou** a taxa de `write()`. Mais política de grupo não muda \(T_s\).

### Fórmula do write concorrente

Com \(L\) líderes na mesma CS:

\[
T_{\mathrm{wait}} \approx \frac{L-1}{L}\, T_s,\qquad
\mathrm{QPS} \approx \frac{1}{T_s}\ \text{quando } L \gg 1.
\]

Rocks já está em \(\mathrm{QPS}=\Theta(1)\) em \(N\) para \(N\ge 2\):
\(T_s \sim\) memcpy. Pedra: \(T_s = T_{\mathrm{encode}} + T_{\mathrm{write()}}\)
(Darwin 88 µs / Linux quieto **890 ns** dentro do `wal.lock()`, RFC-0189
P0.1). O hat 0192: tirar o `write` da CS ⇒ +37% em \(L=4\) (173k→238k)
— teto de modelo, o meter decide. Classe-alvo:

\[
T_s = O\bigl(\mathrm{memcpy}(g\cdot r)\bigr) + O(1)_{\mathrm{ticket}},
\quad \mathrm{syscall}\ \mathbf{fora}\ \mathrm{da\ CS},\ \mathrm{sem}\ \mathtt{dup}(2).
\]

Aí QPS fica \(\Theta(1)\) em \(N\) **e** o intercepto cabe em ≥1.5× no
`_mc4` Linux (alvo deste RFC, não substitui o piso 0041=1×).

## Complexidade por classe de operação

Cada linha é um gargalo de **classe**, não um knob. W = corta neste
RFC ou num filho iff; S = já pago, não reabrir; C = contrato, documentar.

| Op | Pedra hoje | Rocks `sync=false` | Fjall default | Classe | Dono |
|---|---|---|---|---|---|
| **put 1c async** | \(O(1)\) encode + `write()` + **drain** do frame | \(O(1)\) memcpy + `FlushWAL`/record | \(O(1)\) journal append (BufWriter) | **C** Drain=FlushWAL vs Fjall; W vs Rocks se \(T_{\mathrm{write}}\) cair | 0044; `write_pending_frame_lone` |
| **put grupo / mcN** | \(O(g)\) encode + \(O(1)\) `write()`/grupo **na CS**; \(g\approx 1.7\) | WriteThread: memcpy na CS, skiplist paralelo; \(T_s\simµs\) | sem grupo; N writers = N journals | **W rank 1** | `wal.lock` + `O_APPEND` + dup |
| **get hit** | \(O(1)\) mem + bloom + block | idem | block cache | **S** (1c reads 2–3× Rocks) | 0041 / 0161 |
| **get miss** | \(\sum_i \mathrm{FPR}_i = O(L)\) (FPR uniforme) | Monkey: \(\mathrm{FPR}_i \propto 1/n_i\) ⇒ \(\sum \approx O(1)\) | ~2× mais barato @100M | **W** | `probe_miss` 0.29×; R006 |
| **scan** | \(O(\|L0\| + \#níveis)\) setup; 231 µs @10M (97%) | L0 limitado + block cache | similar LSM | **W** | 0223; não k-way |
| **flush no commit** | `flush_check` 28.8% @10M | flush fora do put quando dá | journal rotate | **W** | 0223 split gate×work |
| **compact / p99** | client vs flush vs compact no mesmo pool | rate limiter; SILK prioriza L0 | compacto LSM | **W iff p99** | R017 |
| **delete / GC vlog** | tombstone + vlog incremental | DeleteRange; WA leveled ~16 | KV-sep GC | **W iff WA** | R031 / R018; hydrate 100M já S vs Fjall |

### Arquiteturas para a classe write (a única que escala com \(L\))

| # | Arquitetura | \(T_s\) | Veredito |
|---|---|---|---|
| A | `write()` serial sob `wal.lock` (hoje, default) | \(O(\mathrm{syscall})\) | Baseline. 88 µs Darwin / 890 ns Linux. |
| B | clone-fd + `write_all_at` (0230 P0.4) | \(O(\mathtt{dup})+O(\mathrm{pwrite})+O(\mathrm{relock})\) | **Recusada pelo meter** (−50% QPS). |
| **C** | **mesmo `File` (`Arc`/`&self`), `open` sem `O_APPEND`, ticket, pwrite fora da meta-lock** | \(O(\mathrm{memcpy})+O(1)_{\mathrm{ticket}}\); syscall off-CS | **P0 deste RFC.** `FileExt::write_all_at` já é `&self`. |
| D | mmap / anel de páginas WAL | \(O(\mathrm{memcpy})\); writeback do kernel | P2.6 iff C ainda perder. Tear/hole = mesma recuperação CRC. |
| E | `io_uring` SQE off-lock (`pedradb-io-uring` já existe) | \(O(\mathrm{sqe})\) | P1.4 iff C no Linux ainda <1×. Linux-only. |
| F | WAL shard por CPU + merge na recover | \(T_s / k\) | Ordem de recover; não P0. |
| G | Portar WriteThread / wait-to-grow | mesmo \(T_s\), absorb melhor | **Parked.** `lock_wait=0.28µs` ≪ 15% do gap (0226/0230 P1.3). |

C é o único corte cujo ganho **cresce com \(L\)** *e* ataca a causa
medida do −50% (dup + `O_APPEND`). D/E/F não substituem C: se C
igualar memcpy-class, QPS \(\approx 1/T_s\) já é a classe Rocks.

## Problems This Solves

- **Problem:** o ticket de 0230 é um syscall extra em fd `O_APPEND` com
  `dup` por grupo. Sem `open_rw` e sem pwrite no **mesmo** `File`, a
  classe continua \(O(\mathrm{syscall})\) na CS — pior que A.
- **Problem:** Rocks satura em N=2 porque \(T_s\simµs\). Pedra sobe com
  N e perde em todo N. Política de grupo (0226) e staging (0209) já
  foram medidas: não mudam a classe.
- **Problem:** Fjall 1c write 0.44–0.77× é Drain=FlushWAL (C), não um
  bug de grupo. Um buffer 1c que **não** `write()` antes do Ok mente a
  classe (0044 + p209b: ycsb_f 1c 1.88→1.09).
- **Problem:** miss \(O(L)\), scan \(O(\|L0\|)\), flush_check no commit
  continuam W; 0230 os parkeou por iff. Sem um mapa por op, o próximo
  fire redescobre o grupo.

## Proposed Solution

1. **Classe C no WAL (P0).** `Env::open_rw` (create+read+write, **sem**
   `O_APPEND`) quando `PEDRA_WAL_PWRITE=1`. Cursor de produção =
   ticket. `pwrite` honra o offset no path de append real.
2. **Sem `dup(2)`.** `File` partilhado (`Arc<File>` ou `write_all_at`
   em `&self` após soltar a meta-lock). Hot path de grupo **não** chama
   `try_clone_handle`. Default do pin continua **off** até o meter.
3. Scoreboard desta tabela vive neste RFC. Flip de linha no mesmo
   commit do número.
4. Restante da tabela = P1/P2 **iff** o PHASE/QPS residual nomear o
   dono depois de C. Uma policy LSM (R012), não um tuner.

**Alvo (não substitui 0041=1×):**

- Write concorrente same-class: Linux min-of-3 **≥ 1.5×** Rocks default
  em overwrite_mc4 (hoje 0.557×) depois de C; Darwin DIAG sobe vs AS-IS
  ou o pin fica off (honest no-flip, como 0230).
- Resto das linhas W: ≥ 1.0× Rocks **ou** C explícito.
- Fjall: absoluto ≥ 1.0 nas linhas que perdem **ou** C Drain=FlushWAL
  no 1c (já nomeado no 0230 P1.2).

## Delivery slices (mandatory)

### P0 — classe C no WAL (útil sozinho: pwrite passa a ser pwrite)

- [x] **P0.1** Este RFC + mapa de complexidade + veredito A–G —
      status: `done`
- [ ] **P0.2** `Env::open_rw` (write, não append) usado pelo WAL quando
      `PEDRA_WAL_PWRITE=1`. `pwrite` honra o offset num ficheiro aberto
      pelo path de produção (não só `Wal::create`). Named `rfc0231_*`.
      — status: `todo`
- [ ] **P0.3** Off-lock **sem clone de fd**: `write_all_at` em `&File`
      (ou `Arc<File>`) após soltar a meta-lock. Teste estrutural: o
      path de grupo com pin on **não** chama `try_clone_handle`. Bytes
      WAL idênticos ao AS-IS. — status: `todo`
- [ ] **P0.4** Meter Darwin DIAG `_mc4` vs AS-IS vs Rocks (`clients=4`
      `sync: false`). Flip de default **só** se o ratio subir e as
      guardas nomeadas não recuarem; senão pin fica off. Linux cartaz
      unpaid neste host. — status: `todo`

### P1 — intercepto memcpy-class no Linux + 1c honesto

- [ ] **P1.1** Linux 3-run quiet overwrite_mc4 (cartaz 0.557×). —
      status: `todo`
- [ ] **P1.2** Flip `PEDRA_WAL_PWRITE` default ON **iff** P1.1 subiu e
      guardas (mc50, G1 1c, ycsb_c) não recuaram. — status: `todo`
- [ ] **P1.3** 1c vs Fjall absoluto: C Drain=FlushWAL permanece **ou**
      drain 1c cujo `write()` é memcpy-class (não staging cego 0044).
      — status: `todo`
- [ ] **P1.4** SQE `io_uring` off-lock **iff** P1.1 ainda <1× depois de
      C. — status: `todo`

### P2 — o resto da tabela (cada um iff; uma strategy, não tuner)

- [ ] **P2.1** Monkey: \(\mathrm{FPR}_i \propto 1/n_i\) **iff**
      `probe_miss` 100M ainda W. — status: `todo`
- [ ] **P2.2** `flush_check` fora do commit **iff** 0223 split
      (`flush_work_ns`) nomear work no put. — status: `todo`
- [ ] **P2.3** Bound de L0 / cursor lazy **iff** `deps_scan` setup
      ainda W pós-settle. — status: `todo`
- [ ] **P2.4** SILK (priorizar L0, preemptar alto) **iff** p99 restar
      após P2.2. — status: `todo`
- [ ] **P2.5** Uma policy (Lethe **ou** HashKV **ou** Dostoevsky) **iff**
      WA/delete/vlog-GC for o dono medido. Não as três. — status: `todo`
- [ ] **P2.6** mmap/anel WAL **iff** C + E ainda perderem no cartaz.
      — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Este RFC + mapa Θ por op + A–G | done | — | 2026-09-16 |
| P0.2 | p0 | WAL `open_rw` sem `O_APPEND` | todo | — | 2026-09-16 |
| P0.3 | p0 | pwrite no mesmo File, sem `dup` | todo | — | 2026-09-16 |
| P0.4 | p0 | Meter `_mc4` DIAG; default off até subir | todo | — | 2026-09-16 |
| P1.1 | p1 | Linux overwrite_mc4 3-run | todo | — | 2026-09-16 |
| P1.2 | p1 | Flip default iff P1.1 + guardas | todo | — | 2026-09-16 |
| P1.3 | p1 | 1c Fjall C ou FlushWAL-honesto | todo | — | 2026-09-16 |
| P1.4 | p1 | io_uring SQE iff C <1× Linux | todo | — | 2026-09-16 |
| P2.1 | p2 | Monkey FPR iff miss | todo | — | 2026-09-16 |
| P2.2 | p2 | flush_check fora iff | todo | — | 2026-09-16 |
| P2.3 | p2 | L0/scan setup iff | todo | — | 2026-09-16 |
| P2.4 | p2 | SILK iff p99 | todo | — | 2026-09-16 |
| P2.5 | p2 | Uma policy LSM iff WA | todo | — | 2026-09-16 |
| P2.6 | p2 | mmap WAL iff C+E perdem | todo | — | 2026-09-16 |

## Acceptance Criteria

- **Tests:** `rfc0231_pwrite_honors_offset_on_rw_fd` (path de append
  real, não `O_APPEND`; offset do ticket = bytes no ficheiro);
  `rfc0231_no_dup_per_group` (estrutural: com pin on, o path de grupo
  não chama `try_clone_handle`); byte-idêntico vs AS-IS no mesmo
  ficheiro (`rfc0193_*` continua a cobrir o create-path). Meter `_mc4`
  JSON `name=deps_cache_overwrite_mc4` `clients=4` `sync: false`.
- **Telemetry / Analytics:** nenhuma sonda default-on (0169). PHASE
  `wal=` / `lock_wait` sob pin. `PEDRA_WAL_PWRITE=1` continua opt-in
  até P1.2.
- **Documentation:** 0230 P1.1 permanece no-flip; este RFC é o corte
  de **classe**. Scoreboard flipa no mesmo commit. Fjall só absoluto.
- **Screenshots:** backend-only.

## Out of scope

- G1 1c como win; peer `sync=true`; Darwin como cartaz Linux; Rocks
  colapsado (peer ≲157k quieto ou `sync: true`) como vitória.
- Wait-to-grow / `PEDRA_GROUP_WINDOW_US>0`.
- Portar `db/write_thread.cc` sem o iff de `lock_wait` (G).
- Staging cego em 1c (0044 + p209b).
- Fjall como coluna de ratio.
- Auto-tuner eterno de compaction (R012 takeaway C).
- Implementar Dostoevsky **e** Lethe **e** HashKV no mesmo fire
  (P2.5 = uma).
- Substituir o piso RFC-0041 (1×) por 2× neste RFC.
- Montanha/Raft neste programa (write local primeiro).
- Reabrir 0230 P0.4 (clone-fd) como default.
