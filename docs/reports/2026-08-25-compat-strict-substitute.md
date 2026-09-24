# rocksdb-compat como substituto estrito do Rocks — só vantagem, nunca defeito

**Data:** 2026-08-25 (addendum ao [launch-readiness](2026-08-25-launch-readiness.md))
**Âmbito:** **só** `crates/rocksdb-compat` (+ shim `crates/rocksdb` 0.21). Kernel `Db`, Montanha, TLS, seL4, Intel-como-meta: **fora**.
**RFC:** [0062](../rfc/0062-launch-readiness-remaining-gaps.md)
**Peer:** o Rocks que o host rust-rocksdb realmente liga — default `WriteOptions.sync=false`. Quando o host chama `set_sync(true)`, o peer é Rocks `sync=true`. Nunca misturar as duas.

---

## 0. Por que não rodei “Linux Intel no caixote”

Não existe Intel na pool. Conferido agora:

| facto | valor |
|---|---|
| região | `brasil` |
| `caixote_id` de **todos** os serviços | `platform-4d79d23b377dd353ab9bbd7a83e3680d` |
| cpuinfo das baterias | **AMD** Ryzen Threadripper PRO 3975WX (4 vCPU virt) |
| BYOC | só o Mac aarch64, `disconnected` |
| metal extra que já rodámos | Railway **AMD** EPYC 9655, 48 vCPU — também não é Intel |

RFC-0059 já dizia Intel = “não-meta”. Eu elevei isso a blocker de lançamento no relatório anterior. **Erro meu.** O Linux que temos é AMD. “Todos os Linux” = esta VM 4 vCPU **e** o metal 48 vCPU, não um fornecedor que a plataforma não tem.

Por que não disparei uma bateria **nesta** volta: a volta era escrever o relatório; `ej-backfill` está a 8 vCPU no mesmo host; a última tentativa de provisionar `linux-diag-6` falhou com saldo negativo e “all containers failed before first start”. Isso **não** desculpa ter inventado Intel. A remesura que falta é **AMD + `pwrite`**, não Xeon.

---

## 1. G1, sync-peer, full-sync — o que é cada um (compat only)

Três comparações. Eu misturei a (B) com a (C) no relatório e pareceu que o compat é 1000× pior em sync. **Não é.**

| nome | compat (`rocksdb-compat::Options::sync`) | Rocks (`WriteOptions.sync`) | o que mede | o que **não** é |
|---|---|---|---|---|
| **A. Drop-in** (default do crate) | `false` (RFC-0054) | `false` (default de fábrica) | substituto na config que 99% dos hosts usa | “G1” |
| **B. Same-class sync** (“sync-peer”, `ROCKS_PARITY_SYNC=1`) | `set_sync(true)` | `true` | substituto quando o host pediu durabilidade | vitória oficial contra o Rocks *default* |
| **C. G1 vs default** (`PEDRA_PARITY_G1=1`) | `true` | `false` | “mais durável **e** mais rápido que o Rocks que as pessoas correm” | full-sync; **não** é “somos ruins em sync” |

### O que é “sync-peer” e por que AGENTS.md diz que nunca “batemos o Rocks” nele

**Sync-peer** = o lado Rocks do harness também liga `WriteOptions.sync=true` (`ROCKS_PARITY_SYNC=1`). É uma **coluna de medição**, não o default do crate.

A regra do AGENTS.md não diz que somos lentos. Diz: **não chames uma vitória contra o Rocks lento de “batemos o Rocks”.** O Rocks que as pessoas correm é `sync=false` (rápido, perde writes no power loss). Comparar o nosso `set_sync(true)` com o Rocks `sync=true` é justo como *same-class*; vender isso como o cartaz é trapacear o denominador.

Para o **substituto**, a regra certa é outra:

- Host não mexeu em sync → coluna **A**. Temos de ser **sempre >1×**.
- Host chamou `set_sync(true)` → coluna **B**. Temos de ser **sempre >1×** contra Rocks `sync=true`.
- Coluna **C** (nós sincamos, eles não) **não entra** no contrato de substituto. É um boast de produto. Em 1 cliente write-per-op é fisicamente `1/fdatasync` vs zero barriers — 0.001–0.056×. Isso **não** prova que o caminho sync do compat é podre.

### Darwin atual (este Mac) — não citar 2026-08-16

`tikv-ycsb-0035-fullsync` (16/08) é motor morto: iterador eager, pré-F219,
pubfix, pack32, default async. **Não cronometrar isso.** O que existe
**nesta caixa**:

**Coluna A — default do compat** (`PEDRA_PARITY_ASYNC=1` vs Rocks `sync=false`).
rearm11, 2026-08-23, 3 rounds, ops=2000, JSON diz explicitamente
`NOT G1, not official` no campo durability:

| shape | × Rocks default |
|---|---:|
| ycsb_a / d / e | 4.79 / 4.59 / 4.96 |
| kvrocks_set / get / scan | 4.4 / 5.7 / **28** |
| apply / mvcc | 2.08 / 2.87 |
| raftlog | 1.05 |

Isto é o “amassávamos o Rocks async”. **G1 estava desligado.**

**Mesmo dia, mesmo protocolo smoke (1024/200), irmãos floor1x vs floor1x-g1,
2026-08-24 — a prova de que o motor não “piorou”:**

| ycsb_a neste Mac | p50 | qps | vs Rocks default |
|---|---:|---:|---:|
| A (async, G1 off) | **0.40 µs** | 2.08 M | **3.48×** |
| C (G1 on = `F_FULLFSYNC`) | **3.82 ms** | 339 | **0.001×** |

3.82 ms / 0.40 µs ≈ **9 500×**. Não é regressão: é uma barreira Darwin
completa por op contra **zero** do peer. Leituras na mesma bateria G1
continuam a ganhar (C 1.22×, mvcc 1.80×) — o read path não sinca.

**Coluna B — G1 vs Rocks `sync=true`+`F_FULLFSYNC`** (mesma barreira).
Medido 2026-08-25 neste Mac, 1 run, load 15–23 (suja), protocolo 1024/200:
[`findings/2026-08-25-g1-vs-rocks-sync-ff`](../../findings/2026-08-25-g1-vs-rocks-sync-ff/README.md).

| ycsb_a | p50 | qps | ratio |
|---|---:|---:|---:|
| Pedra G1 | 4.85 ms | 198 | |
| Rocks FF | 4.73 ms | 324 | **0.61** |

p50 **empatou**. O 0.61× é cauda (p99 28 vs 9) sob load, não o syscall.
Leituras C 1.69×, mvcc 1.52×; raftlog 1.13×. Apply 0.17× tem max 2.9 s —
contaminação, não cartaz. Quiet 3/3 = RFC-0062 P1.1.

O único G1 que já bateu Rocks async em escrita é **group commit**, não 1c:
`apply_mc4` **2.788×** (head3). 1c write-per-op com G1 **nunca** ganhou
(RFC-0041 P0.2 já era 0.056× no Linux com `fdatasync` 26 µs; no Darwin
`F_FULLFSYNC` 3.8 ms o mesmo contrato cai a 0.001×).

Coluna A Linux: 13/16 ≥2×. Buraco = `deps_raftlog` min<1×; blob rounds <1×.

---

## 2. O que falta para Linux >1× **sempre**, em todos os Linux, no compat

Contrato: `min(3 rounds quietos) > 1.0` em **toda** shape oficial, coluna **A**, em:

- caixote brasil 4 vCPU virt (AMD Threadripper) — a classe do gate
- metal Linux (já temos AMD EPYC 48 vCPU)

Não: “precisa Intel”. Não: “precisa 2× em raftlog 1c”.

### Onde já somos >1× no Linux (coluna A)

YCSB A–F, lock, overwrite, mvcc, kvrocks get/set/scan/pipeline, apply (~1.95). Surreal 1.5.4: 4/4 >1× (write 2.41, read 1.79, txn 1.40, scan 1.33).

### O único shape oficial que **não** é >1× sempre: `deps_raftlog`

| evidência | mediana | p50 | o que prova |
|---|---:|---|---|
| linux-anti1 4 vCPU (pré-pwrite) | 0.91 | — | bateria oficial: **FAIL 1×** |
| linux-diag5 isolado | **0.78** | fases ~11 µs, wall 240 µs | cauda de stall naquela VM |
| Railway metal 48 vCPU **pré-pwrite** | **0.81** clean | batch 5.8 µs, **zero** op >1 ms | **não é 4 vCPU, não é Intel, não é stall** — p50 de mem+wal |
| linux-diag6 4 vCPU **pós-pwrite** | **0.979** (2/5 >1×) | **empatado** 10.0–10.4 vs 10.1–11.5 µs | o imposto `submit_and_wait` saiu; p50 empatou; p99 nosso ~32 vs ~20 |

Leitura: em Linux quieto o append de 16 chaves **empatou no p50** depois do `pwrite`. Perdemos nos rounds em que o Rocks está quieto no p99 (nós 32 µs, eles 20 µs). Ganhamos quando o Rocks flusha. Isso **não** é “sempre >1×”.

Metal 0.81 é **pré-pwrite**. Hipótese viva nº 1 (a que se mede a seguir, não Intel): a mesma bateria metal/4 vCPU **com** `pwrite` passa de 0.81/0.78 para ≥1.0 na mediana **e** no min. A 4 vCPU já foi a 0.979 — falta o min>1.0.

### O que cortar para min>1.0 (não 2×)

p50 já empatou. Falta ~5–25% de margem para o p99 e para o Rocks quieto:

1. **Remesura oficial** 3×16 shapes, `pwrite` atual, `rm -rf` do DB **entre shapes** (diag-6 mostrou `ycsb_c_big` a deixar 1.1M keys no memtable do raftlog — isso **não** é o caminho de um host Rocks típico; é poluição do harness).
2. Se ainda min<1.0 com memtable pequeno: o p99 32 vs 20 µs. Candidatos nomeados, um de cada vez, número antes de teoria:
   - encode WAL do batch de 16
   - insert BTree de 16 chaves sequenciais (raftlog)
   - publish/count-cache no caminho (já foi 28% uma vez; F204)
3. **Não** skiplist, **não** journal-CF, **não** skip fsync — o p50 já empatou sem isso.
4. 2× neste shape 1c: recusado enquanto o p50 estiver empatado. Substituição pede **>1× sempre**, não 2×.

`kvrocks_blob_set` (rounds 0.85–1.4) volta para o cartaz se “nunca defeito”. Hoje está parked. Ou des-estaciona e fecha >1×, ou o anúncio não diz “todas as shapes”.

---

## 3. Barra de substituto: 100% paridade, só vantagem, nunca defeito

Definição operacional (senão “100%” é C++ ABI + 15 anos de frota e ninguém fecha):

Um programa P escrito contra **rust-rocksdb 0.22 / 0.21**, reconstruído contra `rocksdb-compat` (ou o shim `rocksdb` 0.21):

| # | invariante | hoje | fecha como |
|---|---|---|---|
| S1 | **compila** com a superfície que P chama | Surreal 1.5.4 sim; `Checkpoint`/`BackupEngine`/`TransactionDB`/Env/SstFileManager **não** | implementar os **tipos** que o compilador encontra; não C++ |
| S2 | **mesmo KV observável** (put/get/scan/batch/CF/snapshot/OCC) | sim no adversarial + model | manter; tombstone vs unlink tem de *acabar* com as keys (já) **e** libertar espaço (compact) |
| S3 | **nunca silent-wrong** | sim (CRC, skip-any ausente, fence) | vantagem — pode ser *mais* estrito que Rocks se o host não depender do modo podre |
| S4 | **throughput ≥ Rocks** com os **mesmos** `WriteOptions.sync` que P setou, Linux, min 3 rounds >1.0, todas as shapes que P exercita | 13/16 sim; raftlog não; blob às vezes não | §2 |
| S5 | knobs que P seta ou **fazem o que o nome diz** ou **não deixam P mais lento** que o Rocks com esse knob | dezenas de `set_*` são `{}` | Wired, ou Inert **provado** (teste: ligar o knob não piora vs Rocks), nunca silent-wrong de correção |
| S6 | crash/open: PIT do compat = default Rocks; FailClosed = opt-in mais seguro | sim (RFC-0047) | vantagem se o default do drop-in continuar PIT |

**Fora desta barra** (não são defeitos do *crate substituto*; são outro produto):

- Abrir um diretório SST C++ Rocks já existente (layout Pedra). Migração ≠ drop-in de library.
- JNI / ABI C++.
- `paranoid_checks=false` e skip-any: Rocks aceita silent-wrong. Nós recusamos. Isso é **vantagem** desde que o host não tenha `if paranoid == false { /* required */ }`. Se tiver, o compile/runtime tem de ser um erro **tipado**, não um no-op.
- Anos de frota / firmware. Não se programa.

---

## 4. Inventário de defeitos do compat (os que o host sente)

### 4.1 Performance (S4) — defeitos reais

| defeito | evidência | fecho |
|---|---|---|
| `deps_raftlog` Linux min<1.0 | 0.78–0.98 | remesura pwrite + rm-per-shape; se falhar, p99 encode/mem (§2) |
| `kvrocks_blob_set` rounds <1.0 | 0.85–1.4 | des-estacionar; vlog spill vs inline; não overindex |
| harness 256 MiB memtable vs Rocks 64 MiB | documentado em rocksdb-compat.md | o **produto** drop-in é 4 MiB; o bench pina 256 para não flushar. Host que seta 64 MiB tem de ver 64 MiB de verdade (já `set_write_buffer_size` está wired) |

Coluna C (G1 vs async) **não é defeito do substituto.** Default do compat é async.

### 4.2 Compile (S1) — defeitos reais

| API rust-rocksdb | compat | fecho |
|---|---|---|
| `DB`, put/get/delete/batch/snapshot/iter/CF/ingest/WBWI/OCC/merge/filter | há | — |
| `Checkpoint` | **não** (core tem `create_checkpoint`, tipo rust-rocksdb não) | wrap |
| `BackupEngine` | **não** (`pedra backup` é outro nome) | wrap `pedradb-ops` |
| `TransactionDB` 2PL | **não** (só Optimistic) | v2; v1: o host 2PL **não** substitui — falha de compile, honesto |
| `Env` / `SstFileManager` / rate limiter | **não** | Surreal **v2**; v1.5 não precisa |
| Titan / UDT / per-CF block cache | no-op | se o host *depende*, é defeito. TiKV depende. Surreal 1.5 não. |

v1 do substituto = rust-rocksdb superfície que **Surreal 1.5 + KV típico** compilam. v2 = TransactionDB + Env + Titan. Dizer “100% C++ Rocks” é mentira; dizer “100% da superfície 0.21 que o shim já prova” é a barra fechável.

### 4.3 Knobs no-op (S5) — defeitos se o host ficou mais lento ou inseguro

- `set_block_cache` no-op: Rocks ficaria mais rápido em working set > cache de página. Nós temos AnswerCache (resposta, não bloco). **Teste:** `ycsb_c_big` já é 2.46× sem block cache — hoje **não** é defeito de velocidade. Continua Inert até um host perder.
- `set_enable_pipelined_write` / concurrent memtable: RFC-0055 — 1c oficial não paga. Host 50-writer: **defeito potencial**. Medir mc50 no Linux; se <1× vs Rocks com o knob, implementar ou admitir v2.
- `set_verify_checksums(false)` no-op que *mantém* CRC = **vantagem** (mais seguro, não mais lento de forma material). Não NotSupported se o host só chama e segue. Não desligar CRC nunca.
- `set_checksum_type` no-op: Rocks XXH3 vs nosso CRC32C. Inobservável no KV. Inert ok.

### 4.4 Semântica mais segura (S3) — vantagens, desde que S4 não caia

| divergência | host vê | ok? |
|---|---|---|
| `delete_file_in_range` tombstone+compact, não unlink | keys sumiram; espaço depois do compact | ok se compact corre |
| skip-any ausente | open recusa em vez de adivinhar | vantagem |
| fence incondicional | handle para até reopen | mais estrito; `resume()` existe (RFC-0047) |
| prefix CFs | `list_cf`/scan isolado funciona; `live_files` não são ficheiros Rocks | ok para KV; falha se P parseia `*.sst` por CF no filesystem |

### 4.5 O que eu meti no relatório anterior e **não** é defeito do compat

TLS default, encrypt-at-rest, seL4, Montanha GA, Intel, RFC-0038 no kernel FailClosed (compat já é PIT). Desculpa. Fora deste documento.

---

## 5. Proposta (o que executar)

Ordem que satisfaz “>1× sempre no Linux” **e** “substituto só com vantagem”:

1. **Remesura Linux coluna A com `pwrite`**, 3 rounds, `rm` entre shapes, no caixote 4 vCPU AMD que **é** o Linux que temos. Gate: `min_ratio > 1.0` em **todas** as oficiais (raftlog incluso). Metal EPYC se o 4 vCPU passar no min, para “todos os Linux” que já medimos.
2. Se raftlog min≤1.0: um corte no p99 (encode / 16 inserts), **número antes de teoria**. Não Intel. Não 2×.
3. `kvrocks_blob_set` de volta ao gate >1× (nunca defeito).
4. Coluna **B** Linux: `set_sync(true)` vs Rocks `sync=true`, mesmo gate min>1.0. É o full-sync *de verdade*. A tabela 0.001× não se cita como “somos ruins”.
5. `Checkpoint` + `BackupEngine` com os nomes rust-rocksdb, wrapping o que já existe — S1 para hosts 0.22.
6. Inventário S5: cada `set_*` Wired / Inert-provado / v2. CRC nunca desliga.
7. v2 (não bloqueia v1): TransactionDB, Env, Titan, CFs físicos — só se um host nomeado não compilar.

**Pronto para ser o substituto** quando 1–6 verdes. Até lá: Surreal 1.5.4 já substitui na prática com 4/4 >1×; o cartaz “100% / nunca defeito / todos os Linux” ainda mente no raftlog min<1.0 e nos tipos em falta.
