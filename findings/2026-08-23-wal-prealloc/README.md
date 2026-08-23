# WAL preallocation — fix do stall de extent APFS (8 MiB) no caminho de commit

**Status: landed na branch `wal-prealloc` (base d343044); bateria oficial
rearm8 gated aguardando caixa quieta.**

## Causa-raiz (números em `findings/2026-08-22-rearm7/`)

- `deps_raftlog` rearm7: **0,35×** (43,8 k vs 124,6 k) com **p50 7,5–7,7 µs
  = paridade com rocks (6,9–8,5)** — o gap inteiro era cauda: um stall de
  20–38 ms por perna na fase `wal`.
- Probe per-batch (`raftlog_tail_probe`): stalls de 7–33 ms em offsets
  **fixos** (a cada ~7,1 MiB de WAL; batches 3711/7423/11106/14818/18501),
  100% na fase wal, residual ~0.
- Microbench C (`apfs_append.c`, incluído abaixo): appends plain de 64 KiB
  travam **9–54 ms exatamente a cada 8 MiB** (writes #129/#257/#385/#513/#641,
  2 runs); com `fcntl(F_PREALLOCATE, F_ALLOCATEALL)` os stalls de fronteira
  caem para ~1,1 ms e não há nenhum >5 ms — é o APFS alocando um novo extent
  bloqueando o `write(2)` que estende o arquivo. RocksDB pré-aloca o WAL
  (`PosixWritableFile` prealloc) — o peer nunca mostra isso (max 0,04–4 ms).

## Fix

- `pedradb_posix::preallocate_file` — macOS `fcntl(F_PREALLOCATE,
  F_ALLOCATEALL | F_PEOFPOSMODE)`; no-op em outras plataformas (ext4/xfs não
  travam append assim; revisitar se uma caixa Linux medir cauda parecida).
- `EnvFile::preallocate(len)` — default no-op (sim/DST); `File` delega ao
  posix. Best-effort: falha = degrada para append plain.
- `Wal::reserve_space` — reserva `WAL_PREALLOC_CHUNK = 8 MiB` à frente do
  ponto de append, re-reserva ao cruzar a fronteira; ancorada na posição
  real (segmento recuperado nunca super-reserva). Chamado nos 3 sinks de
  escrita (`write_pending_frame_if`, `append_record`, `append_write_ops`/
  `append_records`).
- `WalWriter::position()` — rastreamento de posição **em memória** (âncora
  na construção, +bytes a cada write com sucesso). A 1ª versão consultava
  `stream_position()` no meio do commit — isso chama `flush()`, que o
  `FailingEnv` classifica como `OpClass::Write` e **consumia a falha
  one-shot injetada** (2 testes adversariais ficaram verdes→vermelhos;
  detectado antes do land). Sem I/O agora: sem syscall, sem efeito colateral.

Semântica: a reserva **não muda o tamanho lógico** (testado:
`prealloc_keeps_logical_size_and_recovers` — `metadata_len == append point`,
recover_span vê exatamente os registros). Leitores nunca observam a região
reservada. Custo: fcntl de ~0,5–1,5 ms a cada 8 MiB (amortizado ~0,4 µs por
batch raftlog); disco: ≤1 chunk extra por segmento vivo, devolvido no GC.

## Efeito (probe, mesmo box, intercalado)

`raftlog_tail_probe` 20k×16 (mesma caixa, minutos de distância):

| build | max | top stalls |
|---|---:|---|
| d343044 (sem fix) | 26,8–32,6 ms | 22,9–32,6 ms nos offsets fixos (fase wal) |
| wal-prealloc | 0,8–2,1 ms | 0,5–1,9 ms (custo do próprio fcntl) |

p50 inalterado (4,0–4,1 µs). Sob caixa suja (load ≥10, caixote-api+FSEvents
+miri concorrentes) o próprio fcntl de reserva pode travar (1 run: 21 ms em
offset não-determinístico) — a bateria oficial é gated e decide.

Bench A/B raftlog-only 6 rounds intercalados (caixa SUJA load ~10-12, NON-
OFFICIAL, ruído domina — old max 0,9–5,3 / new max 2,9–35,5, qps
inconclusivo pareado; gravado em `ab-bench-dirty.txt`).

## Projeção

rearm7: wall 45,6 ms = p50 7,6 µs × 2000 (15,2 ms) + stall (~26 ms) + caudas
menores. Sem o stall de extent: wall ≈ 17–20 ms ⇒ **~100–118 k qps ≈
0,85–0,95× rocks** (p50 em paridade; restante = p95/p99 ~20/63 µs de writes
64 KiB sob disco sujo vs rocks 10–17/12–20 µs). ycsb_a r1 (0,43 artifact com
max 3,1 ms) deve sumir. 2× (RFC-0041 P1.3) exige ainda cortar o p50 (~7,5 →
~3,8 µs) — candidatos: run-suffix (patch preservado em
`findings/2026-08-23-runsuffix-wip/`, −74% no piso de insert), fold
deep-clone, submit-path.

## Artefatos

- `probe-before.txt` / `probe-after.txt` — saídas do `raftlog_tail_probe`.
- `apfs_append.c` — microbench (plain vs F_PREALLOCATE).
- `ab-bench-dirty.txt` — A/B bench caixa suja (honesto: inconclusivo).
- Bateria oficial: `findings/2026-08-23-rearm8/` (gated).

## A/B suite-context (caixa suja load ~10-12, NON-OFFICIAL) — `ab-suite-dirty.txt`

3 rounds intercalados, suíte completa ycsb,deps: `deps_raftlog` mediana
old 72,6 k → new 80,5 k (**+10,9%**, new vence os 3 rounds), max_ms
4,3–10,5 → 4,3–5,0 ms; ycsb_a max 14–25 → 11,6–12,6 µs (artifact r1 da
rearm7 some); demais shapes no ruído (apply/cache ~0,95–1,06). Na caixa
QUIETA da rearm7 (stalls 20–38 ms/round) a diferença projetada é maior.
