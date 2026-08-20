# RFC-0044 P1 — kvrocks async/async (dirty)

## `get-longwindow/` — GET em janela longa (P1.3): **4.4–5.5× vs peer são**

`ROCKS_PARITY_ONLY=kvrocks_get`, 20 M ops (default da suíte = 2 000),
load ~14, Rocks **saudável** (1.56–1.62 M qps, p50 0.5 µs):

| run | Pedra | ratio vs Rocks 1.62 M/1.56 M |
|---|---:|---:|
| base r1 | 8.41 M | 5.20 |
| base r2 | 7.06 M | 4.54 |
| revert-check | 8.64 M | 5.3 |

A janela de 2 000 ops da suíte **subestima** o GET dos dois, mas
desproporcionalmente o Pedra (cold start do point-cache + overhead fixo).
Medição no lado do engine: `Instant::now`+`elapsed` custa **62 ns/op
dentro da janela** para os dois engines — em op de ~130 ns medido isso
comprime o ratio do mais rápido (engine ~70 ns vs Rocks ~500 ns ≈ 7×
real → 4.5–5.2 medido).

Front array direto (1024 slots, fingerprint-first) no `AnswerCache`:
**negativo** — 7.68/6.92 M vs base 8.41/7.06 M. Com chave de 8 B e mapa
quente, lock+hash+probe+clone-RC dominam por igual nos dois caminhos;
trocar só o probe não move. Revertido; JSONs `compat-front-*`.

P1.3 no oficial (full-suite, 2 000 ops) segue 3.97†. Conclusão: o 5× do
GET é questão de janela de medição + peer são tanto quanto de engine.
Árbitro: P2.1 (quieta 3×).

## `kvrocks-l14/` — same-run na melhor janela da sessão (load ~14)

| shape | Pedra | Rocks | ratio | nota |
|---|---:|---:|---:|---|
| set_mc50 | 292 k | 32 k | **9.24** | Rocks doente (p50 0.43 ms, max 20 ms) |
| SET 1c | 1.67 M | 296 k | **5.66** | Rocks no nível saudável; **crossing real** |
| scan | 688 k | 49 k | **14.0** | — |
| GET | 3.38 M | 800 k | 4.22 | Rocks GET baixo de novo (saudável ~1.7 M) |
| blob | 153 k | 50 k | 3.03 | melhor que 1.62; copies de 16 KB dominam |
| pipeline | 38 k | 40 k | **0.96** | p50 Pedra 4.4 µs vs 22 µs; cauda `write()` 4 ms decide o wall |

Pipeline p50 continua 5× melhor que o Rocks; o wall é um punhado de
`write()`s de 64 KiB que pararam 1–4 ms (disco sujo). Na `kvrocks-64k`
(load ~14, disco calmo) foi **11.3×**. mc50 9.24 usa peer doente — vs
Rocks saudável (54–92 k) fica 3.2–5.4. **Not official.** Arbitro = 3×
quieta (P2.1).

## `kvrocks-merge/` + A/B — merge de escritores async (P0.5): **negativo**

Hipótese: agrupar os escritores async num líder (um encode + um
`write()` por grupo, sem espera de catch-up) fecharia o 5× do mc50.
A/B pareado (mesma carga, alternando `PEDRA_ASYNC_GROUP` 1/0,
`ROCKS_PARITY_ONLY=kvrocks_set_mc50`, 5 rounds):

| round | merge qps | bypass qps | merge/bypass |
|---|---:|---:|---:|
| 1 | 106 k | 434 k | 0.24 |
| 2 | 102 k | 636 k | 0.16 |
| 3 | 69 k | 526 k | 0.13 |
| 4 | 44 k | 433 k | 0.10 |
| 5 | 84 k | 311 k | 0.27 |

**Mediana 0.19× — 5× pior.** Com 50 threads em 12 CPUs sujas, o líder é
ponto único de agendamento: seguido parado = todos parados (p50 merge
~200 µs vs bypass 0.6–1.3 µs). O bypass (N threads + um write lock, o
formato Rocks) é o default; o merge fica atrás de `PEDRA_ASYNC_GROUP=1`
para reteste em caixa quieta.

Same-window bypass vs Rocks (mc50-only, 3 rounds pareados):
Pedra 351–377 k (p50 0.7 µs), Rocks 70–92 k (p50 128–366 µs),
**ratio 4.0–5.0, mediana 4.41**. Full-suite (kvrocks-64k): 3.38. O gap
restante é handoff de lock sob oversubscription + caixa suja — arbitro
honesto é a remesura quieta (P2.1). JSONs: `kvrocks-merge/` (full
same-run compat+rocks) e os rounds A/B em `kvrocks-merge/ab/`.

## `kvrocks-64k/` + `ycsb-64k/` — `ASYNC_WAL_BUFFER` 64 KiB

Rocks `writable_file_max_buffer_size`. Encode antes do Ok. `write()` ao
encher 64 KiB. G1 intocado. Não é 1 MiB.

| shape | ratio @ 64 KiB | @ 32 KiB | cada `write()` |
|---|---:|---:|---:|
| scan | **50.0** | 41 | 50 |
| pipeline | **11.3** | 8.55 | 4.99 |
| SET 1c | **3.86** | 2.61 | 1.29 |
| mc50 | **3.38** | 3.40 | 1.00 |
| blob | **1.62** | 0.85 | 1.96 |
| GET | 3.97† | 1.17 | 1.11 |
| ycsb A | 2.18 | — | 1.03 |
| ycsb F | 1.24 | — | 0.92 |

† Rocks GET 888 k nesta run (baixo). JSON em `kvrocks-64k/compare/`.


**Not official. Not the product. Not “we beat Rocks.”**

Column: Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `WriteOptions.sync=false`.
Mix: 4096 / 2000 / 1 KB / zipfian / `ROCKS_DEPS_BATCH=32`.
G1 default is unchanged.

## `kvrocks-write/` + `ycsb-write/` — contrato certo (`write()` no Ok)

Load 14–25 / 12 CPUs. Async = `write()`, **sem** `fdatasync`. Os 5× de
SET/pipeline/blob em `kvrocks3/`–`kvrocks4/` **não valem** (acked em
userspace). JSON: `kvrocks-write/compare/compare_report.json`.

| shape | Pedra | Rocks | ratio |
|---|---:|---:|---:|
| scan | 1.15 M | 23 k | **49.8** |
| pipeline | 75 k | 15 k | **4.99** |
| ycsb_e | 594 k | 122 k | **4.85** |
| ycsb_c | 5.84 M | 1.61 M | 3.63 |
| blob | 76 k | 39 k | 1.96 |
| ycsb_d | 2.37 M | 1.27 M | 1.86 |
| ycsb_b | 2.15 M | 1.21 M | 1.78 |
| SET 1c | 229 k | 178 k | 1.29 |
| GET | 1.88 M | 1.69 M | 1.11 |
| ycsb_a | 440 k | 426 k | 1.03 |
| set_mc50 | 73 k | 73 k | **0.997** |
| ycsb_f | 308 k | 336 k | **0.92** |

## `kvrocks-32k/` — `write()` a cada bloco WAL 32 KiB (Rocks-shaped)

Não é 1 MiB. Ops já encoded no frame. Crash de processo pode perder o
tail &lt; 32 KiB (classe Rocks `WritableFileWriter`).

| shape | write() cada put | 32 KiB block |
|---|---:|---:|
| SET 1c | 1.29 | **2.61** |
| set_mc50 | 1.00 | **3.40** |
| pipeline | 4.99 | **8.55** |
| blob 16 KB | 1.96 | **0.85** (2 blobs enchem o bloco) |
| GET | 1.11 | 1.17 |
| scan | 49.8 | 41.2 |

## `kvrocks/` — first remesure (shared-Bytes pipeline only)

Load: dirty. `compat.sync=false` `rocks.sync=false`.

| shape | Pedra | Rocks | ratio | vs x5b | ≥5? |
|---|---:|---:|---:|---:|:---:|
| `kvrocks_scan` | 804k | 20.7k | **38.85** | 44 | yes |
| `kvrocks_set_mc50` | 237k | 55.6k | 4.27 | 5.07 | no (was yes) |
| `kvrocks_set` | 588k | 205k | 2.88 | 2.14 | no |
| `kvrocks_get` | 1.87M | 1.10M | 1.70 | 1.43 | no |
| `kvrocks_pipelined_set` | 12.3k | 11.3k | 1.09 | 1.56 | no |
| `kvrocks_blob_set` | 42.2k | 48.6k | 0.87 | 1.96 | **lose** |

Pipeline p50 Pedra 37 µs vs Rocks 51 µs (already faster on median); wall
mean ~81 µs. Interned `Bytes` in `put_batch_same` did **not** close 5×.

Load at remesure start: **29 / 39 / 46 on 12 CPUs** (`uptime` 17:58). Dirty.

## `kvrocks2/` — WAL v2 reuse + intern + insert_many + get_probe

Code (RFC-0044 P1.1/P1.3, this slice):

- WAL record v2: consecutive interned values stored once (`kind | 0x80`)
- `BatchOp::put` interns identical payloads (SET / blob)
- `MemTable::insert_many` (one `tail_ord` invalidate per batch)
- `put_batch_same` moves default-CF keys (`Bytes::from`)
- dirty-points gen-bump at `>= 32` (pipeline is 32)
- `get_probe` / `DB::contains` (no 1 KB `to_vec` on GET canary)

Load: 35–49 / 12 CPUs. `compat.sync=false` `rocks.sync=false`.

Pedra **CPU** (vs `kvrocks/` this folder): pipeline 12.3k → **50.3k** (p50 37 µs → 13 µs);
blob 42k → **100k**. WAL v2 + intern worked on Pedra.

Rocks on this run is **sick** (SET 24k vs 205k on `kvrocks/`; pipeline 5.7k vs 11.3k).
Do **not** sell 21× / 8.85× as the 5× close.

| shape | Pedra | Rocks this run | ratio | vs healthier Rocks `kvrocks/` | ≥5 vs healthy? |
|---|---:|---:|---:|---:|:---:|
| `kvrocks_scan` | 683k | 13.5k | **50.6** | 33 | yes |
| `kvrocks_set` | 515k | 24.3k | 21.2† | 2.52 | no |
| `kvrocks_pipelined_set` | **50.3k** | 5.7k | 8.85† | **4.45** | no (close) |
| `kvrocks_set_mc50` | 224k | 43.8k | **5.11** | 4.03 | p0 was 5.07 |
| `kvrocks_blob_set` | 100k | 26.6k | 3.77† | 2.06 | no |
| `kvrocks_get` | 1.33M | 1.03M | 1.30 | 1.21 | no |

† Rocks qps collapsed under load; Pedra p50 is the number that counts until a quiet 3× (P2.1).

## `kvrocks3/` — WAL 1-op coalesce + keys fora da janela

Load ~50 / 12 CPUs. `compat.sync=false` `rocks.sync=false`.

Pedra: SET p50 **0.9 µs** (981 k), pipeline p50 **5 µs** (134 k), blob p50 **0.8 µs** (1.0 M).
Rocks nesta run está **sano** no SET/pipeline (248 k / 12.8 k), ao contrário de `kvrocks2/`.

| shape | Pedra | Rocks | this | vs Rocks `kvrocks/` | ≥5? |
|---|---:|---:|---:|---:|:---:|
| `kvrocks_pipelined_set` | **134 k** | 12.8 k | **10.5** | **11.8** | **sim** |
| `kvrocks_blob_set` | **1.00 M** | 48.2 k | **20.8** | **20.6** | **sim** |
| `kvrocks_set` | 981 k | 248 k | 3.95 | **4.79** | quase (quiet) |
| `kvrocks_set_mc50` | 267 k | 54.9 k | 4.86 | 4.80 | p0 5.07 |
| `kvrocks_get` | 3.66 M | 1.58 M | 2.32 | 3.33 | não (p50 <50 ns; wall=cauda) |
| `kvrocks_scan` | 46 k | 24.9 k | 1.85 | 2.23 | **não nesta run** (max 35 ms); x5b/kvrocks2 era 33–44× |

Pipeline e blob fecham o piso contra o peer saudável. SET 1c falta ~4% (981 k vs 1.02 M).
GET 5× é cauda, não p50. SCAN 35 ms = uma parada na caixa suja — não reescrever o 33× anterior.

## `kvrocks4/` + `ycsb/` — get_probe + ytab no YCSB

Load 14–25 / 12 CPUs. Ainda dirty.

Kvrocks vs Rocks `kvrocks/` (peer 205 k SET / 11.3 k pipeline):

| shape | Pedra | this Rocks | this | vsH | ≥5 vsH? |
|---|---:|---:|---:|---:|:---:|
| set | **1.02 M** | 139 k | 7.37† | **5.00** | **sim** |
| pipeline | 112 k | 15.2 k | **7.39** | **9.92** | **sim** |
| blob | 808 k | 54.3 k | **14.9** | **16.6** | **sim** |
| scan | 321 k | 23.0 k | **13.9** | **15.5** | **sim** |
| mc50 | 264 k | 77.1 k | 3.43 | 4.76 | p0 5.07 |
| get | 3.78 M | 1.65 M | 2.29 | 3.44 | não |

† Rocks SET 139 k vs 205 k saudável — o 7.37 não é o close; o 5.00 vsH é.

YCSB same-run (os dois com ytab/`get_probe`):

| shape | Pedra | Rocks | ratio | vs x5b Rocks |
|---|---:|---:|---:|---:|
| A | 683 k | 372 k | 1.84 | 2.32 |
| B | 1.19 M | 1.11 M | 1.07 | 1.47 |
| C | 2.66 M | 1.58 M | 1.69 | 3.81 |
| D | 1.79 M | 1.23 M | 1.46 | 3.55 |
| E | 435 k | 111 k | 3.91 | 5.92 |
| F | 549 k | 297 k | 1.85 | 2.22 |

Rocks também acelerou com keys fora da janela; same-run E caiu de 5.61 (x5b) para 3.91. P2.2 não fecha nesta caixa.
