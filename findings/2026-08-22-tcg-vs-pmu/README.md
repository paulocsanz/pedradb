# TCG + libs de determinismo **não** dão bench de performance sem viés

**Date:** 2026-08-22
**Box:** macOS 14.6.1 arm64, load 10–14 (dirty — experiment-grade, not the quiet 3× arbiter)
**SUT:** `rocksdb-compat` via `hotpath_probe` (no per-op `Instant`)
**QEMU:** Homebrew 10.1.2 system-mode plugin; Ubuntu 24.04 `qemu-x86_64` 8.2.2 user-mode

A pergunta era: dá para usar QEMU TCG + `det_clock`/`det_io` para comparar produtos sem viés de máquina, com precisão de hardware/cache, e achar onde o Pedra ainda é ineficiente vs RocksDB.

Resposta medida: **a caixa TCG deste repo (RFC 0005) é para replay bit-exact, não para ranking de velocidade.** O que o TCG *consegue* ser é um metro de *trabalho* (instruções guest + acessos de memória) independente do host. Wall-clock, cache real e `fdatasync` sob TCG mentem — e as libs de determinismo *apagariam* o sinal de tempo de propósito.

---

## 1. O que foi medido

| Experiência | Artefacto | Resultado |
|---|---|---|
| memcpy 1 KiB vs 16 KiB, mesmo nº de ops | `copy_bias.txt` | rácio wall **muda com o tradutor** |
| Guest insn count TCG, 2 runs | `tcg_insn_count.txt` | **bit-idêntico** entre runs |
| CPU-only Pedra (async WAL) | `native_probe.txt` | GET 31 ns; SET 2.8 µs; blob 45 µs; pipe 6.5 µs/batch |
| `sample` 10 s @ 1 ms | `*_sample.txt` | onde o tempo vai de verdade |

`hotpath_probe` (`crates/rocksdb-parity-bench/examples/hotpath_probe.rs`) tira o `Instant::now()` por op — no GET oficial isso era ~61% das samples (`findings/2026-08-22-p13-get-tls-cache/`).

---

## 2. TCG wall-time enviesa o ranking (número, não teoria)

Mesmo workload: `OPS=2^20` memcpy 1 KiB depois 16 KiB.

| Plataforma | 1 KiB | 16 KiB | **16k / 1k** |
|---|---:|---:|---:|
| clang nativo arm64 | 13.9 ms | 274 ms | **19.6** |
| Docker linux/arm64 (Virtualization.framework) | 14.0 ms | 236–253 ms | **16.8–18.0** |
| Docker linux/amd64 (Rosetta, **não** TCG) | 16.0 ms | 212 ms | **13.2** |
| `qemu-x86_64` TCG user (Ubuntu 8.2.2 no VM arm64) | 108–112 ms | 23.7–24.7 **s** | **220** |

TCG diz que 16 KiB é **220×** o 1 KiB. O hardware desta caixa diz **~20×**. Distorsão de **11×** no ranking. Wall-time TCG apontaria o blob como *o* problema por uma ordem de magnitude a mais do que o silício.

O QEMU documenta isto sem rodeios: *icount “should not be confused with cycle accurate emulation — QEMU does not attempt to emulate how long an instruction would take on real hardware”* (`https://www.qemu.org/docs/master/devel/tcg-icount.html`).

`det_clock` intercepta `clock_gettime` / `nanosleep` e devolve um relógio lógico. `det_io` pode **drop** de `fdatasync`. As duas libs existem para DST (mesmo seed ⇒ mesma história). Usá-las num bench de performance **apaga** o sinal que o Rocks-parity precisa (tempo real de `write`/`fdatasync`/cache). A coluna async do Pedra (`PEDRA_PARITY_ASYNC=1`) já é o equivalente honesto de “fsync drop” — e está marcada *not G1, not official*.

---

## 3. O que o TCG *é* bom: trabalho guest, determinístico

Plugin `plugin/insn_count.c` (API 5, qemu-system 10.1.2) em guests RISC-V `virt`, 50k / 20k / 2M ops fixos, **2 runs**:

| Guest | run1 insns | run2 insns | memops |
|---|---:|---:|---:|
| add-loop 2M | 10 000 031 | 10 000 031 | 4 000 008 |
| copy 1 KiB × 20k | 102 564 133 | 102 564 133 | 41 001 031 |
| copy 16 KiB × 20k | 1 638 645 579 | 1 638 645 579 | 655 416 391 |

- 16k/1k **insns = 15.977**; **memops = 15.985**. É o factor 16 do payload, ~5 insn/byte dum loop RISC-V sem SIMD.
- Bit-idêntico entre runs: o metro *não* vaza o host.
- Ubuntu `qemu-user` 8.2.2 **não** foi compilado com plugins (`-plugin` = unknown option). No Mac, Homebrew só tem `qemu-system-*`. Contar insn dum processo Pedra inteiro nesta caixa pede um guest Linux (RFC 0005) — lento, e o kernel entra na conta se não se separar seed/timed.

O plugin oficial `contrib/plugins/cache.c` do QEMU modela L1/L2 **com geometria que tu escolhes** (não a do host). Isso *é* cache sem viés de máquina — um modelo, não o Apple M-series. Não correu nesta sessão (user-mode sem plugins; system-mode só insn). `MEASURE` se um dia quisermos locality cross-engine.

---

## 4. Onde o Pedra gasta tempo de verdade (rocksdb-compat, async)

Probe nativo, 1024 chaves, payload 1 KiB / blob 16 KiB, `PEDRA_PARITY_ASYNC=1`.

Janela curta (2M get/set):

| phase | ops | ns/op | qps |
|---|---:|---:|---:|
| get | 2 000 000 | **30.9** | 32.4 M |
| set | 2 000 000 | 2797 | 357 k |
| blob | 250 000 | 45 079 | 22.2 k |
| pipe (32-wide) | 20 000 | 6496 | 154 k batches ≈ 4.9 M keys/s |

`sample` 10 s (top-of-stack, dirty):

### GET — já não é o sítio

TLS `LastGetTable::get_key` + `memcmp` + um pouco de `AnswerCache`. 22–32 M qps CPU-only. O gap oficial ≥5× vs Rocks GET é o harness (`Instant` ×2) + o Rocks a ~400 ns, não o motor a 31 ns. Não caçar TCG aqui. O P1.3 (lazy epoch, 2048×probe-8) já fez o trabalho.

*Caveat do probe com `SECONDS>0`:* o deadline chama `Instant::now()` **por op** e volta a poluir (`mach_absolute_time` no sample GET). Usar `OPS=` fixo para perfilar o engine.

### SET 1-op — `write()` 87% da main thread

```
write                          7544
HashMap::remove (AnswerCache)   186
memcmp                          147
crc32c                           89+65
MemTable::insert_map_gc          67
BTreeMap::insert                 61
memmove                          52
```

Mesmo sem `fdatasync`, o `write()` do WAL a 64 KiB é o teto desta caixa. O CPU que sobra (13%) é invalidar o point-cache + CRC + GC de versões no BTree. 14 s de overwrite em 1024 chaves empilha versões (horizonte 24 h, RFC-0046) — qps cai 357 k → 199 k.

### blob 16 KiB — também `write()`, não memcpy userspace

```
write     8687
cvwait    8137   (worker compact)
memmove     17
crc32c      40
```

O RFC-0044 dizia “copies de 16 KB dominam”. Neste perfil o memcpy userspace é ruído; o que escala com 16× payload é o **`write()` do frame WAL**. `OpenOptions.large_value_threshold` default **`None`** (sempre inline). `rocksdb-compat` **não liga vlog**. Cada blob de 16 KiB entra em WAL + memtable. 4 blobs enchem o buffer de 64 KiB ⇒ um `write()` — bate com ~10 k qps na janela longa (99 µs/op).

### pipe 32× — memtable + allocator + write

Além de `write`: `insert_map_gc`, `BTreeMap::insert`, `malloc`/`free` tiny, `apply_ops_owned`, `gc_below_floor`, `KeyCodec::encode_pooled`. Payload internado (`put_batch_same` / WAL v2) já tirou a cópia do valor; o que resta é **chave + versão + heap**.

---

## 5. O que optimizar para ir *ainda* além do Rocks (coluna async)

Ordem pelo perfil, não por TCG:

| # | Sítio | Porquê | Alavanca | RFC |
|---|---|---|---|---|
| 1 | **blob inline no WAL** | `write()` de 16 KiB; vlog opt-in nunca ligado no compat | `large_value_threshold` (WiscKey já no core) no drop-in / no shape `kvrocks_blob_set` | 0014 / 0044 P1.2 |
| 2 | **SET 1-op `write()`** | 87% da sample; teto same-class = Rocks file writer 64 KiB | não aumentar o buffer acima de 64 KiB (contrato); io_uring / batch de 1-op não existe neste shape | 0044 |
| 3 | **AnswerCache `HashMap::remove` por put** | 2º sítio de CPU no SET | invalidação mais barata no overwrite quente (já há dirty-log no CountCache) | 0044 GET path |
| 4 | **MemTable versões no overwrite** | `insert_map_gc` + BTree + deque; 14 s de zipf empilha | horizonte já é 24 h; no *bench* overwrite quente o Rocks GC'a, nós não — é produto, não bug. `auto_reclaim` / `ROCKS_PARITY_RETENTION=rocks` é outra coluna | 0046 D5 |
| 5 | **pipe allocator + encode de chave** | malloc tiny + `encode_pooled` | pool de `BatchOp` / InternalKey; já há KEY_POOL | 0044 P1.1 |
| 6 | **mc50 / `deps_lock_prewrite`** | fora deste probe | merge líder já falsificado (0.19×); lock manager 0.94 | 0044 P0.5, 0041 |

O que **não** é alavanca:

- Contar insn TCG do GET (já 31 ns).
- `det_clock` no harness (esconde o Instant, não o motor).
- Wall-time TCG do blob (220× vs 20× nativo — apontaria o sítio certo pela razão errada, com magnitude falsa).
- Largar G1 para “bater o Rocks” (AGENTS.md).

---

## 6. Receita se um dia quisermos metro sem viés de máquina

1. **Trabalho ISA:** guest insn + memops (plugin TCG *ou* PMU `INST_RETIRED` no mesmo ISA). Dois motores, mesmo schedule, mesmo binário de harness. Número estável = este finding §3.
2. **Locality, geometria fixa:** QEMU `cache` plugin *ou* Cachegrind, L1/L2 declarados. Não a cache do host.
3. **Tempo real / onde optimizar:** `sample` / `xctrace` / `perf` no hardware alvo, caixa quieta, **sem** `Instant` no loop. É o §4.
4. **I/O:** modelo explícito (fd p50 isolado, já em rfc0041-p02) — TCG virtio não substitui.

As três primeiras são independentes; misturá-las num único “QEMU TCG bench” é o viés que esta sessão mediu.

---

## 7. Shipped 2026-08-23 — vlog async buffer + BlobDB knobs

`set_enable_blob_files` / `set_min_blob_size` were no-ops. Wiring them
without fixing `ValueLog::append` (it `sync_all`ed every record) would have
made blob *slower*. Now:

- spill uses `append_pending` (64 KiB `write()`, get can read the tail)
- G1: one vlog fsync per commit **before** the WAL pointer is durable
- async: `write()` only, same class as WAL
- drop-in default stays **off**; parity harness enables 4 KiB
  (`ROCKS_PARITY_MIN_BLOB=0` = inline)

A/B dirty (`blob_ab.txt`, `set_ab.txt`, load ~8–9):

| | inline | vlog 4 KiB |
|---|---:|---:|
| blob 16 KiB | 9.1 k qps | **11.2 k (+22%)** |
| GET 1 KiB | 33.5 M | 34.0 M |
| SET 1 KiB | 103 k | 94 k (paired; disk dirty) |

Not 5×. The 16 KiB still hits the kernel; it just is not copied into every
WAL record / memtable value.

## Reproduzir

```bash
# CPU-only Pedra (async, not G1)
PEDRA_PARITY_ASYNC=1 cargo run --release -p rocksdb-parity-bench --example hotpath_probe -- /tmp/hp
PEDRA_PARITY_ASYNC=1 PHASE=set SECONDS=14 cargo run --release -p rocksdb-parity-bench --example hotpath_probe -- /tmp/hp-set &
sample $! 10 1 -mayDie -file set_sample.txt

# TCG wall-time bias + (se qemu-user-static) insn
bash findings/2026-08-22-tcg-vs-pmu/run_tcg_copy_bias.sh
```
