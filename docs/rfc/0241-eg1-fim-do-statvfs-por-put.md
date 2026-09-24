# RFC-0241 — EG1: fim do statvfs por put no caminho 1c (cache de veredito Ok no admit de disco)

**Status:** in-progress (P0 landado `851fb714`; P1.1 DIAG fechado; P1.2 onda
Linux :p243 em voo desde 16:46Z)
**Updated:** 2026-09-22
**ID:** 0241
**Parents:** [TRAJETORIA](../TRAJETORIA.md)
(EG1 — velocidade; 54%: A4/A5/C2 compartilham o dono caminho de escrita
per-op),
[0240](0240-eg1-remeters-desconfundados-a8-a4.md)
(P1.2 DIAG: sink do WAL refutado por A/B; o custo "wal" não era o sink)

## Contexto e tese

O profile Darwin (sample 15 s mid-run, binário HEAD 8cd7044c, forma
`fjall_seq_1m` 60M ops — DIAG, nunca prova) decompõe a thread principal do
put compat:

| componente | % das amostras da main |
|---|---|
| `pwrite` (drain do WAL — fallback de sink no Darwin; no Linux é store mmap RFC-0233) | 38,6% |
| **`statvfs`/`statfs` via `probe_available_bytes`** (`ensure_disk_pressure_admitted` no `encode_async_one`) | **22,6%** |
| `RawRwLock::lock_shared_slow` (put espera flush/compact segurarem o write lock) | 12,6% |
| `fcntl` (prealloc de chunk 64 MiB do WAL — ~1×/chunk, barato) | 2,4% |
| `MemTable::shard_insert` (o "custo de mem" do DIAG p241) | 0,8% |

Em HEAD, `PEDRA_DISK_PROBE_CACHE_MS` default **0** desliga o cache de
veredito: **todo put 1c paga um `statvfs`** (~0,8 µs no Darwin; syscalls
comparáveis no Linux virtio). Workloads de overwrite (A4
`deps_cache_overwrite_mc4`, A5 RMW, kvrocks SET) não dão flush — nada
aquece o veredito — e a sonda cai em toda op. Fjall/RocksDB não sondam o
filesystem por insert; o custo é exclusivo do Pedra e mora no meio do
bucket "wal"/"prepare" das fase-stats p241 (que rodaram com flushes <1 s
no fill de 1M chaves novas — por isso o prepare mediu 0,057 µs lá e o
custo só apareceu no profile de overwrite).

**Tese:** reusar veredito `Ok` por 1000 ms (knob existente
`PEDRA_DISK_PROBE_CACHE_MS`, semântica RFC-0217 P2.4: só `Ok` é cacheado;
soft band e refuse sondam a cada commit; `=0` restaura o modo estrito)
remove o syscall por op do caminho quente de TODAS as formas de escrita.
Corte de uma linha de default + ajuste do teste de piso (que contava com o
default 0) + teste nomeado.

## P0 — corte

- [x] **P0.1** `db_open_kernel.rs`: `unwrap_or(0)` → `unwrap_or(1000)` no
  `disk_probe_cache_ms` (comentário datado RFC-0241).
  — status: `landado (851fb714)`
- [x] **P0.2** `db_kernel.rs`: teste de piso
  `put_under_hard_floor_is_disk_pressure_get_ok` passa a fixar
  `disk_probe_cache_ms = 0` no degrau de enchimento súbito (a janela de
  1 s mascararia o refuse — mesmo ajuste que o semântica do knob exige).
  — status: `landado (851fb714)`
- [x] **P0.3** teste nomeado `rfc0241_disk_probe_default_caches_ok_not_per_put`
  (SpaceEnv com contador de sondas): 50 puts com default ⇒ 1–2 sondas;
  `cache_ms = 0` ⇒ +3 sondas em 3 puts (opt-out preservado).
  — status: `verde no suite completo (Darwin dev, 8 threads; conjunto de
  falhas idêntico ao HEAD — os vermelhos Darwin pré-existentes)`

## P1 — verificação

- [x] **P1.1** DIAG Darwin A/B: mesma janela 60M ops pós-corte —
  **`statvfs` = 0 amostras** (era 22,6%). A janela mediu 79 270 → 140 880
  qps, mas a máquina estava sob carga externa (load 37–97 logo depois; a
  forma de fill 1M variou 14k–56k no mesmo binário) — **a magnitude é
  DIAG de direção; o sinal limpo é o profile**. A onda Linux (P1.2) mede
  a magnitude real, com sink mmap e box quieto.
  — status: `fechado (findings/2026-09-22-rfc0241-statvfs-per-put-diag/)`
- [ ] **P1.2** Onda Linux pagante (quando a API do gate box voltar):
  re-meter A4 assíncrono (`deps_cache_overwrite_mc4` 25M mc4 — a forma
  mais exposta: overwrite puro, zero flush para aquecer o cache) + C2
  (`fjall_seq_1m`, régua absoluta fjall re-baselined no mesmo dia) + A5.
  3 rodadas, canary ≥165000/rodada, peer `sync:false`, sem env de
  instrumentação. Paga ⇒ fatia fecha com finding datado; não paga ⇒
  número novo datado + próximo dono (o profile aponta o próximo:
  contenção read-lock vs flush no 12,6%).
  — status: `em voo (imagem :p243, digest sha256:83e0bb5b…, container
  cnt_a864f9ebb51943f78d3a44fa6f47a540 desde 16:46Z)`. O 502 era o host
  recusando exec com "Container must be running"; restart ficou preso em
  `starting` e o redeploy recriou `/data2` (rodadas p242 r1/r2 perdidas).
  Exec funciona via `/bin/sh` no initramfs, volume em `/mnt/vol-data2`.
  Canary r1 = 146 231 (abaixo do piso 165 000 — rodada provavelmente
  inválida, box acabado de subir). A8 r1 compat semeando 100M.

## Regra para refactors em voo

Qualquer variante `&self` do admit de disco (tipo a que aparece em WIPs de
reestruturação do commit path) **precisa gravar o veredito `Ok`** com
mutabilidade interior (atomic de ns-desde-época), senão o corte morre no
rebase: `probe_cached` só acerta se alguém escreveu `disk_ok_probe_at`. O
teste nomeado P0.3 pinna a propriedade pela contagem de sondas, não pela
estrutura — ele vale para qualquer caminho que admita put.

## Protocolo do meter

Igual ao 0240 (A8/A4/C2): binário musl da árvore landed, 3 rodadas
alternadas, canary `deps_cache_overwrite_mc4` ≥165000 (recalibrado
2026-09-22), quiet load1<2 ×2, sem `PEDRA_WRITE_PHASE_STATS`/
`PEDRA_WRITE_SPIN`, JSON por run em arquivo. C2 é régua absoluta contra
fjall no mesmo host/protocolo/dia.

## Status (living)

| slice | status | evidência |
|---|---|---|
| P0.1 default 1000 ms | landado 851fb714 | db_open_kernel.rs |
| P0.2 ajuste teste de piso | landado 851fb714 | db_kernel.rs |
| P0.3 teste nomeado | verde no suite (Darwin) | `rfc0241_disk_probe_default_caches_ok_not_per_put` |
| P1.1 DIAG A/B Darwin | fechado: statvfs 22,6% → 0%; janela 79 270 → 140 880 qps (+77,7%) | findings/2026-09-22-rfc0241-statvfs-per-put-diag/ |
| P1.2 onda Linux A8/A4/C2/A5 | em voo :p243 desde 16:46Z; canary r1 146231 (sub-piso) | container cnt_a864f9eb… |

## Repensamento datado

- **2026-09-22:** o 0240 P1.2 perseguiu o bucket "wal" como sink
  (refutado por A/B) e "mem" como BTree (0,8% no profile — morto). O
  profile mudou o alvo: o custo escondido era um **syscall de sonda de
  disco por put**, invisível nas fase-stats p241 porque o fill daquela
  forma dava flushes frequentes. Lição de método registrada: decomposição
  por fase não vê custos que o protocolo de medida aquece; profile de
  função numa forma que NÃO aquece (overwrite) expõe. Próximo dono se o
  corte não pagar no Linux: contenção read-lock vs flush (12,6%).
